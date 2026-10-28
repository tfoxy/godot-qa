use godot_qa_scenario::scenario_doc::{
    ScenarioError, find_scenario_by_name, list_scenarios, load_and_normalize_scenario_document,
    load_scenario_document, normalize_scenario_document, validate_scenario_document,
};
use godot_qa_scenario::time_control::MAX_TIME_SCALE;
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

fn failure_json(error: ScenarioError) -> serde_json::Value {
    serde_json::to_value(error.failure).unwrap()
}

#[test]
fn unicode_artifact_names_validate_and_preserve_exact_identity() {
    let root = temp_dir("scenario-unicode-artifact-names");
    for name in [
        "playable_1hyvä_two_player",
        "1HYVÄ",
        "玩家_二人.開始-1",
        "١مرحلة",
        "Ascii_1.dot-hyphen",
    ] {
        let document = json!({
            "name": name,
            "scene": "res://ui/main_menu.tscn",
            "steps": [
                {"snapshot": name},
                {"record_state": {"name": name, "source": "provider", "provider": "state"}},
                {"audio_capture": {"name": name, "duration_seconds": 1.0}},
                {"audio_export": {"name": name, "target": "qa:audio", "method": "qa_export"}}
            ]
        });
        let path = root.join(format!("{name}.json"));
        write(&path, &serde_json::to_string(&document).unwrap());
        let normalized = load_and_normalize_scenario_document(&path).unwrap();
        assert_eq!(normalized["name"], name);
        assert_eq!(normalized["steps"][0]["snapshot"], name);
        for (index, kind) in [
            (1, "record_state"),
            (2, "audio_capture"),
            (3, "audio_export"),
        ] {
            assert_eq!(normalized["steps"][index][kind]["name"], name);
        }
        assert_eq!(find_scenario_by_name(&root, name), Some(path));
    }
    // Discovery and lookups retain case and accents instead of transliterating.
    assert!(find_scenario_by_name(&root, "1HYVA").is_none());
    assert!(find_scenario_by_name(&root, "1hyvä").is_none());
    assert!(find_scenario_by_name(&root, "1HYVA\u{308}").is_none());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn artifact_name_validation_rejects_unsafe_unicode_paths_and_controls() {
    for name in [
        "",
        ".",
        "..",
        ".hyvä",
        "_hyvä",
        "-hyvä",
        "../hyvä",
        "hyvä/escape",
        "hyvä\\escape",
        "hyvä:escape",
        "hyvä\u{2215}escape", // Unicode division slash is not a path-name letter.
        "hyvä\u{ff0f}escape", // Fullwidth slash.
        "hyvä\u{ff3c}escape", // Fullwidth backslash.
        "hyvä name",
        "hyvä\u{a0}name",   // Non-breaking space.
        "hyvä\u{3000}name", // Ideographic space.
        "hyvä\tname",
        "hyvä\nname",
        "hyvä\0name",
        "hyvä\u{7f}name",
        "hyvä\u{85}name",
        "hyvä\u{61c}name",  // Arabic letter mark (bidi control).
        "hyvä\u{200e}name", // Left-to-right mark.
        "hyvä\u{202e}name", // Right-to-left override.
        "hyvä\u{2066}name", // Left-to-right isolate.
        "hyvä\u{200b}name", // Zero-width space.
        "hyvä\u{200d}name", // Zero-width joiner.
        "hyvä\u{feff}name",
        "hyvä😀",
        "hyva\u{308}", // Decomposed spelling is not silently composed.
    ] {
        let base = json!({"name": "safe", "scene": "res://main.tscn", "steps": []});
        let mut document = base.clone();
        document["name"] = json!(name);
        assert!(
            validate_scenario_document(&document, Path::new("scenario.json")).is_err(),
            "scenario name accepted {name:?}"
        );
        for step in [
            json!({"snapshot": name}),
            json!({"record_state": {"name": name, "source": "provider", "provider": "state"}}),
            json!({"audio_capture": {"name": name, "duration_seconds": 1.0}}),
            json!({"audio_export": {"name": name, "target": "qa:audio", "method": "qa_export"}}),
        ] {
            let mut document = base.clone();
            document["steps"] = json!([step]);
            assert!(
                validate_scenario_document(&document, Path::new("scenario.json")).is_err(),
                "artifact step accepted {name:?}: {document}"
            );
        }
    }
}

#[test]
fn list_scenarios_prefers_declared_names_and_stable_sorting() {
    let root = temp_dir("scenario-list");
    let scenarios_dir = root.join("qa/scenarios/examples");
    write(
        &scenarios_dir.join("z_last.yaml"),
        "name: Zebra\nsteps: []\n",
    );
    write(
        &scenarios_dir.join("a_first.json"),
        "{\"name\": \"Alpha\"}\n",
    );
    write(&scenarios_dir.join("fallback.json"), "{}\n");

    assert_eq!(
        list_scenarios(&scenarios_dir),
        vec![
            json!({"name": "Alpha", "path": scenarios_dir.join("a_first.json")}),
            json!({"name": "Zebra", "path": scenarios_dir.join("z_last.yaml")}),
            json!({"name": "fallback", "path": scenarios_dir.join("fallback.json")}),
        ]
    );
}

#[test]
fn find_scenario_by_name_matches_declared_name_and_stem() {
    let root = temp_dir("scenario-find");
    let scenarios_dir = root.join("qa/scenarios");
    let declared = scenarios_dir.join("declared.yaml");
    let fallback = scenarios_dir.join("fallback.json");
    write(
        &declared,
        "name: Friendly Name\nscene: res://ui/main_menu.tscn\nsteps: []\n",
    );
    write(&fallback, "{}\n");

    assert_eq!(
        find_scenario_by_name(&scenarios_dir, "Friendly Name"),
        Some(declared)
    );
    assert_eq!(
        find_scenario_by_name(&scenarios_dir, "fallback"),
        Some(fallback)
    );
    assert_eq!(find_scenario_by_name(&scenarios_dir, "missing"), None);
}

#[test]
fn load_scenario_document_reads_supported_yaml_without_external_loader() {
    let root = temp_dir("scenario-yaml");
    let scenario_path = root.join("scenario.yaml");
    write(
        &scenario_path,
        &[
            "name: main_menu_start_opens_pause",
            "scene: res://ui/main_menu.tscn",
            "",
            "godot_args:",
            "  - --single-window",
            "",
            "project_args:",
            "  - --save-slot",
            "  - qa-profile",
            "  - --seed=42",
            "",
            "viewport:",
            "  width: 1920",
            "  height: 1080",
            "",
            "steps:",
            "  - wait_frames: 5",
            "  - snapshot: before",
            "",
            "assert:",
            "  - visible: qa:pause_menu",
            "  - no_errors: true",
            "",
            "viewports:",
            "  - [1280, 720]",
            "  - [1920, 1080]",
        ]
        .join("\n"),
    );

    let loaded = load_scenario_document(&scenario_path).unwrap();

    assert_eq!(
        loaded,
        json!({
            "name": "main_menu_start_opens_pause",
            "scene": "res://ui/main_menu.tscn",
            "godot_args": ["--single-window"],
            "project_args": ["--save-slot", "qa-profile", "--seed=42"],
            "viewport": {"width": 1920, "height": 1080},
            "steps": [{"wait_frames": 5}, {"snapshot": "before"}],
            "assert": [{"visible": "qa:pause_menu"}, {"no_errors": true}],
            "viewports": [[1280, 720], [1920, 1080]],
        })
    );
}

#[test]
fn load_scenario_document_resolves_yaml_anchors_and_aliases() {
    let root = temp_dir("scenario-yaml-anchors");
    let scenario_path = root.join("scenario.yaml");
    write(
        &scenario_path,
        "name: example\nscene: res://ui/main_menu.tscn\nsteps:\n  - &step { wait_frames: 1 }\n  - *step\n",
    );

    let loaded = load_scenario_document(&scenario_path).unwrap();

    assert_eq!(
        loaded,
        json!({
            "name": "example",
            "scene": "res://ui/main_menu.tscn",
            "steps": [{"wait_frames": 1}, {"wait_frames": 1}],
        })
    );
}

#[test]
fn load_scenario_document_resolves_yaml_merge_keys() {
    let root = temp_dir("scenario-yaml-merge");
    let scenario_path = root.join("scenario.yaml");
    write(
        &scenario_path,
        &[
            "defaults: &defaults",
            "  window_frames: 12",
            "  sample_every_frames: 1",
            "name: example",
            "scene: res://ui/main_menu.tscn",
            "steps: []",
            "assert:",
            "  - never:",
            "      target: qa:warning",
            "      forbidden: DESYNC",
            "      <<: *defaults",
        ]
        .join("\n"),
    );

    let loaded = load_scenario_document(&scenario_path).unwrap();
    assert_eq!(
        loaded["assert"][0]["never"],
        json!({
            "target": "qa:warning",
            "forbidden": "DESYNC",
            "window_frames": 12,
            "sample_every_frames": 1,
        })
    );
}

#[test]
fn load_scenario_document_reports_actionable_yaml_parse_failure() {
    let root = temp_dir("scenario-yaml-malformed");
    let scenario_path = root.join("scenario.yaml");
    write(
        &scenario_path,
        "name: example\nscene: res://ui/main_menu.tscn\nsteps:\n  - foo: [1, 2\n",
    );

    let error = load_scenario_document(&scenario_path).unwrap_err();
    let failure = failure_json(error);
    assert_eq!(failure["type"], json!("scenario_yaml_parse_failed"));
    assert_eq!(failure["path"], json!(scenario_path));
    assert!(
        failure["message"].as_str().unwrap().contains("line"),
        "expected line info in message, got {failure:?}"
    );
    assert!(
        failure["actual"].as_str().unwrap().starts_with("line "),
        "expected a location summary, got {failure:?}"
    );
}

/// Regression corpus replicating the YAML/JSON shapes the five consumer game
/// projects commit. The historical hand-rolled subset parser mis-parsed several
/// of these (inline maps, grouped `state_matches.matches`, colon-bearing plain
/// scalars); the real YAML parser plus strict validation must accept them all.
#[test]
fn load_and_validate_accepts_consumer_scenario_shapes() {
    let root = temp_dir("scenario-consumer-corpus");

    // blink-blitz shape: grouped `state_matches` with an inline-map `matches`
    // list, `source: method`, colon-bearing plain scalars, inline `args`, a
    // `where: {..}` inline-map selector, and a nested `assert` inside `steps`.
    let blink = root.join("blink.yaml");
    write(
        &blink,
        &[
            "name: king_circle_smoke",
            "scene: res://scenes/main.tscn",
            "determinism:",
            "  seed: 424242",
            "  test_mode: true",
            "steps:",
            "  - wait_frames: 75",
            "  - call_method:",
            "      target: qa:match_controller",
            "      method: qa_start_minigame",
            "      args: [king_circle]",
            "  - call_method:",
            "      target: qa:match_controller",
            "      method: qa_force_scoreboard",
            "      args: []",
            "  - assert:",
            "      - exists: qa:dodgeball_arena",
            "assert:",
            "  - exists: qa:minigame_root",
            "  - state_matches:",
            "      target: qa:minigame_root",
            "      source: method",
            "      method: get_qa_minigame_state",
            "      matches:",
            "        - {path: phase, operator: equals, expected: playing}",
            "        - {path: timer_remaining, operator: present}",
            "        - {path: players, operator: present}",
            "  - state_matches:",
            "      target: qa:minigame_root",
            "      source: method",
            "      method: get_qa_minigame_state",
            "      path: bots",
            "      where: {team: red}",
            "      value_path: current_job",
            "      operator: present",
            "  - no_errors: true",
        ]
        .join("\n"),
    );
    let doc = load_and_normalize_scenario_document(&blink).expect("blink-blitz shape");
    let matches = doc["assert"][1]["state_matches"]["matches"]
        .as_array()
        .expect("matches list survives parsing");
    assert_eq!(matches.len(), 3);
    assert_eq!(matches[0]["path"], json!("phase"));
    assert_eq!(matches[0]["expected"], json!("playing"));
    assert_eq!(matches[1]["operator"], json!("present"));
    assert_eq!(
        doc["steps"][1]["call_method"]["args"],
        json!(["king_circle"])
    );

    // river-raft shape: JSON body inside a `.yaml` extension, `source: provider`
    // with a block-style `matches` list, negative/high-precision floats,
    // `not_exists`, and both `viewport` + `viewports`.
    let river = root.join("river.yaml");
    write(
        &river,
        r#"{
  "name": "bot_single_obstacle_avoidance",
  "scene": "res://scenes/main.tscn",
  "viewport": {"width": 1920, "height": 1080},
  "viewports": [[1280, 720]],
  "steps": [
    {"input_joy_motion": {"axis": "left_x", "value": -1.0, "device": 0, "frames": 2}}
  ],
  "assert": [
    {"state_matches": {
      "source": "provider",
      "provider": "bot_single_obstacle_avoidance",
      "matches": [
        {"path": "target_reached", "operator": "equals", "expected": true},
        {"path": "closest_approach", "operator": "greater_than", "expected": 1.5001}
      ]
    }},
    {"not_exists": "qa:debug_overlay"},
    {"no_errors": true}
  ]
}
"#,
    );
    let doc = load_and_normalize_scenario_document(&river).expect("river-raft JSON-in-yaml shape");
    assert_eq!(
        doc["assert"][0]["state_matches"]["source"],
        json!("provider")
    );
    assert_eq!(doc["steps"][0]["input_joy_motion"]["value"], json!(-1.0));

    // coop-2 shape: compact JSON with `where`/`value_path` and an empty `path`.
    let coop = root.join("coop.json");
    write(
        &coop,
        r#"{"name":"six_player_join","scene":"res://main.tscn","steps":[{"wait_frames":30}],
"assert":[{"state_matches":{"target":"qa:lobby","source":"method","method":"get_qa_lobby_state",
"path":"","where":{"index":0},"value_path":"device","operator":"equals","expected":0}}]}"#,
    );
    let doc = load_and_normalize_scenario_document(&coop).expect("coop-2 where/value_path shape");
    assert_eq!(
        doc["assert"][0]["state_matches"]["where"]["index"],
        json!(0)
    );
}

