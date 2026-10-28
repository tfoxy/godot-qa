use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

struct Scratch(PathBuf);

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn copy_tree(source: &Path, destination: &Path) {
    fs::create_dir_all(destination).unwrap();
    for entry in fs::read_dir(source).unwrap() {
        let entry = entry.unwrap();
        if entry.file_type().unwrap().is_dir() {
            copy_tree(&entry.path(), &destination.join(entry.file_name()));
        } else {
            fs::copy(entry.path(), destination.join(entry.file_name())).unwrap();
        }
    }
}

#[test]
fn locator_clicks_fire_actions_across_window_canvas_and_stretch_transforms() {
    if std::env::var_os("GODOT_QA_VISUAL_TESTS").is_none() {
        return;
    }
    let godot = std::env::var("GODOT_BIN").expect("GODOT_BIN required for visual integration test");
    let repo = Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .unwrap();
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let scratch = Scratch(std::env::temp_dir().join(format!(
        "godot-qa-scaled-click-{}-{nonce}",
        std::process::id()
    )));
    copy_tree(
        &repo.join("addons/godot_qa"),
        &scratch.0.join("addons/godot_qa"),
    );
    fs::write(
        scratch.0.join("project.godot"),
        "config_version=5\n[application]\nconfig/name=\"Scaled locator click regression\"\n[rendering]\nrenderer/rendering_method=\"gl_compatibility\"\n",
    )
    .unwrap();
    fs::write(
        scratch.0.join("test.gd"),
        include_str!("../../../examples/sample_project/qa/tests/scaled_locator_click_test.gd"),
    )
    .unwrap();
    let mut child = Command::new(godot)
        .arg("--path")
        .arg(&scratch.0)
        .args(["--script", "res://test.gd"])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let started = Instant::now();
    let mut timed_out = false;
    while child.try_wait().unwrap().is_none() {
        if started.elapsed() > Duration::from_secs(45) {
            timed_out = true;
            child.kill().unwrap();
            break;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    let output = child.wait_with_output().unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        !timed_out && output.status.success(),
        "rendered locator-click checks failed (timeout={timed_out}):\n{stdout}\n{stderr}"
    );
    let report: Value = stdout
        .lines()
        .find_map(|line| serde_json::from_str::<Value>(line).ok())
        .expect("fixture emits a JSON report");
    assert_eq!(report["ok"], true, "{report}");
    assert_eq!(report["failures"], serde_json::json!([]));
    assert_eq!(report["pressed"], 5);
}
