use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus};
use std::time::{SystemTime, UNIX_EPOCH};

fn project_root() -> PathBuf {
    let suffix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "godot-qa-cli-results-{suffix}-{}",
        std::process::id()
    ));
    fs::create_dir_all(root.join("qa")).unwrap();
    fs::write(root.join("project.godot"), "[application]\n").unwrap();
    root
}

fn run(project: &Path, godot: &Path, args: &[&str]) -> (ExitStatus, Value) {
    let output = Command::new(env!("CARGO_BIN_EXE_godot-qa"))
        .arg("--project")
        .arg(project)
        .arg("--godot-bin")
        .arg(godot)
        .arg("--json")
        .args(args)
        .output()
        .unwrap();
    let body = serde_json::from_slice(&output.stdout).unwrap_or_else(|error| {
        panic!(
            "invalid JSON: {error}\nstdout: {}\nstderr: {}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        )
    });
    (output.status, body)
}

#[test]
fn import_zero_exit_with_log_error_fails_unless_ignored() {
    for line in [
        "ERROR: fixture import failure",
        "SCRIPT ERROR: fixture import failure",
    ] {
        let project = project_root();
        let godot =
            godot_qa_test_support::fixture(&project.join("fake-godot"), &[("stderr", line)]);
        let (status, body) = run(&project, &godot, &["import"]);
        assert!(!status.success(), "{body:#}");
        assert_eq!(body["ok"], false);
        assert_eq!(body["data"]["process"]["exit_code"], 0);
        let failure = body["failures"]
            .as_array()
            .unwrap()
            .iter()
            .find(|failure| failure["type"] == "godot_log_error")
            .unwrap();
        assert_eq!(failure["command"], "import");
        assert_eq!(failure["actual"], line);
        assert_eq!(failure["path"], body["data"]["artifacts"]["logs"]);
        let saved: Value = serde_json::from_slice(
            &fs::read(body["data"]["artifacts"]["result"].as_str().unwrap()).unwrap(),
        )
        .unwrap();
        assert_eq!(saved["ok"], false);

        fs::write(
            project.join("qa/config.yaml"),
            "logs:\n  ignore: [fixture import failure]\n",
        )
        .unwrap();
        let (status, body) = run(&project, &godot, &["import"]);
        assert!(status.success(), "{body:#}");
        assert_eq!(body["ok"], true);
        assert!(body["failures"].as_array().unwrap().is_empty());
        fs::remove_dir_all(project).unwrap();
    }
}

fn check_project() -> PathBuf {
    let project = project_root();
    fs::create_dir_all(project.join("addons/godot_qa")).unwrap();
    fs::write(project.join("addons/godot_qa/plugin.cfg"), "[plugin]\n").unwrap();
    fs::write(project.join("probe.gd"), "extends Node\n").unwrap();
    fs::write(
        project.join("probe.tscn"),
        "[gd_scene format=3]\n[node name=\"Probe\" type=\"Node\"]\n",
    )
    .unwrap();
    project
}

#[test]
fn check_tests_overflowing_timeout_is_structured() {
    assert_overflowing_timeout(&["check", "--tests", "--timeout-seconds", "1e100"], 1e100);
}

#[test]
fn check_script_overflowing_timeout_is_structured() {
    assert_overflowing_timeout(
        &[
            "check",
            "--script",
            "res://probe.gd",
            "--timeout-seconds",
            "1e100",
        ],
        1e100,
    );
}

#[test]
fn check_tests_overflowing_deadline_is_structured() {
    assert_overflowing_timeout(&["check", "--tests", "--timeout-seconds", "1e19"], 1e19);
}

#[test]
fn check_script_overflowing_deadline_is_structured() {
    assert_overflowing_timeout(
        &[
            "check",
            "--script",
            "res://probe.gd",
            "--timeout-seconds",
            "1e19",
        ],
        1e19,
    );
}

fn assert_overflowing_timeout(args: &[&str], seconds: f64) {
    let project = check_project();
    fs::create_dir_all(project.join("qa/tests")).unwrap();
    fs::write(
        project.join("qa/tests/manifest.json"),
        r#"{"version":1,"tests":[{"id":"probe","kind":"script","target":"res://probe.gd"}]}"#,
    )
    .unwrap();
    let godot =
        godot_qa_test_support::fixture(&project.join("fake-godot"), &[("version", "4.5.2.stable")]);
    let (status, body) = run(&project, &godot, args);
    assert!(!status.success(), "{body:#}");
    assert_eq!(body["ok"], false);
    let failure = &body["failures"][0];
    assert_eq!(failure["type"], "invalid_params");
    assert_eq!(failure["command"], "check");
    assert_eq!(failure["actual"], seconds);
    assert!(
        failure["expected"]
            .as_str()
            .unwrap()
            .contains("representable")
    );
    fs::remove_dir_all(project).unwrap();
}

#[test]
fn final_result_write_failure_is_not_success() {
    let project = check_project();
    let source = project.join("artifact_godot.rs");
    fs::write(&source, include_str!("fixtures/artifact_godot.rs")).unwrap();
    let godot = project.join(format!("fake-godot{}", std::env::consts::EXE_SUFFIX));
    let compiled = Command::new(std::env::var_os("RUSTC").unwrap_or_else(|| "rustc".into()))
        .args(["--edition=2024", "-o"])
        .arg(&godot)
        .arg(source)
        .output()
        .unwrap();
    assert!(
        compiled.status.success(),
        "{}",
        String::from_utf8_lossy(&compiled.stderr)
    );
    for args in [
        vec!["import"],
        vec!["check", "--scene", "res://probe.tscn"],
        vec!["check", "--script", "res://probe.gd"],
    ] {
        let (status, body) = run(&project, &godot, &args);
        assert!(!status.success(), "{body:#}");
        assert_eq!(body["ok"], false);
        let failure = body["failures"]
            .as_array()
            .unwrap()
            .iter()
            .find(|f| f["type"] == "artifact_write_failed")
            .unwrap();
        assert_eq!(failure["command"], args[0]);
        assert_eq!(failure["path"], body["data"]["artifacts"]["result"]);
        assert!(Path::new(failure["path"].as_str().unwrap()).is_dir());
        assert!(failure["actual"].is_string());
        assert_eq!(
            body["failures"].as_array().unwrap().len(),
            1,
            "validation should otherwise pass: {body:#}"
        );
    }
    fs::remove_dir_all(project).unwrap();
}

#[test]
fn artifact_preparation_failure_names_failed_path() {
    for args in [
        vec!["import"],
        vec!["check", "--all-scenes"],
        vec!["check", "--all-scripts"],
    ] {
        let project = check_project();
        let godot = godot_qa_test_support::fixture(
            &project.join("fake-godot"),
            &[("version", "4.5.2.stable")],
        );
        let blocked = project.join(".qa/artifacts").join(args[0]);
        fs::create_dir_all(blocked.parent().unwrap()).unwrap();
        fs::write(&blocked, "blocks the command's run directory").unwrap();
        let (status, body) = run(&project, &godot, &args);
        assert!(!status.success(), "{body:#}");
        let failure = body["failures"]
            .as_array()
            .unwrap()
            .iter()
            .find(|f| f["type"] == "artifact_write_failed")
            .expect("structured artifact failure");
        assert_eq!(failure["command"], args[0]);
        assert!(Path::new(failure["path"].as_str().unwrap()).starts_with(&blocked));
        fs::remove_dir_all(project).unwrap();
    }
}
