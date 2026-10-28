use godot_qa_core::config::ProjectConfig;
use godot_qa_scenario::lint::{LintOptions, run_scenario_lint};
use serde_json::json;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

fn temp_dir(name: &str) -> PathBuf {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let path = std::env::temp_dir().join(format!("godot-qa-{name}-{unique}"));
    fs::create_dir_all(&path).unwrap();
    path
}

fn write(path: &Path, contents: &str) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(path, contents).unwrap();
}

fn config(root: &Path) -> ProjectConfig {
    ProjectConfig {
        project_root: root.to_path_buf(),
        repo_root: root.to_path_buf(),
        verbose: false,
        godot_binary: None,
        qa_dir: root.join("qa"),
        artifacts_dir: root.join(".qa/artifacts"),
        scenarios_dir: root.join("qa/scenarios"),
        addon_dir: root.join("addons/godot_qa"),
        repo_addon_dir: root.join("addons/godot_qa"),
        addon_identity_cache: Default::default(),
        godot_launch_prefix: Vec::new(),
        godot_windowed_launch_prefix: Vec::new(),
        godot_args: Vec::new(),
        godot_project_args: Vec::new(),
        godot_launch_mode: godot_qa_core::config::GodotLaunchMode::Auto,
        godot_audio_mode: godot_qa_core::config::GodotAudioMode::Auto,
        scenario_warm_default: false,
        scenario_warm_strict_default: false,
        scenario_jobs_default: None,
        project_config_failure: None,
        artifacts_keep_runs: None,
        scenario_schedule_default: None,
        log_ignore_patterns: Vec::new(),
        config_warnings: Vec::new(),
    }
}

#[test]
fn scenario_lint_warnings_only_exit_zero() {
    let root = temp_dir("scenario-lint-warning");
    write(
        &root.join("qa/scenarios/sample.json"),
        &json!({
            "name": "sample",
            "scene": "res://scene.tscn",
            "steps": [],
            "unknown": true
        })
        .to_string(),
    );

    let result = run_scenario_lint(&config(&root), LintOptions::default());
    assert!(result.ok);
    assert_eq!(result.exit_code, 0);
    assert_eq!(result.command, "scenario lint");
    assert_eq!(result.failures.len(), 0);
    assert_eq!(result.data["summary"]["warnings"], 1);
    assert_eq!(result.data["summary"]["errors"], 0);
}

#[test]
fn scenario_lint_strict_warnings_exit_nonzero() {
    let root = temp_dir("scenario-lint-strict");
    write(
        &root.join("qa/scenarios/sample.json"),
        &json!({
            "name": "sample",
            "scene": "res://scene.tscn",
            "steps": [],
            "unknown": true
        })
        .to_string(),
    );

    let result = run_scenario_lint(
        &config(&root),
        LintOptions {
            scenario: None,
            strict: true,
        },
    );
    assert!(!result.ok);
    assert_eq!(result.exit_code, 1);
    assert_eq!(result.data["summary"]["warnings"], 1);
}

#[test]
fn scenario_lint_remains_static_and_excludes_visual_risk_findings() {
    let root = temp_dir("scenario-lint-no-visual-risk");
    write(
        &root.join("qa/scenarios/visual_snapshot.json"),
        &json!({
            "name": "visual_snapshot",
            "scene": "res://scene.tscn",
            "snapshots": {"save_screenshot": true},
            "steps": [{"snapshot": "after"}],
            "unknown": true
        })
        .to_string(),
    );

    let result = run_scenario_lint(&config(&root), LintOptions::default());

    assert!(result.ok);
    assert_eq!(result.exit_code, 0);
    assert_eq!(result.data["summary"]["warnings"], 1);
    assert!(result.data.get("visual_risk").is_none());
    let result_json = serde_json::to_string(&result.data).unwrap();
    assert!(!result_json.contains("visual-risk."));
    assert!(!result_json.contains("screenshot_low_information"));

    let strict_result = run_scenario_lint(
        &config(&root),
        LintOptions {
            scenario: None,
            strict: true,
        },
    );
    assert!(!strict_result.ok);
    assert_eq!(strict_result.exit_code, 1);
    assert_eq!(strict_result.data["summary"]["warnings"], 1);
    assert_eq!(strict_result.data["summary"]["strict"], true);
    assert!(strict_result.data.get("visual_risk").is_none());
    let strict_result_json = serde_json::to_string(&strict_result.data).unwrap();
    assert!(!strict_result_json.contains("visual-risk."));
}