#[test]
fn validate_accepts_and_rejects_wait_until_step() {
    let path = Path::new("scenario.json");

    let valid = json!({
        "name": "wait_until_demo",
        "scene": "res://main.tscn",
        "steps": [{
            "wait_until": {
                "source": "method",
                "target": "qa:main_menu",
                "method": "get_qa_state",
                "path": "status",
                "operator": "equals",
                "expected": "ready",
                "timeout_frames": 120,
                "poll_every_frames": 10
            }
        }],
    });
    validate_scenario_document(&valid, path).expect("valid wait_until should validate");

    let valid_provider = json!({
        "name": "wait_until_provider",
        "scene": "res://main.tscn",
        "steps": [{
            "wait_until": {
                "source": "provider",
                "provider": "race_feel",
                "path": "phase",
                "operator": "present",
                "timeout_frames": 60
            }
        }],
    });
    validate_scenario_document(&valid_provider, path)
        .expect("provider wait_until without expected should validate");

    // Missing timeout_frames.
    let missing_timeout = json!({
        "name": "x", "scene": "res://main.tscn",
        "steps": [{ "wait_until": { "source": "provider", "provider": "p", "path": "a", "operator": "present" } }],
    });
    assert!(validate_scenario_document(&missing_timeout, path).is_err());

    // Unknown operator.
    let bad_operator = json!({
        "name": "x", "scene": "res://main.tscn",
        "steps": [{ "wait_until": { "source": "provider", "provider": "p", "path": "a", "operator": "is_totally", "expected": 1, "timeout_frames": 5 } }],
    });
    assert!(validate_scenario_document(&bad_operator, path).is_err());

    // equals without expected.
    let missing_expected = json!({
        "name": "x", "scene": "res://main.tscn",
        "steps": [{ "wait_until": { "source": "provider", "provider": "p", "path": "a", "operator": "equals", "timeout_frames": 5 } }],
    });
    assert!(validate_scenario_document(&missing_expected, path).is_err());

    // Unknown field inside wait_until.
    let unknown_field = json!({
        "name": "x", "scene": "res://main.tscn",
        "steps": [{ "wait_until": { "source": "provider", "provider": "p", "path": "a", "operator": "present", "timeout_frames": 5, "bogus": true } }],
    });
    let error = validate_scenario_document(&unknown_field, path).unwrap_err();
    assert!(error.failure.message.as_deref().unwrap().contains("bogus"));
}

#[test]
fn validate_rejects_unknown_assertion_and_step_keys() {
    let path = Path::new("scenario.json");

    let unknown_assertion = json!({
        "name": "typo",
        "scene": "res://main.tscn",
        "steps": [],
        "assert": [{ "stat_matches": { "path": "x" } }],
    });
    let error = validate_scenario_document(&unknown_assertion, path).unwrap_err();
    assert_eq!(error.failure.kind, "scenario_validation_failed");
    assert!(
        error
            .failure
            .message
            .as_deref()
            .unwrap()
            .contains("stat_matches"),
        "message should name the offending key: {:?}",
        error.failure.message
    );

    let unknown_step = json!({
        "name": "typo",
        "scene": "res://main.tscn",
        "steps": [{ "wait_framez": 5 }],
    });
    let error = validate_scenario_document(&unknown_step, path).unwrap_err();
    assert!(
        error
            .failure
            .message
            .as_deref()
            .unwrap()
            .contains("wait_framez"),
        "message should name the offending step key: {:?}",
        error.failure.message
    );
}

#[test]
fn validate_scenario_document_rejects_missing_required_fields() {
    let root = temp_dir("scenario-validate-required");
    let scenario_path = root.join("scenario.json");

    let error =
        validate_scenario_document(&json!({"name": "example"}), &scenario_path).unwrap_err();

    assert_eq!(
        failure_json(error),
        json!({
            "type": "scenario_validation_failed",
            "path": scenario_path,
            "message": "scenario document is missing required fields",
            "expected": ["name", "scene", "steps"],
            "actual": {"missing_fields": ["scene", "steps"]},
        })
    );
}

#[test]
fn validate_scenario_document_rejects_invalid_operation_payloads() {
    let root = temp_dir("scenario-validate-ops");
    let scenario_path = root.join("scenario.json");
    let document = json!({
        "name": "main_menu_start_opens_pause",
        "scene": "res://ui/main_menu.tscn",
        "steps": [{"wait_frames": 1}],
        "assert": [{"screenshot_match": {"snapshot": "after_start", "baseline": "qa/baselines/example.png", "threshold": 2.0}}],
    });

    let error = validate_scenario_document(&document, &scenario_path).unwrap_err();

    assert_eq!(
        failure_json(error),
        json!({
            "type": "scenario_validation_failed",
            "path": scenario_path,
            "message": "screenshot_match assertion field 'threshold' must be between 0.0 and 1.0",
            "expected": "number between 0.0 and 1.0",
            "actual": 2.0,
        })
    );
}

#[test]
fn validate_scenario_document_accepts_saved_joypad_input_steps() {
    let root = temp_dir("scenario-validate-joypad-input");
    let scenario_path = root.join("scenario.json");
    let document = json!({
        "name": "joypad_input",
        "scene": "res://ui/fixtures/live_input_probe.tscn",
        "steps": [
            {"input_joy_button": "start"},
            {"input_joy_button": {"button": "start", "device": 2, "frames": 2}},
            {"input_joy_motion": {"axis": "left_x", "value": 1.0, "device": 2, "frames": 2}}
        ],
    });

    validate_scenario_document(&document, &scenario_path).unwrap();
    let normalized = normalize_scenario_document(&document);
    assert_eq!(normalized["steps"][0]["input_joy_button"], "start");
    assert_eq!(normalized["steps"][1]["input_joy_button"]["device"], 2);
    assert_eq!(normalized["steps"][2]["input_joy_motion"]["axis"], "left_x");
}

#[test]
fn validate_scenario_document_rejects_invalid_saved_joypad_input_steps() {
    let root = temp_dir("scenario-validate-invalid-joypad-input");
    let scenario_path = root.join("scenario.json");

    let invalid_button = json!({
        "name": "joypad_input",
        "scene": "res://ui/fixtures/live_input_probe.tscn",
        "steps": [{"input_joy_button": {"button": "start", "device": -1}}],
    });
    let error = validate_scenario_document(&invalid_button, &scenario_path).unwrap_err();
    assert_eq!(
        failure_json(error),
        json!({
            "type": "scenario_validation_failed",
            "path": scenario_path,
            "message": "input_joy_button step field 'device' must be a non-negative integer",
            "expected": "non-negative integer",
            "actual": -1,
        })
    );

    let invalid_motion = json!({
        "name": "joypad_input",
        "scene": "res://ui/fixtures/live_input_probe.tscn",
        "steps": [{"input_joy_motion": {"axis": "left_x", "value": 1.5}}],
    });
    let error = validate_scenario_document(&invalid_motion, &scenario_path).unwrap_err();
    assert_eq!(
        failure_json(error),
        json!({
            "type": "scenario_validation_failed",
            "path": scenario_path,
            "message": "input_joy_motion step field 'value' must be between -1.0 and 1.0",
            "expected": "number between -1.0 and 1.0",
            "actual": 1.5,
        })
    );
}

#[test]
fn validate_scenario_document_accepts_input_joy_state_steps() {
    let root = temp_dir("scenario-validate-joy-state");
    let scenario_path = root.join("scenario.json");
    let document = json!({
        "name": "joy_state",
        "scene": "res://ui/fixtures/live_input_probe.tscn",
        "steps": [
            {"input_joy_state": {
                "inputs": [
                    {"device": 0, "button": {"name": "a", "pressed": true}},
                    {"device": 3, "button": {"name": "x", "pressed": true}},
                    {"device": 3, "motion": {"axis": "left_x", "value": -1.0}}
                ],
                "frames": 30
            }},
            // frames omitted (defaults to 1) and frames: 0 (emit same-frame).
            {"input_joy_state": {"inputs": [{"device": 5, "button": {"name": "b", "pressed": false}}]}},
            {"input_joy_state": {"inputs": [{"device": 0, "motion": {"axis": "left_x", "value": 0}}], "frames": 0}}
        ],
    });

    validate_scenario_document(&document, &scenario_path).unwrap();
    let normalized = normalize_scenario_document(&document);
    assert_eq!(
        normalized["steps"][0]["input_joy_state"]["inputs"][0]["device"],
        0
    );
    assert_eq!(normalized["steps"][0]["input_joy_state"]["frames"], 30);
}

#[test]
fn validate_scenario_document_rejects_invalid_input_joy_state_steps() {
    let root = temp_dir("scenario-validate-invalid-joy-state");
    let scenario_path = root.join("scenario.json");

    let with_step = |step: serde_json::Value| {
        json!({
            "name": "joy_state",
            "scene": "res://ui/fixtures/live_input_probe.tscn",
            "steps": [{"input_joy_state": step}],
        })
    };
    let reject = |step: serde_json::Value| {
        failure_json(validate_scenario_document(&with_step(step), &scenario_path).unwrap_err())
    };
    let message = |value: &serde_json::Value| value["message"].as_str().unwrap().to_string();

    // Value must be an object.
    assert_eq!(
        failure_json(
            validate_scenario_document(&with_step(json!("a")), &scenario_path).unwrap_err()
        )["message"],
        json!("input_joy_state step value must be an object")
    );

    // inputs missing / not a list.
    assert_eq!(
        reject(json!({"frames": 2}))["message"],
        json!(
            "input_joy_state step field 'inputs' must be a non-empty list of {device, motion|button} objects"
        )
    );
    // inputs empty.
    assert_eq!(
        reject(json!({"inputs": []}))["message"],
        json!(
            "input_joy_state step field 'inputs' must be a non-empty list of {device, motion|button} objects"
        )
    );
    // Unknown top-level field.
    assert!(
        message(&reject(json!({"inputs": [{"device": 0, "button": {"name": "a", "pressed": true}}], "hold": true})))
            .contains("uses unsupported field 'hold'")
    );
    // Negative frames.
    assert_eq!(
        reject(
            json!({"inputs": [{"device": 0, "button": {"name": "a", "pressed": true}}], "frames": -1})
        )["message"],
        json!("input_joy_state step field 'frames' must be a non-negative integer")
    );
    // Entry not an object.
    assert_eq!(
        reject(json!({"inputs": ["a"]}))["message"],
        json!("input_joy_state input 0 must be an object")
    );
    // Negative device.
    assert_eq!(
        reject(json!({"inputs": [{"device": -1, "button": {"name": "a", "pressed": true}}]}))["message"],
        json!("input_joy_state input 0 field 'device' must be a non-negative integer")
    );
    // Missing device.
    assert_eq!(
        reject(json!({"inputs": [{"button": {"name": "a", "pressed": true}}]}))["message"],
        json!("input_joy_state input 0 field 'device' must be a non-negative integer")
    );
    // Both motion and button.
    assert_eq!(
        reject(
            json!({"inputs": [{"device": 0, "motion": {"axis": "left_x", "value": 0.0}, "button": {"name": "a", "pressed": true}}]})
        )["message"],
        json!("input_joy_state input 0 must use exactly one of 'motion' or 'button'")
    );
    // Neither motion nor button.
    assert_eq!(
        reject(json!({"inputs": [{"device": 0}]}))["message"],
        json!("input_joy_state input 0 must use exactly one of 'motion' or 'button'")
    );
    // Empty axis.
    assert_eq!(
        reject(json!({"inputs": [{"device": 0, "motion": {"axis": "", "value": 0.0}}]}))["message"],
        json!("input_joy_state input 0 motion 'axis' must be a non-empty string")
    );
    // Motion value out of range (locks message/expected/actual).
    assert_eq!(
        reject(json!({"inputs": [{"device": 0, "motion": {"axis": "left_x", "value": 1.5}}]})),
        json!({
            "type": "scenario_validation_failed",
            "path": scenario_path,
            "message": "input_joy_state input 0 motion 'value' must be between -1.0 and 1.0",
            "expected": "number between -1.0 and 1.0",
            "actual": 1.5,
        })
    );
    // Motion value non-numeric.
    assert_eq!(
        reject(json!({"inputs": [{"device": 0, "motion": {"axis": "left_x", "value": "x"}}]}))["message"],
        json!("input_joy_state input 0 motion 'value' must be a number between -1.0 and 1.0")
    );
    // Button name empty.
    assert_eq!(
        reject(json!({"inputs": [{"device": 0, "button": {"name": "", "pressed": true}}]}))["message"],
        json!("input_joy_state input 0 button 'name' must be a non-empty string")
    );
    // Button pressed not boolean.
    assert_eq!(
        reject(json!({"inputs": [{"device": 0, "button": {"name": "a", "pressed": "yes"}}]}))["message"],
        json!("input_joy_state input 0 button 'pressed' must be a boolean")
    );
}

#[test]
fn validate_scenario_document_accepts_joy_connection_steps() {
    let root = temp_dir("scenario-validate-joy-connect");
    let scenario_path = root.join("scenario.json");
    let document = json!({
        "name": "joy_connect",
        "scene": "res://ui/fixtures/live_input_probe.tscn",
        "steps": [
            // Multi-device connect with an optional name...
            {"joy_connect": {"devices": [0, 1, 2, 3, 4, 5], "name": "QA Virtual Pad"}},
            // ...name omitted (defaults runtime-side)...
            {"joy_connect": {"devices": [0]}},
            // ...and a mid-scenario single-device disconnect (no name).
            {"joy_disconnect": {"devices": [3]}}
        ],
    });

    validate_scenario_document(&document, &scenario_path).unwrap();
    let normalized = normalize_scenario_document(&document);
    assert_eq!(
        normalized["steps"][0]["joy_connect"]["devices"][5],
        json!(5)
    );
    assert_eq!(
        normalized["steps"][0]["joy_connect"]["name"],
        json!("QA Virtual Pad")
    );
    assert_eq!(normalized["steps"][2]["joy_disconnect"]["devices"][0], 3);
}

