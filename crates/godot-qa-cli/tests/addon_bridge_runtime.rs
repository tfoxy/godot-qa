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
            .join(format!("bridge-{}-{case}", std::process::id())),
    );
    copy_addon(
        &repo.join("addons/godot_qa"),
        &scratch.0.join("addons/godot_qa"),
    );
    fs::write(scratch.0.join("project.godot"), "config_version=5\n").unwrap();
    fs::write(
        scratch.0.join("test.gd"),
        include_str!("fixtures/addon_bridge.gd"),
    )
    .unwrap();
    fs::write(
        scratch.0.join("provider.gd"),
        include_str!("fixtures/bridge_provider.gd"),
    )
    .unwrap();
    let mut command = Command::new(std::env::var("GODOT_BIN").unwrap_or_else(|_| "godot".into()));
    command
        .args(["--headless", "--quit-after", "600", "--path"])
        .arg(&scratch.0);
    command.args(["--script", "res://test.gd", "--", case]);
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
    assert_eq!(
        report["failures"],
        serde_json::json!([]),
        "{case}: {report}"
    );
    assert!(report["checks"].as_array().unwrap().len() >= 2);
}

#[test]
fn live_temporal_artifacts_stay_in_session_root() {
    run_regression("temporal_paths");
}

#[test]
fn failed_scene_change_preserves_status_and_reload() {
    run_regression("invalid_scene_change");
}

#[test]
fn scene_reload_replaces_providers_before_ready() {
    run_regression("scene_providers");
}

#[test]
fn scene_reload_replaces_provider_with_freed_child_owner() {
    run_regression("scene_providers_freed_child");
}

#[test]
fn scene_reload_replaces_providers_after_unrelated_stale_owner() {
    run_regression("scene_providers_unrelated_stale_owner");
}

#[test]
fn live_command_runtime_object_counts_stabilize() {
    run_regression("runtime_cleanup");
}

#[test]
fn file_response_failure_retains_result_without_redispatch() {
    run_regression("file_publication");
}

#[test]
fn unauthenticated_connections_and_request_bytes_are_bounded() {
    run_regression("transport_limits");
}

#[test]
fn retained_logs_have_monotonic_cursors_and_truncation_metadata() {
    run_regression("log_retention");
}