#[test]
fn scenario_lint_invalid_scenario_error_exit_nonzero() {
    let root = temp_dir("scenario-lint-invalid");
    write(
        &root.join("qa/scenarios/bad.json"),
        "{\"name\":\"bad\",\"scene\":\"res://scene.tscn\",\"steps\":[]",
    );

    let result = run_scenario_lint(&config(&root), LintOptions::default());
    assert!(!result.ok);
    assert_eq!(result.exit_code, 1);
    assert_eq!(result.data["summary"]["errors"], 1);
    assert_eq!(result.failures[0].kind, "scenario_parse_failed");
}

#[test]
fn scenario_lint_missing_selected_scenario_error() {
    let root = temp_dir("scenario-lint-missing-selected");
    write(
        &root.join("qa/scenarios/sample.json"),
        &json!({"name":"sample","scene":"res://scene.tscn","steps":[]}).to_string(),
    );

    let result = run_scenario_lint(
        &config(&root),
        LintOptions {
            scenario: Some("missing".to_string()),
            strict: false,
        },
    );
    assert!(!result.ok);
    assert_eq!(result.exit_code, 1);
    assert_eq!(result.data["summary"]["errors"], 1);
    assert_eq!(result.failures[0].kind, "scenario_not_found");
}

#[test]
fn scenario_lint_duplicate_scenario_name_error() {
    let root = temp_dir("scenario-lint-duplicate");
    write(
        &root.join("qa/scenarios/a.yaml"),
        "name: duplicate\nscene: res://scene.tscn\nsteps: []\n",
    );
    write(
        &root.join("qa/scenarios/b.yaml"),
        "name: duplicate\nscene: res://scene.tscn\nsteps: []\n",
    );

    let result = run_scenario_lint(&config(&root), LintOptions::default());
    assert!(!result.ok);
    assert_eq!(result.exit_code, 1);
    assert!(
        result.data["findings"]
            .as_array()
            .unwrap()
            .iter()
            .any(|f| f["rule_id"] == "duplicate_scenario_name")
    );
}

#[test]
fn scenario_lint_manual_play_rules_emit_expected_ids() {
    let root = temp_dir("scenario-lint-manual-rules");
    write(
        &root.join("qa/scenarios/rules.json"),
        &json!({
            "name":"rules",
            "scene":"res://scene.tscn",
            "lint": {"visual_coverage": true},
            "steps":[
                {"click":"qa:start"},
                {"set_property":{"target":"qa:player","property":"visible","value":true}},
                {"snapshot":"after_click"}
            ],
            "assert":[
                {"text_contains":{"target":"qa:debug_label","expected":"state token"}}
            ]
        })
        .to_string(),
    );

    write(
        &root.join("qa/scenarios/rules_with_screenshots.json"),
        &json!({
            "name":"rules_with_screenshots",
            "scene":"res://scene.tscn",
            "lint": {"visual_coverage": true},
            "snapshots": {"save_screenshot": true},
            "steps":[{"click":"qa:start"}, {"snapshot":"after_click"}],
            "assert":[{"text_contains":{"target":"qa:debug_label","expected":"debug state"}}]
        })
        .to_string(),
    );

    let result = run_scenario_lint(&config(&root), LintOptions::default());
    assert!(result.ok);
    let findings = result.data["findings"].as_array().unwrap();
    let rule_ids: Vec<&str> = findings
        .iter()
        .filter_map(|f| f["rule_id"].as_str())
        .collect();
    for rule in [
        "GQA-LINT-001",
        "GQA-LINT-002",
        "GQA-LINT-003",
        "GQA-LINT-004",
        "GQA-LINT-005",
    ] {
        assert!(
            rule_ids.contains(&rule),
            "missing expected rule {rule}; got {:?}",
            rule_ids
        );
        let finding = findings.iter().find(|f| f["rule_id"] == rule).unwrap();
        assert_eq!(finding["severity"], "warning");
        let scenario_path = finding["scenario_path"].as_str().unwrap();
        assert!(
            scenario_path.contains("rules.json")
                || scenario_path.contains("rules_with_screenshots.json")
        );
        assert!(finding["reason"].as_str().unwrap().len() > 4);
        assert!(finding["suggested_fix"].as_str().unwrap().len() > 4);
    }
}