#[test]
fn validate_scenario_document_rejects_invalid_joy_connection_steps() {
    let root = temp_dir("scenario-validate-invalid-joy-connect");
    let scenario_path = root.join("scenario.json");

    let with_step = |step: serde_json::Value| {
        json!({
            "name": "joy_connect",
            "scene": "res://ui/fixtures/live_input_probe.tscn",
            "steps": [step],
        })
    };
    let reject = |step: serde_json::Value| {
        failure_json(validate_scenario_document(&with_step(step), &scenario_path).unwrap_err())
    };
    let message = |value: &serde_json::Value| value["message"].as_str().unwrap().to_string();

    // Value must be an object.
    assert_eq!(
        reject(json!({"joy_connect": "a"}))["message"],
        json!("joy_connect step value must be an object")
    );
    // devices missing / not a list.
    assert_eq!(
        reject(json!({"joy_connect": {"name": "Pad"}}))["message"],
        json!("joy_connect step field 'devices' must be a non-empty list of device integers")
    );
    // devices empty.
    assert_eq!(
        reject(json!({"joy_connect": {"devices": []}}))["message"],
        json!("joy_connect step field 'devices' must be a non-empty list of device integers")
    );
    // Negative device.
    assert_eq!(
        reject(json!({"joy_connect": {"devices": [0, -1]}}))["message"],
        json!("joy_connect step 'devices' entries must be non-negative integers")
    );
    // Non-integer device.
    assert_eq!(
        reject(json!({"joy_connect": {"devices": [0, 1.5]}}))["message"],
        json!("joy_connect step 'devices' entries must be non-negative integers")
    );
    // Duplicate device.
    assert_eq!(
        reject(json!({"joy_connect": {"devices": [0, 1, 0]}}))["message"],
        json!("joy_connect step 'devices' entries must be unique")
    );
    // Empty name on connect.
    assert_eq!(
        reject(json!({"joy_connect": {"devices": [0], "name": "   "}}))["message"],
        json!("joy_connect step field 'name' must be a non-empty string")
    );
    // Unknown top-level field on connect.
    assert!(
        message(&reject(
            json!({"joy_connect": {"devices": [0], "guid": "x"}})
        ))
        .contains("uses unsupported field 'guid'")
    );
    // joy_disconnect does NOT accept a name (it is unknown there).
    assert!(
        message(&reject(
            json!({"joy_disconnect": {"devices": [0], "name": "Pad"}})
        ))
        .contains("joy_disconnect step uses unsupported field 'name'")
    );
    // joy_disconnect still validates its devices list.
    assert_eq!(
        reject(json!({"joy_disconnect": {"devices": [1, 1]}}))["message"],
        json!("joy_disconnect step 'devices' entries must be unique")
    );
    assert_eq!(
        reject(json!({"joy_disconnect": {"devices": []}}))["message"],
        json!("joy_disconnect step field 'devices' must be a non-empty list of device integers")
    );
}

#[test]
fn validate_scenario_document_accepts_record_state_steps() {
    let root = temp_dir("scenario-validate-record-state");
    let scenario_path = root.join("scenario.json");
    let document = json!({
        "name": "record_state",
        "scene": "res://ui/fixtures/qa_state_schema_demo.tscn",
        "steps": [
            // Provider source with an optional path selection...
            {"record_state": {
                "name": " match_summary ",
                "source": "provider",
                "provider": "schema_demo",
                "path": "score"
            }},
            // ...and method source with a distinct name.
            {"record_state": {
                "name": "raw_state",
                "source": "method",
                "target": "qa:schema_state_provider",
                "method": "get_qa_schema_demo_state"
            }}
        ],
    });

    validate_scenario_document(&document, &scenario_path).unwrap();
    let normalized = normalize_scenario_document(&document);
    // Name is trimmed on normalize (same as snapshot names).
    assert_eq!(
        normalized["steps"][0]["record_state"]["name"],
        json!("match_summary")
    );
    assert_eq!(
        normalized["steps"][1]["record_state"]["method"],
        json!("get_qa_schema_demo_state")
    );
}

#[test]
fn validate_scenario_document_rejects_invalid_record_state_steps() {
    let root = temp_dir("scenario-validate-invalid-record-state");
    let scenario_path = root.join("scenario.json");

    let with_step = |step: serde_json::Value| {
        json!({
            "name": "record_state",
            "scene": "res://ui/fixtures/qa_state_schema_demo.tscn",
            "steps": [{"record_state": step}],
        })
    };
    let reject = |step: serde_json::Value| {
        failure_json(validate_scenario_document(&with_step(step), &scenario_path).unwrap_err())
    };
    let message = |value: &serde_json::Value| value["message"].as_str().unwrap().to_string();

    // Value must be an object.
    assert_eq!(
        failure_json(
            validate_scenario_document(&with_step(json!("x")), &scenario_path).unwrap_err()
        )["message"],
        json!("record_state step value must be an object")
    );
    // Missing name.
    assert_eq!(
        reject(json!({"source": "provider", "provider": "schema_demo"}))["message"],
        json!("record_state step field 'name' must be a non-empty string")
    );
    // Name with an unsafe charset.
    assert_eq!(
        reject(json!({"name": "bad/name", "source": "provider", "provider": "p"}))["message"],
        json!("record_state step field 'name' must be a safe artifact path segment")
    );
    // Unknown top-level field (assertion-style operator has no place here).
    assert!(
        message(&reject(json!({
            "name": "m",
            "source": "provider",
            "provider": "p",
            "operator": "equals"
        })))
        .contains("uses unsupported field 'operator'")
    );
    // Missing / unsupported source.
    assert_eq!(
        reject(json!({"name": "m"}))["message"],
        json!("record_state step field 'source' must be 'provider' or 'method'")
    );
    // provider source without a provider (reuses the shared field validator).
    assert_eq!(
        reject(json!({"name": "m", "source": "provider"}))["message"],
        json!("record_state assertion field 'provider' must be a non-empty string")
    );
    // method source without a target.
    assert_eq!(
        reject(json!({"name": "m", "source": "method", "method": "get_qa_x_state"}))["message"],
        json!("record_state assertion field 'target' must be a non-empty string")
    );
    // method source without a method.
    assert_eq!(
        reject(json!({"name": "m", "source": "method", "target": "qa:x"}))["message"],
        json!("record_state assertion field 'method' must be a non-empty string")
    );
    // Malformed path (reuses validate_state_path).
    assert_eq!(
        reject(json!({"name": "m", "source": "provider", "provider": "p", "path": "a..b"}))["message"],
        json!("record_state assertion field 'path' must use non-empty dot-separated state paths")
    );

    // Duplicate name across two record_state steps is rejected: names key the
    // evidence map, so a collision would silently overwrite an earlier record.
    let duplicate = json!({
        "name": "record_state",
        "scene": "res://ui/fixtures/qa_state_schema_demo.tscn",
        "steps": [
            {"record_state": {"name": "dup", "source": "provider", "provider": "p"}},
            {"record_state": {"name": " dup ", "source": "provider", "provider": "q"}}
        ],
    });
    assert_eq!(
        failure_json(validate_scenario_document(&duplicate, &scenario_path).unwrap_err())["message"],
        json!("record_state step name 'dup' is already used by an earlier record_state step")
    );
}

#[test]
fn validate_scenario_document_accepts_drive_policy_steps() {
    let root = temp_dir("scenario-validate-drive-policy");
    let scenario_path = root.join("scenario.json");
    let document = json!({
        "name": "drive_policy",
        "scene": "res://scenes/match.tscn",
        "steps": [
            {"drive_policy": {
                "policy": " crew_bot ",
                "params": {"skill": "competent"},
                "devices": [0, 1],
                "max_frames": 3600,
                "tick_every_frames": 6,
                "until": {
                    "source": "provider",
                    "provider": "match_state",
                    "path": "room.completed",
                    "operator": "equals",
                    "expected": true
                }
            }},
            {"drive_policy": {
                "policy": "builtin:steer_toward",
                "devices": [0],
                "max_frames": 600
            }}
        ],
    });

    validate_scenario_document(&document, &scenario_path).unwrap();
    let normalized = normalize_scenario_document(&document);
    assert_eq!(normalized["steps"][0]["drive_policy"]["policy"], "crew_bot");
    assert_eq!(
        normalized["steps"][1]["drive_policy"]["policy"],
        "builtin:steer_toward"
    );
}

#[test]
fn validate_scenario_document_rejects_invalid_drive_policy_steps() {
    let path = Path::new("scenario.json");
    let wrap = |drive_policy: serde_json::Value| {
        json!({
            "name": "drive_policy",
            "scene": "res://scenes/match.tscn",
            "steps": [{"drive_policy": drive_policy}],
        })
    };

    // Non-object payload.
    let error = validate_scenario_document(&wrap(json!("crew_bot")), path).unwrap_err();
    assert_eq!(
        error.failure.message.as_deref(),
        Some("drive_policy step value must be an object")
    );

    // Missing policy.
    let error = validate_scenario_document(&wrap(json!({"devices": [0], "max_frames": 60})), path)
        .unwrap_err();
    assert_eq!(
        error.failure.message.as_deref(),
        Some("drive_policy step field 'policy' must be a non-empty string")
    );

    // Missing devices.
    let error = validate_scenario_document(&wrap(json!({"policy": "bot", "max_frames": 60})), path)
        .unwrap_err();
    assert_eq!(
        error.failure.message.as_deref(),
        Some("drive_policy step field 'devices' must be a non-empty list of device integers")
    );

    // Empty devices.
    let error = validate_scenario_document(
        &wrap(json!({"policy": "bot", "devices": [], "max_frames": 60})),
        path,
    )
    .unwrap_err();
    assert_eq!(
        error.failure.message.as_deref(),
        Some("drive_policy step field 'devices' must be a non-empty list of device integers")
    );

    // Negative device entry.
    let error = validate_scenario_document(
        &wrap(json!({"policy": "bot", "devices": [0, -1], "max_frames": 60})),
        path,
    )
    .unwrap_err();
    assert_eq!(
        error.failure.message.as_deref(),
        Some("drive_policy step 'devices' entries must be non-negative integers")
    );

    // Missing max_frames.
    let error = validate_scenario_document(&wrap(json!({"policy": "bot", "devices": [0]})), path)
        .unwrap_err();
    assert_eq!(
        error.failure.message.as_deref(),
        Some("drive_policy step field 'max_frames' is required")
    );

    // Non-positive max_frames.
    let error = validate_scenario_document(
        &wrap(json!({"policy": "bot", "devices": [0], "max_frames": 0})),
        path,
    )
    .unwrap_err();
    assert_eq!(
        error.failure.message.as_deref(),
        Some("drive_policy step field 'max_frames' must be a positive integer")
    );

    // Non-positive tick_every_frames.
    let error = validate_scenario_document(
        &wrap(json!({"policy": "bot", "devices": [0], "max_frames": 60, "tick_every_frames": 0})),
        path,
    )
    .unwrap_err();
    assert_eq!(
        error.failure.message.as_deref(),
        Some("drive_policy assertion field 'tick_every_frames' must be a positive integer")
    );

    // Non-object params.
    let error = validate_scenario_document(
        &wrap(json!({"policy": "bot", "devices": [0], "max_frames": 60, "params": [1]})),
        path,
    )
    .unwrap_err();
    assert_eq!(
        error.failure.message.as_deref(),
        Some("drive_policy step field 'params' must be an object when present")
    );

    // Unknown step field.
    let error = validate_scenario_document(
        &wrap(json!({"policy": "bot", "devices": [0], "max_frames": 60, "bogus": true})),
        path,
    )
    .unwrap_err();
    assert_eq!(
        error.failure.message.as_deref(),
        Some("drive_policy step uses unsupported field 'bogus'")
    );

    // until without a source.
    let error = validate_scenario_document(
        &wrap(json!({"policy": "bot", "devices": [0], "max_frames": 60, "until": {"path": "a", "operator": "present"}})),
        path,
    )
    .unwrap_err();
    assert_eq!(
        error.failure.message.as_deref(),
        Some("drive_policy step 'until' field 'source' must be 'provider' or 'method'")
    );

    // until with a value operator but no expected.
    let error = validate_scenario_document(
        &wrap(json!({"policy": "bot", "devices": [0], "max_frames": 60, "until": {"source": "provider", "provider": "p", "path": "a", "operator": "equals"}})),
        path,
    )
    .unwrap_err();
    assert_eq!(
        error.failure.message.as_deref(),
        Some("drive_policy step 'until' field 'expected' is required for operator 'equals'")
    );

    // until must not carry its own frame budget; max_frames bounds the step.
    let error = validate_scenario_document(
        &wrap(json!({"policy": "bot", "devices": [0], "max_frames": 60, "until": {"source": "provider", "provider": "p", "path": "a", "operator": "present", "timeout_frames": 30}})),
        path,
    )
    .unwrap_err();
    assert_eq!(
        error.failure.message.as_deref(),
        Some("drive_policy step 'until' uses unsupported field 'timeout_frames'")
    );
}

#[test]
fn validate_scenario_document_accepts_time_control() {
    let root = temp_dir("scenario-validate-time-control");
    let scenario_path = root.join("scenario.json");
    let document = json!({
        "name": "accelerated_timer",
        "scene": "res://ui/main_menu.tscn",
        "steps": [{"wait_seconds": 3.0}],
        "time_control": {
            "time_scale": 4.0,
            "physics_ticks_per_second": 120,
            "max_physics_steps_per_frame": 16,
        },
    });

    validate_scenario_document(&document, &scenario_path).unwrap();
    assert_eq!(
        normalize_scenario_document(&document)["time_control"],
        json!({
            "time_scale": 4.0,
            "physics_ticks_per_second": 120,
            "max_physics_steps_per_frame": 16,
        })
    );

    let without_physics = json!({
        "name": "accelerated_timer_without_physics",
        "scene": "res://ui/main_menu.tscn",
        "steps": [{"wait_seconds": 3.0}],
        "time_control": {"time_scale": 2.5},
    });
    validate_scenario_document(&without_physics, &scenario_path).unwrap();
    assert_eq!(
        normalize_scenario_document(&without_physics)["time_control"],
        json!({"time_scale": 2.5})
    );

    let unaccelerated = json!({
        "name": "normal_timer",
        "scene": "res://ui/main_menu.tscn",
        "steps": [{"wait_seconds": 3.0}],
    });
    validate_scenario_document(&unaccelerated, &scenario_path).unwrap();
    assert!(
        normalize_scenario_document(&unaccelerated)
            .get("time_control")
            .is_none()
    );
}

