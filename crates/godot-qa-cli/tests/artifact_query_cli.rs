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

fn write_json(path: &Path, value: &Value) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(path, serde_json::to_string_pretty(value).unwrap()).unwrap();
}

fn artifact_run() -> PathBuf {
    let root = env::temp_dir().join(format!("godot-qa-artifact-query-cli-{}", unique_suffix()));
    let run_root = root.join(".qa/artifacts/combat_flow/100Z");
    write_json(
        &run_root.join("snapshots/impact/tree.json"),
        &json!({
            "root": {
                "path": "/root/Main",
                "name": "Main",
                "type": "Node2D",
                "qa_id": "",
                "visible": true,
                "inside_tree": true,
                "groups": [],
                "children": [{
                    "path": "/root/Main/Projectile",
                    "name": "Projectile",
                    "type": "Area2D",
                    "qa_id": "weapon_projectile",
                    "visible": true,
                    "inside_tree": true,
                    "groups": [],
                    "children": []
                }]
            }
        }),
    );
    run_root
}

#[test]
fn artifact_query_cli_returns_matching_nodes() {
    let run_root = artifact_run();
    let output = Command::new(env!("CARGO_BIN_EXE_godot-qa"))
        .arg("--json")
        .arg("artifact")
        .arg("query")
        .arg(&run_root)
        .arg("--snapshot")
        .arg("impact")
        .arg("--qa-id")
        .arg("weapon_projectile")
        .arg("--visible")
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let body: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(body["ok"], true);
    assert_eq!(body["command"], "artifact query");
    assert_eq!(body["data"]["count"], 1);
    assert_eq!(body["data"]["matches"][0]["path"], "/root/Main/Projectile");
}