#[test]
fn scenario_lint_fixture_before_input_does_not_warn_masking() {
    let root = temp_dir("scenario-lint-fixture-before-input");
    write(
        &root.join("qa/scenarios/fixture_ok.json"),
        &json!({
            "name":"fixture_ok",
            "scene":"res://scene.tscn",
            "steps":[
                {"set_property":{"target":"qa:player","property":"visible","value":true}},
                {"click":"qa:start"}
            ],
            "assert":[{"no_errors": true}]
        })
        .to_string(),
    );

    let result = run_scenario_lint(&config(&root), LintOptions::default());
    let findings = result.data["findings"].as_array().unwrap();
    assert!(!findings.iter().any(|f| f["rule_id"] == "GQA-LINT-001"));
}

#[test]
fn scenario_lint_treats_drive_policy_as_player_input() {
    let root = temp_dir("scenario-lint-drive-policy-input");
    // drive_policy is policy-driven real input: it must never be flagged as a
    // fixture/state mutation itself, and a mutation placed after it must still
    // warn exactly as if it followed a click or joypad step.
    write(
        &root.join("qa/scenarios/policy_driven.json"),
        &json!({
            "name":"policy_driven",
            "scene":"res://scene.tscn",
            "steps":[
                {"drive_policy":{"policy":"crew_bot","devices":[0],"max_frames":600}},
                {"set_property":{"target":"qa:player","property":"visible","value":true}}
            ],
            "assert":[{"no_errors": true}]
        })
        .to_string(),
    );

    let result = run_scenario_lint(&config(&root), LintOptions::default());
    let findings = result.data["findings"].as_array().unwrap();
    let masking: Vec<&serde_json::Value> = findings
        .iter()
        .filter(|f| f["rule_id"] == "GQA-LINT-001")
        .collect();
    assert_eq!(masking.len(), 1, "findings: {findings:?}");
    assert_eq!(masking[0]["step_index"], 1);
    assert_eq!(masking[0]["target"], "step[1]");
}

#[test]
fn scenario_lint_treats_input_joy_state_as_player_input() {
    let root = temp_dir("scenario-lint-joy-state-input");
    // input_joy_state is open-loop real joypad input: it must never be flagged
    // as a fixture/state mutation itself, and a mutation placed after it must
    // still warn exactly as if it followed a click or another joypad step.
    write(
        &root.join("qa/scenarios/joy_state.json"),
        &json!({
            "name":"joy_state",
            "scene":"res://scene.tscn",
            "steps":[
                {"input_joy_state":{"inputs":[{"device":0,"button":{"name":"a","pressed":true}}]}},
                {"set_property":{"target":"qa:player","property":"visible","value":true}}
            ],
            "assert":[{"no_errors": true}]
        })
        .to_string(),
    );

    let result = run_scenario_lint(&config(&root), LintOptions::default());
    let findings = result.data["findings"].as_array().unwrap();
    let masking: Vec<&serde_json::Value> = findings
        .iter()
        .filter(|f| f["rule_id"] == "GQA-LINT-001")
        .collect();
    assert_eq!(masking.len(), 1, "findings: {findings:?}");
    assert_eq!(masking[0]["step_index"], 1);
    assert_eq!(masking[0]["target"], "step[1]");
}

