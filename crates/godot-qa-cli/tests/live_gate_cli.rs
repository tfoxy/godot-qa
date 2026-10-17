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

fn make_project_root(prefix: &str) -> PathBuf {
    let root = env::temp_dir().join(format!("godot-qa-{prefix}-{}", unique_suffix()));
    fs::create_dir_all(&root).unwrap();
    fs::write(root.join("project.godot"), "[application]\n").unwrap();
    root
}

fn write(path: &Path, contents: &str) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(path, contents).unwrap();
}

fn run_live_gate_list(project_root: &Path) -> (std::process::ExitStatus, Value) {
    let output = Command::new(env!("CARGO_BIN_EXE_godot-qa"))
        .arg("--json")
        .arg("--project")
        .arg(project_root)
        .arg("live-gate")
        .arg("list")
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
fn live_gate_list_cli_contract_outputs_json_envelope_and_gate_metadata() {
    let project_root = make_project_root("live-gate-cli-list");
    write(
        &project_root.join("qa/live_gates/hidden_overlay.json"),
        &json!({
            "name": "hidden_overlay",
            "scene": "res://ui/main_menu.tscn",
            "overlay": {"required": "hidden"},
            "anchors": {
                "required": ["main_menu", "start_button"],
                "optional": ["settings_button"]
            },
            "methods": ["qa_get_menu_state"],
            "probes": ["qa_menu_ready"]
        })
        .to_string(),
    );

    let (status, body) = run_live_gate_list(&project_root);

    assert!(status.success());
    assert_eq!(body["ok"], true);
    assert_eq!(body["command"], "live-gate list");
    assert!(body["message"].is_string());
    assert!(body["data"].is_object());
    assert!(body["failures"].is_array());

    let gates = body["data"]["gates"].as_array().unwrap();
    assert_eq!(gates.len(), 1);
    let gate = &gates[0];
    assert_eq!(gate["name"], "hidden_overlay");
    assert!(gate["path"].is_string());
    assert_eq!(gate["scene"], "res://ui/main_menu.tscn");
    assert_eq!(gate["overlay"], "hidden");
    assert_eq!(gate["required_anchor_count"], 2);
    assert_eq!(gate["optional_anchor_count"], 1);
    assert_eq!(gate["methods"], json!(["qa_get_menu_state"]));
    assert_eq!(gate["probes"], json!(["qa_menu_ready"]));

    fs::remove_dir_all(project_root).unwrap();
}

#[test]
fn live_gate_list_cli_contract_includes_sample_hidden_overlay_fixture() {
    let project_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join("examples/sample_project");

    let (status, body) = run_live_gate_list(&project_root);

    assert!(status.success());
    let gates = body["data"]["gates"].as_array().unwrap();
    let gate = gates
        .iter()
        .find(|gate| gate["name"] == "hidden_overlay_inspection")
        .expect("sample hidden overlay live gate should be listed");

    assert_eq!(gate["scene"], "res://ui/main_menu.tscn");
    assert_eq!(gate["overlay"], "hidden");
    assert_eq!(gate["required_anchor_count"], 3);
    assert_eq!(gate["optional_anchor_count"], 1);
    assert_eq!(gate["methods"], json!(["get_raft_motion_qa_state"]));
    assert_eq!(gate["probes"], json!(["qa_probe_surface"]));
}

#[test]
fn live_gate_list_cli_contract_missing_live_gates_dir_exits_nonzero() {
    let project_root = make_project_root("live-gate-cli-missing");

    let (status, body) = run_live_gate_list(&project_root);

    assert_eq!(status.code(), Some(1));
    assert_eq!(body["ok"], false);
    assert_eq!(body["command"], "live-gate list");
    assert_eq!(body["failures"][0]["type"], "missing_live_gates_dir");
    assert_eq!(body["failures"][0]["command"], "live-gate list");

    fs::remove_dir_all(project_root).unwrap();
}

#[test]
fn live_gate_list_cli_contract_duplicate_names_exits_nonzero_with_failure() {
    let project_root = make_project_root("live-gate-cli-duplicate");
    write(
        &project_root.join("qa/live_gates/first.json"),
        r#"{"name":"duplicate","scene":"res://a.tscn"}"#,
    );
    write(
        &project_root.join("qa/live_gates/nested/second.json"),
        r#"{"name":"duplicate","scene":"res://b.tscn"}"#,
    );

    let (status, body) = run_live_gate_list(&project_root);

    assert_eq!(status.code(), Some(1));
    assert_eq!(body["ok"], false);
    assert_eq!(body["command"], "live-gate list");
    assert_eq!(body["failures"][0]["type"], "duplicate_live_gate_name");
    assert_eq!(body["failures"][0]["command"], "live-gate list");

    fs::remove_dir_all(project_root).unwrap();
}
