use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

fn copy_addon(source: &Path, destination: &Path) {
    fs::create_dir_all(destination).unwrap();
    for entry in fs::read_dir(source).unwrap() {
        let entry = entry.unwrap();
        if entry.file_type().unwrap().is_dir() {
            copy_addon(&entry.path(), &destination.join(entry.file_name()));
        } else {
            fs::copy(entry.path(), destination.join(entry.file_name())).unwrap();
        }
    }
}

struct Scratch(PathBuf);
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn real_asset_trial_capture_facts_reject_unobserved_bindings() {
    if std::env::var_os("GODOT_QA_VISUAL_TESTS").is_none() {
        return;
    }
    let godot = std::env::var("GODOT_BIN").expect("GODOT_BIN required for visual integration test");
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let scratch = Scratch(std::env::temp_dir().join(format!(
        "godot-qa-asset-evidence-{}-{nonce}",
        std::process::id()
    )));
    let repo = Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .unwrap();
    copy_addon(
        &repo.join("addons/godot_qa"),
        &scratch.0.join("addons/godot_qa"),
    );
    fs::write(
        scratch.0.join("project.godot"),
        "config_version=5\n[application]\nconfig/name=\"Asset evidence test\"\n[display]\nwindow/size/viewport_width=128\nwindow/size/viewport_height=96\n[rendering]\nrenderer/rendering_method=\"gl_compatibility\"\n",
    )
    .unwrap();
    fs::write(
        scratch.0.join("test.gd"),
        include_str!("fixtures/asset_trial_evidence.gd"),
    )
    .unwrap();
    let output = Command::new(godot)
        .arg("--path")
        .arg(&scratch.0)
        .args([
            "--rendering-method",
            "gl_compatibility",
            "--script",
            "res://test.gd",
        ])
        .output()
        .unwrap();
    let report: Value = serde_json::from_slice(
        &fs::read(scratch.0.join("report.json")).unwrap_or_else(|error| {
            panic!(
                "missing runtime report: {error}\nstdout: {}\nstderr: {}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            )
        }),
    )
    .unwrap();
    assert!(
        output.status.success(),
        "runtime negative evidence checks: {report}\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(report["failures"], serde_json::json!([]));
    assert_eq!(report["checks"].as_array().unwrap().len(), 15);
}