#[test]
fn scenario_lint_permits_joy_disconnect_after_input() {
    let root = temp_dir("scenario-lint-joy-disconnect-after-input");
    // joy_connect/joy_disconnect are setup, not player input: the canonical
    // controller-drop scenario connects pads, presses a button, then disconnects
    // one mid-scenario. GQA-LINT-001 must NOT flag joy_disconnect placed after
    // input (it only flags the four state-mutation steps), and the joypad-
    // connection steps must not themselves count as the interactive marker or a
    // mutation. So this whole scenario is clean of GQA-LINT-001 findings.
    write(
        &root.join("qa/scenarios/joy_drop.json"),
        &json!({
            "name":"joy_drop",
            "scene":"res://scene.tscn",
            "steps":[
                {"joy_connect":{"devices":[0,1]}},
                {"input_joy_button":"a"},
                {"joy_disconnect":{"devices":[1]}}
            ],
            "assert":[{"visible":"qa:pause_menu"}]
        })
        .to_string(),
    );

    let result = run_scenario_lint(&config(&root), LintOptions::default());
    let findings = result.data["findings"].as_array().unwrap();
    let masking: Vec<&serde_json::Value> = findings
        .iter()
        .filter(|f| f["rule_id"] == "GQA-LINT-001")
        .collect();
    assert!(masking.is_empty(), "findings: {findings:?}");
}

#[test]
fn scenario_lint_suppression_hides_warning_and_reports_metadata() {
    let root = temp_dir("scenario-lint-suppression-ok");
    write(
        &root.join("qa/scenarios/suppressed.json"),
        &json!({
            "name":"suppressed",
            "scene":"res://scene.tscn",
            "lint": {
                "suppressions": {
                    "GQA-LINT-004": {"reason":"Intentional smoke flow", "target": null}
                }
            },
            "steps":[{"click":"qa:start"}],
            "assert":[{"text_contains":{"target":"qa:title","expected":"Menu"}}]
        })
        .to_string(),
    );

    let result = run_scenario_lint(&config(&root), LintOptions::default());
    assert!(result.ok);
    assert_eq!(result.data["summary"]["suppressed"], 1);
    assert!(
        !result.data["findings"]
            .as_array()
            .unwrap()
            .iter()
            .any(|f| f["rule_id"] == "GQA-LINT-004")
    );
    let suppressed = result.data["suppressed_findings"].as_array().unwrap();
    assert_eq!(suppressed.len(), 1);
    assert_eq!(suppressed[0]["rule_id"], "GQA-LINT-004");
    assert_eq!(
        suppressed[0]["suppression"]["reason"],
        "Intentional smoke flow"
    );
}

#[test]
fn scenario_lint_suppression_does_not_hide_other_scenarios_findings() {
    let root = temp_dir("scenario-lint-suppression-scope");
    write(&root.join("ui/ids.gd"), "# qa:start\n# qa:title\n");
    write(
        &root.join("qa/scenarios/a_unsuppressed.json"),
        &json!({
            "name":"a_unsuppressed",
            "scene":"res://scene.tscn",
            "steps":[{"click":"qa:start"}],
            "assert":[{"text_contains":{"target":"qa:title","expected":"Menu"}}]
        })
        .to_string(),
    );
    write(
        &root.join("qa/scenarios/z_suppressed.json"),
        &json!({
            "name":"z_suppressed",
            "scene":"res://scene.tscn",
            "lint": {
                "suppressions": {
                    "GQA-LINT-004": {"reason":"Intentional smoke flow"}
                }
            },
            "steps":[{"click":"qa:start"}],
            "assert":[{"text_contains":{"target":"qa:title","expected":"Menu"}}]
        })
        .to_string(),
    );

    let result = run_scenario_lint(&config(&root), LintOptions::default());
    assert!(result.ok);
    let findings = result.data["findings"].as_array().unwrap();
    let active_missing_negative: Vec<_> = findings
        .iter()
        .filter(|f| f["rule_id"] == "GQA-LINT-004")
        .collect();
    assert_eq!(active_missing_negative.len(), 1);
    assert_eq!(active_missing_negative[0]["scenario"], "a_unsuppressed");

    let suppressed = result.data["suppressed_findings"].as_array().unwrap();
    assert!(
        suppressed
            .iter()
            .any(|f| f["rule_id"] == "GQA-LINT-004" && f["scenario"] == "z_suppressed")
    );
}

