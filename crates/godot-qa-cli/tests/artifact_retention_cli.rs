//! `artifacts.keep_runs` on the single `scenario run` path, and `godot-qa artifact prune`.
//!
//! Bundle retention is covered in `scenario_bundle_cli.rs`; this file pins the CLI-layer
//! prune that runs after one scenario, including the default of keeping everything, plus the
//! on-demand whole-root sweep that `artifact prune` performs.

use serde_json::{Value, json};
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

fn unique_suffix() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or(0);
    format!("{nanos}-{}", std::process::id())
}

fn write(path: &Path, contents: &str) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(path, contents).unwrap();
}

fn write_fake_godot(path: &Path) {
    godot_qa_test_support::fixture(
        path,
        &[
            ("request_prefix", "--qa-run-request="),
            ("scenario_proof", "passed"),
            (
                "write:result.json",
                r#"{"ok":true,"command":"scenario run","message":"scenario run completed","data":{"runtime":"fake"},"failures":[]}"#,
            ),
        ],
    );
}

fn make_scenario_project(prefix: &str, config_yaml: Option<&str>) -> PathBuf {
    let root = env::temp_dir().join(format!("godot-qa-{prefix}-{}", unique_suffix()));
    fs::create_dir_all(&root).unwrap();
    fs::write(root.join("project.godot"), "[application]\n").unwrap();
    // `scenario run` preflights the installed addon; the fake Godot never reads it.
    write(
        &root.join("addons/godot_qa/plugin.cfg"),
        &format!("[plugin]\nversion=\"{}\"\n", env!("CARGO_PKG_VERSION")),
    );
    write_fake_godot(&root.join("fake-godot"));
    if let Some(config_yaml) = config_yaml {
        write(&root.join("qa/config.yaml"), config_yaml);
    }
    write(
        &root.join("qa/scenarios/retention_demo.json"),
        r#"{"name":"retention_demo","scene":"res://Retention.tscn","steps":[]}"#,
    );
    root
}