#[test]
fn validate_scenario_document_rejects_invalid_time_control() {
    let root = temp_dir("scenario-validate-bad-time-control");
    let scenario_path = root.join("scenario.json");
    let base = json!({
        "name": "accelerated_timer",
        "scene": "res://ui/main_menu.tscn",
        "steps": [{"wait_seconds": 3.0}],
    });

    let invalid_cases = [
        (json!(false), "time_control must be an object"),
        (json!({}), "time_control.time_scale is required"),
        (
            json!({"time_scale": "fast"}),
            "time_control.time_scale must be a positive finite number",
        ),
        (
            json!({"time_scale": 0.0}),
            &format!("time_control.time_scale must be greater than 0.0 and <= {MAX_TIME_SCALE}"),
        ),
        (
            json!({"time_scale": -1.0}),
            &format!("time_control.time_scale must be greater than 0.0 and <= {MAX_TIME_SCALE}"),
        ),
        (
            json!({"time_scale": MAX_TIME_SCALE + 1.0}),
            &format!("time_control.time_scale must be greater than 0.0 and <= {MAX_TIME_SCALE}"),
        ),
        (
            json!({"time_scale": 2.0, "physics_ticks_per_second": 0}),
            "time_control.physics_ticks_per_second must be an integer >= 1",
        ),
        (
            json!({"time_scale": 2.0, "physics_ticks_per_second": 60.5}),
            "time_control.physics_ticks_per_second must be an integer >= 1",
        ),
        (
            json!({"time_scale": 2.0, "max_physics_steps_per_frame": 0}),
            "time_control.max_physics_steps_per_frame must be an integer >= 1",
        ),
        (
            json!({"time_scale": 2.0, "max_physics_steps_per_frame": "many"}),
            "time_control.max_physics_steps_per_frame must be an integer >= 1",
        ),
    ];

    for (time_control, message) in invalid_cases {
        let mut document = base.clone();
        document["time_control"] = time_control;

        let error = validate_scenario_document(&document, &scenario_path).unwrap_err();
        assert_eq!(error.failure.kind, "scenario_validation_failed");
        assert_eq!(
            error.failure.path.as_deref(),
            Some(scenario_path.to_str().unwrap())
        );
        assert_eq!(error.failure.message.as_deref(), Some(message));
    }
}

#[test]
fn validate_scenario_document_accepts_positive_fixed_fps() {
    let root = temp_dir("scenario-validate-fixed-fps");
    let scenario_path = root.join("scenario.json");
    let document = json!({
        "name": "fastest_frame_pacing",
        "scene": "res://ui/main_menu.tscn",
        "determinism": {"fixed_fps": 144},
        "steps": [],
    });

    validate_scenario_document(&document, &scenario_path).unwrap();
    assert_eq!(
        normalize_scenario_document(&document)["determinism"]["fixed_fps"],
        144
    );
}

#[test]
fn validate_scenario_document_rejects_invalid_fixed_fps_and_time_control_conflict() {
    let root = temp_dir("scenario-validate-bad-fixed-fps");
    let scenario_path = root.join("scenario.json");
    let base = json!({
        "name": "fastest_frame_pacing",
        "scene": "res://ui/main_menu.tscn",
        "steps": [],
    });

    for fixed_fps in [json!(0), json!(-1), json!("60"), json!(60.5)] {
        let mut document = base.clone();
        document["determinism"] = json!({"fixed_fps": fixed_fps});

        let error = validate_scenario_document(&document, &scenario_path).unwrap_err();
        assert_eq!(error.failure.kind, "scenario_validation_failed");
        assert_eq!(
            error.failure.message.as_deref(),
            Some("determinism.fixed_fps must be a positive integer")
        );
    }

    let conflict = json!({
        "name": "ambiguous_pacing",
        "scene": "res://ui/main_menu.tscn",
        "determinism": {"fixed_fps": 60},
        "time_control": {"time_scale": 1.0},
        "steps": [],
    });
    let error = validate_scenario_document(&conflict, &scenario_path).unwrap_err();
    assert_eq!(error.failure.kind, "scenario_validation_failed");
    assert_eq!(
        error.failure.message.as_deref(),
        Some("fixed_fps cannot be combined with time_control")
    );
}

#[test]
fn validate_scenario_document_accepts_setup_step_payloads() {
    let root = temp_dir("scenario-validate-setup");
    let scenario_path = root.join("scenario.json");
    let document = json!({
        "name": "setup_steps",
        "scene": "res://ui/main_menu.tscn",
        "steps": [
            {"teleport_node": {"target": "qa:player", "to": "qa:island_landing"}},
            {"teleport_node": {"target": "qa:player", "position": {"x": 12.5, "y": 8, "z": 1}, "preserve_velocity": true}},
            {"teleport_node": {"target": "qa:player", "to_point": {"provider": "qa:team_1_raft", "method": "qa_point_probe", "id": "station:motor"}, "offset": {"x": 3.0, "y": 0.0}}},
            {"fixture_teleport": {"target": "qa:player", "to": "qa:island_landing"}},
            {"fixture_teleport": {"target": "qa:player", "to_point": {"provider": "qa:test_island", "method": "qa_point_probe", "id": "extraction"}}},
            {"spawn_scene": {"scene": "res://fixtures/loot.tscn", "parent": "qa:arena", "name": "Loot", "qa_id": "test_loot", "at": "qa:spawn_anchor"}},
            {"spawn_scene": {"scene": "res://fixtures/loot.tscn", "position": {"x": 10, "y": 20}}},
            {"set_property": {"target": "qa:team_1", "property": "scrap", "value": 10}},
            {"call_method": {"target": "qa:island_manager", "method": "qa_spawn_test_loot", "args": ["cannon"]}}
        ],
    });

    validate_scenario_document(&document, &scenario_path).unwrap();
}

#[test]
fn validate_scenario_document_rejects_invalid_fixture_teleport() {
    let root = temp_dir("scenario-validate-fixture-teleport");
    let scenario_path = root.join("scenario.json");
    let document = json!({
        "name": "setup_steps",
        "scene": "res://ui/main_menu.tscn",
        "steps": [
            {"fixture_teleport": {"target": "qa:player", "to": " ", "preserve_velocity": false}}
        ],
    });

    let error = validate_scenario_document(&document, &scenario_path).unwrap_err();

    assert_eq!(
        failure_json(error),
        json!({
            "type": "scenario_validation_failed",
            "path": scenario_path,
            "message": "fixture_teleport step field 'to' must be a non-empty string",
            "expected": "non-empty string",
            "actual": " ",
        })
    );
}

#[test]
fn validate_scenario_document_rejects_non_fixture_call_method() {
    let root = temp_dir("scenario-validate-call-method");
    let scenario_path = root.join("scenario.json");
    let document = json!({
        "name": "setup_steps",
        "scene": "res://ui/main_menu.tscn",
        "steps": [
            {"call_method": {"target": "qa:manager", "method": "spawn_test_loot"}}
        ],
    });

    let error = validate_scenario_document(&document, &scenario_path).unwrap_err();

    assert_eq!(
        failure_json(error),
        json!({
            "type": "scenario_validation_failed",
            "path": scenario_path,
            "message": "call_method step field 'method' must name a qa_ fixture method",
            "expected": "method name beginning with qa_",
            "actual": "spawn_test_loot",
        })
    );
}

#[test]
fn validate_scenario_document_rejects_ambiguous_teleport_destination() {
    let root = temp_dir("scenario-validate-teleport-destination");
    let scenario_path = root.join("scenario.json");
    let document = json!({
        "name": "setup_steps",
        "scene": "res://ui/main_menu.tscn",
        "steps": [
            {"teleport_node": {"target": "qa:player", "to": "qa:island_landing", "position": {"x": 1, "y": 2}}}
        ],
    });

    let error = validate_scenario_document(&document, &scenario_path).unwrap_err();

    assert_eq!(
        failure_json(error),
        json!({
            "type": "scenario_validation_failed",
            "path": scenario_path,
            "message": "teleport_node step cannot combine destination fields",
            "expected": "only one of 'to', 'position', or 'to_point'",
            "actual": {"target": "qa:player", "to": "qa:island_landing", "position": {"x": 1, "y": 2}},
        })
    );
}

#[test]
fn validate_scenario_document_rejects_invalid_screenshot_quality_flag() {
    let root = temp_dir("scenario-validate-screenshot-quality");
    let scenario_path = root.join("scenario.json");
    let document = json!({
        "name": "main_menu_start_opens_pause",
        "scene": "res://ui/main_menu.tscn",
        "steps": [{"snapshot": "after_start"}],
        "snapshots": {
            "save_screenshot": true,
            "fail_on_low_information": "yes"
        },
    });

    let error = validate_scenario_document(&document, &scenario_path).unwrap_err();

    assert_eq!(
        failure_json(error),
        json!({
            "type": "scenario_validation_failed",
            "path": scenario_path,
            "message": "snapshots.fail_on_low_information must be a boolean",
            "expected": "boolean",
            "actual": "str",
        })
    );
}

#[test]
fn normalize_scenario_document_trims_snapshot_and_screenshot_match_fields() {
    let normalized = normalize_scenario_document(&json!({
        "name": " main_menu_start_opens_pause ",
        "scene": " res://ui/main_menu.tscn ",
        "steps": [
            {"snapshot": " after_start "},
            {"teleport_node": {"target": " qa:player ", "to": " qa:anchor "}},
            {"fixture_teleport": {"target": " qa:player ", "to_point": {"provider": " qa:raft ", "method": " qa_point_probe ", "id": " cell:0,2 "}}},
            {"spawn_scene": {"scene": " res://fixtures/loot.tscn ", "parent": " qa:arena ", "name": " Loot ", "qa_id": " test_loot ", "at": " qa:anchor "}},
            {"set_property": {"target": " qa:team_1 ", "property": " scrap ", "value": 10}},
            {"call_method": {"target": " qa:manager ", "method": " qa_setup ", "args": []}}
        ],
        "assert": [{
            "screenshot_match": {
                "snapshot": " after_start ",
                "baseline": " qa/baselines/main_menu_start_opens_pause/after_start.png ",
                "threshold": 0.02
            }
        }]
    }));

    assert_eq!(
        normalized,
        json!({
            "name": "main_menu_start_opens_pause",
            "scene": "res://ui/main_menu.tscn",
            "viewport": {},
            "determinism": {},
            "snapshots": {},
            "steps": [
                {"snapshot": "after_start"},
                {"teleport_node": {"target": "qa:player", "to": "qa:anchor"}},
                {"fixture_teleport": {"target": "qa:player", "to_point": {"provider": "qa:raft", "method": "qa_point_probe", "id": "cell:0,2"}}},
                {"spawn_scene": {"scene": "res://fixtures/loot.tscn", "parent": "qa:arena", "name": "Loot", "qa_id": "test_loot", "at": "qa:anchor"}},
                {"set_property": {"target": "qa:team_1", "property": "scrap", "value": 10}},
                {"call_method": {"target": "qa:manager", "method": "qa_setup", "args": []}}
            ],
            "assert": [{
                "screenshot_match": {
                    "snapshot": "after_start",
                    "baseline": "qa/baselines/main_menu_start_opens_pause/after_start.png",
                    "threshold": 0.02
                }
            }],
            "viewports": [],
        })
    );
}

#[test]
fn scenario_godot_args_reject_legacy_launch_args() {
    let scenario_path = Path::new("scenario.json");
    let document = json!({
        "name": "example",
        "scene": "res://ui/main_menu.tscn",
        "launch_args": ["--rendering-driver", "opengl3"],
        "steps": [],
    });

    let error = validate_scenario_document(&document, scenario_path).unwrap_err();

    assert_eq!(error.failure.kind, "scenario_validation_failed");
    let message = error.failure.message.as_deref().unwrap_or_default();
    assert!(message.contains("launch_args"));
    assert!(message.contains("godot_args"));
    assert!(message.contains("project_args"));
}

#[test]
fn scenario_godot_args_validate_and_normalize_under_own_name() {
    let scenario_path = Path::new("scenario.json");
    let document = json!({
        "name": "example",
        "scene": "res://ui/main_menu.tscn",
        "godot_args": ["--rendering-driver", "opengl3", "--single-window"],
        "steps": [],
    });

    validate_scenario_document(&document, scenario_path).unwrap();
    let normalized = normalize_scenario_document(&document);

    assert_eq!(
        normalized["godot_args"],
        json!(["--rendering-driver", "opengl3", "--single-window"])
    );
    assert!(normalized.get("launch_args").is_none());
}

#[test]
fn scenario_godot_args_reject_non_string_or_empty_values() {
    let scenario_path = Path::new("scenario.json");
    for godot_args in [json!([123]), json!([""])] {
        let document = json!({
            "name": "example",
            "scene": "res://ui/main_menu.tscn",
            "godot_args": godot_args,
            "steps": [],
        });

        let error = validate_scenario_document(&document, scenario_path).unwrap_err();
        assert_eq!(error.failure.kind, "scenario_validation_failed");
        assert!(
            error
                .failure
                .message
                .as_deref()
                .unwrap_or_default()
                .contains("godot_args")
        );
    }
}

#[test]
fn project_args_validate_and_normalize_reserved_contract() {
    let scenario_path = Path::new("scenario.json");
    let document = json!({
        "name": "example",
        "scene": "res://ui/main_menu.tscn",
        "project_args": ["--save-slot", "qa"],
        "steps": [],
    });

    validate_scenario_document(&document, scenario_path).unwrap();
    let normalized = normalize_scenario_document(&document);

    assert_eq!(normalized["project_args"], json!(["--save-slot", "qa"]));
    assert!(normalized.get("godot_args").is_none());
    assert!(normalized.get("launch_args").is_none());
}

#[test]
fn project_args_coexist_with_godot_args() {
    let scenario_path = Path::new("scenario.json");
    let document = json!({
        "name": "example",
        "scene": "res://ui/main_menu.tscn",
        "godot_args": ["--rendering-driver", "opengl3"],
        "project_args": ["--save-slot", "qa"],
        "steps": [],
    });

    validate_scenario_document(&document, scenario_path).unwrap();
    let normalized = normalize_scenario_document(&document);

    assert_eq!(
        normalized["godot_args"],
        json!(["--rendering-driver", "opengl3"])
    );
    assert_eq!(normalized["project_args"], json!(["--save-slot", "qa"]));
    assert!(normalized.get("launch_args").is_none());
}

#[test]
fn project_args_are_optional_for_normal_scenarios() {
    let scenario_path = Path::new("scenario.json");
    let document = json!({
        "name": "example",
        "scene": "res://ui/main_menu.tscn",
        "steps": [],
    });

    validate_scenario_document(&document, scenario_path).unwrap();
    let normalized = normalize_scenario_document(&document);

    assert!(normalized.get("project_args").is_none());
}

#[test]
fn project_args_reject_non_array_values() {
    let scenario_path = Path::new("scenario.json");
    let document = json!({
        "name": "example",
        "scene": "res://ui/main_menu.tscn",
        "project_args": "--save-slot qa",
        "steps": [],
    });

    let error = validate_scenario_document(&document, scenario_path).unwrap_err();

    assert_eq!(
        failure_json(error),
        json!({
            "type": "scenario_validation_failed",
            "path": scenario_path,
            "message": "scenario field 'project_args' must be a list of non-empty strings",
            "expected": "list of non-empty strings",
            "actual": "str",
        })
    );
}

