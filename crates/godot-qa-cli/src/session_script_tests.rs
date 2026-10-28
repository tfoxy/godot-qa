#![allow(dead_code, unused_imports)]

use super::runtime;
use super::{
    AudioCommands, Cli, Commands, InputCommands, InputJoyCommands, PathCandidate,
    PhysicsCollisionReportArgs, PhysicsCommands, PhysicsInspectArgs, PhysicsOverlayArgs,
    SESSION_SCRIPT_WAIT_BUSY_UNKNOWN_BACKOFF_MS, ScreenshotCommands, SessionCommands,
    SessionTimeScaleArgs, SurfaceAuditArgs, SurfaceCommands, TraceCommands, TraceRecordArgs,
    audio_analyze_result, binary_surface_has_incompatible_duplicates,
    binary_surface_mismatch_failure, binary_surface_warnings, coalesced_reload_count, command_path,
    discover_binary_command_surface, doctor_binary_surface_report_for_candidates,
    event_summarize_params, event_trace_params, external_harness_path, is_relevant_watch_path,
    physics_collision_report_params, physics_inspect_params, physics_overlay_params,
    public_command_surface, screenshot_result, script_diagnostic_excerpt,
    session_frame_pacing_matches, session_frame_pacing_request,
    session_script_failure_is_retryable, session_script_frame_pacing_request,
    session_script_project_args, session_script_step_op, session_script_wait_busy_backoff_ms,
    session_time_scale_params, snapshot_filter_params, surface_audit_params,
    surface_audit_provider_methods, trace_record_params,
};
use clap::Parser;
use godot_qa_core::config::ProjectConfig;
use godot_qa_core::results::{CommandResult, FailureDetail};
use image::{ImageBuffer, Rgba};
use serde_json::{Map, json};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

fn temp_dir(name: &str) -> PathBuf {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let path = std::env::temp_dir().join(format!("godot-qa-cli-{name}-{unique}"));
    fs::create_dir_all(&path).unwrap();
    path
}