#[test]
fn scenario_lint_suppression_missing_reason_is_error() {
    let root = temp_dir("scenario-lint-suppression-missing-reason");
    write(
        &root.join("qa/scenarios/bad_suppression.yaml"),
        "name: bad_suppression\nscene: res://scene.tscn\nsteps: []\nlint:\n  suppressions:\n    GQA-LINT-001:\n      reason: \"\"\n",
    );

    let result = run_scenario_lint(&config(&root), LintOptions::default());
    assert!(!result.ok);
    assert_eq!(result.exit_code, 1);
    assert!(
        result.data["findings"]
            .as_array()
            .unwrap()
            .iter()
            .any(|f| { f["rule_id"] == "malformed_suppression" && f["severity"] == "error" })
    );
}

#[test]
fn scenario_lint_suppression_cannot_hide_contract_error() {
    let root = temp_dir("scenario-lint-suppression-contract");
    write(
        &root.join("qa/scenarios/a.yaml"),
        "name: duplicate\nscene: res://scene.tscn\nsteps: []\nlint:\n  suppressions:\n    duplicate_scenario_name:\n      reason: \"try to suppress\"\n",
    );
    write(
        &root.join("qa/scenarios/b.yaml"),
        "name: duplicate\nscene: res://scene.tscn\nsteps: []\n",
    );

    let result = run_scenario_lint(&config(&root), LintOptions::default());
    assert!(!result.ok);
    let findings = result.data["findings"].as_array().unwrap();
    assert!(
        findings
            .iter()
            .any(|f| f["rule_id"] == "duplicate_scenario_name")
    );
    assert!(
        findings
            .iter()
            .any(|f| f["rule_id"] == "malformed_suppression")
    );
}

#[test]
fn scenario_lint_references_detects_missing_and_known_qa_ids() {
    let root = temp_dir("scenario-lint-references");
    write(
        &root.join("ui/known.gd"),
        "func _ready():\n    set_meta(\"qa_id\", \"known_button\")\n",
    );

    write(
        &root.join("qa/scenarios/base.yaml"),
        "name: base\nscene: res://scene.tscn\nsteps: []\n",
    );

    write(
        &root.join("qa/scenarios/refs.yaml"),
        "name: refs\nscene: res://scene.tscn\nsteps:\n  - click: qa:known_button\n  - click: qa:unknown_button\nassert:\n  - visible: qa:unknown_button\n",
    );

    let result = run_scenario_lint(&config(&root), LintOptions::default());
    let findings = result.data["findings"].as_array().unwrap();
    // The obsolete `missing_scenario_reference` rule is gone: `extends` no longer
    // names a scenario, so it is never reported here.
    assert!(
        !findings
            .iter()
            .any(|f| f["rule_id"] == "missing_scenario_reference")
    );
    assert!(
        findings.iter().any(
            |f| f["rule_id"] == "missing_qa_id_reference" && f["target"] == "qa:unknown_button"
        )
    );
    assert!(
        !findings
            .iter()
            .any(|f| f["rule_id"] == "missing_qa_id_reference" && f["target"] == "qa:known_button")
    );
}

#[test]
fn scenario_lint_missing_extends_base_reports_invalid_scenario_file() {
    let root = temp_dir("scenario-lint-missing-base");
    // A concrete scenario whose `extends` base does not exist now fails at load
    // with `scenario_extends_missing`, surfaced by lint as `invalid_scenario_file`.
    write(
        &root.join("qa/scenarios/child.yaml"),
        "name: child\nextends: no_such_base.yaml\n",
    );

    let result = run_scenario_lint(&config(&root), LintOptions::default());
    assert!(!result.ok);
    let findings = result.data["findings"].as_array().unwrap();
    let finding = findings
        .iter()
        .find(|f| f["rule_id"] == "invalid_scenario_file")
        .expect("expected invalid_scenario_file for missing extends base");
    assert!(
        finding["scenario_path"]
            .as_str()
            .unwrap()
            .ends_with("child.yaml")
    );
    assert!(
        result
            .failures
            .iter()
            .any(|f| f.kind == "scenario_extends_missing")
    );
}