#[test]
fn project_args_reject_non_string_entries() {
    let scenario_path = Path::new("scenario.json");
    let document = json!({
        "name": "example",
        "scene": "res://ui/main_menu.tscn",
        "project_args": ["--save-slot", 123],
        "steps": [],
    });

    let error = validate_scenario_document(&document, scenario_path).unwrap_err();

    assert_eq!(
        failure_json(error),
        json!({
            "type": "scenario_validation_failed",
            "path": scenario_path,
            "message": "scenario field 'project_args' must contain only strings",
            "expected": "string",
            "actual": {"index": 1, "actual": "int"},
        })
    );
}

#[test]
fn project_args_reject_empty_string_entries() {
    let scenario_path = Path::new("scenario.json");
    let document = json!({
        "name": "example",
        "scene": "res://ui/main_menu.tscn",
        "project_args": ["--save-slot", ""],
        "steps": [],
    });

    let error = validate_scenario_document(&document, scenario_path).unwrap_err();

    assert_eq!(
        failure_json(error),
        json!({
            "type": "scenario_validation_failed",
            "path": scenario_path,
            "message": "scenario field 'project_args' must contain only non-empty strings",
            "expected": "non-empty string",
            "actual": {"index": 1, "actual": ""},
        })
    );
}

#[test]
fn project_args_reject_legacy_launch_args() {
    let scenario_path = Path::new("scenario.json");
    let document = json!({
        "name": "example",
        "scene": "res://ui/main_menu.tscn",
        "project_args": ["--save-slot", "qa"],
        "launch_args": ["--legacy"],
        "steps": [],
    });

    let error = validate_scenario_document(&document, scenario_path).unwrap_err();

    assert_eq!(
        failure_json(error),
        json!({
            "type": "scenario_validation_failed",
            "path": scenario_path,
            "message": "scenario field 'launch_args' is not supported; use 'godot_args' for Godot engine flags before '--' or 'project_args' for reserved project/game flags after '--'",
            "expected": "godot_args or project_args",
            "actual": ["--legacy"],
        })
    );
}

#[test]
fn load_and_normalize_scenario_document_runs_full_pipeline() {
    let root = temp_dir("scenario-pipeline");
    let scenario_path = root.join("scenario.yml");
    write(
        &scenario_path,
        "name: example\nscene: res://ui/main_menu.tscn\nsteps:\n  - snapshot: before\nassert:\n  - exists: qa:pause_menu\n",
    );

    let normalized = load_and_normalize_scenario_document(&scenario_path).unwrap();

    assert_eq!(
        normalized,
        json!({
            "name": "example",
            "scene": "res://ui/main_menu.tscn",
            "viewport": {},
            "determinism": {},
            "snapshots": {},
            "steps": [{"snapshot": "before"}],
            "assert": [{"exists": "qa:pause_menu"}],
            "viewports": [],
        })
    );
}

#[test]
fn temporal_validate_accepts_supported_bounded_payloads() {
    let root = temp_dir("scenario-temporal-valid");
    let scenario_path = root.join("scenario.json");
    let document = json!({
        "name": "temporal_valid",
        "scene": "res://ui/main_menu.tscn",
        "steps": [{"wait_frames": 1}],
        "assert": [
            {"max_delta_position": {"target": "qa:player", "window_frames": 12, "sample_every_frames": 2, "max": 32}},
            {"follows": {"leader": "qa:leader", "follower": "qa:follower", "window_seconds": 0.5, "tolerance": 5.0}},
            {"within_limits": {"target": "qa:speed", "window_frames": 10, "min": 0.0, "max": 20.0}},
            {"never": {"target": "qa:status", "window_frames": 30, "property": "state", "forbidden": ["error", "panic"]}}
        ],
    });

    validate_scenario_document(&document, &scenario_path).unwrap();
}

#[test]
fn temporal_validate_rejects_missing_target() {
    let root = temp_dir("scenario-temporal-missing-target");
    let scenario_path = root.join("scenario.json");
    let document = json!({
        "name": "temporal_invalid",
        "scene": "res://ui/main_menu.tscn",
        "steps": [{"wait_frames": 1}],
        "assert": [{"max_delta_position": {"window_frames": 12, "max": 32}}],
    });

    let error = validate_scenario_document(&document, &scenario_path).unwrap_err();
    assert_eq!(
        failure_json(error),
        json!({
            "type": "scenario_validation_failed",
            "path": scenario_path,
            "message": "max_delta_position assertion field 'target' must be a non-empty string",
            "expected": "non-empty string",
            "actual": null,
        })
    );
}

#[test]
fn temporal_validate_rejects_ambiguous_window_fields() {
    let root = temp_dir("scenario-temporal-ambiguous-window");
    let scenario_path = root.join("scenario.json");
    let document = json!({
        "name": "temporal_invalid",
        "scene": "res://ui/main_menu.tscn",
        "steps": [{"wait_frames": 1}],
        "assert": [{"follows": {"leader": "qa:a", "follower": "qa:b", "window_frames": 10, "window_seconds": 1.0, "tolerance": 1}}],
    });

    let error = validate_scenario_document(&document, &scenario_path).unwrap_err();
    assert_eq!(
        failure_json(error),
        json!({
            "type": "scenario_validation_failed",
            "path": scenario_path,
            "message": "follows assertion must declare exactly one of 'window_frames' or 'window_seconds'",
            "expected": "exactly one window form",
            "actual": {"leader": "qa:a", "follower": "qa:b", "window_frames": 10, "window_seconds": 1.0, "tolerance": 1},
        })
    );
}

#[test]
fn temporal_validate_rejects_negative_window() {
    let root = temp_dir("scenario-temporal-negative-window");
    let scenario_path = root.join("scenario.json");
    let document = json!({
        "name": "temporal_invalid",
        "scene": "res://ui/main_menu.tscn",
        "steps": [{"wait_frames": 1}],
        "assert": [{"within_limits": {"target": "qa:node", "window_seconds": -1, "min": 0, "max": 1}}],
    });

    let error = validate_scenario_document(&document, &scenario_path).unwrap_err();
    assert_eq!(
        failure_json(error),
        json!({
            "type": "scenario_validation_failed",
            "path": scenario_path,
            "message": "within_limits assertion field 'window_seconds' must be a positive number",
            "expected": "positive number",
            "actual": -1,
        })
    );
}

#[test]
fn temporal_validate_rejects_non_numeric_bound() {
    let root = temp_dir("scenario-temporal-non-numeric-bound");
    let scenario_path = root.join("scenario.json");
    let document = json!({
        "name": "temporal_invalid",
        "scene": "res://ui/main_menu.tscn",
        "steps": [{"wait_frames": 1}],
        "assert": [{"max_delta_position": {"target": "qa:node", "window_frames": 10, "max": "high"}}],
    });

    let error = validate_scenario_document(&document, &scenario_path).unwrap_err();
    assert_eq!(
        failure_json(error),
        json!({
            "type": "scenario_validation_failed",
            "path": scenario_path,
            "message": "max_delta_position assertion field 'max' must be a number",
            "expected": "number",
            "actual": "high",
        })
    );
}

#[test]
fn temporal_validate_rejects_empty_forbidden_values() {
    let root = temp_dir("scenario-temporal-empty-forbidden");
    let scenario_path = root.join("scenario.json");
    let document = json!({
        "name": "temporal_invalid",
        "scene": "res://ui/main_menu.tscn",
        "steps": [{"wait_frames": 1}],
        "assert": [{"never": {"target": "qa:status", "window_frames": 8, "token": "state", "forbidden": [""]}}],
    });

    let error = validate_scenario_document(&document, &scenario_path).unwrap_err();
    assert_eq!(
        failure_json(error),
        json!({
            "type": "scenario_validation_failed",
            "path": scenario_path,
            "message": "never assertion field 'forbidden' must be a string or list of non-empty strings",
            "expected": "string or list of non-empty strings",
            "actual": [""],
        })
    );
}

#[test]
fn temporal_validate_rejects_unsupported_observation_fields() {
    let root = temp_dir("scenario-temporal-observation-field");
    let scenario_path = root.join("scenario.json");
    let document = json!({
        "name": "temporal_invalid",
        "scene": "res://ui/main_menu.tscn",
        "steps": [{"wait_frames": 1}],
        "assert": [{"never": {"target": "qa:status", "window_frames": 8, "value": "bad", "forbidden": "x"}}],
    });

    let error = validate_scenario_document(&document, &scenario_path).unwrap_err();
    assert_eq!(
        failure_json(error),
        json!({
            "type": "scenario_validation_failed",
            "path": scenario_path,
            "message": "never assertion must declare exactly one observation source: 'property', 'text', or 'token'",
            "expected": ["property", "text", "token"],
            "actual": {"target": "qa:status", "window_frames": 8, "value": "bad", "forbidden": "x"},
        })
    );
}

#[test]
fn within_operator_validation_contracts() {
    let path = Path::new("scenario.json");

    // wait_until accepts `within` with a positive tolerance.
    let valid_wait_until = json!({
        "name": "within_wait_until",
        "scene": "res://main.tscn",
        "steps": [{
            "wait_until": {
                "source": "provider",
                "provider": "match_state",
                "path": "core.temperature",
                "operator": "within",
                "expected": 100,
                "tolerance": 5,
                "timeout_frames": 60
            }
        }],
    });
    validate_scenario_document(&valid_wait_until, path)
        .expect("wait_until within with tolerance should validate");

    // state_matches accepts `within` in both the flat form and matches entries.
    let valid_state_matches = json!({
        "name": "within_state_matches",
        "scene": "res://main.tscn",
        "steps": [],
        "assert": [
            {"state_matches": {"target": "qa:core", "source": "method", "method": "qa_core_state", "path": "x", "operator": "within", "expected": 100, "tolerance": 5}},
            {"state_matches": {"target": "qa:core", "source": "method", "method": "qa_core_state", "matches": [
                {"path": "x", "operator": "within", "expected": 100, "tolerance": 0.5}
            ]}},
        ],
    });
    validate_scenario_document(&valid_state_matches, path)
        .expect("state_matches within with tolerance should validate");

    // drive_policy `until` accepts `within` with a tolerance.
    let valid_drive_policy = json!({
        "name": "within_drive_policy",
        "scene": "res://main.tscn",
        "steps": [{
            "drive_policy": {
                "policy": "crew_bot",
                "devices": [0],
                "max_frames": 60,
                "until": {
                    "source": "provider",
                    "provider": "match_state",
                    "path": "core.temperature",
                    "operator": "within",
                    "expected": 100,
                    "tolerance": 5
                }
            }
        }],
    });
    validate_scenario_document(&valid_drive_policy, path)
        .expect("drive_policy until within with tolerance should validate");

    // Missing tolerance fails with a clear message.
    let missing_tolerance = json!({
        "name": "x",
        "scene": "res://main.tscn",
        "steps": [{
            "wait_until": {"source": "provider", "provider": "p", "path": "a", "operator": "within", "expected": 1, "timeout_frames": 5}
        }],
    });
    let error = validate_scenario_document(&missing_tolerance, path).unwrap_err();
    assert_eq!(
        failure_json(error),
        json!({
            "type": "scenario_validation_failed",
            "path": "scenario.json",
            "message": "wait_until step field 'tolerance' must be a positive number for operator 'within'",
            "expected": "positive number",
            "actual": null,
        })
    );

    // Non-positive and non-numeric tolerances fail; missing expected fails.
    for (name, payload) in [
        (
            "zero_tolerance",
            json!({"path": "a", "operator": "within", "expected": 1, "tolerance": 0}),
        ),
        (
            "negative_tolerance",
            json!({"path": "a", "operator": "within", "expected": 1, "tolerance": -2}),
        ),
        (
            "string_tolerance",
            json!({"path": "a", "operator": "within", "expected": 1, "tolerance": "close"}),
        ),
        (
            "missing_expected",
            json!({"path": "a", "operator": "within", "tolerance": 1}),
        ),
    ] {
        let mut wait_until = payload;
        wait_until["source"] = json!("provider");
        wait_until["provider"] = json!("p");
        wait_until["timeout_frames"] = json!(5);
        let document = json!({
            "name": "x",
            "scene": "res://main.tscn",
            "steps": [{"wait_until": wait_until}],
        });
        assert!(
            validate_scenario_document(&document, path).is_err(),
            "{name} should fail scenario validation"
        );
    }

    // state_matches check entries missing tolerance fail early.
    let state_matches_missing_tolerance = json!({
        "name": "x",
        "scene": "res://main.tscn",
        "steps": [],
        "assert": [{"state_matches": {"target": "qa:core", "matches": [
            {"path": "x", "operator": "within", "expected": 100}
        ]}}],
    });
    let error = validate_scenario_document(&state_matches_missing_tolerance, path).unwrap_err();
    assert_eq!(
        failure_json(error)["message"],
        "state_matches assertion check field 'tolerance' must be a positive number for operator 'within'"
    );

    // observe_until property conditions using `within` require tolerance.
    let observe_until_missing_tolerance = json!({
        "name": "x",
        "scene": "res://main.tscn",
        "steps": [],
        "assert": [{"observe_until": {"target": "qa:status", "window_frames": 4, "property": "value", "operator": "within", "expected": 10}}],
    });
    let error = validate_scenario_document(&observe_until_missing_tolerance, path).unwrap_err();
    assert_eq!(
        failure_json(error)["message"],
        "observe_until assertion field 'tolerance' must be a positive number for operator 'within'"
    );
}

