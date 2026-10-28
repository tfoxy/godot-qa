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
    fs::create_dir_all(root.join("qa/scenarios")).unwrap();
    fs::create_dir_all(root.join("addons/godot_qa")).unwrap();
    fs::write(root.join("project.godot"), "[application]\n").unwrap();
    fs::write(root.join("addons/godot_qa/plugin.cfg"), "[plugin]\n").unwrap();
    root
}

fn write(path: &Path, contents: &str) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(path, contents).unwrap();
}

fn write_fake_godot(path: &Path, screenshot_path: &Path) {
    let result = json!({
        "ok":true,"command":"scenario run","message":"scenario run completed",
        "data":{"runtime":"fake"},"snapshots":["after"],
        "snapshot_artifacts":{"after":{"screenshot":screenshot_path}},"failures":[]
    })
    .to_string();
    godot_qa_test_support::fixture(
        path,
        &[
            ("request_prefix", "--qa-run-request="),
            ("scenario_proof", "passed"),
            ("write:result.json", &result),
        ],
    );
}

fn save_visual_risk_png(path: &Path) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    let image = image::ImageBuffer::from_fn(160, 160, |x, y| {
        if x < 148 || y % 2 == 0 {
            image::Rgba([40u8, 40, 48, 255])
        } else {
            image::Rgba([80u8, 80, 88, 255])
        }
    });
    image.save(path).unwrap();
}