fn run_scenario(project_root: &Path) -> (std::process::ExitStatus, Value) {
    let output = Command::new(env!("CARGO_BIN_EXE_godot-qa"))
        .arg("--json")
        .arg("--project")
        .arg(project_root)
        .arg("--godot-bin")
        .arg(project_root.join(format!("fake-godot{}", env::consts::EXE_SUFFIX)))
        .arg("scenario")
        .arg("run")
        .arg("retention_demo")
        .output()
        .unwrap();
    let body = serde_json::from_slice(&output.stdout).unwrap_or_else(|error| {
        panic!(
            "invalid stdout JSON: {error}\nstdout: {}\nstderr: {}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        )
    });
    (output.status, body)
}

fn run_dir_count(dir: &Path) -> usize {
    let Ok(entries) = fs::read_dir(dir) else {
        return 0;
    };
    entries
        .flatten()
        .filter(|entry| entry.metadata().map(|meta| meta.is_dir()).unwrap_or(false))
        .count()
}

#[test]
fn scenario_run_prunes_its_own_scenario_when_keep_runs_is_set() {
    let project_root = make_scenario_project(
        "scenario-run-retention",
        Some("artifacts:\n  keep_runs: 2\n"),
    );
    let scenario_dir = project_root.join(".qa/artifacts/retention_demo");

    let (first_status, first_body) = run_scenario(&project_root);
    assert!(first_status.success(), "{first_body:#?}");
    assert_eq!(first_body["data"]["retention"]["keep_runs"], 2);
    assert_eq!(
        first_body["data"]["retention"]["pruned"]["scenario_runs"],
        0
    );
    assert_eq!(first_body["data"]["retention"]["errors"], json!([]));

    let (second_status, _) = run_scenario(&project_root);
    assert!(second_status.success());
    assert_eq!(run_dir_count(&scenario_dir), 2);

    // The third run pushes the oldest out of the window.
    let (third_status, third_body) = run_scenario(&project_root);
    assert!(third_status.success());
    assert_eq!(
        third_body["data"]["retention"]["pruned"]["scenario_runs"],
        1
    );
    assert_eq!(run_dir_count(&scenario_dir), 2);
    // Retention is housekeeping: it never changes the verdict or the exit code.
    assert_eq!(third_body["ok"], true);
    let run_root = third_body["data"]["artifacts"]["run_root"]
        .as_str()
        .unwrap();
    assert!(Path::new(run_root).is_dir(), "the newest run must survive");

    fs::remove_dir_all(project_root).unwrap();
}

#[test]
fn scenario_run_keeps_every_run_without_keep_runs() {
    let project_root = make_scenario_project("scenario-run-no-retention", None);
    let scenario_dir = project_root.join(".qa/artifacts/retention_demo");

    for _ in 0..3 {
        let (status, body) = run_scenario(&project_root);
        assert!(status.success(), "{body:#?}");
        // Unset `artifacts.keep_runs` emits no `retention` key at all.
        assert!(body["data"]["retention"].is_null());
    }

    assert_eq!(run_dir_count(&scenario_dir), 3);

    fs::remove_dir_all(project_root).unwrap();
}

// ---------------------------------------------------------------------------
// `godot-qa artifact prune`
// ---------------------------------------------------------------------------

/// A project whose `.qa/artifacts/` already holds history, built directly on disk: `artifact
/// prune` never launches Godot, so nothing here needs a fake binary or a scenario file.
fn make_prune_project(prefix: &str, config_yaml: Option<&str>) -> PathBuf {
    let root = env::temp_dir().join(format!("godot-qa-{prefix}-{}", unique_suffix()));
    fs::create_dir_all(&root).unwrap();
    fs::write(root.join("project.godot"), "[application]\n").unwrap();
    if let Some(config_yaml) = config_yaml {
        write(&root.join("qa/config.yaml"), config_yaml);
    }
    let artifacts = root.join(".qa/artifacts");
    for run_id in ["1000Z", "1001Z", "1002Z"] {
        write(
            &artifacts.join("main_menu").join(run_id).join("logs.txt"),
            "x",
        );
    }
    // Not a run id: retention must leave it exactly where it is.
    write(&artifacts.join("main_menu/baselines/menu.png"), "png");
    for run_id in ["2000Z", "2001Z"] {
        write(
            &artifacts
                .join("gameplay_flow")
                .join(run_id)
                .join("logs.txt"),
            "x",
        );
    }
    for run_id in ["9000", "9100"] {
        write(
            &artifacts
                .join("bundles/regression_gate")
                .join(run_id)
                .join("summary.json"),
            "{}",
        );
    }
    // `9100 - 3600` is the session cutoff, so this one is far outside it.
    write(&artifacts.join("live/session-1000000-aaaa/logs.txt"), "x");
    // Inside the window: 6000s is newer than the 5500s cutoff.
    write(&artifacts.join("live/session-6000000-bbbb/logs.txt"), "x");
    root
}

fn run_prune(project_root: &Path, extra_args: &[&str]) -> (std::process::ExitStatus, Value) {
    let output = Command::new(env!("CARGO_BIN_EXE_godot-qa"))
        .arg("--json")
        .arg("--project")
        .arg(project_root)
        .args(["artifact", "prune"])
        .args(extra_args)
        .output()
        .unwrap();
    let body = serde_json::from_slice(&output.stdout).unwrap_or_else(|error| {
        panic!(
            "invalid stdout JSON: {error}\nstdout: {}\nstderr: {}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        )
    });
    (output.status, body)
}

fn names(dir: &Path) -> Vec<String> {
    let mut entries: Vec<String> = fs::read_dir(dir)
        .unwrap()
        .flatten()
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .collect();
    entries.sort();
    entries
}

#[test]
fn artifact_prune_dry_run_reports_the_plan_and_deletes_nothing() {
    let project_root = make_prune_project("prune-dry-run", Some("artifacts:\n  keep_runs: 1\n"));
    let artifacts = project_root.join(".qa/artifacts");

    let (status, body) = run_prune(&project_root, &["--dry-run"]);

    assert!(status.success(), "{body:#?}");
    let retention = &body["data"]["retention"];
    assert_eq!(retention["keep_runs"], 1);
    assert_eq!(retention["dry_run"], true);
    assert_eq!(retention["would_remove"]["scenario_runs"], 3);
    assert_eq!(retention["would_remove"]["bundle_runs"], 1);
    assert_eq!(retention["would_remove"]["sessions"], 1);
    assert_eq!(retention["errors"], json!([]));
    // A dry run still prices the plan so an operator can decide whether it is worth it.
    assert!(retention["bytes_reclaimed"].as_u64().unwrap() > 0);
    // The audit list is exhaustive, not a sample.
    let would_remove: Vec<&str> = retention["would_remove_paths"]
        .as_array()
        .unwrap()
        .iter()
        .map(|path| path.as_str().unwrap())
        .collect();
    assert_eq!(would_remove.len(), 5);
    let oldest_menu_run = artifacts.join("main_menu").join("1000Z");
    assert!(
        would_remove
            .iter()
            .any(|path| Path::new(path) == oldest_menu_run.as_path()),
        "expected {} in {would_remove:?}",
        oldest_menu_run.display()
    );
    // A dry run must not carry the real-prune keys at all.
    assert!(retention["pruned"].is_null());
    assert!(retention["removed_paths"].is_null());

    // Nothing moved on disk.
    assert_eq!(
        names(&artifacts.join("main_menu")),
        vec!["1000Z", "1001Z", "1002Z", "baselines"]
    );
    assert_eq!(names(&artifacts.join("bundles/regression_gate")).len(), 2);
    assert_eq!(names(&artifacts.join("live")).len(), 2);

    fs::remove_dir_all(project_root).unwrap();
}

#[test]
fn artifact_prune_keeps_the_newest_runs_per_scenario_and_bundle() {
    let project_root = make_prune_project("prune-real", Some("artifacts:\n  keep_runs: 1\n"));
    let artifacts = project_root.join(".qa/artifacts");

    let (status, body) = run_prune(&project_root, &[]);

    assert!(status.success(), "{body:#?}");
    let retention = &body["data"]["retention"];
    assert_eq!(retention["dry_run"], false);
    assert_eq!(retention["pruned"]["scenario_runs"], 3);
    assert_eq!(retention["pruned"]["bundle_runs"], 1);
    assert_eq!(retention["pruned"]["sessions"], 1);
    assert_eq!(retention["errors"], json!([]));
    assert_eq!(retention["removed_paths"].as_array().unwrap().len(), 5);
    assert!(retention["would_remove"].is_null());

    // The newest run survives everywhere, and the non-run entry is untouched.
    assert_eq!(
        names(&artifacts.join("main_menu")),
        vec!["1002Z", "baselines"]
    );
    assert_eq!(names(&artifacts.join("gameplay_flow")), vec!["2001Z"]);
    assert_eq!(
        names(&artifacts.join("bundles/regression_gate")),
        vec!["9100"]
    );
    assert_eq!(names(&artifacts.join("live")), vec!["session-6000000-bbbb"]);

    // Rerunning it is a clean no-op rather than an error.
    let (again_status, again_body) = run_prune(&project_root, &[]);
    assert!(again_status.success(), "{again_body:#?}");
    assert_eq!(
        again_body["data"]["retention"]["pruned"]["scenario_runs"],
        0
    );
    assert_eq!(again_body["data"]["retention"]["bytes_reclaimed"], 0);

    fs::remove_dir_all(project_root).unwrap();
}

#[cfg(unix)]
#[test]
fn artifact_prune_never_touches_a_latest_link_or_a_running_session() {
    let project_root = make_prune_project("prune-live", Some("artifacts:\n  keep_runs: 1\n"));
    let artifacts = project_root.join(".qa/artifacts");
    let bundle_dir = artifacts.join("bundles/regression_gate");
    // The convenience link a bundle run maintains: relative, and never a prune candidate.
    std::os::unix::fs::symlink("9100", bundle_dir.join("latest")).unwrap();
    let live = artifacts.join("live");
    // An old session pointed at by `latest`, and an equally old one owned by this test
    // process. Both are inside the age cutoff and both must survive.
    write(&live.join("session-1000000-cccc/logs.txt"), "x");
    std::os::unix::fs::symlink(live.join("session-1000000-cccc"), live.join("latest")).unwrap();
    write(&live.join("session-1000000-dddd/logs.txt"), "x");
    write(
        &live.join("session-1000000-dddd/session.json"),
        &json!({"pid": std::process::id()}).to_string(),
    );

    let (status, body) = run_prune(&project_root, &[]);

    assert!(status.success(), "{body:#?}");
    // Only the ownerless, unlinked, old session goes.
    assert_eq!(body["data"]["retention"]["pruned"]["sessions"], 1);
    assert_eq!(
        names(&live),
        vec![
            "latest",
            "session-1000000-cccc",
            "session-1000000-dddd",
            "session-6000000-bbbb",
        ]
    );
    assert_eq!(
        fs::read_link(live.join("latest")).unwrap(),
        live.join("session-1000000-cccc")
    );
    // The bundle link survives the bundle prune and still resolves to the retained run.
    assert_eq!(names(&bundle_dir), vec!["9100", "latest"]);
    assert_eq!(
        fs::read_link(bundle_dir.join("latest")).unwrap(),
        PathBuf::from("9100")
    );
    assert!(bundle_dir.join("latest").is_dir());

    fs::remove_dir_all(project_root).unwrap();
}

#[test]
fn artifact_prune_keep_runs_flag_overrides_the_configured_policy() {
    let project_root = make_prune_project("prune-override", Some("artifacts:\n  keep_runs: 1\n"));
    let artifacts = project_root.join(".qa/artifacts");

    let (status, body) = run_prune(&project_root, &["--keep-runs", "2"]);

    assert!(status.success(), "{body:#?}");
    assert_eq!(body["data"]["retention"]["keep_runs"], 2);
    // `keep_runs: 1` would have left one run per scenario; the flag wins.
    assert_eq!(body["data"]["retention"]["pruned"]["scenario_runs"], 1);
    assert_eq!(body["data"]["retention"]["pruned"]["bundle_runs"], 0);
    assert_eq!(
        names(&artifacts.join("main_menu")),
        vec!["1001Z", "1002Z", "baselines"]
    );
    assert_eq!(names(&artifacts.join("gameplay_flow")).len(), 2);

    fs::remove_dir_all(project_root).unwrap();
}

#[test]
fn artifact_prune_without_a_retention_policy_exits_two() {
    let project_root = make_prune_project("prune-unconfigured", None);
    let artifacts = project_root.join(".qa/artifacts");

    let (status, body) = run_prune(&project_root, &[]);

    assert_eq!(status.code(), Some(2), "{body:#?}");
    assert_eq!(body["ok"], false);
    let failure = &body["failures"][0];
    assert_eq!(failure["type"], "retention_not_configured");
    assert_eq!(failure["command"], "artifact prune");
    assert!(
        failure["path"]
            .as_str()
            .unwrap()
            .replace('\\', "/")
            .ends_with("qa/config.yaml"),
        "{failure:#?}"
    );
    assert_eq!(
        failure["remediation"],
        "set artifacts.keep_runs in qa/config.yaml or pass --keep-runs"
    );
    // Refusing to guess means refusing to delete.
    assert_eq!(names(&artifacts.join("main_menu")).len(), 4);

    // The flag alone is enough; no config file is required.
    let (flag_status, flag_body) = run_prune(&project_root, &["--keep-runs", "1", "--dry-run"]);
    assert!(flag_status.success(), "{flag_body:#?}");
    assert_eq!(flag_body["data"]["retention"]["keep_runs"], 1);

    fs::remove_dir_all(project_root).unwrap();
}

#[test]
fn artifact_prune_rejects_keep_runs_zero() {
    let project_root = make_prune_project("prune-zero", Some("artifacts:\n  keep_runs: 2\n"));
    let artifacts = project_root.join(".qa/artifacts");

    let (status, body) = run_prune(&project_root, &["--keep-runs", "0"]);

    assert_eq!(status.code(), Some(2), "{body:#?}");
    assert_eq!(body["ok"], false);
    assert_eq!(body["failures"][0]["type"], "invalid_arguments");
    assert_eq!(body["failures"][0]["actual"], "0");
    // "Keep nothing" must never fall through to the configured policy either.
    assert_eq!(names(&artifacts.join("main_menu")).len(), 4);

    fs::remove_dir_all(project_root).unwrap();
}

#[test]
fn artifact_prune_separates_a_missing_artifacts_root_from_an_unreadable_one() {
    let project_root = make_prune_project("prune-root", Some("artifacts:\n  keep_runs: 1\n"));

    // A project that has never produced artifacts has nothing to prune, not a failure.
    let (missing_status, missing_body) = run_prune(&project_root, &["--artifact-dir", "never-ran"]);
    assert!(missing_status.success(), "{missing_body:#?}");
    assert_eq!(
        missing_body["data"]["retention"]["pruned"]["scenario_runs"],
        0
    );
    assert_eq!(missing_body["data"]["retention"]["errors"], json!([]));

    // A file where the artifacts root should be is actionable, and names the path.
    write(
        &project_root.join("not-a-dir"),
        "definitely not a directory",
    );
    let (bad_status, bad_body) = run_prune(&project_root, &["--artifact-dir", "not-a-dir"]);
    assert_eq!(bad_status.code(), Some(1), "{bad_body:#?}");
    assert_eq!(bad_body["ok"], false);
    let failure = &bad_body["failures"][0];
    assert_eq!(failure["type"], "artifact_prune_failed");
    assert!(
        failure["path"].as_str().unwrap().ends_with("not-a-dir"),
        "{failure:#?}"
    );
    assert!(failure["actual"].as_str().unwrap().contains("not-a-dir"));

    fs::remove_dir_all(project_root).unwrap();
}