#[test]
fn temporal_state_observation_validation_contracts() {
    let path = Path::new("scenario.json");

    // within_limits and never accept provider/method state observations; a
    // provider observation does not require a node target.
    let valid = json!({
        "name": "temporal_state_observations",
        "scene": "res://main.tscn",
        "steps": [],
        "assert": [
            {"within_limits": {"source": "provider", "provider": "match_state", "path": "core.instability", "window_frames": 30, "min": 0, "max": 0.8}},
            {"within_limits": {"source": "method", "target": "qa:match_controller", "method": "qa_instability_state", "path": "value", "window_frames": 30, "min": 0, "max": 0.8}},
            {"never": {"source": "provider", "provider": "match_state", "path": "core.phase", "forbidden": ["meltdown"], "window_frames": 30}},
            {"never": {"source": "method", "target": "qa:match_controller", "method": "qa_instability_state", "path": "phase", "forbidden": "meltdown", "window_frames": 30}},
        ],
    });
    validate_scenario_document(&valid, path)
        .expect("state-observation temporal assertions should validate");

    for (name, assertion) in [
        (
            "unsupported_source",
            json!({"within_limits": {"source": "metadata", "target": "qa:node", "window_frames": 4, "min": 0, "max": 1}}),
        ),
        (
            "provider_source_missing_provider",
            json!({"within_limits": {"source": "provider", "path": "a", "window_frames": 4, "min": 0, "max": 1}}),
        ),
        (
            "method_source_missing_target",
            json!({"within_limits": {"source": "method", "method": "qa_state", "window_frames": 4, "min": 0, "max": 1}}),
        ),
        (
            "method_source_missing_method",
            json!({"within_limits": {"source": "method", "target": "qa:node", "window_frames": 4, "min": 0, "max": 1}}),
        ),
        (
            "malformed_observation_path",
            json!({"within_limits": {"source": "provider", "provider": "p", "path": "a..b", "window_frames": 4, "min": 0, "max": 1}}),
        ),
        (
            "never_source_with_legacy_observation",
            json!({"never": {"source": "provider", "provider": "p", "path": "a", "token": "state", "forbidden": "x", "window_frames": 4}}),
        ),
        (
            "never_source_missing_forbidden",
            json!({"never": {"source": "provider", "provider": "p", "path": "a", "window_frames": 4}}),
        ),
    ] {
        let document = json!({
            "name": "x",
            "scene": "res://main.tscn",
            "steps": [],
            "assert": [assertion],
        });
        assert!(
            validate_scenario_document(&document, path).is_err(),
            "{name} should fail scenario validation"
        );
    }
}

#[test]
fn trend_assertion_validation_contracts() {
    let path = Path::new("scenario.json");

    let valid = json!({
        "name": "trend_valid",
        "scene": "res://main.tscn",
        "steps": [],
        "assert": [
            {"trend": {"target": "qa:actor", "property": "position", "trend": "increasing", "window_frames": 12}},
            {"trend": {"target": "qa:actor", "trend": "non_decreasing", "min_delta": 4, "window_seconds": 0.5}},
            {"trend": {"source": "provider", "provider": "match_state", "path": "core.instability", "trend": "decreasing", "tolerance": 0.01, "min_delta": 0.1, "window_frames": 30}},
            {"trend": {"source": "method", "target": "qa:match_controller", "method": "qa_instability_state", "path": "value", "trend": "non_increasing", "window_frames": 30, "sample_every_frames": 2}},
        ],
    });
    validate_scenario_document(&valid, path).expect("valid trend assertions should validate");

    let missing_direction = json!({
        "name": "x",
        "scene": "res://main.tscn",
        "steps": [],
        "assert": [{"trend": {"target": "qa:actor", "window_frames": 12}}],
    });
    let error = validate_scenario_document(&missing_direction, path).unwrap_err();
    assert_eq!(
        failure_json(error),
        json!({
            "type": "scenario_validation_failed",
            "path": "scenario.json",
            "message": "trend assertion field 'trend' must be a supported direction",
            "expected": ["increasing", "decreasing", "non_increasing", "non_decreasing"],
            "actual": null,
        })
    );

    for (name, assertion) in [
        (
            "unsupported_direction",
            json!({"trend": {"target": "qa:actor", "trend": "sideways", "window_frames": 12}}),
        ),
        (
            "missing_window",
            json!({"trend": {"target": "qa:actor", "trend": "increasing"}}),
        ),
        (
            "missing_target_without_source",
            json!({"trend": {"trend": "increasing", "window_frames": 12}}),
        ),
        (
            "negative_min_delta",
            json!({"trend": {"target": "qa:actor", "trend": "increasing", "min_delta": -1, "window_frames": 12}}),
        ),
        (
            "negative_tolerance",
            json!({"trend": {"target": "qa:actor", "trend": "increasing", "tolerance": -0.5, "window_frames": 12}}),
        ),
        (
            "zero_sample_cadence",
            json!({"trend": {"target": "qa:actor", "trend": "increasing", "window_frames": 12, "sample_every_frames": 0}}),
        ),
    ] {
        let document = json!({
            "name": "x",
            "scene": "res://main.tscn",
            "steps": [],
            "assert": [assertion],
        });
        assert!(
            validate_scenario_document(&document, path).is_err(),
            "{name} should fail scenario validation"
        );
    }
}

#[test]
fn frame_time_assertion_validation_contracts() {
    let path = Path::new("scenario.json");

    let valid = json!({
        "name": "frame_time_valid",
        "scene": "res://main.tscn",
        "steps": [{"wait_frames": 5}],
        "assert": [
            {"frame_time": {"window_frames": 300, "max_avg_ms": 16.7, "max_p95_ms": 20, "min_fps": 55, "sample_every_frames": 1}},
            {"frame_time": {"window_frames": 60, "min_fps": 55}},
        ],
    });
    validate_scenario_document(&valid, path).expect("valid frame_time assertions should validate");

    let no_bounds = json!({
        "name": "x",
        "scene": "res://main.tscn",
        "steps": [],
        "assert": [{"frame_time": {"window_frames": 300}}],
    });
    let error = validate_scenario_document(&no_bounds, path).unwrap_err();
    assert_eq!(
        failure_json(error)["message"],
        "frame_time assertion requires at least one bound: 'max_avg_ms', 'max_p95_ms', or 'min_fps'"
    );

    for (name, assertion) in [
        (
            "non_positive_bound",
            json!({"frame_time": {"window_frames": 60, "max_avg_ms": 0}}),
        ),
        (
            "non_numeric_bound",
            json!({"frame_time": {"window_frames": 60, "min_fps": "fast"}}),
        ),
        ("missing_window", json!({"frame_time": {"min_fps": 55}})),
        (
            "temporal_sampling_rejected",
            json!({"frame_time": {"window_frames": 60, "min_fps": 55, "temporal_sampling": {"mode": "fixed_delta", "fixed_delta_seconds": 0.016}}}),
        ),
    ] {
        let document = json!({
            "name": "x",
            "scene": "res://main.tscn",
            "steps": [],
            "assert": [assertion],
        });
        assert!(
            validate_scenario_document(&document, path).is_err(),
            "{name} should fail scenario validation"
        );
    }

    // frame_time cannot be combined with determinism.fixed_fps, whether the
    // assertion is top-level or step-local.
    let fixed_fps_top_level = json!({
        "name": "x",
        "scene": "res://main.tscn",
        "determinism": {"fixed_fps": 60},
        "steps": [],
        "assert": [{"frame_time": {"window_frames": 60, "min_fps": 55}}],
    });
    let error = validate_scenario_document(&fixed_fps_top_level, path).unwrap_err();
    assert_eq!(
        failure_json(error),
        json!({
            "type": "scenario_validation_failed",
            "path": "scenario.json",
            "message": "frame_time assertions cannot be combined with determinism.fixed_fps: forced frame pacing makes wall-clock frame times meaningless; drop fixed_fps or move the frame_time gate into a separate perf scenario",
            "expected": "no determinism.fixed_fps alongside frame_time assertions",
            "actual": {"determinism.fixed_fps": 60},
        })
    );

    let fixed_fps_step_local = json!({
        "name": "x",
        "scene": "res://main.tscn",
        "determinism": {"fixed_fps": 60},
        "steps": [{"assert": [{"frame_time": {"window_frames": 60, "min_fps": 55}}]}],
        "assert": [],
    });
    assert!(
        validate_scenario_document(&fixed_fps_step_local, path).is_err(),
        "step-local frame_time under fixed_fps should fail scenario validation"
    );

    // Without fixed_fps the same step-local assertion is fine.
    let step_local_ok = json!({
        "name": "frame_time_step_local",
        "scene": "res://main.tscn",
        "steps": [{"assert": [{"frame_time": {"window_frames": 60, "min_fps": 55}}]}],
        "assert": [],
    });
    validate_scenario_document(&step_local_ok, path)
        .expect("step-local frame_time without fixed_fps should validate");
}

#[test]
fn temporal_observe_until_validation_contracts() {
    let root = temp_dir("scenario-observe-until-contracts");
    let scenario_path = root.join("scenario.json");
    let base_document = || {
        json!({
            "name": "observe_until_valid",
            "scene": "res://ui/main_menu.tscn",
            "determinism": {
                "fixed_fps": 60,
                "temporal_sampling": {
                    "mode": "fixed_delta",
                    "fixed_delta_seconds": 0.0166667
                }
            },
            "steps": [{"wait_frames": 1}],
            "assert": []
        })
    };

    for assertion in [
        json!({"observe_until": {"target": "qa:status", "window_frames": 12, "sample_every_frames": 2, "property": "state", "operator": "equals", "expected": "pending"}}),
        json!({"observe_until": {"target": "qa:spinner", "window_seconds": 0.25, "visible": true}}),
        json!({"observe_until": {"target": "qa:manager", "window_frames": 8, "source": "qa:manager", "method": "qa_state", "matches": {"player.pending": true}}}),
        json!({"observe_until": {"target": "qa:status", "window_frames": 12, "visible": true, "temporal_sampling": {"mode": "max_frame_duration", "max_frame_duration_seconds": 0.0166667}}}),
    ] {
        let mut document = base_document();
        document["assert"] = json!([assertion]);
        validate_scenario_document(&document, &scenario_path).unwrap();
    }

    for (name, patch) in [
        (
            "unbounded",
            json!({"assert": [{"observe_until": {"target": "qa:status", "property": "state", "operator": "equals", "expected": "pending"}}]}),
        ),
        (
            "negative_window",
            json!({"assert": [{"observe_until": {"target": "qa:status", "window_seconds": -0.1, "visible": true}}]}),
        ),
        (
            "zero_sample_cadence",
            json!({"assert": [{"observe_until": {"target": "qa:status", "window_frames": 2, "sample_every_frames": 0, "visible": true}}]}),
        ),
        (
            "malformed_state_path",
            json!({"assert": [{"observe_until": {"target": "qa:manager", "window_frames": 8, "source": "qa:manager", "method": "qa_state", "matches": {"player..pending": true}}}]}),
        ),
        (
            "malformed_temporal_sampling_mode",
            json!({"determinism": {"temporal_sampling": {"mode": "adaptive", "fixed_delta_seconds": 0.016}}}),
        ),
        (
            "mismatched_temporal_sampling_field",
            json!({"determinism": {"temporal_sampling": {"mode": "max_frame_duration", "fixed_delta_seconds": 0.016}}}),
        ),
        (
            "observe_until_invalid_local_temporal_sampling",
            json!({"assert": [{"observe_until": {"target": "qa:status", "window_frames": 2, "visible": true, "temporal_sampling": {"mode": "fixed_delta", "max_frame_duration_seconds": 0.016}}}]}),
        ),
        (
            "max_delta_invalid_local_temporal_sampling",
            json!({"assert": [{"max_delta_position": {"target": "qa:actor", "window_frames": 2, "max": 4, "temporal_sampling": {"mode": "adaptive", "fixed_delta_seconds": 0.016}}}]}),
        ),
        (
            "follows_invalid_local_temporal_sampling",
            json!({"assert": [{"follows": {"leader": "qa:leader", "follower": "qa:follower", "window_frames": 2, "tolerance": 4, "temporal_sampling": {"mode": "fixed_delta", "fixed_delta_seconds": 0}}}]}),
        ),
        (
            "within_limits_invalid_local_temporal_sampling",
            json!({"assert": [{"within_limits": {"target": "qa:actor", "window_frames": 2, "min": 0, "max": 4, "temporal_sampling": {"mode": "max_frame_duration", "fixed_delta_seconds": 0.016}}}]}),
        ),
        (
            "distance_changed_invalid_local_temporal_sampling",
            json!({"assert": [{"distance_changed": {"a": "qa:a", "b": "qa:b", "window_frames": 2, "min": 1, "temporal_sampling": "fixed"}}]}),
        ),
        (
            "never_invalid_local_temporal_sampling",
            json!({"assert": [{"never": {"target": "qa:status", "property": "state", "window_frames": 2, "forbidden": ["done"], "temporal_sampling": {"mode": "max_frame_duration", "max_frame_duration_seconds": -0.016}}}]}),
        ),
    ] {
        let mut document = base_document();
        if let Some(assertions) = patch.get("assert") {
            document["assert"] = assertions.clone();
        }
        if let Some(determinism) = patch.get("determinism") {
            document["determinism"] = determinism.clone();
            document["assert"] = json!([{"observe_until": {"target": "qa:spinner", "window_frames": 1, "visible": true}}]);
        }
        let error = match validate_scenario_document(&document, &scenario_path) {
            Ok(()) => panic!("{name} should fail scenario validation"),
            Err(error) => error,
        };
        assert_eq!(
            failure_json(error)["type"],
            "scenario_validation_failed",
            "{name}"
        );
    }

    let normalized = normalize_scenario_document(&json!({
        "name": " observe_until_normalize ",
        "scene": " res://ui/main_menu.tscn ",
        "steps": [],
        "assert": [{
            "observe_until": {
                "target": " qa:manager ",
                "window_frames": 8,
                "source": " qa:manager ",
                "method": " qa_state ",
                "matches": {" player.pending ": true}
            }
        }]
    }));
    assert_eq!(
        normalized["assert"][0]["observe_until"]["target"],
        "qa:manager"
    );
    assert_eq!(
        normalized["assert"][0]["observe_until"]["source"],
        "qa:manager"
    );
    assert_eq!(
        normalized["assert"][0]["observe_until"]["method"],
        "qa_state"
    );
    assert_eq!(
        normalized["assert"][0]["observe_until"]["matches"],
        json!({"player.pending": true})
    );
}

#[test]
fn temporal_normalize_trims_temporal_string_fields() {
    let normalized = normalize_scenario_document(&json!({
        "name": " temporal ",
        "scene": " res://ui/main_menu.tscn ",
        "steps": [],
        "assert": [
            {"max_delta_position": {"target": " qa:player ", "window_frames": 12, "max": 32}},
            {"follows": {"leader": " qa:leader ", "follower": " qa:follower ", "window_frames": 12, "tolerance": 2}},
            {"within_limits": {"target": " qa:speed ", "window_frames": 12, "min": 0, "max": 5}},
            {"never": {"target": " qa:status ", "property": " state ", "window_frames": 12, "forbidden": [" error ", " panic "]}}
        ]
    }));

    assert_eq!(
        normalized["assert"][0]["max_delta_position"]["target"],
        "qa:player"
    );
    assert_eq!(normalized["assert"][1]["follows"]["leader"], "qa:leader");
    assert_eq!(
        normalized["assert"][1]["follows"]["follower"],
        "qa:follower"
    );
    assert_eq!(
        normalized["assert"][2]["within_limits"]["target"],
        "qa:speed"
    );
    assert_eq!(normalized["assert"][3]["never"]["target"], "qa:status");
    assert_eq!(normalized["assert"][3]["never"]["property"], "state");
    assert_eq!(
        normalized["assert"][3]["never"]["forbidden"],
        json!(["error", "panic"])
    );
}

