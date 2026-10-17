use serde_json::Value;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus};
use std::time::{SystemTime, UNIX_EPOCH};

fn unique_suffix() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or(0);
    format!("{nanos}-{}", std::process::id())
}

fn make_project_root(prefix: &str) -> PathBuf {
    let root = env::temp_dir().join(format!("godot-qa-{prefix}-{}", unique_suffix()));
    fs::create_dir_all(root.join("addons/godot_qa")).unwrap();
    fs::create_dir_all(root.join("tools")).unwrap();
    fs::write(root.join("project.godot"), "[application]\n").unwrap();
    fs::write(root.join("addons/godot_qa/plugin.cfg"), "[plugin]\n").unwrap();
    root
}

fn fake_godot(root: &Path) -> PathBuf {
    godot_qa_test_support::fixture(
        &root.join("tools/godot"),
        &[("stdout", "Godot_v4.3-stable\n")],
    )
}

fn run_check(project_root: &Path, godot_bin: &Path, check_args: &[&str]) -> (ExitStatus, Value) {
    let output = Command::new(env!("CARGO_BIN_EXE_godot-qa"))
        .arg("--json")
        .arg("--project")
        .arg(project_root)
        .arg("--godot-bin")
        .arg(godot_bin)
        .arg("check")
        .args(check_args)
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

#[test]
fn top_level_version_flag_succeeds() {
    let output = Command::new(env!("CARGO_BIN_EXE_godot-qa"))
        .arg("--version")
        .output()
        .unwrap();

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("godot-qa"));
    assert!(stdout.contains(env!("CARGO_PKG_VERSION")));
    assert!(output.stderr.is_empty());
}

#[test]
fn check_tests_reports_missing_manifest_without_scenarios_dir() {
    let project_root = make_project_root("check-tests-no-scenarios");
    let godot = fake_godot(&project_root);

    let (status, body) = run_check(&project_root, &godot, &["--tests"]);

    fs::remove_dir_all(project_root).unwrap();

    assert!(!status.success());
    assert_eq!(body["ok"], false);
    assert_eq!(body["command"], "check");
    assert_eq!(body["data"]["mode"], "tests");
    assert_eq!(body["failures"][0]["type"], "test_manifest_missing");
    let failure_types: Vec<&str> = body["failures"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|failure| failure["type"].as_str())
        .collect();
    assert!(!failure_types.contains(&"missing_scenarios_dir"));
}

#[test]
fn check_all_scenes_succeeds_with_empty_project_without_scenarios_dir() {
    let project_root = make_project_root("check-all-scenes-no-scenarios");
    let godot = fake_godot(&project_root);

    let (status, body) = run_check(&project_root, &godot, &["--all-scenes"]);

    fs::remove_dir_all(project_root).unwrap();

    assert!(status.success(), "{body}");
    assert_eq!(body["ok"], true);
    assert_eq!(body["command"], "check");
    assert_eq!(body["data"]["mode"], "all_scenes");
    assert_eq!(body["data"]["checked_scene_count"], 0);
    assert!(body["data"]["scenes"].as_array().unwrap().is_empty());
    assert!(body["failures"].as_array().unwrap().is_empty());
}
