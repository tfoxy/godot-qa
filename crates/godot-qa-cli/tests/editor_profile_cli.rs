use serde_json::{Value, json};
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
    fs::create_dir_all(&root).unwrap();
    fs::write(
        root.join("project.godot"),
        "[application]\nconfig/name=\"Editor Profile Fixture\"\n",
    )
    .unwrap();
    root
}

fn write(path: &Path, contents: &str) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(path, contents).unwrap();
}

fn run_editor_profile_export(project_root: &Path) -> (ExitStatus, Value) {
    let output = Command::new(env!("CARGO_BIN_EXE_godot-qa"))
        .arg("--json")
        .arg("--project")
        .arg(project_root)
        .arg("profile")
        .arg("editor")
        .arg("export")
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

fn latest_path(project_root: &Path) -> PathBuf {
    project_root.join(".qa/artifacts/editor-profiler/latest.json")
}

#[test]
fn editor_profile_export_reports_missing_latest_artifact() {
    let project_root = make_project_root("editor-profile-missing");

    let (status, body) = run_editor_profile_export(&project_root);

    assert!(!status.success(), "body: {body:#}");
    assert_eq!(body["ok"], false);
    assert_eq!(body["command"], "profile editor export");
    assert_eq!(body["failures"][0]["type"], "editor_profile_missing");
    assert!(
        body["data"]["guidance"]
            .as_str()
            .unwrap()
            .contains("Godot QA Profiler tab")
    );

    fs::remove_dir_all(project_root).unwrap();
}

#[test]
fn editor_profile_export_returns_valid_script_functions() {
    let project_root = make_project_root("editor-profile-valid");
    write(
        &latest_path(&project_root),
        r#"{
  "source": "editor_script_functions",
  "capture_method": "editor_debugger_raw_servers_profile",
  "experimental": true,
  "schema_version": 1,
  "run_id": "fixture",
  "captured_at": "2026-05-19T00:00:00Z",
  "godot_version": "4.6.test",
  "project_root": "/tmp/project",
  "session_id": "unknown",
  "frame_count": 12,
  "script_functions": [
    {
      "signature": "res://player.gd::Player::_process",
      "script": "res://player.gd",
      "function": "_process",
      "call_count": 120,
      "total_ms": 14.2,
      "self_ms": 8.1,
      "internal_ms": 0.0,
      "max_total_ms": 3.4
    }
  ],
  "frames": [],
  "warnings": []
}
"#,
    );

    let (status, body) = run_editor_profile_export(&project_root);

    assert!(status.success(), "body: {body:#}");
    assert_eq!(body["ok"], true);
    assert_eq!(body["data"]["source"], "editor_script_functions");
    assert_eq!(body["data"]["experimental"], true);
    assert_eq!(body["data"]["frame_count"], 12);
    assert_eq!(body["data"]["script_function_count"], 1);
    assert_eq!(
        body["data"]["script_functions"][0]["signature"],
        "res://player.gd::Player::_process"
    );
    let profile_path = PathBuf::from(body["data"]["artifacts"]["profile"].as_str().unwrap());
    assert!(profile_path.exists());

    fs::remove_dir_all(project_root).unwrap();
}

#[test]
fn editor_profile_export_reports_malformed_latest_artifact() {
    let project_root = make_project_root("editor-profile-invalid");
    write(&latest_path(&project_root), r#"{"source": "wrong"}"#);

    let (status, body) = run_editor_profile_export(&project_root);

    assert!(!status.success(), "body: {body:#}");
    assert_eq!(body["failures"][0]["type"], "editor_profile_invalid");
    assert!(
        body["failures"][0]["expected"]
            .as_str()
            .unwrap()
            .contains("editor_script_functions")
    );
    assert!(
        body["failures"][0]["path"]
            .as_str()
            .unwrap()
            .replace('\\', "/")
            .ends_with(".qa/artifacts/editor-profiler/latest.json")
    );

    fs::remove_dir_all(project_root).unwrap();
}

#[test]
fn editor_profile_export_bounds_retained_frames() {
    let project_root = make_project_root("editor-profile-bounded");
    let frames: Vec<Value> = (0..3005)
        .map(|index| json!({ "frame_number": index }))
        .collect();
    let payload = json!({
        "source": "editor_script_functions",
        "capture_method": "editor_debugger_raw_servers_profile",
        "experimental": true,
        "godot_version": "4.6.test",
        "frame_count": 3005,
        "script_functions": [],
        "frames": frames,
        "warnings": []
    });
    write(
        &latest_path(&project_root),
        &serde_json::to_string_pretty(&payload).unwrap(),
    );

    let (status, body) = run_editor_profile_export(&project_root);

    assert!(status.success(), "body: {body:#}");
    let retained = body["data"]["profile"]["frames"].as_array().unwrap();
    assert_eq!(retained.len(), 3000);
    assert_eq!(retained[0]["frame_number"], 5);

    fs::remove_dir_all(project_root).unwrap();
}