fn run_visual_risk_scenario(
    project_root: &Path,
    extra_args: &[&str],
) -> (std::process::ExitStatus, Value) {
    let output = Command::new(env!("CARGO_BIN_EXE_godot-qa"))
        .arg("scenario")
        .arg("run")
        .arg("visual_risk_demo")
        .args(extra_args)
        .arg("--project")
        .arg(project_root)
        .arg("--godot-bin")
        .arg(project_root.join(format!("fake-godot{}", env::consts::EXE_SUFFIX)))
        .arg("--json")
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

fn setup_visual_risk_project(prefix: &str) -> PathBuf {
    let project_root = make_project_root(prefix);
    write(
        &project_root.join("qa/scenarios/visual_risk_demo.json"),
        &json!({
            "name": "visual_risk_demo",
            "scene": "res://VisualRisk.tscn",
            "steps": [{"snapshot": "after"}],
            "snapshots": {"save_screenshot": true}
        })
        .to_string(),
    );
    let screenshot_path = project_root.join("risk.png");
    save_visual_risk_png(&screenshot_path);
    write_fake_godot(
        &project_root.join(format!("fake-godot{}", env::consts::EXE_SUFFIX)),
        &screenshot_path,
    );
    project_root
}

#[test]
fn scenario_visual_risk_cli_warning_mode_exits_zero_with_machine_readable_warning() {
    let project_root = setup_visual_risk_project("scenario-visual-risk-warning");

    let (status, body) = run_visual_risk_scenario(&project_root, &["--visual-risk"]);

    assert!(status.success());
    assert_eq!(body["ok"], true);
    assert_eq!(body["failures"], json!([]));
    assert_eq!(body["data"]["visual_risk"]["strict"], json!(false));
    assert_eq!(
        body["data"]["visual_risk"]["findings"][0]["rule_id"],
        json!("visual-risk.low_visual_diversity")
    );
    assert_eq!(
        body["data"]["visual_risk"]["findings"][0]["severity"],
        json!("warning")
    );
    assert_eq!(
        body["data"]["visual_risk"]["findings"][0]["artifacts"]["screenshot"],
        json!(project_root.join("risk.png").to_string_lossy().to_string())
    );
    assert_eq!(
        body["data"]["visual_risk"]["findings"][0]["actual"]["unique_color_count"],
        json!(2)
    );
    assert_eq!(
        body["data"]["visual_risk"]["findings"][0]["actual"]["width"],
        json!(160)
    );
    assert_eq!(
        body["data"]["visual_risk"]["findings"][0]["actual"]["height"],
        json!(160)
    );
    assert_eq!(
        body["data"]["warnings"][0]["rule_id"],
        json!("visual-risk.low_visual_diversity")
    );
    assert_eq!(body["data"]["warnings"][0]["severity"], json!("warning"));
    assert_eq!(
        body["data"]["warnings"][0]["artifacts"]["screenshot"],
        json!(project_root.join("risk.png").to_string_lossy().to_string())
    );
    assert_eq!(
        body["data"]["warnings"][0]["actual"]["unique_color_count"],
        json!(2)
    );
    let request_path = body["data"]["artifacts"]["request_path"].as_str().unwrap();
    let request: Value = serde_json::from_str(&fs::read_to_string(request_path).unwrap()).unwrap();
    assert!(
        request["run"]["command"]
            .as_str()
            .unwrap()
            .contains("--visual-risk")
    );

    fs::remove_dir_all(project_root).unwrap();
}

#[test]
fn scenario_visual_risk_cli_strict_mode_exits_one_with_rule_id_failure() {
    let project_root = setup_visual_risk_project("scenario-visual-risk-strict");

    let (status, body) = run_visual_risk_scenario(&project_root, &["--visual-risk-strict"]);

    assert_eq!(status.code(), Some(1));
    assert_eq!(body["ok"], false);
    assert_eq!(body["data"]["visual_risk"]["strict"], json!(true));
    assert_eq!(
        body["data"]["visual_risk"]["findings"][0]["rule_id"],
        json!("visual-risk.low_visual_diversity")
    );
    assert_eq!(
        body["data"]["visual_risk"]["findings"][0]["severity"],
        json!("warning")
    );
    assert_eq!(
        body["data"]["visual_risk"]["findings"][0]["artifacts"]["screenshot"],
        json!(project_root.join("risk.png").to_string_lossy().to_string())
    );
    assert_eq!(
        body["data"]["visual_risk"]["findings"][0]["actual"]["dominant_color_ratio"],
        json!(0.9625)
    );
    assert_eq!(
        body["failures"][0]["type"],
        "visual-risk.low_visual_diversity"
    );
    assert_eq!(
        body["failures"][0]["rule_id"],
        json!("visual-risk.low_visual_diversity")
    );
    assert_eq!(
        body["failures"][0]["actual"]["unique_color_count"],
        json!(2)
    );
    assert_eq!(
        body["failures"][0]["artifacts"]["screenshot"],
        json!(project_root.join("risk.png").to_string_lossy().to_string())
    );
    let request_path = body["data"]["artifacts"]["request_path"].as_str().unwrap();
    let request: Value = serde_json::from_str(&fs::read_to_string(request_path).unwrap()).unwrap();
    assert!(
        request["run"]["command"]
            .as_str()
            .unwrap()
            .contains("--visual-risk-strict")
    );

    fs::remove_dir_all(project_root).unwrap();
}

#[test]
fn scenario_run_rejects_old_addon_success_without_execution_proof() {
    let project_root = setup_visual_risk_project("scenario-missing-execution-proof");
    // An older addon can report success without returning an identity or execution
    // ledger. Deliberately omit the fixture's proof emulation for this result.
    godot_qa_test_support::fixture(
        &project_root.join(format!("fake-godot{}", env::consts::EXE_SUFFIX)),
        &[
            ("request_prefix", "--qa-run-request="),
            (
                "write:result.json",
                r#"{"ok":true,"command":"scenario run","data":{"runtime":"old-addon"},"failures":[]}"#,
            ),
        ],
    );

    let (status, body) = run_visual_risk_scenario(&project_root, &[]);

    assert!(!status.success(), "body: {body:#}");
    assert_eq!(body["ok"], false);
    let failure = body["failures"]
        .as_array()
        .unwrap()
        .iter()
        .find(|failure| failure["type"] == "assertion_proof_missing")
        .unwrap_or_else(|| panic!("missing proof failure: {body:#}"));
    let message = failure["message"].as_str().unwrap();
    assert!(
        message.contains("update") && message.contains("addon"),
        "{message}"
    );

    fs::remove_dir_all(project_root).unwrap();
}