#[test]
fn temporal_examples_load_and_normalize() {
    let repo_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .unwrap();
    let scenarios_dir = repo_root.join("examples/sample_project/qa/scenarios/examples");

    for file_name in [
        "temporal_assertions_demo.yaml",
        "temporal_assertions_failure_demo.yaml",
        "temporal_observe_until_demo.yaml",
        "temporal_observe_until_late_failure_demo.yaml",
    ] {
        let path = scenarios_dir.join(file_name);
        let normalized = load_and_normalize_scenario_document(&path).unwrap();
        assert!(
            normalized.is_object(),
            "normalized scenario should be an object"
        );
        assert_eq!(
            normalized["scene"],
            json!("res://ui/main_menu.tscn"),
            "scenario {file_name} should target main menu scene"
        );
    }
}

#[test]
fn proximity_validate_accepts_supported_payloads() {
    let root = temp_dir("scenario-proximity-valid");
    let scenario_path = root.join("scenario.json");
    let document = json!({
        "name": "proximity_valid",
        "scene": "res://ui/main_menu.tscn",
        "steps": [{"wait_frames": 1}],
        "assert": [
            {"distance": {"a": "qa:actor", "b": "qa:target", "max": 96}},
            {"distance": {"a": "qa:actor", "b": "qa:target", "min": 32}},
            {"distance_changed": {"a": "qa:actor", "b": "qa:target", "window_frames": 20, "sample_every_frames": 2, "min": 8}}
        ],
    });

    validate_scenario_document(&document, &scenario_path).unwrap();
}

#[test]
fn proximity_validate_rejects_missing_locators() {
    let root = temp_dir("scenario-proximity-missing-locators");
    let scenario_path = root.join("scenario.json");
    let document = json!({
        "name": "proximity_invalid",
        "scene": "res://ui/main_menu.tscn",
        "steps": [{"wait_frames": 1}],
        "assert": [{"distance": {"a": "qa:actor", "max": 96}}],
    });

    let error = validate_scenario_document(&document, &scenario_path).unwrap_err();
    assert_eq!(
        failure_json(error),
        json!({
            "type": "scenario_validation_failed",
            "path": scenario_path,
            "message": "distance assertion field 'b' must be a non-empty string",
            "expected": "non-empty string",
            "actual": null,
        })
    );
}

#[test]
fn proximity_validate_rejects_missing_bounds() {
    let root = temp_dir("scenario-proximity-missing-bounds");
    let scenario_path = root.join("scenario.json");
    let document = json!({
        "name": "proximity_invalid",
        "scene": "res://ui/main_menu.tscn",
        "steps": [{"wait_frames": 1}],
        "assert": [{"distance": {"a": "qa:actor", "b": "qa:target"}}],
    });

    let error = validate_scenario_document(&document, &scenario_path).unwrap_err();
    assert_eq!(
        failure_json(error),
        json!({
            "type": "scenario_validation_failed",
            "path": scenario_path,
            "message": "distance assertion requires at least one of 'min' or 'max'",
            "expected": "at least one of 'min' or 'max'",
            "actual": {"a": "qa:actor", "b": "qa:target"},
        })
    );
}

#[test]
fn proximity_validate_rejects_negative_or_non_numeric_thresholds() {
    let root = temp_dir("scenario-proximity-thresholds");
    let scenario_path = root.join("scenario.json");

    let negative = json!({
        "name": "proximity_invalid",
        "scene": "res://ui/main_menu.tscn",
        "steps": [{"wait_frames": 1}],
        "assert": [{"distance": {"a": "qa:actor", "b": "qa:target", "max": -1}}],
    });
    let error = validate_scenario_document(&negative, &scenario_path).unwrap_err();
    assert_eq!(
        failure_json(error),
        json!({
            "type": "scenario_validation_failed",
            "path": scenario_path,
            "message": "distance assertion field 'max' must be a finite non-negative number",
            "expected": "finite non-negative number",
            "actual": -1,
        })
    );

    let non_numeric = json!({
        "name": "proximity_invalid",
        "scene": "res://ui/main_menu.tscn",
        "steps": [{"wait_frames": 1}],
        "assert": [{"distance_changed": {"a": "qa:actor", "b": "qa:target", "window_frames": 10, "min": "up"}}],
    });
    let error = validate_scenario_document(&non_numeric, &scenario_path).unwrap_err();
    assert_eq!(
        failure_json(error),
        json!({
            "type": "scenario_validation_failed",
            "path": scenario_path,
            "message": "distance_changed assertion field 'min' must be a finite non-negative number",
            "expected": "finite non-negative number",
            "actual": "up",
        })
    );
}

#[test]
fn proximity_validate_rejects_ambiguous_or_missing_temporal_window() {
    let root = temp_dir("scenario-proximity-window");
    let scenario_path = root.join("scenario.json");

    let ambiguous = json!({
        "name": "proximity_invalid",
        "scene": "res://ui/main_menu.tscn",
        "steps": [{"wait_frames": 1}],
        "assert": [{"distance_changed": {"a": "qa:actor", "b": "qa:target", "window_frames": 10, "window_seconds": 0.5, "min": 2}}],
    });
    let error = validate_scenario_document(&ambiguous, &scenario_path).unwrap_err();
    assert_eq!(
        failure_json(error),
        json!({
            "type": "scenario_validation_failed",
            "path": scenario_path,
            "message": "distance_changed assertion must declare exactly one of 'window_frames' or 'window_seconds'",
            "expected": "exactly one window form",
            "actual": {"a": "qa:actor", "b": "qa:target", "window_frames": 10, "window_seconds": 0.5, "min": 2},
        })
    );

    let missing = json!({
        "name": "proximity_invalid",
        "scene": "res://ui/main_menu.tscn",
        "steps": [{"wait_frames": 1}],
        "assert": [{"distance_changed": {"a": "qa:actor", "b": "qa:target", "min": 2}}],
    });
    let error = validate_scenario_document(&missing, &scenario_path).unwrap_err();
    assert_eq!(
        failure_json(error),
        json!({
            "type": "scenario_validation_failed",
            "path": scenario_path,
            "message": "distance_changed assertion must declare exactly one of 'window_frames' or 'window_seconds'",
            "expected": "exactly one window form",
            "actual": {"a": "qa:actor", "b": "qa:target", "min": 2},
        })
    );
}

#[test]
fn proximity_normalize_trims_locator_fields() {
    let normalized = normalize_scenario_document(&json!({
        "name": " proximity ",
        "scene": " res://ui/main_menu.tscn ",
        "steps": [],
        "assert": [
            {"distance": {"a": " qa:actor ", "b": " qa:target ", "max": 96}},
            {"distance_changed": {"a": " qa:actor ", "b": " qa:target ", "window_frames": 12, "min": 4}}
        ]
    }));

    assert_eq!(normalized["assert"][0]["distance"]["a"], "qa:actor");
    assert_eq!(normalized["assert"][0]["distance"]["b"], "qa:target");
    assert_eq!(normalized["assert"][0]["distance"]["max"], 96);
    assert_eq!(normalized["assert"][1]["distance_changed"]["a"], "qa:actor");
    assert_eq!(
        normalized["assert"][1]["distance_changed"]["b"],
        "qa:target"
    );
    assert_eq!(normalized["assert"][1]["distance_changed"]["min"], 4);
}

#[test]
fn validate_scenario_document_accepts_visual_assertions() {
    let root = temp_dir("scenario-visual-valid");
    let scenario_path = root.join("scenario.json");
    let document = json!({
        "name": "visual_valid",
        "scene": "res://ui/main_menu.tscn",
        "steps": [{"wait_frames": 1}],
        "assert": [
            {"visible_pixels": {"target": " qa:important_actor ", "snapshot": " occluded ", "min_ratio": 0.25, "min_count": 12}},
            {"not_occluded": {"target": " qa:important_actor ", "snapshot": " occluded ", "occluder": " qa:overlay ", "min_ratio": 0.1, "min_count": 1}},
            {"renders_above": {"target": " qa:foreground ", "other": " qa:background ", "snapshot": " order "}},
            {"renders_below": {"target": " qa:background ", "other": " qa:foreground ", "snapshot": " order "}}
        ],
    });

    validate_scenario_document(&document, &scenario_path).unwrap();
    let normalized = normalize_scenario_document(&document);
    assert_eq!(
        normalized["assert"][0]["visible_pixels"]["target"],
        "qa:important_actor"
    );
    assert_eq!(
        normalized["assert"][0]["visible_pixels"]["snapshot"],
        "occluded"
    );
    assert_eq!(
        normalized["assert"][1]["not_occluded"]["target"],
        "qa:important_actor"
    );
    assert_eq!(
        normalized["assert"][1]["not_occluded"]["occluder"],
        "qa:overlay"
    );
    assert_eq!(
        normalized["assert"][2]["renders_above"]["target"],
        "qa:foreground"
    );
    assert_eq!(
        normalized["assert"][2]["renders_above"]["other"],
        "qa:background"
    );
    assert_eq!(
        normalized["assert"][3]["renders_below"]["target"],
        "qa:background"
    );
    assert_eq!(
        normalized["assert"][3]["renders_below"]["other"],
        "qa:foreground"
    );
    assert_eq!(
        normalized["assert"][3]["renders_below"]["snapshot"],
        "order"
    );
}

#[test]
fn validate_scenario_document_rejects_invalid_visual_assertions() {
    let root = temp_dir("scenario-visual-invalid");
    let scenario_path = root.join("scenario.json");

    let invalid_ratio = json!({
        "name": "visual_invalid",
        "scene": "res://ui/main_menu.tscn",
        "steps": [{"wait_frames": 1}],
        "assert": [{"visible_pixels": {"target": "qa:actor", "snapshot": "after", "min_ratio": 1.5}}],
    });
    let error = validate_scenario_document(&invalid_ratio, &scenario_path).unwrap_err();
    assert_eq!(
        failure_json(error),
        json!({
            "type": "scenario_validation_failed",
            "path": scenario_path,
            "message": "visible_pixels assertion field 'min_ratio' must be between 0.0 and 1.0",
            "expected": "number between 0.0 and 1.0",
            "actual": 1.5,
        })
    );

    let negative_count = json!({
        "name": "visual_invalid",
        "scene": "res://ui/main_menu.tscn",
        "steps": [{"wait_frames": 1}],
        "assert": [{"not_occluded": {"target": "qa:actor", "snapshot": "after", "min_count": -1}}],
    });
    let error = validate_scenario_document(&negative_count, &scenario_path).unwrap_err();
    assert_eq!(
        failure_json(error),
        json!({
            "type": "scenario_validation_failed",
            "path": scenario_path,
            "message": "not_occluded assertion field 'min_count' must be a non-negative integer",
            "expected": "non-negative integer",
            "actual": -1,
        })
    );

    let missing_target = json!({
        "name": "visual_invalid",
        "scene": "res://ui/main_menu.tscn",
        "steps": [{"wait_frames": 1}],
        "assert": [{"visible_pixels": {"min_ratio": 0.5}}],
    });
    let error = validate_scenario_document(&missing_target, &scenario_path).unwrap_err();
    assert_eq!(
        failure_json(error),
        json!({
            "type": "scenario_validation_failed",
            "path": scenario_path,
            "message": "visible_pixels assertion field 'target' must be a non-empty string",
            "expected": "non-empty string",
            "actual": null,
        })
    );

    let missing_other = json!({
        "name": "visual_invalid",
        "scene": "res://ui/main_menu.tscn",
        "steps": [{"wait_frames": 1}],
        "assert": [{"renders_above": {"target": "qa:actor", "snapshot": "after"}}],
    });
    let error = validate_scenario_document(&missing_other, &scenario_path).unwrap_err();
    assert_eq!(
        failure_json(error),
        json!({
            "type": "scenario_validation_failed",
            "path": scenario_path,
            "message": "renders_above assertion field 'other' must be a non-empty string",
            "expected": "non-empty string",
            "actual": null,
        })
    );

    let missing_snapshot = json!({
        "name": "visual_invalid",
        "scene": "res://ui/main_menu.tscn",
        "steps": [{"wait_frames": 1}],
        "assert": [{"visible_pixels": {"target": "qa:actor", "min_ratio": 0.5}}],
    });
    let error = validate_scenario_document(&missing_snapshot, &scenario_path).unwrap_err();
    assert_eq!(
        failure_json(error),
        json!({
            "type": "scenario_validation_failed",
            "path": scenario_path,
            "message": "visible_pixels assertion field 'snapshot' must be a non-empty string",
            "expected": "non-empty string",
            "actual": null,
        })
    );

    let missing_threshold = json!({
        "name": "visual_invalid",
        "scene": "res://ui/main_menu.tscn",
        "steps": [{"wait_frames": 1}],
        "assert": [{"visible_pixels": {"target": "qa:actor", "snapshot": "after"}}],
    });
    let error = validate_scenario_document(&missing_threshold, &scenario_path).unwrap_err();
    assert_eq!(
        failure_json(error),
        json!({
            "type": "scenario_validation_failed",
            "path": scenario_path,
            "message": "visible_pixels assertion requires at least one of 'min_ratio' or 'min_count'",
            "expected": "at least one of 'min_ratio' or 'min_count'",
            "actual": {"target": "qa:actor", "snapshot": "after"},
        })
    );
}

#[test]
fn surface_probe_validation_and_normalization() {
    let root = temp_dir("scenario-surface-probe");
    let scenario_path = root.join("scenario.json");
    let document = json!({
        "name": "surface_probe_demo",
        "scene": "res://ui/main_menu.tscn",
        "steps": [{"wait_frames": 1}],
        "assert": [{
            "surface_probe": {
                "target": " qa:actor ",
                "provider": " qa:provider ",
                "provider_method": " qa_probe ",
                "coordinate_space": " world_2d ",
                "points": [{"x": 1, "y": 2, "expected": " walkable "}],
                "surfaces": ["walkable", "blocked"],
                "radius": 16.0
            }
        }]
    });
    validate_scenario_document(&document, &scenario_path).unwrap();
    let normalized = normalize_scenario_document(&document);
    assert_eq!(
        normalized["assert"][0]["surface_probe"]["target"],
        "qa:actor"
    );
    assert_eq!(
        normalized["assert"][0]["surface_probe"]["provider"],
        "qa:provider"
    );
    assert_eq!(
        normalized["assert"][0]["surface_probe"]["provider_method"],
        "qa_probe"
    );
    assert_eq!(
        normalized["assert"][0]["surface_probe"]["coordinate_space"],
        "world_2d"
    );
    assert_eq!(
        normalized["assert"][0]["surface_probe"]["points"][0]["expected"],
        "walkable"
    );
}

