use serde_json::Value;
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

fn make_project_root(prefix: &str) -> PathBuf {
    let root = env::temp_dir().join(format!("godot-qa-{prefix}-{}", unique_suffix()));
    fs::create_dir_all(&root).unwrap();
    fs::write(root.join("project.godot"), "[application]\n").unwrap();
    root
}

fn run_scenario_init(
    project_root: &Path,
    extra_args: &[&str],
) -> (std::process::ExitStatus, Value) {
    let output = Command::new(env!("CARGO_BIN_EXE_godot-qa"))
        .arg("--json")
        .arg("--project")
        .arg(project_root)
        .arg("scenario")
        .arg("init")
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

#[test]
fn scenario_init_outputs_valid_json_template_without_godot() {
    let project_root = make_project_root("scenario-init-template");
    let (status, body) = run_scenario_init(
        &project_root,
        &[
            "prototype_schema_probe",
            "--scene",
            "res://scenes/main/main.tscn",
            "--viewport",
            "1280x720",
        ],
    );

    assert!(status.success());
    assert_eq!(body["ok"], true);
    assert_eq!(body["command"], "scenario init");
    assert_eq!(body["data"]["template"]["name"], "prototype_schema_probe");
    assert_eq!(
        body["data"]["template"]["scene"],
        "res://scenes/main/main.tscn"
    );
    assert_eq!(body["data"]["template"]["viewport"]["width"], 1280);
    assert_eq!(body["data"]["template"]["viewport"]["height"], 720);
    assert_eq!(body["data"]["template"]["steps"][0]["wait_frames"], 5);
    assert_eq!(body["data"]["template"]["assert"][0]["no_errors"], true);

    fs::remove_dir_all(project_root).unwrap();
}

#[test]
fn scenario_init_writes_out_and_refuses_overwrite() {
    let project_root = make_project_root("scenario-init-out");
    let out = "qa/scenarios/candidates/prototype_schema_probe.json";
    let (status, body) = run_scenario_init(
        &project_root,
        &[
            "prototype_schema_probe",
            "--scene",
            "res://scenes/main/main.tscn",
            "--out",
            out,
        ],
    );

    assert!(status.success());
    assert_eq!(body["ok"], true);
    assert!(project_root.join(out).is_file());

    let (status, body) = run_scenario_init(
        &project_root,
        &[
            "prototype_schema_probe",
            "--scene",
            "res://scenes/main/main.tscn",
            "--out",
            out,
        ],
    );

    assert_eq!(status.code(), Some(1));
    assert_eq!(body["ok"], false);
    assert_eq!(body["failures"][0]["type"], "scenario_output_exists");

    fs::remove_dir_all(project_root).unwrap();
}

#[test]
fn scenario_init_rejects_invalid_viewport_and_name() {
    let project_root = make_project_root("scenario-init-invalid");
    let (status, body) = run_scenario_init(
        &project_root,
        &[
            "prototype_schema_probe",
            "--scene",
            "res://scenes/main/main.tscn",
            "--viewport",
            "wide",
        ],
    );

    assert_eq!(status.code(), Some(1));
    assert_eq!(body["failures"][0]["type"], "invalid_viewport");

    let (status, body) = run_scenario_init(
        &project_root,
        &[
            "../unsafe",
            "--scene",
            "res://scenes/main/main.tscn",
            "--viewport",
            "1280x720",
        ],
    );

    assert_eq!(status.code(), Some(1));
    assert_eq!(body["failures"][0]["type"], "scenario_validation_failed");

    fs::remove_dir_all(project_root).unwrap();
}