#[test]
fn scenario_lint_template_structural_error_reports_invalid_scenario_file() {
    let root = temp_dir("scenario-lint-template-error");
    // Abstract templates are excluded from the normal lint loop but are validated
    // structurally: a malformed `vars` block fails as `invalid_scenario_file`.
    write(
        &root.join("qa/scenarios/templates/broken.base.yaml"),
        "abstract: true\nvars:\n  - not\n  - an\n  - object\nscene: res://x.tscn\n",
    );

    let result = run_scenario_lint(&config(&root), LintOptions::default());
    assert!(!result.ok);
    let findings = result.data["findings"].as_array().unwrap();
    let finding = findings
        .iter()
        .find(|f| f["rule_id"] == "invalid_scenario_file")
        .expect("expected invalid_scenario_file for malformed template vars");
    assert!(
        finding["scenario_path"]
            .as_str()
            .unwrap()
            .ends_with("broken.base.yaml")
    );
    assert!(
        result
            .failures
            .iter()
            .any(|f| f.kind == "scenario_vars_invalid")
    );
}

#[test]
fn scenario_lint_abstract_templates_are_exempt_from_duplicate_scenario_name() {
    let root = temp_dir("scenario-lint-template-duplicate");
    // Two abstract templates share the stem `base` in different subdirectories.
    // Because abstract files are excluded from listing, this is NOT a duplicate.
    write(
        &root.join("qa/scenarios/templates/a/base.yaml"),
        "abstract: true\nscene: res://x.tscn\nsteps: []\n",
    );
    write(
        &root.join("qa/scenarios/templates/b/base.yaml"),
        "abstract: true\nscene: res://x.tscn\nsteps: []\n",
    );
    write(
        &root.join("qa/scenarios/concrete.yaml"),
        "name: concrete\nscene: res://x.tscn\nsteps: []\n",
    );

    let result = run_scenario_lint(&config(&root), LintOptions::default());
    let findings = result.data["findings"].as_array().unwrap();
    assert!(
        !findings
            .iter()
            .any(|f| f["rule_id"] == "duplicate_scenario_name")
    );
}

#[test]
fn scenario_lint_gqa_rule_fires_on_inherited_field() {
    let root = temp_dir("scenario-lint-inherited-field");
    // The base claims project-flag coverage via `lint.validates_project_flag`.
    // The concrete child inherits that block but supplies no `project_args`, so
    // GQA-LINT-008 must fire against the resolved (inherited) document.
    write(
        &root.join("qa/scenarios/templates/flag.base.yaml"),
        "abstract: true\nscene: res://x.tscn\nsteps: []\nlint:\n  validates_project_flag: true\n",
    );
    write(
        &root.join("qa/scenarios/flag_child.yaml"),
        "name: flag_child\nextends: templates/flag.base.yaml\n",
    );

    let result = run_scenario_lint(&config(&root), LintOptions::default());
    let findings = result.data["findings"].as_array().unwrap();
    let finding = findings
        .iter()
        .find(|f| f["rule_id"] == "GQA-LINT-008" && f["scenario"] == "flag_child")
        .expect("expected GQA-LINT-008 to fire on the inherited lint block");
    assert_eq!(finding["severity"], "warning");
}

#[test]
fn scenario_lint_references_accepts_spawned_qa_ids() {
    let root = temp_dir("scenario-lint-references-spawn");
    write(
        &root.join("qa/scenarios/spawn.yaml"),
        "name: spawn\nscene: res://scene.tscn\nsteps:\n  - spawn_scene:\n      scene: res://x.tscn\n      qa_id: spawned_id\n  - click: qa:spawned_id\nassert:\n  - visible: qa:spawned_id\n",
    );

    let result = run_scenario_lint(&config(&root), LintOptions::default());
    let findings = result.data["findings"].as_array().unwrap();
    assert!(
        !findings
            .iter()
            .any(|f| f["rule_id"] == "missing_qa_id_reference" && f["target"] == "qa:spawned_id")
    );
}