#[test]
fn surface_probe_rejects_invalid_bounded_payloads() {
    let root = temp_dir("scenario-surface-probe-invalid");
    let scenario_path = root.join("scenario.json");
    let bad = json!({
        "name": "surface_probe_demo",
        "scene": "res://ui/main_menu.tscn",
        "steps": [{"wait_frames": 1}],
        "assert": [{"surface_probe": {"target": "qa:a", "provider": "qa:p", "coordinate_space": "screen", "points": [{"x": 1, "y": 2}]}}]
    });
    assert!(validate_scenario_document(&bad, &scenario_path).is_err());
}

#[test]
fn surface_probe_rejects_missing_points() {
    let root = temp_dir("scenario-surface-probe-missing-points");
    let scenario_path = root.join("scenario.json");
    let bad = json!({
        "name": "surface_probe_demo",
        "scene": "res://ui/main_menu.tscn",
        "steps": [{"wait_frames": 1}],
        "assert": [{"surface_probe": {"target": "qa:a", "provider": "qa:p"}}]
    });
    assert!(validate_scenario_document(&bad, &scenario_path).is_err());
}

#[test]
fn surface_probe_rejects_empty_target() {
    let root = temp_dir("scenario-surface-probe-empty-target");
    let scenario_path = root.join("scenario.json");
    let bad = json!({
        "name": "surface_probe_demo",
        "scene": "res://ui/main_menu.tscn",
        "steps": [{"wait_frames": 1}],
        "assert": [{"surface_probe": {"target": " ", "provider": "qa:p", "points": [{"x": 1, "y": 2}]}}]
    });
    assert!(validate_scenario_document(&bad, &scenario_path).is_err());
}

#[test]
fn surface_probe_rejects_too_many_points() {
    let root = temp_dir("scenario-surface-probe-too-many-points");
    let scenario_path = root.join("scenario.json");
    let points: Vec<serde_json::Value> = (0..257).map(|idx| json!({"x": idx, "y": 0})).collect();
    let bad = json!({
        "name": "surface_probe_demo",
        "scene": "res://ui/main_menu.tscn",
        "steps": [{"wait_frames": 1}],
        "assert": [{"surface_probe": {"target": "qa:a", "provider": "qa:p", "points": points}}]
    });
    assert!(validate_scenario_document(&bad, &scenario_path).is_err());
}

#[test]
fn surface_probe_rejects_nonnumeric_point_coordinate() {
    let root = temp_dir("scenario-surface-probe-nonnumeric");
    let scenario_path = root.join("scenario.json");
    let bad = json!({
        "name": "surface_probe_demo",
        "scene": "res://ui/main_menu.tscn",
        "steps": [{"wait_frames": 1}],
        "assert": [{"surface_probe": {"target": "qa:a", "provider": "qa:p", "points": [{"x": "left", "y": 2}]}}]
    });
    assert!(validate_scenario_document(&bad, &scenario_path).is_err());
}

#[test]
fn surface_probe_rejects_unsupported_coordinate_space() {
    let root = temp_dir("scenario-surface-probe-bad-space");
    let scenario_path = root.join("scenario.json");
    let bad = json!({
        "name": "surface_probe_demo",
        "scene": "res://ui/main_menu.tscn",
        "steps": [{"wait_frames": 1}],
        "assert": [{"surface_probe": {"target": "qa:a", "provider": "qa:p", "coordinate_space": "tilemap", "points": [{"x": 1, "y": 2}]}}]
    });
    assert!(validate_scenario_document(&bad, &scenario_path).is_err());
}

#[test]
fn surface_probe_rejects_expected_label_not_in_surfaces() {
    let root = temp_dir("scenario-surface-probe-unknown-label");
    let scenario_path = root.join("scenario.json");
    let bad = json!({
        "name": "surface_probe_demo",
        "scene": "res://ui/main_menu.tscn",
        "steps": [{"wait_frames": 1}],
        "assert": [{
            "surface_probe": {
                "target": "qa:a",
                "provider": "qa:p",
                "surfaces": ["walkable", "blocked"],
                "points": [{"x": 1, "y": 2, "expected": "lava"}]
            }
        }]
    });
    assert!(validate_scenario_document(&bad, &scenario_path).is_err());
}

#[test]
fn surface_matches_rejects_missing_target() {
    let root = temp_dir("scenario-surface-matches-invalid");
    let scenario_path = root.join("scenario.json");
    let bad = json!({
        "name": "surface_matches_demo",
        "scene": "res://ui/main_menu.tscn",
        "steps": [{"wait_frames": 1}],
        "assert": [{"surface_matches": {"target": " ", "surface": "walkable"}}]
    });
    assert!(validate_scenario_document(&bad, &scenario_path).is_err());
}

// ---------------------------------------------------------------------------
// Scenario templating (extends + vars + abstract)
// ---------------------------------------------------------------------------

#[test]
fn template_extends_missing_base_fails_at_load() {
    let root = temp_dir("scenario-template-missing-base");
    let child = root.join("child.yaml");
    write(&child, "name: child\nextends: base.yaml\n");

    let error = load_scenario_document(&child).unwrap_err();
    let failure = failure_json(error);
    assert_eq!(failure["type"], json!("scenario_extends_missing"));
    assert!(
        failure["actual"]["resolved"]
            .as_str()
            .unwrap()
            .ends_with("base.yaml")
    );
}

#[test]
fn template_extends_self_cycle_is_rejected() {
    let root = temp_dir("scenario-template-self-cycle");
    let child = root.join("loop.yaml");
    write(
        &child,
        "name: loop\nextends: loop.yaml\nscene: res://x.tscn\n",
    );

    let error = load_scenario_document(&child).unwrap_err();
    let failure = failure_json(error);
    assert_eq!(failure["type"], json!("scenario_extends_cycle"));
    let chain = failure["actual"].as_array().unwrap();
    assert_eq!(chain.len(), 2);
}

#[test]
fn template_extends_indirect_cycle_is_rejected() {
    let root = temp_dir("scenario-template-indirect-cycle");
    let a = root.join("a.yaml");
    let b = root.join("b.yaml");
    write(&a, "name: a\nextends: b.yaml\n");
    write(&b, "extends: a.yaml\n");

    let error = load_scenario_document(&a).unwrap_err();
    let failure = failure_json(error);
    assert_eq!(failure["type"], json!("scenario_extends_cycle"));
    let chain = failure["actual"].as_array().unwrap();
    // a -> b -> a
    assert_eq!(chain.len(), 3);
}

#[test]
fn template_three_deep_chain_resolves_and_merges() {
    let root = temp_dir("scenario-template-three-deep");
    write(
        &root.join("grandbase.yaml"),
        "abstract: true\nscene: res://ui/main_menu.tscn\nvars:\n  n: 1\nsteps:\n  - wait_frames: \"${n}\"\n",
    );
    write(
        &root.join("base.yaml"),
        "extends: grandbase.yaml\nviewport:\n  width: 800\n  height: 600\n",
    );
    write(
        &root.join("mid.yaml"),
        "extends: base.yaml\ndeterminism:\n  seed: 5\n",
    );
    let child = root.join("child.yaml");
    write(&child, "name: chained\nextends: mid.yaml\nvars:\n  n: 3\n");

    let resolved = load_scenario_document(&child).unwrap();
    assert_eq!(resolved["name"], json!("chained"));
    assert_eq!(resolved["scene"], json!("res://ui/main_menu.tscn"));
    assert_eq!(resolved["viewport"]["width"], json!(800));
    assert_eq!(resolved["determinism"]["seed"], json!(5));
    // exact-match ${n} became an integer, re-parameterized by the child var.
    assert_eq!(resolved["steps"][0]["wait_frames"], json!(3));
    assert!(resolved["steps"][0]["wait_frames"].is_i64());
    assert!(resolved.get("abstract").is_none());
    assert!(resolved.get("extends").is_none());
    assert!(resolved.get("vars").is_none());
}

#[test]
fn template_extends_depth_limit_is_rejected() {
    let root = temp_dir("scenario-template-depth-limit");
    // file 0 extends file 1 extends ... extends file 10 (the root base).
    for level in 0..10 {
        write(
            &root.join(format!("f{level}.yaml")),
            &format!("extends: f{}.yaml\n", level + 1),
        );
    }
    write(
        &root.join("f10.yaml"),
        "abstract: true\nscene: res://x.tscn\nsteps: []\n",
    );

    let error = load_scenario_document(&root.join("f0.yaml")).unwrap_err();
    let failure = failure_json(error);
    assert_eq!(failure["type"], json!("scenario_extends_invalid"));
}

#[test]
fn template_yaml_child_extends_json_base() {
    let root = temp_dir("scenario-template-yaml-json");
    write(
        &root.join("base.json"),
        "{\"abstract\": true, \"scene\": \"res://ui/main_menu.tscn\", \"steps\": [{\"wait_frames\": 2}]}",
    );
    let child = root.join("child.yaml");
    write(&child, "name: json_base_child\nextends: base.json\n");

    let resolved = load_scenario_document(&child).unwrap();
    assert_eq!(resolved["name"], json!("json_base_child"));
    assert_eq!(resolved["scene"], json!("res://ui/main_menu.tscn"));
    assert_eq!(resolved["steps"][0]["wait_frames"], json!(2));
}

#[test]
fn template_abstract_entry_is_rejected() {
    let root = temp_dir("scenario-template-abstract-entry");
    let base = root.join("base.yaml");
    write(
        &base,
        "abstract: true\nscene: res://ui/main_menu.tscn\nsteps: []\n",
    );

    let error = load_scenario_document(&base).unwrap_err();
    let failure = failure_json(error);
    assert_eq!(failure["type"], json!("scenario_abstract"));
    assert_eq!(failure["expected"], json!("concrete scenario"));
}

#[test]
fn list_scenarios_excludes_abstract_templates() {
    let root = temp_dir("scenario-template-list-excludes");
    let scenarios_dir = root.join("qa/scenarios/examples");
    write(
        &scenarios_dir.join("concrete.yaml"),
        "name: Concrete\nscene: res://x.tscn\nsteps: []\n",
    );
    write(
        &scenarios_dir.join("templates").join("base.yaml"),
        "abstract: true\nscene: res://x.tscn\nsteps: []\n",
    );
    write(
        &scenarios_dir.join("templates").join("base.json"),
        "{\"abstract\": true, \"scene\": \"res://x.tscn\", \"steps\": []}",
    );

    let listed = list_scenarios(&scenarios_dir);
    let names: Vec<&str> = listed
        .iter()
        .filter_map(|entry| entry.get("name").and_then(|n| n.as_str()))
        .collect();
    assert_eq!(names, vec!["Concrete"]);
}

#[test]
fn template_resolved_document_validates_and_normalizes_end_to_end() {
    let root = temp_dir("scenario-template-end-to-end");
    write(
        &root.join("base.yaml"),
        &[
            "abstract: true",
            "scene: res://ui/main_menu.tscn",
            "vars:",
            "  frames: 5",
            "  slot: qa-profile",
            "viewport:",
            "  width: 1920",
            "  height: 1080",
            "project_args:",
            "  - --save-slot",
            "  - \"${slot}\"",
            "steps:",
            "  - wait_frames: \"${frames}\"",
            "  - snapshot: before",
            "assert:",
            "  - no_errors: true",
        ]
        .join("\n"),
    );
    let child = root.join("child.yaml");
    write(
        &child,
        "name: end_to_end\nextends: base.yaml\nvars:\n  frames: 9\n",
    );

    let normalized = load_and_normalize_scenario_document(&child).unwrap();
    assert_eq!(normalized["name"], json!("end_to_end"));
    assert_eq!(normalized["scene"], json!("res://ui/main_menu.tscn"));
    assert_eq!(normalized["viewport"]["width"], json!(1920));
    assert_eq!(normalized["steps"][0]["wait_frames"], json!(9));
    assert_eq!(normalized["steps"][1]["snapshot"], json!("before"));
    assert_eq!(
        normalized["project_args"],
        json!(["--save-slot", "qa-profile"])
    );
}

#[test]
fn template_unresolved_placeholder_reports_location() {
    let root = temp_dir("scenario-template-unresolved");
    let child = root.join("child.yaml");
    write(&child, "name: unresolved\nextends: base.yaml\n");
    write(
        &root.join("base.yaml"),
        "abstract: true\nscene: res://x.tscn\nsteps:\n  - tap_action: \"${missing}\"\n",
    );

    let error = load_scenario_document(&child).unwrap_err();
    let failure = failure_json(error);
    assert_eq!(failure["type"], json!("scenario_var_unresolved"));
    let entries = failure["actual"].as_array().unwrap();
    assert_eq!(entries[0]["placeholder"], json!("${missing}"));
    assert_eq!(entries[0]["location"], json!("steps[0].tap_action"));
}

#[test]
fn template_base_parse_error_propagates_with_chain() {
    let root = temp_dir("scenario-template-base-parse-error");
    let child = root.join("child.yaml");
    write(&child, "name: child\nextends: base.yaml\n");
    // Malformed YAML base (unterminated flow sequence).
    write(
        &root.join("base.yaml"),
        "abstract: true\nsteps:\n  - foo: [1, 2\n",
    );

    let error = load_scenario_document(&child).unwrap_err();
    let failure = failure_json(error);
    assert_eq!(failure["type"], json!("scenario_yaml_parse_failed"));
    assert!(failure["path"].as_str().unwrap().ends_with("base.yaml"));
    assert!(
        failure["message"]
            .as_str()
            .unwrap()
            .contains("via extends chain")
    );
}

#[test]
fn plain_scenario_without_template_keys_preserves_literal_placeholder() {
    let root = temp_dir("scenario-template-fast-path");
    let scenario_path = root.join("plain.yaml");
    // No extends/vars/abstract: fast path, literal ${...} is preserved verbatim.
    write(
        &scenario_path,
        "name: plain\nscene: res://x.tscn\nsteps:\n  - tap_action: \"${literal}\"\n",
    );

    let resolved = load_scenario_document(&scenario_path).unwrap();
    assert_eq!(resolved["steps"][0]["tap_action"], json!("${literal}"));
}
