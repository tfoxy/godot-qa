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
    fs::create_dir_all(&root).unwrap();
    fs::write(
        root.join("project.godot"),
        "[application]\nconfig/name=\"Diagnostics Fixture\"\n",
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

fn fake_godot_with_warning(project_root: &Path) -> PathBuf {
    godot_qa_test_support::fixture(
        &project_root.join("tools/godot-warning"),
        &[
            ("version", "4.6.test"),
            ("mode", "diagnostics"),
            (
                "log",
                r#"GDScript::reload: The parameter "character" is never used in the function "_s6_1_bot_intent_qa_state()". If this is intended, prefix it with an underscore: "_character".
<GDScript Error> UNUSED_PARAMETER
<GDScript Source> bot_navigation.gd:1625 @ GDScript::reload()
"#,
            ),
            (
                "stderr",
                r#"GDScript::reload: The local function parameter "device_index" is shadowing an already-declared variable at line 448 in the current class.
<GDScript Error> SHADOWED_VARIABLE
<GDScript Source> match_config.gd:464 @ GDScript::reload()
"#,
            ),
        ],
    )
}

fn write_editor_bridge_latest(project_root: &Path) {
    write(
        &project_root.join(".qa/artifacts/editor-diagnostics/latest.json"),
        r#"{
  "capture_method": "editor_plugin_debugger_errors_panel",
  "run_id": "editor-fixture",
  "captured_at": "2026-05-19T00:00:00Z",
  "diagnostic_count": 2,
  "warning_count": 2,
  "error_count": 0,
  "diagnostics": [
    {
      "severity": "warning",
      "code": "SHADOWED_VARIABLE",
      "message": "The local function parameter \"device_index\" is shadowing an already-declared variable at line 448 in the current class.",
      "script": "match_config.gd",
      "line": 464,
      "column": null,
      "source": "editor_bridge",
      "raw": "editor bridge duplicate raw"
    },
    {
      "severity": "warning",
      "code": "INTEGER_DIVISION",
      "message": "Integer division. Decimal part will be discarded.",
      "script": "river_generation.gd",
      "line": 122,
      "column": null,
      "source": "editor_bridge",
      "raw": "editor bridge integer division raw"
    }
  ]
}
"#,
    );
}

fn run_diagnostics(project_root: &Path, godot_bin: &Path, strict: bool) -> (ExitStatus, Value) {
    let mut command = Command::new(env!("CARGO_BIN_EXE_godot-qa"));
    command
        .arg("--json")
        .arg("--project")
        .arg(project_root)
        .arg("--godot-bin")
        .arg(godot_bin)
        .arg("diagnostics")
        .arg("editor");
    if strict {
        command.arg("--strict-warnings");
    }
    let output = command.output().unwrap();
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
fn diagnostics_editor_reports_warning_entries_and_artifacts() {
    let project_root = make_project_root("diagnostics-editor");
    let fake_godot = fake_godot_with_warning(&project_root);

    let (status, body) = run_diagnostics(&project_root, &fake_godot, false);

    assert!(status.success(), "body: {body:#}");
    assert_eq!(body["ok"], true);
    assert_eq!(body["command"], "diagnostics editor");
    assert_eq!(body["data"]["error_count"], 0);
    assert_eq!(body["data"]["warning_count"], 2);
    assert_eq!(body["data"]["diagnostic_count"], 2);
    assert!(body["data"]["artifacts"]["logs"].as_str().is_some());
    assert!(
        body["data"]["artifacts"]["raw_godot_log"]
            .as_str()
            .is_some()
    );
    let diagnostics = body["data"]["diagnostics"].as_array().unwrap();
    assert!(diagnostics.iter().any(|entry| {
        entry["code"] == "SHADOWED_VARIABLE"
            && entry["script"] == "match_config.gd"
            && entry["line"] == 464
    }));
    assert!(diagnostics.iter().any(|entry| {
        entry["code"] == "UNUSED_PARAMETER"
            && entry["script"] == "bot_navigation.gd"
            && entry["line"] == 1625
    }));
    assert_eq!(
        fs::read_to_string(project_root.join("project.godot")).unwrap(),
        "[application]\nconfig/name=\"Diagnostics Fixture\"\n"
    );

    fs::remove_dir_all(project_root).unwrap();
}

#[test]
fn diagnostics_editor_merges_editor_bridge_and_log_diagnostics() {
    let project_root = make_project_root("diagnostics-editor-bridge");
    let fake_godot = fake_godot_with_warning(&project_root);
    write_editor_bridge_latest(&project_root);

    let (status, body) = run_diagnostics(&project_root, &fake_godot, false);

    assert!(status.success(), "body: {body:#}");
    assert_eq!(body["ok"], true);
    assert_eq!(body["data"]["warning_count"], 3);
    assert_eq!(body["data"]["diagnostic_count"], 3);
    assert_eq!(
        body["data"]["editor_bridge"]["capture_method"],
        "editor_plugin_debugger_errors_panel"
    );
    assert_eq!(body["data"]["editor_bridge"]["available"], true);
    assert!(
        Path::new(body["data"]["artifacts"]["editor_bridge"].as_str().unwrap())
            .ends_with(".qa/artifacts/editor-diagnostics/latest.json")
    );
    let capture_methods = body["data"]["capture_methods"].as_array().unwrap();
    assert!(capture_methods.iter().any(|method| {
        method
            .as_str()
            .is_some_and(|value| value == "editor_plugin_debugger_errors_panel")
    }));
    let diagnostics = body["data"]["diagnostics"].as_array().unwrap();
    assert!(diagnostics.iter().any(|entry| {
        entry["code"] == "SHADOWED_VARIABLE"
            && entry["script"] == "match_config.gd"
            && entry["source"] == "editor_bridge"
    }));
    assert!(diagnostics.iter().any(|entry| {
        entry["code"] == "INTEGER_DIVISION" && entry["script"] == "river_generation.gd"
    }));

    fs::remove_dir_all(project_root).unwrap();
}

#[test]
fn diagnostics_editor_reports_missing_editor_bridge_as_advisory_note() {
    let project_root = make_project_root("diagnostics-editor-missing-bridge");
    let fake_godot = fake_godot_with_warning(&project_root);

    let (status, body) = run_diagnostics(&project_root, &fake_godot, false);

    assert!(status.success(), "body: {body:#}");
    assert_eq!(body["data"]["editor_bridge"]["available"], false);
    assert!(
        body["data"]["notes"][0]
            .as_str()
            .unwrap()
            .contains("No editor-run diagnostics bridge artifact found")
    );

    fs::remove_dir_all(project_root).unwrap();
}

#[test]
fn diagnostics_editor_strict_warnings_fail() {
    let project_root = make_project_root("diagnostics-editor-strict");
    let fake_godot = fake_godot_with_warning(&project_root);

    let (status, body) = run_diagnostics(&project_root, &fake_godot, true);

    assert!(!status.success(), "body: {body:#}");
    assert_eq!(body["ok"], false);
    assert_eq!(
        body["failures"][0]["type"],
        "editor_diagnostics_warnings_present"
    );
    assert_eq!(body["failures"][0]["actual"], 2);
    assert_eq!(
        fs::read_to_string(project_root.join("project.godot")).unwrap(),
        "[application]\nconfig/name=\"Diagnostics Fixture\"\n"
    );

    fs::remove_dir_all(project_root).unwrap();
}
