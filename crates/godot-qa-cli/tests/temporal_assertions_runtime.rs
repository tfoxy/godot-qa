use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

fn copy_addon(source: &Path, destination: &Path) {
    fs::create_dir_all(destination).unwrap();
    for entry in fs::read_dir(source).unwrap() {
        let entry = entry.unwrap();
        if entry.file_type().unwrap().is_dir() {
            copy_addon(&entry.path(), &destination.join(entry.file_name()));
        } else {
            fs::copy(entry.path(), destination.join(entry.file_name())).unwrap();
        }
    }
}

struct Scratch(PathBuf);
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn run_regression(case: &str) {
    let repo = Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .unwrap();
    let scratch = Scratch(
        repo.join(".local")
            .join(format!("temporal-{}-{case}", std::process::id())),
    );
    copy_addon(
        &repo.join("addons/godot_qa"),
        &scratch.0.join("addons/godot_qa"),
    );
    fs::write(scratch.0.join("project.godot"), "config_version=5\n").unwrap();
    fs::write(
        scratch.0.join("test.gd"),
        include_str!("fixtures/temporal_assertions.gd"),
    )
    .unwrap();
    let mut command = Command::new(std::env::var("GODOT_BIN").unwrap_or_else(|_| "godot".into()));
    command
        .args(["--headless", "--quit-after", "600", "--path"])
        .arg(&scratch.0)
        .args(["--script", "res://test.gd", "--", case]);
    #[cfg(target_os = "linux")]
    if case == "evidence_write_failure" {
        use std::os::unix::process::CommandExt;
        // Limit only this Godot child. Its large evidence write must fail, while
        // the small regression report still fits. Ignore SIGXFSZ to inspect EFBIG.
        unsafe {
            command.pre_exec(|| {
                libc::signal(libc::SIGXFSZ, libc::SIG_IGN);
                let limit = libc::rlimit {
                    rlim_cur: 32768,
                    rlim_max: 32768,
                };
                if libc::setrlimit(libc::RLIMIT_FSIZE, &limit) != 0 {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }
    }
    let output = command.output().unwrap();
    let diagnostics = format!(
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let report: Value = serde_json::from_slice(
        &fs::read(scratch.0.join("report.json")).unwrap_or_else(|error| {
            panic!("{case}: missing runtime report: {error}\n{diagnostics}")
        }),
    )
    .unwrap();
    assert!(output.status.success(), "{case}: {report}\n{diagnostics}");
    assert!(
        !String::from_utf8_lossy(&output.stderr).contains("SCRIPT ERROR"),
        "{diagnostics}"
    );
    if case == "non_finite_properties" {
        assert!(
            !String::from_utf8_lossy(&output.stderr).contains("ERROR:"),
            "{diagnostics}"
        );
        for field in ["serialized_results", "serialized_evidence"] {
            let documents = report[field].as_array().unwrap();
            assert_eq!(documents.len(), 5, "{report}");
            for document in documents {
                serde_json::from_str::<Value>(document.as_str().unwrap())
                    .unwrap_or_else(|error| panic!("{field}: invalid strict JSON: {error}"));
            }
        }
    }
    assert_eq!(
        report["failures"],
        serde_json::json!([]),
        "{case}: {report}"
    );
    assert!(report["checks"].as_array().unwrap().len() >= 2);
}

#[test]
fn within_limits_rejects_non_finite_observations() {
    run_regression("non_finite_observations");
}

#[test]
fn within_limits_rejects_non_finite_bounds() {
    run_regression("non_finite_bounds");
}

#[test]
fn frame_time_window_shorter_than_cadence() {
    run_regression("short_window");
}

#[test]
fn frame_time_remainder_uses_frame_weighted_aggregates() {
    run_regression("remainder_window");
}

#[test]
fn temporal_evidence_reports_directory_creation_failure() {
    run_regression("evidence_mkdir_failure");
}

#[test]
fn temporal_evidence_reports_open_failure() {
    run_regression("evidence_open_failure");
}

#[test]
#[cfg(target_os = "linux")]
fn temporal_evidence_reports_write_failure() {
    run_regression("evidence_write_failure");
}

#[test]
fn observe_until_native_properties_round_trip_through_json() {
    run_regression("native_properties");
}

#[test]
fn observe_until_non_finite_properties_round_trip_through_json() {
    run_regression("non_finite_properties");
}