#[test]
fn scenario_lint_project_flag_claim_warns_without_project_args() {
    let root = temp_dir("scenario-lint-project-flag-warns");
    write(
        &root.join("qa/scenarios/project_flag.json"),
        &json!({
            "name": "project_flag",
            "scene": "res://scene.tscn",
            "lint": {"validates_project_flag": true},
            "steps": []
        })
        .to_string(),
    );

    let result = run_scenario_lint(&config(&root), LintOptions::default());
    assert!(result.ok);
    let findings = result.data["findings"].as_array().unwrap();
    let finding = findings
        .iter()
        .find(|f| f["rule_id"] == "GQA-LINT-008")
        .expect("expected project flag lint warning");
    assert_eq!(finding["severity"], "warning");
    assert_eq!(finding["scenario"], "project_flag");
    assert_eq!(finding["target"], "lint.validates_project_args");
    assert_eq!(finding["suppressible"], true);
}

#[test]
fn scenario_lint_project_flag_claim_passes_with_project_args() {
    let root = temp_dir("scenario-lint-project-flag-project-args");
    write(
        &root.join("qa/scenarios/project_flag.json"),
        &json!({
            "name": "project_flag",
            "scene": "res://scene.tscn",
            "lint": {"validates_project_args": true},
            "project_args": ["--save-slot", "qa"],
            "steps": []
        })
        .to_string(),
    );

    let result = run_scenario_lint(&config(&root), LintOptions::default());
    let findings = result.data["findings"].as_array().unwrap();
    assert!(!findings.iter().any(|f| f["rule_id"] == "GQA-LINT-008"));
}

#[test]
fn scenario_lint_project_flag_claim_passes_with_equivalent_fixture_metadata() {
    let root = temp_dir("scenario-lint-project-flag-fixture");
    write(
        &root.join("qa/scenarios/project_flag_fixture.json"),
        &json!({
            "name": "project_flag_fixture",
            "scene": "res://scene.tscn",
            "lint": {
                "validates_project_flag": true,
                "equivalent_project_args_fixture": "fixture injects --save-slot before startup"
            },
            "steps": []
        })
        .to_string(),
    );

    let result = run_scenario_lint(&config(&root), LintOptions::default());
    let findings = result.data["findings"].as_array().unwrap();
    assert!(!findings.iter().any(|f| f["rule_id"] == "GQA-LINT-008"));
}

#[test]
fn scenario_lint_project_flag_claim_warning_is_suppressible() {
    let root = temp_dir("scenario-lint-project-flag-suppressed");
    write(
        &root.join("qa/scenarios/project_flag_suppressed.json"),
        &json!({
            "name": "project_flag_suppressed",
            "scene": "res://scene.tscn",
            "lint": {
                "validates_project_args": true,
                "suppressions": {
                    "GQA-LINT-008": {"reason": "legacy fixture wiring covers the flag"}
                }
            },
            "steps": []
        })
        .to_string(),
    );

    let result = run_scenario_lint(&config(&root), LintOptions::default());
    assert!(result.ok);
    assert_eq!(result.data["summary"]["suppressed"], 1);
    assert!(
        !result.data["findings"]
            .as_array()
            .unwrap()
            .iter()
            .any(|f| f["rule_id"] == "GQA-LINT-008")
    );
    assert!(
        result.data["suppressed_findings"]
            .as_array()
            .unwrap()
            .iter()
            .any(|f| f["rule_id"] == "GQA-LINT-008")
    );
}

