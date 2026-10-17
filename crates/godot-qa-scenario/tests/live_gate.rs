use godot_qa_core::config::ProjectConfig;
use godot_qa_core::results::{CommandResult, FailureDetail};
use godot_qa_scenario::live_gate::{
    LiveGateRunHooks, LiveGateRunOptions, list_live_gates, load_live_gate_manifest,
    run_live_gate_with_hooks, validate_live_gate_manifest,
};
use serde_json::{Map, Value, json};
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

fn make_config(root: &Path) -> ProjectConfig {
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

fn make_run_config(root: &Path) -> ProjectConfig {
    let mut config = make_config(root);
    config.godot_binary = Some(
        godot_qa_test_support::fixture(&root.join("fake-godot"), &[])
            .to_string_lossy()
            .into_owned(),
    );
    fs::create_dir_all(&config.addon_dir).unwrap();
    config
}

fn ok_result(command: &str, data: Value) -> CommandResult {
    CommandResult {
        ok: true,
        command: command.to_string(),
        message: "ok".to_string(),
        data: data.as_object().cloned().unwrap_or_default(),
        failures: Vec::new(),
        exit_code: 0,
    }
}

fn fail_result(command: &str, kind: &str, target: Option<&str>) -> CommandResult {
    CommandResult {
        ok: false,
        command: command.to_string(),
        message: "failed".to_string(),
        data: Map::new(),
        failures: vec![FailureDetail {
            kind: kind.to_string(),
            command: Some(command.to_string()),
            target: target.map(str::to_string),
            message: Some(kind.to_string()),
            actual: target.map(|target| json!({"target": target})),
            ..FailureDetail::default()
        }],
        exit_code: 1,
    }
}

#[allow(clippy::result_large_err)]
fn load_attached_session(_: &ProjectConfig, _: &str) -> Result<Value, FailureDetail> {
    Ok(json!({"session_id": "attached"}))
}

#[test]
fn live_gate_manifest_contract_valid_manifests_and_listing_metadata() {
    let root = temp_dir("live-gate-manifest");
    let json_path = root.join("qa/live_gates/hidden_overlay.json");
    let yaml_path = root.join("qa/live_gates/nested/menu_gate.yaml");
    write(
        &json_path,
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
    write(
        &yaml_path,
        "name: menu_gate\nscene: res://ui/settings.tscn\noverlay: hidden\nanchors:\n  required:\n    - settings_panel\nmethods:\n  - qa_get_settings_state\nprobes:\n  - qa_settings_ready\n",
    );

    assert_eq!(
        load_live_gate_manifest(&json_path).unwrap()["name"],
        "hidden_overlay"
    );
    assert_eq!(
        load_live_gate_manifest(&yaml_path).unwrap()["name"],
        "menu_gate"
    );

    let listed = list_live_gates(&make_config(&root));
    assert!(listed.failures.is_empty());
    assert_eq!(listed.gates.len(), 2);
    assert_eq!(listed.gates[0].name, "hidden_overlay");
    assert_eq!(listed.gates[0].overlay_summary(), "hidden");
    assert_eq!(listed.gates[0].required_anchors.len(), 2);
    assert_eq!(listed.gates[0].optional_anchors.len(), 1);
    assert_eq!(listed.gates[0].methods, vec!["qa_get_menu_state"]);
    assert_eq!(listed.gates[0].probes, vec!["qa_menu_ready"]);

    fs::remove_dir_all(root).unwrap();
}

#[test]
fn live_gate_manifest_contract_missing_live_gates_dir_lists_empty() {
    let root = temp_dir("live-gate-missing-dir");
    let listed = list_live_gates(&make_config(&root));
    assert!(listed.gates.is_empty());
    assert!(listed.failures.is_empty());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn live_gate_manifest_contract_reports_duplicate_names() {
    let root = temp_dir("live-gate-duplicate");
    write(
        &root.join("qa/live_gates/first.json"),
        r#"{"name":"duplicate","scene":"res://a.tscn"}"#,
    );
    write(
        &root.join("qa/live_gates/nested/second.json"),
        r#"{"name":"duplicate","scene":"res://b.tscn"}"#,
    );

    let listed = list_live_gates(&make_config(&root));
    assert_eq!(listed.gates.len(), 2);
    assert!(
        listed
            .failures
            .iter()
            .any(|f| f.kind == "duplicate_live_gate_name")
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn live_gate_manifest_contract_rejects_unsafe_names_and_anchors() {
    let root = temp_dir("live-gate-unsafe");
    let path = root.join("qa/live_gates/bad.json");

    let err = validate_live_gate_manifest(
        &json!({"name":"../escape","scene":"res://main.tscn"}),
        &path,
    )
    .unwrap_err();
    assert_eq!(err.kind, "live_gate_validation_failed");

    let err = validate_live_gate_manifest(
        &json!({
            "name":"safe",
            "scene":"res://main.tscn",
            "anchors":{"required":["../node"]}
        }),
        &path,
    )
    .unwrap_err();
    assert_eq!(err.kind, "unsafe_live_gate_anchor");
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn live_gate_manifest_contract_rejects_unsupported_keys() {
    let root = temp_dir("live-gate-unsupported");
    let path = root.join("qa/live_gates/bad.json");

    let err = validate_live_gate_manifest(
        &json!({"name":"safe","scene":"res://main.tscn","script":"echo unsafe"}),
        &path,
    )
    .unwrap_err();
    assert_eq!(err.kind, "live_gate_unsupported_key");

    let err = validate_live_gate_manifest(
        &json!({
            "name":"safe",
            "scene":"res://main.tscn",
            "anchors":{"unknown":["main"]}
        }),
        &path,
    )
    .unwrap_err();
    assert_eq!(err.kind, "live_gate_unsupported_key");
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn live_gate_manifest_contract_validates_required_optional_methods_and_probes() {
    let root = temp_dir("live-gate-fields");
    let path = root.join("qa/live_gates/bad.json");

    let err = validate_live_gate_manifest(
        &json!({
            "name":"safe",
            "scene":"res://main.tscn",
            "anchors":{"required":"main"}
        }),
        &path,
    )
    .unwrap_err();
    assert_eq!(err.kind, "live_gate_validation_failed");
    assert_eq!(err.expected, Some(json!("list")));

    let err = validate_live_gate_manifest(
        &json!({"name":"safe","scene":"res://main.tscn","methods":[""]}),
        &path,
    )
    .unwrap_err();
    assert_eq!(err.kind, "live_gate_validation_failed");

    let err = validate_live_gate_manifest(
        &json!({"name":"safe","scene":"res://main.tscn","probes":[7]}),
        &path,
    )
    .unwrap_err();
    assert_eq!(err.kind, "live_gate_validation_failed");
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn live_gate_run_contract() {
    let root = temp_dir("live-gate-run");
    write(
        &root.join("qa/live_gates/hidden_overlay.json"),
        &json!({
            "name": "hidden_overlay",
            "scene": "res://ui/main_menu.tscn",
            "overlay": "hidden",
            "anchors": {"required": ["main_menu", "missing_anchor"]},
            "methods": ["qa_get_menu_state"],
            "probes": ["qa_probe_surface"]
        })
        .to_string(),
    );
    let config = make_run_config(&root);
    let hooks = LiveGateRunHooks {
        session_start: &|_, scene, _, _| {
            assert_eq!(scene, Some("res://ui/main_menu.tscn"));
            ok_result("session start", json!({"session_id": "s1"}))
        },
        session_stop: &|_| ok_result("session stop", json!({})),
        load_session: &load_attached_session,
        protocol: &|_, command, protocol_command, params| {
            assert_eq!(command, "live-gate run");
            match protocol_command {
                godot_qa_runtime::SESSION_STATUS => ok_result(command, json!({"ready": true})),
                godot_qa_runtime::ERRORS_READ => ok_result(command, json!({"errors": []})),
                godot_qa_runtime::ASSERT_STATE_MATCHES => {
                    let method = params.get("method").and_then(Value::as_str).unwrap_or("");
                    let target = params.get("target").and_then(Value::as_str).unwrap_or("");
                    if method == "get_hidden_overlay_qa_state" {
                        ok_result(command, json!({"overlay": {"visible": false}}))
                    } else if target == "qa:main_menu" {
                        ok_result(command, json!({"method": method}))
                    } else {
                        fail_result(command, "target_not_found", Some(target))
                    }
                }
                godot_qa_runtime::NODE_INSPECT => {
                    let target = params.get("target").and_then(Value::as_str).unwrap_or("");
                    if target == "qa:main_menu" {
                        ok_result(command, json!({"resolved": "path:/root/Main/MainMenu"}))
                    } else {
                        fail_result(command, "target_not_found", Some(target))
                    }
                }
                godot_qa_runtime::ASSERT_SURFACE_PROBE => {
                    let target = params.get("target").and_then(Value::as_str).unwrap_or("");
                    fail_result(command, "surface_probe_mismatch", Some(target))
                }
                other => fail_result(command, other, None),
            }
        },
    };

    let result = run_live_gate_with_hooks(
        &config,
        "hidden_overlay",
        LiveGateRunOptions {
            viewport: "1920x1080".to_string(),
            ..LiveGateRunOptions::default()
        },
        &hooks,
    );

    assert!(!result.ok);
    assert_eq!(result.command, "live-gate run");
    let anchors = result.data["anchors"].as_array().unwrap();
    assert_eq!(anchors.len(), 2);
    assert_eq!(anchors[0]["name"], "main_menu");
    assert_eq!(anchors[0]["resolved"], "path:/root/Main/MainMenu");
    assert_eq!(anchors[0]["methods"][0]["name"], "qa_get_menu_state");
    assert_eq!(anchors[0]["probes"][0]["name"], "qa_probe_surface");
    assert_eq!(anchors[0]["status"], "failed");
    assert_eq!(anchors[1]["name"], "missing_anchor");
    assert_eq!(anchors[1]["status"], "failed");
    assert!(
        result
            .failures
            .iter()
            .any(|failure| failure.kind == "target_not_found")
    );
    assert!(
        result
            .failures
            .iter()
            .any(|failure| failure.kind == "surface_probe_mismatch")
    );
    assert!(result.data["artifacts"]["result"].as_str().is_some());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn live_gate_optional_anchor_contract() {
    let root = temp_dir("live-gate-optional");
    write(
        &root.join("qa/live_gates/optional.json"),
        &json!({
            "name": "optional",
            "scene": "res://ui/main_menu.tscn",
            "anchors": {"required": ["main_menu"], "optional": ["settings_button"]}
        })
        .to_string(),
    );
    let config = make_run_config(&root);
    let hooks = LiveGateRunHooks {
        session_start: &|_, _, _, _| ok_result("session start", json!({"session_id": "s1"})),
        session_stop: &|_| ok_result("session stop", json!({})),
        load_session: &load_attached_session,
        protocol: &|_, command, protocol_command, params| match protocol_command {
            godot_qa_runtime::SESSION_STATUS => ok_result(command, json!({"ready": true})),
            godot_qa_runtime::ERRORS_READ => ok_result(command, json!({"errors": []})),
            godot_qa_runtime::NODE_INSPECT => {
                let target = params.get("target").and_then(Value::as_str).unwrap_or("");
                if target == "qa:main_menu" {
                    ok_result(command, json!({"resolved": "path:/root/Main/MainMenu"}))
                } else {
                    fail_result(command, "target_not_found", Some(target))
                }
            }
            other => fail_result(command, other, None),
        },
    };

    let default_result = run_live_gate_with_hooks(
        &config,
        "optional",
        LiveGateRunOptions {
            viewport: "1920x1080".to_string(),
            ..LiveGateRunOptions::default()
        },
        &hooks,
    );
    assert!(default_result.ok);
    assert_eq!(default_result.data["anchors"][1]["status"], "warning");
    assert_eq!(
        default_result.data["anchors"][1]["warnings"][0]["type"],
        "target_not_found"
    );

    let strict_result = run_live_gate_with_hooks(
        &config,
        "optional",
        LiveGateRunOptions {
            strict: true,
            viewport: "1920x1080".to_string(),
            ..LiveGateRunOptions::default()
        },
        &hooks,
    );
    assert!(!strict_result.ok);
    assert_eq!(strict_result.data["anchors"][1]["status"], "failed");
    assert!(
        strict_result
            .failures
            .iter()
            .any(|failure| failure.kind == "target_not_found")
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn hidden_overlay_missing_anchor_contract() {
    let root = temp_dir("hidden-overlay-missing-anchor");
    write(
        &root.join("qa/live_gates/hidden_overlay.json"),
        &json!({
            "name": "hidden_overlay",
            "scene": "res://ui/main_menu.tscn",
            "overlay": "hidden",
            "anchors": {
                "required": ["missing_required", "also_missing_required"],
                "optional": ["missing_optional"]
            }
        })
        .to_string(),
    );
    let config = make_run_config(&root);
    let hooks = LiveGateRunHooks {
        session_start: &|_, _, _, _| ok_result("session start", json!({"session_id": "s1"})),
        session_stop: &|_| ok_result("session stop", json!({})),
        load_session: &load_attached_session,
        protocol: &|_, command, protocol_command, params| match protocol_command {
            godot_qa_runtime::SESSION_STATUS => ok_result(command, json!({"ready": true})),
            godot_qa_runtime::ERRORS_READ => ok_result(command, json!({"errors": []})),
            godot_qa_runtime::ASSERT_STATE_MATCHES => ok_result(
                command,
                json!({"overlay": {"hidden": true, "visible_in_tree": false}}),
            ),
            godot_qa_runtime::NODE_INSPECT => {
                let target = params.get("target").and_then(Value::as_str).unwrap_or("");
                fail_result(command, "target_not_found", Some(target))
            }
            other => fail_result(command, other, None),
        },
    };

    let result = run_live_gate_with_hooks(
        &config,
        "hidden_overlay",
        LiveGateRunOptions::default(),
        &hooks,
    );

    assert!(!result.ok);
    assert_eq!(result.exit_code, 1);
    assert!(result.failures.iter().any(|failure| {
        failure.kind == "target_not_found" || failure.kind == "missing_required_anchor"
    }));
    let anchors = result.data["anchors"].as_array().unwrap();
    assert_eq!(anchors[0]["status"], "failed");
    assert_eq!(anchors[1]["status"], "failed");
    assert_eq!(anchors[2]["status"], "warning");
    assert_eq!(result.data["warnings"].as_array().unwrap().len(), 1);
    assert_eq!(
        anchors
            .iter()
            .filter(|anchor| anchor["required"] == json!(true) && anchor["status"] == "failed")
            .count(),
        2
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn hidden_overlay_optional_anchor_warning_contract() {
    let root = temp_dir("hidden-overlay-optional-anchor");
    write(
        &root.join("qa/live_gates/hidden_overlay.json"),
        &json!({
            "name": "hidden_overlay",
            "scene": "res://ui/main_menu.tscn",
            "overlay": "hidden",
            "anchors": {"required": ["main_menu"], "optional": ["settings_button"]}
        })
        .to_string(),
    );
    let config = make_run_config(&root);
    let hooks = LiveGateRunHooks {
        session_start: &|_, _, _, _| ok_result("session start", json!({"session_id": "s1"})),
        session_stop: &|_| ok_result("session stop", json!({})),
        load_session: &load_attached_session,
        protocol: &|_, command, protocol_command, params| match protocol_command {
            godot_qa_runtime::SESSION_STATUS => ok_result(command, json!({"ready": true})),
            godot_qa_runtime::ERRORS_READ => ok_result(command, json!({"errors": []})),
            godot_qa_runtime::ASSERT_STATE_MATCHES => ok_result(
                command,
                json!({"overlay": {"hidden": true, "visible_in_tree": false}}),
            ),
            godot_qa_runtime::NODE_INSPECT => {
                let target = params.get("target").and_then(Value::as_str).unwrap_or("");
                if target == "qa:main_menu" {
                    ok_result(command, json!({"resolved": "path:/root/Main/MainMenu"}))
                } else {
                    fail_result(command, "target_not_found", Some(target))
                }
            }
            other => fail_result(command, other, None),
        },
    };

    let default_result = run_live_gate_with_hooks(
        &config,
        "hidden_overlay",
        LiveGateRunOptions::default(),
        &hooks,
    );
    assert!(default_result.ok);
    assert_eq!(
        default_result.data["warnings"][0]["type"],
        "target_not_found"
    );
    assert_eq!(default_result.data["anchors"][1]["status"], "warning");

    let strict_result = run_live_gate_with_hooks(
        &config,
        "hidden_overlay",
        LiveGateRunOptions {
            strict: true,
            ..LiveGateRunOptions::default()
        },
        &hooks,
    );
    assert!(!strict_result.ok);
    assert_eq!(strict_result.data["anchors"][1]["status"], "failed");
    assert!(
        strict_result.data["warnings"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn hidden_overlay_rejected_method_contract() {
    let root = temp_dir("hidden-overlay-rejected-method");
    write(
        &root.join("qa/live_gates/hidden_overlay.json"),
        &json!({
            "name": "hidden_overlay",
            "scene": "res://ui/main_menu.tscn",
            "overlay": "hidden",
            "anchors": {"required": ["main_menu"]},
            "methods": ["unsafe_method"]
        })
        .to_string(),
    );
    let config = make_run_config(&root);
    let hooks = LiveGateRunHooks {
        session_start: &|_, _, _, _| ok_result("session start", json!({"session_id": "s1"})),
        session_stop: &|_| ok_result("session stop", json!({})),
        load_session: &load_attached_session,
        protocol: &|_, command, protocol_command, params| match protocol_command {
            godot_qa_runtime::SESSION_STATUS => ok_result(command, json!({"ready": true})),
            godot_qa_runtime::ERRORS_READ => ok_result(command, json!({"errors": []})),
            godot_qa_runtime::ASSERT_STATE_MATCHES => {
                let method = params.get("method").and_then(Value::as_str).unwrap_or("");
                if method == "get_hidden_overlay_qa_state" {
                    ok_result(
                        command,
                        json!({"overlay": {"hidden": true, "visible_in_tree": false}}),
                    )
                } else {
                    CommandResult {
                        ok: false,
                        command: command.to_string(),
                        message: "method rejected".to_string(),
                        data: Map::new(),
                        failures: vec![FailureDetail {
                            kind: "method_rejected".to_string(),
                            command: Some(command.to_string()),
                            target: Some("qa:main_menu".to_string()),
                            message: Some("method was rejected by the QA allowlist".to_string()),
                            actual: Some(json!({
                                "method": method,
                                "allowed_patterns": ["^qa_", "^get_hidden_overlay_qa_state$"],
                                "reason": "method_name_not_allowed"
                            })),
                            ..FailureDetail::default()
                        }],
                        exit_code: 1,
                    }
                }
            }
            godot_qa_runtime::NODE_INSPECT => {
                ok_result(command, json!({"resolved": "path:/root/Main/MainMenu"}))
            }
            other => fail_result(command, other, None),
        },
    };

    let result = run_live_gate_with_hooks(
        &config,
        "hidden_overlay",
        LiveGateRunOptions::default(),
        &hooks,
    );

    assert!(!result.ok);
    let failure = &result.data["anchors"][0]["methods"][0]["failures"][0];
    assert_eq!(failure["type"], "method_rejected");
    assert_eq!(failure["actual"]["method"], "unsafe_method");
    assert_eq!(failure["actual"]["allowed_patterns"][0], "^qa_");
    assert_eq!(failure["actual"]["reason"], "method_name_not_allowed");
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn hidden_overlay_probe_failure_contract() {
    let root = temp_dir("hidden-overlay-probe-failure");
    write(
        &root.join("qa/live_gates/hidden_overlay.json"),
        &json!({
            "name": "hidden_overlay",
            "scene": "res://ui/main_menu.tscn",
            "overlay": "hidden",
            "anchors": {"required": ["playfield"]},
            "probes": [{
                "name": "qa_probe_surface",
                "points": [{"x": 12.0, "y": 34.0, "expected": "walkable"}]
            }]
        })
        .to_string(),
    );
    let config = make_run_config(&root);
    let hooks = LiveGateRunHooks {
        session_start: &|_, _, _, _| ok_result("session start", json!({"session_id": "s1"})),
        session_stop: &|_| ok_result("session stop", json!({})),
        load_session: &load_attached_session,
        protocol: &|_, command, protocol_command, params| match protocol_command {
            godot_qa_runtime::SESSION_STATUS => ok_result(command, json!({"ready": true})),
            godot_qa_runtime::ERRORS_READ => ok_result(command, json!({"errors": []})),
            godot_qa_runtime::ASSERT_STATE_MATCHES => ok_result(
                command,
                json!({"overlay": {"hidden": true, "visible_in_tree": false}}),
            ),
            godot_qa_runtime::NODE_INSPECT => {
                ok_result(command, json!({"resolved": "path:/root/Main/Playfield"}))
            }
            godot_qa_runtime::ASSERT_SURFACE_PROBE => CommandResult {
                ok: false,
                command: command.to_string(),
                message: "probe failed".to_string(),
                data: Map::new(),
                failures: vec![FailureDetail {
                    kind: "surface_probe_mismatch".to_string(),
                    command: Some(command.to_string()),
                    target: Some("qa:playfield".to_string()),
                    expected: Some(json!("walkable")),
                    actual: Some(json!({
                        "provider": "qa:playfield",
                        "provider_method": params["provider_method"],
                        "points": [{"point": [12.0, 34.0], "expected": "walkable", "actual": "blocked"}],
                        "anchor_locator": "qa:playfield",
                        "resolved": "path:/root/Main/Playfield"
                    })),
                    ..FailureDetail::default()
                }],
                exit_code: 1,
            },
            other => fail_result(command, other, None),
        },
    };

    let result = run_live_gate_with_hooks(
        &config,
        "hidden_overlay",
        LiveGateRunOptions::default(),
        &hooks,
    );

    assert!(!result.ok);
    let failure = &result.data["anchors"][0]["probes"][0]["failures"][0];
    assert_eq!(failure["type"], "surface_probe_mismatch");
    assert_eq!(failure["expected"], "walkable");
    assert_eq!(failure["actual"]["provider"], "qa:playfield");
    assert_eq!(failure["actual"]["provider_method"], "qa_probe_surface");
    assert_eq!(failure["actual"]["points"][0]["expected"], "walkable");
    assert_eq!(failure["actual"]["points"][0]["actual"], "blocked");
    assert_eq!(failure["actual"]["anchor_locator"], "qa:playfield");
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn hidden_overlay_missing_overlay_hook_contract() {
    let root = temp_dir("hidden-overlay-missing-hook");
    write(
        &root.join("qa/live_gates/hidden_overlay.json"),
        &json!({
            "name": "hidden_overlay",
            "scene": "res://ui/main_menu.tscn",
            "overlay": "hidden",
            "anchors": {"required": ["main_menu"]}
        })
        .to_string(),
    );
    let config = make_run_config(&root);
    let hooks = LiveGateRunHooks {
        session_start: &|_, _, _, _| ok_result("session start", json!({"session_id": "s1"})),
        session_stop: &|_| ok_result("session stop", json!({})),
        load_session: &load_attached_session,
        protocol: &|_, command, protocol_command, params| match protocol_command {
            godot_qa_runtime::SESSION_STATUS => ok_result(command, json!({"ready": true})),
            godot_qa_runtime::ERRORS_READ => ok_result(command, json!({"errors": []})),
            godot_qa_runtime::ASSERT_STATE_MATCHES => {
                let method = params.get("method").and_then(Value::as_str).unwrap_or("");
                if method == "get_hidden_overlay_qa_state" {
                    fail_result(command, "method_not_found", Some("qa:qa_debug_overlay"))
                } else {
                    ok_result(command, json!({}))
                }
            }
            godot_qa_runtime::NODE_INSPECT => {
                ok_result(command, json!({"resolved": "path:/root/Main/MainMenu"}))
            }
            other => fail_result(command, other, None),
        },
    };

    let result = run_live_gate_with_hooks(
        &config,
        "hidden_overlay",
        LiveGateRunOptions::default(),
        &hooks,
    );

    assert!(!result.ok);
    assert_eq!(result.data["overlay"]["status"], "failed");
    assert!(result.failures.iter().any(|failure| {
        failure.kind == "overlay_method_not_found"
            && failure.target.as_deref() == Some("qa:qa_debug_overlay")
    }));
    fs::remove_dir_all(root).unwrap();
}

#[test]
#[allow(clippy::result_large_err)]
fn live_gate_stale_session_contract() {
    let root = temp_dir("live-gate-stale-session");
    write(
        &root.join("qa/live_gates/hidden_overlay.json"),
        &json!({
            "name": "hidden_overlay",
            "scene": "res://ui/main_menu.tscn",
            "anchors": {"required": ["main_menu"]}
        })
        .to_string(),
    );
    let config = make_run_config(&root);
    let hooks = LiveGateRunHooks {
        session_start: &|_, _, _, _| ok_result("session start", json!({})),
        session_stop: &|_| ok_result("session stop", json!({})),
        load_session: &|_, command| {
            Err(FailureDetail {
                kind: "session_connection_failed".to_string(),
                command: Some(command.to_string()),
                path: Some(".qa/session.json".to_string()),
                message: Some("failed to connect to persisted live session".to_string()),
                actual: Some(json!({"type": "connection_refused", "port": 65535})),
                ..FailureDetail::default()
            })
        },
        protocol: &|_, command, protocol_command, _| fail_result(command, protocol_command, None),
    };

    let result = run_live_gate_with_hooks(
        &config,
        "hidden_overlay",
        LiveGateRunOptions {
            attach: true,
            ..LiveGateRunOptions::default()
        },
        &hooks,
    );

    assert!(!result.ok);
    assert_eq!(result.message, "live gate session setup failed");
    assert_eq!(result.failures[0].kind, "session_connection_failed");
    assert_eq!(result.failures[0].path.as_deref(), Some(".qa/session.json"));
    assert_eq!(
        result.failures[0].actual,
        Some(json!({"type": "connection_refused", "port": 65535}))
    );
    fs::remove_dir_all(root).unwrap();
}
