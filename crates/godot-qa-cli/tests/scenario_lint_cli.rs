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

fn run_scenario_lint(
    project_root: &Path,
    extra_args: &[&str],
) -> (std::process::ExitStatus, Value) {
    let output = Command::new(env!("CARGO_BIN_EXE_godot-qa"))
        .arg("--json")
        .arg("--project")
        .arg(project_root)
        .arg("scenario")
        .arg("lint")
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
fn scenario_lint_outputs_json_and_warnings_exit_zero_by_default() {
    let project_root = make_project_root("scenario-lint-cli-default");
    write(
        &project_root.join("qa/scenarios/sample.json"),
        &json!({
            "name": "sample",
            "scene": "res://scene.tscn",
            "steps": [],
            "unknown": true
        })
        .to_string(),
    );

    let (status, body) = run_scenario_lint(&project_root, &[]);

    assert!(status.success());
    assert_eq!(body["ok"], true);
    assert_eq!(body["command"], "scenario lint");
    assert_eq!(body["data"]["summary"]["warnings"], 1);
    assert_eq!(body["data"]["summary"]["errors"], 0);

    fs::remove_dir_all(project_root).unwrap();
}

#[test]
fn scenario_lint_scenario_filter_limits_findings_to_selected_scenario() {
    let project_root = make_project_root("scenario-lint-cli-filter");
    write(
        &project_root.join("qa/scenarios/selected.json"),
        &json!({
            "name": "selected",
            "scene": "res://scene.tscn",
            "steps": [],
            "unknown": true
        })
        .to_string(),
    );
    write(
        &project_root.join("qa/scenarios/other.json"),
        &json!({
            "name": "other",
            "scene": "res://scene.tscn",
            "steps": [],
            "unknown": true
        })
        .to_string(),
    );

    let (status, body) = run_scenario_lint(&project_root, &["--scenario", "selected"]);

    assert!(status.success());
    assert_eq!(body["ok"], true);
    assert_eq!(body["data"]["summary"]["total"], 1);
    let findings = body["data"]["findings"].as_array().unwrap();
    assert_eq!(findings.len(), 1);
    assert_eq!(findings[0]["scenario"], "selected");

    fs::remove_dir_all(project_root).unwrap();
}

#[test]
fn scenario_lint_strict_warnings_exit_nonzero() {
    let project_root = make_project_root("scenario-lint-cli-strict");
    write(
        &project_root.join("qa/scenarios/sample.json"),
        &json!({
            "name": "sample",
            "scene": "res://scene.tscn",
            "steps": [],
            "unknown": true
        })
        .to_string(),
    );

    let (status, body) = run_scenario_lint(&project_root, &["--strict"]);

    assert_eq!(status.code(), Some(1));
    assert_eq!(body["ok"], false);
    assert_eq!(body["data"]["summary"]["warnings"], 1);
    assert_eq!(body["data"]["summary"]["strict"], true);

    fs::remove_dir_all(project_root).unwrap();
}

#[test]
fn scenario_lint_cli_does_not_run_visual_risk_analysis_or_emit_rule_ids() {
    let project_root = make_project_root("scenario-lint-cli-no-visual-risk");
    write(
        &project_root.join("qa/scenarios/visual_snapshot.json"),
        &json!({
            "name": "visual_snapshot",
            "scene": "res://scene.tscn",
            "snapshots": {"save_screenshot": true},
            "steps": [{"snapshot": "after"}],
            "unknown": true
        })
        .to_string(),
    );

    let (status, body) = run_scenario_lint(&project_root, &[]);

    assert!(status.success());
    assert_eq!(body["ok"], true);
    assert_eq!(body["data"]["summary"]["warnings"], 1);
    assert!(body["data"].get("visual_risk").is_none());
    let body_json = serde_json::to_string(&body).unwrap();
    assert!(!body_json.contains("visual-risk."));
    assert!(!body_json.contains("screenshot_low_information"));

    let (strict_status, strict_body) = run_scenario_lint(&project_root, &["--strict"]);
    assert_eq!(strict_status.code(), Some(1));
    assert_eq!(strict_body["ok"], false);
    assert_eq!(strict_body["data"]["summary"]["warnings"], 1);
    assert_eq!(strict_body["data"]["summary"]["strict"], true);
    assert!(strict_body["data"].get("visual_risk").is_none());
    let strict_body_json = serde_json::to_string(&strict_body).unwrap();
    assert!(!strict_body_json.contains("visual-risk."));

    fs::remove_dir_all(project_root).unwrap();
}

#[test]
fn scenario_lint_missing_selected_scenario_returns_json_failure() {
    let project_root = make_project_root("scenario-lint-cli-missing");
    write(
        &project_root.join("qa/scenarios/sample.json"),
        &json!({"name":"sample","scene":"res://scene.tscn","steps":[]}).to_string(),
    );

    let (status, body) = run_scenario_lint(&project_root, &["--scenario", "missing"]);

    assert_eq!(status.code(), Some(1));
    assert_eq!(body["ok"], false);
    assert_eq!(body["command"], "scenario lint");
    assert_eq!(body["failures"][0]["type"], "scenario_not_found");

    fs::remove_dir_all(project_root).unwrap();
}

#[test]
fn scenario_lint_project_flag_claim_outputs_json() {
    let project_root = make_project_root("scenario-lint-cli-project-flag");
    write(
        &project_root.join("qa/scenarios/project_flag.json"),
        &json!({
            "name": "project_flag",
            "scene": "res://scene.tscn",
            "lint": {"validates_project_flag": true},
            "steps": []
        })
        .to_string(),
    );

    let (status, body) = run_scenario_lint(&project_root, &["--scenario", "project_flag"]);

    assert!(status.success());
    assert_eq!(body["ok"], true);
    assert!(body["data"]["summary"]["warnings"].as_u64().unwrap() >= 1);
    let findings = body["data"]["findings"].as_array().unwrap();
    let finding = findings
        .iter()
        .find(|f| f["rule_id"] == "GQA-LINT-008")
        .expect("expected project flag lint warning");
    assert_eq!(finding["severity"], "warning");
    assert_eq!(finding["scenario"], "project_flag");
    assert_eq!(finding["target"], "lint.validates_project_args");

    fs::remove_dir_all(project_root).unwrap();
}

#[test]
fn scenario_lint_rejects_malformed_snapshot_filters() {
    let project_root = make_project_root("scenario-lint-cli-snapshot-filters");
    write(
        &project_root.join("qa/scenarios/filtered.json"),
        &json!({
            "name": "filtered",
            "scene": "res://scene.tscn",
            "snapshots": {
                "target": "qa:panel",
                "max_depth": -1,
                "metadata_keys": ["qa_id"]
            },
            "steps": [{"snapshot": "filtered"}]
        })
        .to_string(),
    );

    let (status, body) = run_scenario_lint(&project_root, &["--scenario", "filtered", "--strict"]);

    assert_eq!(status.code(), Some(1));
    assert_eq!(body["ok"], false);
    assert_eq!(body["failures"][0]["type"], "scenario_validation_failed");
    assert!(
        body["failures"][0]["message"]
            .as_str()
            .unwrap()
            .contains("snapshots.max_depth must be a non-negative integer")
    );

    fs::remove_dir_all(project_root).unwrap();
}

fn sample_project_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples/sample_project")
        .canonicalize()
        .expect("sample project must exist")
}

#[test]
fn scenario_lint_sample_project_tree_is_clean_including_templates() {
    let project_root = sample_project_root();

    let (status, body) = run_scenario_lint(&project_root, &[]);

    // Warning-only lint keeps exit status 0; the template fixtures must not add
    // any contract error such as `invalid_scenario_file`.
    assert!(status.success());
    assert_eq!(body["ok"], true);
    assert_eq!(body["data"]["summary"]["errors"], 0);
    let findings = body["data"]["findings"].as_array().unwrap();
    assert!(
        !findings
            .iter()
            .any(|finding| finding["rule_id"] == "invalid_scenario_file")
    );
}
