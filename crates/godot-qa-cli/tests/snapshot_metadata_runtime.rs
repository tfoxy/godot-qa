use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

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
            .join(format!("snapshot-metadata-{}-{case}", std::process::id())),
    );
    let addon = scratch.0.join("addons/godot_qa");
    fs::create_dir_all(&addon).unwrap();
    fs::copy(
        repo.join("addons/godot_qa/qa_snapshot.gd"),
        addon.join("qa_snapshot.gd"),
    )
    .unwrap();
    fs::write(scratch.0.join("project.godot"), "config_version=5\n").unwrap();
    fs::write(
        scratch.0.join("test.gd"),
        include_str!("fixtures/snapshot_metadata.gd"),
    )
    .unwrap();
    let output = Command::new(std::env::var("GODOT_BIN").unwrap_or_else(|_| "godot".into()))
        .args(["--headless", "--quit-after", "600", "--path"])
        .arg(&scratch.0)
        .args(["--script", "res://test.gd", "--", case])
        .output()
        .unwrap();
    let diagnostics = format!(
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let report: Value = serde_json::from_slice(
        &fs::read(scratch.0.join("report.json"))
            .unwrap_or_else(|error| panic!("{case}: missing report: {error}\n{diagnostics}")),
    )
    .unwrap();
    assert!(output.status.success(), "{case}: {report}\n{diagnostics}");
    assert!(
        !String::from_utf8_lossy(&output.stderr).contains("SCRIPT ERROR"),
        "{diagnostics}"
    );
    assert_eq!(report["failures"], serde_json::json!([]), "{report}");
    assert!(report["checks"].as_array().unwrap().len() >= 3);
}

#[test]
fn snapshot_metadata_preserves_integer_key_values() {
    run_regression("integer_keys");
}

#[test]
fn snapshot_metadata_string_keys_win_collisions_in_either_order() {
    run_regression("mixed_keys");
}