fn test_project_config(root: PathBuf) -> ProjectConfig {
    ProjectConfig {
        project_root: root.clone(),
        repo_root: root.clone(),
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
fn session_script_project_args_validate_non_empty_strings() {
    let step = json!({
        "scene": "res://main.tscn",
        "project_args": ["--seed", "123456"]
    });
    assert_eq!(
        session_script_project_args(&step, 0).unwrap(),
        vec!["--seed".to_string(), "123456".to_string()]
    );

    let invalid = json!({"project_args": ["--seed", ""]});
    let result = session_script_project_args(&invalid, 4).unwrap_err();
    assert!(!result.ok);
    assert_eq!(result.data["step_index"], 4);
    assert_eq!(result.failures[0].kind, "invalid_session_script");
}

#[test]
fn session_script_frame_pacing_accepts_object_and_shorthand() {
    let object = json!({
        "frame_pacing": {"mode": "fastest", "fixed_fps": 120}
    });
    assert_eq!(
        session_script_frame_pacing_request(&object, 0).unwrap(),
        Some(Some(120))
    );

    let shorthand = json!({"fastest": true, "fixed_fps": 90});
    assert_eq!(
        session_script_frame_pacing_request(&shorthand, 0).unwrap(),
        Some(Some(90))
    );

    let derived = json!({"fastest": true});
    assert_eq!(
        session_script_frame_pacing_request(&derived, 0).unwrap(),
        Some(None)
    );
}

#[test]
fn session_script_frame_pacing_rejects_fixed_fps_without_fastest() {
    let result = session_script_frame_pacing_request(&json!({"fixed_fps": 60}), 4)
        .expect_err("fixed fps without fastest fails");
    assert!(!result.ok);
    assert_eq!(result.data["step_index"], 4);
    assert_eq!(result.failures[0].kind, "invalid_session_script");
}

#[test]
fn session_script_call_method_is_supported_operation() {
    let step = json!({
        "call_method": {"target": "qa:main", "method": "qa_prepare_fixture", "args": []}
    });
    let (op, value) = session_script_step_op(&step).expect("single op");
    assert_eq!(op, "call_method");
    assert_eq!(value["method"], "qa_prepare_fixture");
    assert!(super::SESSION_SCRIPT_OPS.contains(&"call_method"));
}

#[test]
fn session_script_fixture_call_is_supported_operation() {
    let step = json!({
        "fixture_call": {
            "driver": "schema_driver",
            "action": "set_status",
            "params": {"status": "live"}
        }
    });
    let (op, value) = session_script_step_op(&step).expect("single op");
    assert_eq!(op, "fixture_call");
    assert_eq!(value["driver"], "schema_driver");
    assert_eq!(value["action"], "set_status");
    assert!(super::SESSION_SCRIPT_OPS.contains(&"fixture_call"));
}

#[test]
fn session_script_fixture_call_rejects_invalid_shape_before_live_command() {
    let root = temp_dir("fixture-call-invalid-shape");
    let config = test_project_config(root.clone());

    let missing_driver = super::run_session_script_step(
        &config,
        &json!({"fixture_call": {"action": "set_status", "params": {}}}),
        2,
    );
    assert!(!missing_driver.ok);
    assert_eq!(missing_driver.data["step_index"], 2);
    assert_eq!(missing_driver.failures[0].kind, "invalid_session_script");
    assert_eq!(
        missing_driver.failures[0].message.as_deref(),
        Some("fixture_call step field 'driver' must be a non-empty string")
    );

    let invalid_params = super::run_session_script_step(
        &config,
        &json!({"fixture_call": {"driver": "schema_driver", "action": "set_status", "params": []}}),
        3,
    );
    assert!(!invalid_params.ok);
    assert_eq!(invalid_params.data["step_index"], 3);
    assert_eq!(invalid_params.failures[0].kind, "invalid_session_script");
    assert_eq!(
        invalid_params.failures[0].message.as_deref(),
        Some("fixture_call step field 'params' must be an object when present")
    );

    let _ = fs::remove_dir_all(root);
}

#[test]
fn session_script_input_joy_button_accepts_object_shape() {
    let root = temp_dir("input-joy-button-object-shape");
    let config = test_project_config(root.clone());

    let result = super::run_session_script_step(
        &config,
        &json!({"input_joy_button": {"button": "start", "device": 2, "frames": 3}}),
        1,
    );
    assert!(!result.ok);
    assert_eq!(result.command, "input joy button");
    assert_ne!(result.failures[0].kind, "invalid_session_script");

    let _ = fs::remove_dir_all(root);
}

#[test]
fn session_script_input_joy_button_rejects_invalid_device_before_live_command() {
    let root = temp_dir("input-joy-button-invalid-device");
    let config = test_project_config(root.clone());

    let result = super::run_session_script_step(
        &config,
        &json!({"input_joy_button": {"button": "start", "device": -1}}),
        2,
    );
    assert!(!result.ok);
    assert_eq!(result.data["step_index"], 2);
    assert_eq!(result.failures[0].kind, "invalid_session_script");
    assert_eq!(
        result.failures[0].message.as_deref(),
        Some("input_joy_button step field 'device' must be a non-negative integer")
    );

    let _ = fs::remove_dir_all(root);
}

#[test]
fn session_script_input_joy_state_forwards_batch_shape() {
    let root = temp_dir("input-joy-state-batch-shape");
    let config = test_project_config(root.clone());

    let result = super::run_session_script_step(
        &config,
        &json!({"input_joy_state": {
            "inputs": [
                {"device": 0, "button": {"name": "a", "pressed": true}},
                {"device": 3, "button": {"name": "x", "pressed": true}}
            ],
            "frames": 30
        }}),
        1,
    );
    // The step forwards to the live bridge (no live game here), so it fails at
    // the transport, not on the outer CLI shape guard.
    assert!(!result.ok);
    assert_eq!(result.command, "input joy state");
    assert_ne!(result.failures[0].kind, "invalid_session_script");

    let _ = fs::remove_dir_all(root);
}

#[test]
fn session_script_controller_lifecycle_steps_reach_live_bridge() {
    let root = temp_dir("controller-lifecycle-steps");
    let config = test_project_config(root.clone());
    for (step, command) in [
        (
            json!({"joy_connect": {"devices": [4], "name": "QA pad"}}),
            "setup joy connect",
        ),
        (
            json!({"joy_disconnect": {"devices": [4]}}),
            "setup joy disconnect",
        ),
        (
            json!({"session_change_scene": "res://ui/main_menu.tscn"}),
            "session change-scene",
        ),
        (json!({"session_reload_scene": {}}), "session reload-scene"),
    ] {
        let result = super::run_session_script_step(&config, &step, 2);
        assert!(!result.ok);
        assert_eq!(result.command, command);
        assert_ne!(result.failures[0].kind, "invalid_session_script");
    }
    let missing_devices =
        super::run_session_script_step(&config, &json!({"joy_connect": {"devices": []}}), 3);
    assert_eq!(missing_devices.failures[0].kind, "invalid_session_script");
    let _ = fs::remove_dir_all(root);
}

#[test]
fn session_script_input_joy_state_rejects_empty_inputs_before_live_command() {
    let root = temp_dir("input-joy-state-empty-inputs");
    let config = test_project_config(root.clone());

    let result =
        super::run_session_script_step(&config, &json!({"input_joy_state": {"inputs": []}}), 4);
    assert!(!result.ok);
    assert_eq!(result.data["step_index"], 4);
    assert_eq!(result.failures[0].kind, "invalid_session_script");
    assert_eq!(
        result.failures[0].message.as_deref(),
        Some("input_joy_state step requires a non-empty inputs list")
    );

    let _ = fs::remove_dir_all(root);
}

#[test]
fn session_script_input_joy_motion_rejects_invalid_frames_before_live_command() {
    let root = temp_dir("input-joy-motion-invalid-frames");
    let config = test_project_config(root.clone());

    let result = super::run_session_script_step(
        &config,
        &json!({"input_joy_motion": {"axis": "left_x", "value": 1.0, "frames": -1}}),
        3,
    );
    assert!(!result.ok);
    assert_eq!(result.data["step_index"], 3);
    assert_eq!(result.failures[0].kind, "invalid_session_script");
    assert_eq!(
        result.failures[0].message.as_deref(),
        Some("input_joy_motion step field 'frames' must be a non-negative integer")
    );

    let _ = fs::remove_dir_all(root);
}

#[test]
fn session_script_input_text_step_op_accepts_string_and_object() {
    let bare = serde_json::json!({"input_text": "abc"});
    let (name, value) = session_script_step_op(&bare).expect("valid step");
    assert_eq!(name, "input_text");
    assert_eq!(value, "abc");

    let object = serde_json::json!({"input_text": {"text": "abc"}});
    let (name, value) = session_script_step_op(&object).expect("valid step");
    assert_eq!(name, "input_text");
    assert_eq!(value, &serde_json::json!({"text": "abc"}));
}

#[test]
fn session_script_input_text_rejects_non_string_before_live_command() {
    let root = temp_dir("input-text-invalid-value");
    let config = test_project_config(root.clone());

    let result = super::run_session_script_step(&config, &json!({"input_text": 42}), 4);
    assert!(!result.ok);
    assert_eq!(result.data["step_index"], 4);
    assert_eq!(result.failures[0].kind, "invalid_session_script");
    assert_eq!(
        result.failures[0].message.as_deref(),
        Some("input_text step requires a text")
    );

    let _ = fs::remove_dir_all(root);
}

#[test]
fn session_script_pause_resume_are_supported_operations() {
    for op in [
        "session_pause",
        "pause_session",
        "session_resume",
        "resume_session",
    ] {
        let step = json!({op: {}});
        let (parsed_op, value) = session_script_step_op(&step).expect("single op");
        assert_eq!(parsed_op, op);
        assert!(value.is_object());
        assert!(super::SESSION_SCRIPT_OPS.contains(&op));
    }
}

#[test]
fn session_script_review_evidence_ops() {
    for op in [
        "screenshot_sequence",
        "trace_record",
        "physics_collision_report",
        "audio_capture",
        "event_trace",
        "session_profile",
    ] {
        let step = json!({op: {}});
        let (parsed_op, value) = session_script_step_op(&step).expect("single op");
        assert_eq!(parsed_op, op);
        assert!(value.is_object());
        assert!(super::SESSION_SCRIPT_OPS.contains(&op));
    }

    let screenshot = super::session_script_screenshot_sequence_args(
        &json!({"out": ".qa/review/frames", "frames": 60, "every": 10}),
        0,
    )
    .unwrap();
    assert_eq!(screenshot.out, ".qa/review/frames");
    assert_eq!(screenshot.frames, Some(60));

    let trace = super::session_script_trace_record_args(
        &json!({
            "target": "qa:player",
            "frames": 120,
            "sample_every_frames": 4,
            "physics": true,
            "state_method": "get_qa_physics_state"
        }),
        0,
    )
    .unwrap();
    assert_eq!(
        trace_record_params(&trace).unwrap()["targets"],
        json!(["qa:player"])
    );

    let collision = super::session_script_physics_collision_report_args(
        &json!({"target": "qa:player", "frames": 120, "max_overlap_depth": 0.25}),
        0,
    )
    .unwrap();
    assert_eq!(
        physics_collision_report_params(&collision).unwrap()["targets"],
        json!(["qa:player"])
    );

    let audio = super::session_script_audio_capture_args(
        &json!({
            "seconds": 1.5,
            "analyze": true,
            "stop_after_state": {
                "target": "qa:race",
                "source": "method",
                "method": "get_qa_race_state",
                "matches": [{"path": "done", "operator": "equals", "expected": true}]
            }
        }),
        0,
    )
    .unwrap();
    assert!(audio.live);
    assert_eq!(audio.seconds, 1.5);
    assert!(
        audio
            .stop_after_state
            .as_deref()
            .is_some_and(|raw| raw.contains("get_qa_race_state"))
    );

    let event = super::session_script_event_trace_args(
        &json!({
            "target": "qa:events",
            "provider": "get_qa_events",
            "frames": 90
        }),
        0,
    )
    .unwrap();
    assert_eq!(
        event_trace_params(&event).unwrap()["target"],
        json!("qa:events")
    );

    let profile = super::session_script_profile_args(&json!({"frames": 30, "top": 5}), 0).unwrap();
    assert_eq!(profile.frames, Some(30));
    assert_eq!(profile.top, 5);

    let config = test_project_config(temp_dir("session-script-evidence-ops"));
    for step in [
        json!({"trace_record": {"target": "", "frames": 10}}),
        json!({"trace_record": {"target": "qa:player", "frames": 0}}),
        json!({"physics_collision_report": {"target": "qa:player", "frames": 0}}),
        json!({"audio_capture": {"seconds": 0}}),
        json!({"event_trace": {
            "target": "qa:events",
            "provider": "qa_mutate_events",
            "frames": 10
        }}),
    ] {
        let result = super::run_session_script_step(&config, &step, 3);
        assert!(!result.ok);
        assert_eq!(result.command, "session run-script");
        assert_eq!(result.data["step_index"], 3);
        assert_eq!(result.failures[0].kind, "invalid_session_script");
    }
}

#[test]
fn script_diagnostic_excerpt_detects_parse_and_compile_failures() {
    let root = temp_dir("script-diagnostics");
    let logs_path = root.join("logs.txt");
    fs::write(
            &logs_path,
            "INFO: boot\nSCRIPT ERROR: Parse Error: Expected statement, found \"Indent\" instead.\n          at: GDScript::reload (res://src/river_generation.gd:417)\nERROR: Failed to load script \"res://src/main.gd\" with error \"Parse error\".\n",
        )
        .unwrap();
    let process_result = godot_qa_process::GodotRunResult {
            command: vec!["godot".to_string()],
            returncode: 0,
            termination: None,
            process_timing: None,
            stdout: "SCRIPT ERROR: Parse Error: unknown identifier\n          at: GDScript::reload (res://src/preloaded_dependency.gd:12)\n".to_string(),
            stderr: "ERROR: Compilation failed.\n".to_string(),
            display: godot_qa_process::resolve_project_display_launch_metadata(&root, &[], true),
            pid: None,
            process_group: None,
            cleanup: None,
            infrastructure_failure_kind: None,
            timed_out: false,
            timeout_ms: None,
            output_truncation: None,
        };
    let excerpt = script_diagnostic_excerpt(&process_result, &logs_path).unwrap();
    assert!(excerpt.contains("Parse Error"));
    assert!(excerpt.contains("Compilation failed"));
    assert!(excerpt.contains("Failed to load script"));
    assert!(excerpt.contains("res://src/preloaded_dependency.gd:12"));
    assert!(excerpt.contains("res://src/river_generation.gd:417"));
    let _ = fs::remove_dir_all(root);
}

#[test]
fn session_run_script_parses_path() {
    let cli = Cli::parse_from([
        "godot-qa",
        "session",
        "run-script",
        ".qa/review-scripts/s2.yaml",
    ]);
    assert_eq!(command_path(&cli.command), "session run-script");
    let Commands::Session(args) = cli.command else {
        panic!("expected session command");
    };
    let SessionCommands::RunScript(script) = args.command else {
        panic!("expected session run-script command");
    };
    assert_eq!(script.path, ".qa/review-scripts/s2.yaml");
}

#[test]
fn session_script_step_op_requires_single_operation() {
    let step = serde_json::json!({"input_key": "A"});
    let (name, value) = session_script_step_op(&step).expect("valid step");
    assert_eq!(name, "input_key");
    assert_eq!(value, "A");

    assert!(session_script_step_op(&serde_json::json!({})).is_none());
    assert!(session_script_step_op(&serde_json::json!({"input_key": "A", "logs": true})).is_none());
}

#[test]
fn session_script_retry_classifies_only_transport_failures() {
    let busy = FailureDetail {
        kind: "session_bridge_busy".to_string(),
        message: Some("QA bridge is already processing a live command".to_string()),
        ..FailureDetail::default()
    };
    assert!(session_script_failure_is_retryable(&busy));

    let eagain = FailureDetail {
        kind: "session_connect_failed".to_string(),
        message: Some("Resource temporarily unavailable (os error 11)".to_string()),
        ..FailureDetail::default()
    };
    assert!(session_script_failure_is_retryable(&eagain));

    let wait_timeout = FailureDetail {
        kind: "session_connect_failed".to_string(),
        message: Some("timed out".to_string()),
        actual: Some(json!("TimeoutError")),
        ..FailureDetail::default()
    };
    assert!(!session_script_failure_is_retryable(&wait_timeout));

    let assertion = FailureDetail {
        kind: "state_matches_mismatch".to_string(),
        message: Some("runtime state did not match expected checks".to_string()),
        ..FailureDetail::default()
    };
    assert!(!session_script_failure_is_retryable(&assertion));

    let target = FailureDetail {
        kind: "target_not_found".to_string(),
        target: Some("qa:raft_0".to_string()),
        ..FailureDetail::default()
    };
    assert!(!session_script_failure_is_retryable(&target));
}

#[test]
fn session_script_wait_busy_backoff_uses_in_progress_frames() {
    let busy = FailureDetail {
        kind: "session_bridge_busy".to_string(),
        message: Some("QA bridge is already processing a live command".to_string()),
        details: Map::from_iter([(
            "in_progress".to_string(),
            json!({
                "command": "session.wait_frames",
                "frames": 420,
                "elapsed_msec": 9000
            }),
        )]),
        ..FailureDetail::default()
    };
    assert_eq!(session_script_wait_busy_backoff_ms(&busy), Some(5500));

    let unknown_frames = FailureDetail {
        kind: "session_bridge_busy".to_string(),
        details: Map::from_iter([(
            "in_progress".to_string(),
            json!({"command": "session.wait_frames"}),
        )]),
        ..FailureDetail::default()
    };
    assert_eq!(
        session_script_wait_busy_backoff_ms(&unknown_frames),
        Some(SESSION_SCRIPT_WAIT_BUSY_UNKNOWN_BACKOFF_MS)
    );

    let input_busy = FailureDetail {
        kind: "session_bridge_busy".to_string(),
        details: Map::from_iter([(
            "in_progress".to_string(),
            json!({"command": "input.tap_action", "frames": 420}),
        )]),
        ..FailureDetail::default()
    };
    assert_eq!(session_script_wait_busy_backoff_ms(&input_busy), None);
}