#[test]
fn scenario_lint_sample_fixture_masking_demo_emits_structured_warning() {
    let repo_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .unwrap();
    let project_root = repo_root.join("examples/sample_project");
    let project_root = project_root.canonicalize().unwrap();
    let cfg = ProjectConfig {
        project_root: project_root.clone(),
        repo_root: repo_root.clone(),
        verbose: false,
        godot_binary: None,
        qa_dir: project_root.join("qa"),
        artifacts_dir: project_root.join(".qa/artifacts"),
        scenarios_dir: project_root.join("qa/scenarios"),
        addon_dir: project_root.join("addons/godot_qa"),
        repo_addon_dir: repo_root.join("addons/godot_qa"),
        addon_identity_cache: Default::default(),
        godot_launch_prefix: Vec::new(),
        godot_windowed_launch_prefix: Vec::new(),
        godot_args: Vec::new(),
        godot_project_args: Vec::new(),
        godot_launch_mode: godot_qa_core::config::GodotLaunchMode::Auto,
        godot_audio_mode: godot_qa_core::config::GodotAudioMode::Auto,
        scenario_warm_default: false,
        scenario_warm_strict_default: false,
        scenario_jobs_default: None,
        project_config_failure: None,
        artifacts_keep_runs: None,
        scenario_schedule_default: None,
        log_ignore_patterns: Vec::new(),
        config_warnings: Vec::new(),
    };

    let result = run_scenario_lint(
        &cfg,
        LintOptions {
            scenario: Some("lint_fixture_masking_demo".to_string()),
            strict: false,
        },
    );
    assert!(result.ok);
    let findings = result.data["findings"].as_array().unwrap();
    let masking = findings
        .iter()
        .find(|f| f["rule_id"] == "GQA-LINT-001")
        .expect("expected fixture masking warning");
    assert_eq!(masking["severity"], "warning");
    assert_eq!(masking["scenario"], "lint_fixture_masking_demo");
    assert!(masking["target"].is_string());
    assert!(masking["reason"].as_str().unwrap().len() > 4);
    assert!(masking["suggested_fix"].as_str().unwrap().len() > 4);
}

#[test]
fn scenario_lint_treats_saved_joypad_steps_as_player_input() {
    let root = temp_dir("scenario-lint-joypad-player-input");
    write(
        &root.join("qa/scenarios/joypad.json"),
        &json!({
            "name": "joypad",
            "scene": "res://scene.tscn",
            "steps": [
                {"input_joy_button": {"button": "start", "device": 2, "frames": 2}},
                {"input_joy_motion": {"axis": "left_x", "value": 1.0, "device": 2, "frames": 2}}
            ],
            "assert": [
                {"text_contains": {"target": "qa:debug_status", "expected": "joined"}}
            ]
        })
        .to_string(),
    );

    let result = run_scenario_lint(&config(&root), LintOptions::default());
    assert!(result.ok);
    let findings = result.data["findings"].as_array().unwrap();
    assert!(
        findings.iter().any(|f| f["rule_id"] == "GQA-LINT-002"),
        "joypad input steps should make debug-only assertion lint active"
    );
    assert!(
        findings.iter().any(|f| f["rule_id"] == "GQA-LINT-004"),
        "joypad input steps should make negative-guard lint active"
    );
}

#[test]
fn scenario_lint_does_not_treat_review_suggestions_as_assertions_or_suppressions() {
    let root = temp_dir("scenario-lint-review-suggestions");
    write(
        &root.join("qa/scenarios/recorder_negative_suggestions_demo.json"),
        &json!({
            "name": "recorder_negative_suggestions_demo",
            "scene": "res://ui/fixtures/recorder_negative_suggestions.tscn",
            "steps": [{"click": "qa:start"}],
            "assert": [],
            "review": {
                "status": "needs_review",
                "generated_by": "scenario record",
                "suggested_assertions": [
                    {"assertion": {"text_equals": {"target": "qa:qa_invalid_action_result", "expected": "invalid:no-op"}}}
                ]
            }
        })
        .to_string(),
    );

    let result = run_scenario_lint(
        &config(&root),
        LintOptions {
            scenario: Some("recorder_negative_suggestions_demo".to_string()),
            strict: false,
        },
    );

    assert!(result.ok);
    let findings = result.data["findings"].as_array().unwrap();
    assert!(
        findings.iter().any(|f| f["rule_id"] == "GQA-LINT-004"),
        "expected missing assertion warning to remain when assert is empty"
    );
    assert_eq!(result.data["summary"]["suppressed"], 0);
}
