use serde_json::Value;
use std::path::{Path, PathBuf};
use std::process::Command;

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .to_path_buf()
}

#[test]
fn collision_report_demo_is_discoverable_in_sample_scenarios() {
    let root = repo_root();
    let output = Command::new(env!("CARGO_BIN_EXE_godot-qa"))
        .arg("--json")
        .arg("--project")
        .arg(root.join("examples/sample_project"))
        .arg("scenario")
        .arg("list")
        .output()
        .unwrap();
    let body: Value = serde_json::from_slice(&output.stdout).unwrap_or_else(|error| {
        panic!(
            "invalid stdout JSON: {error}\nstdout: {}\nstderr: {}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        )
    });
    assert!(output.status.success());
    let scenarios = body["data"]["scenarios"].as_array().unwrap();
    assert!(scenarios.iter().any(|scenario| {
        scenario["name"] == "collision_report_demo"
            && scenario["path"].as_str().is_some_and(|path| {
                Path::new(path).ends_with("qa/scenarios/examples/collision_report_demo.json")
            })
    }));
}
