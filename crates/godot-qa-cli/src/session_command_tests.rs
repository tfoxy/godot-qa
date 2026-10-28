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
    effective_session_profile_max_frame_samples, event_summarize_params, event_trace_params,
    external_harness_path, is_relevant_watch_path, physics_collision_report_params,
    physics_inspect_params, physics_overlay_params, public_command_surface, screenshot_result,
    script_diagnostic_excerpt, session_frame_pacing_matches, session_frame_pacing_request,
    session_script_failure_is_retryable, session_script_frame_pacing_request,
    session_script_project_args, session_script_step_op, session_script_wait_busy_backoff_ms,
    session_time_scale_params, snapshot_filter_params, surface_audit_params,
    surface_audit_provider_methods, trace_record_params, validate_session_profile_args,
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
fn session_start_parses_repeatable_project_args() {
    let cli = Cli::parse_from([
        "godot-qa",
        "session",
        "start",
        "--scene",
        "res://main.tscn",
        "--project-arg",
        "--seed",
        "--project-arg",
        "123456",
    ]);
    let Commands::Session(args) = cli.command else {
        panic!("expected session command");
    };
    let SessionCommands::Start(args) = args.command else {
        panic!("expected session start command");
    };
    assert_eq!(args.scene.as_deref(), Some("res://main.tscn"));
    assert_eq!(args.project_args, vec!["--seed", "123456"]);
    assert_eq!(args.target, super::SessionTargetArg::Desktop);
    assert_eq!(args.viewport, None);
}

#[test]
fn session_start_parses_android_target_contract() {
    let cli = Cli::parse_from([
        "godot-qa",
        "session",
        "start",
        "--target",
        "android",
        "--preset",
        "Android QA",
        "--device",
        "emulator-5554",
        "--package",
        "org.example.qa",
        "--scene",
        "res://main.tscn",
        "--project-arg",
        "--seed=7",
    ]);
    let Commands::Session(args) = cli.command else {
        panic!("expected session command");
    };
    let SessionCommands::Start(args) = args.command else {
        panic!("expected session start command");
    };
    assert_eq!(args.target, super::SessionTargetArg::Android);
    assert_eq!(args.preset.as_deref(), Some("Android QA"));
    assert_eq!(args.device.as_deref(), Some("emulator-5554"));
    assert_eq!(args.package_id.as_deref(), Some("org.example.qa"));
    assert_eq!(args.viewport, None);
    assert_eq!(args.project_args, vec!["--seed=7"]);
}

#[test]
fn session_start_rejects_unknown_target_during_parsing() {
    let error =
        Cli::try_parse_from(["godot-qa", "session", "start", "--target", "ios"]).unwrap_err();
    assert_eq!(error.kind(), clap::error::ErrorKind::InvalidValue);
}

#[test]
fn session_start_parses_fastest_fixed_fps() {
    let cli = Cli::parse_from([
        "godot-qa",
        "session",
        "start",
        "--fastest",
        "--fixed-fps",
        "120",
    ]);
    let Commands::Session(args) = cli.command else {
        panic!("expected session command");
    };
    let SessionCommands::Start(args) = args.command else {
        panic!("expected session start command");
    };
    assert!(args.fastest);
    assert_eq!(args.fixed_fps, Some(120));
    assert_eq!(
        session_frame_pacing_request(args.fastest, args.fixed_fps, "session start").unwrap(),
        Some(Some(120))
    );
}

#[test]
fn session_frame_pacing_match_compares_active_mode_and_fixed_fps() {
    let requested = godot_qa_runtime::SessionFramePacing {
        fixed_fps: 60,
        source: "session".to_string(),
        project_setting: None,
    };
    let status = CommandResult {
        ok: true,
        command: "session status".to_string(),
        message: "ok".to_string(),
        data: Map::from_iter([(
            "frame_pacing".to_string(),
            json!({"mode": "fastest", "fixed_fps": 60, "source": "default"}),
        )]),
        failures: Vec::new(),
        exit_code: 0,
    };
    assert!(session_frame_pacing_matches(&status, &requested));

    let realtime_status = CommandResult {
        data: Map::new(),
        ..status
    };
    assert!(!session_frame_pacing_matches(&realtime_status, &requested));
}

#[test]
fn session_reload_and_watch_parse_and_report_command_paths() {
    let cli = Cli::parse_from(["godot-qa", "session", "start"]);
    let Commands::Session(args) = &cli.command else {
        panic!("expected session command");
    };
    let SessionCommands::Start(args) = &args.command else {
        panic!("expected session start command");
    };
    assert!(args.scene.is_none());
    assert_eq!(command_path(&cli.command), "session start");

    let cli = Cli::parse_from(["godot-qa", "session", "start", "--scene", "res://main.tscn"]);
    let Commands::Session(args) = &cli.command else {
        panic!("expected session command");
    };
    let SessionCommands::Start(args) = &args.command else {
        panic!("expected session start command");
    };
    assert_eq!(args.scene.as_deref(), Some("res://main.tscn"));

    let cli = Cli::parse_from(["godot-qa", "session", "list"]);
    let Commands::Session(args) = &cli.command else {
        panic!("expected session command");
    };
    assert!(matches!(args.command, SessionCommands::List));
    assert_eq!(command_path(&cli.command), "session list");

    let cli = Cli::parse_from(["godot-qa", "session", "reload"]);
    let Commands::Session(args) = &cli.command else {
        panic!("expected session command");
    };
    assert!(matches!(args.command, SessionCommands::Reload(_)));
    assert_eq!(command_path(&cli.command), "session reload");

    let cli = Cli::parse_from([
        "godot-qa",
        "session",
        "reload",
        "--script",
        "res://player.gd",
        "--resource",
        "res://ui/theme.tres",
    ]);
    let Commands::Session(args) = &cli.command else {
        panic!("expected session command");
    };
    let SessionCommands::Reload(reload_args) = &args.command else {
        panic!("expected session reload command");
    };
    assert_eq!(reload_args.scripts, vec!["res://player.gd".to_string()]);
    assert_eq!(
        reload_args.resources,
        vec!["res://ui/theme.tres".to_string()]
    );
    assert_eq!(command_path(&cli.command), "session reload");

    let cli = Cli::parse_from(["godot-qa", "session", "watch", "--debounce-ms", "125"]);
    let Commands::Session(args) = &cli.command else {
        panic!("expected session command");
    };
    let SessionCommands::Watch(args) = &args.command else {
        panic!("expected session watch command");
    };
    assert_eq!(args.debounce_ms, 125);
    assert_eq!(command_path(&cli.command), "session watch");

    let cli = Cli::parse_from([
        "godot-qa",
        "session",
        "time-scale",
        "4",
        "--physics-ticks-per-second",
        "240",
        "--max-physics-steps-per-frame",
        "16",
    ]);
    let Commands::Session(args) = &cli.command else {
        panic!("expected session command");
    };
    let SessionCommands::TimeScale(args) = &args.command else {
        panic!("expected session time-scale command");
    };
    assert_eq!(args.time_scale, 4.0);
    assert_eq!(args.physics_ticks_per_second, Some(240));
    assert_eq!(args.max_physics_steps_per_frame, Some(16));
    assert_eq!(command_path(&cli.command), "session time-scale");

    let cli = Cli::parse_from([
        "godot-qa",
        "session",
        "profile",
        "--frames",
        "10",
        "--max-frame-samples",
        "20",
        "--top",
        "5",
    ]);
    let Commands::Session(args) = &cli.command else {
        panic!("expected session command");
    };
    let SessionCommands::Profile(args) = &args.command else {
        panic!("expected session profile command");
    };
    assert_eq!(args.frames, Some(10));
    assert_eq!(args.seconds, None);
    assert_eq!(args.max_frame_samples, Some(20));
    assert_eq!(args.top, 5);
    assert_eq!(
        effective_session_profile_max_frame_samples(args, args.frames),
        20
    );
    assert_eq!(command_path(&cli.command), "session profile");
}

#[test]
fn session_time_scale_params_validate_bounds() {
    let valid = SessionTimeScaleArgs {
        time_scale: 8.0,
        physics_ticks_per_second: Some(480),
        max_physics_steps_per_frame: Some(16),
    };
    let params = session_time_scale_params(&valid).unwrap();
    assert_eq!(params["time_scale"], json!(8.0));
    assert_eq!(params["physics_ticks_per_second"], json!(480));
    assert_eq!(params["max_physics_steps_per_frame"], json!(16));

    let invalid_scale = session_time_scale_params(&SessionTimeScaleArgs {
        time_scale: 0.0,
        physics_ticks_per_second: None,
        max_physics_steps_per_frame: None,
    })
    .unwrap_err();
    assert_eq!(invalid_scale.kind, "invalid_params");
    assert_eq!(invalid_scale.command.as_deref(), Some("session time-scale"));

    let invalid_physics = session_time_scale_params(&SessionTimeScaleArgs {
        time_scale: 1.0,
        physics_ticks_per_second: Some(0),
        max_physics_steps_per_frame: None,
    })
    .unwrap_err();
    assert_eq!(invalid_physics.kind, "invalid_params");
    assert_eq!(
        invalid_physics.message.as_deref(),
        Some("session time-scale requires physics_ticks_per_second >= 1")
    );
}

#[test]
fn session_profile_rejects_frames_and_seconds_together() {
    let error = Cli::try_parse_from([
        "godot-qa",
        "session",
        "profile",
        "--frames",
        "10",
        "--seconds",
        "1",
    ])
    .unwrap_err();
    assert_eq!(error.kind(), clap::error::ErrorKind::ArgumentConflict);
}

#[test]
fn session_profile_defaults_frame_sample_limit_to_requested_frames() {
    let cli = Cli::parse_from(["godot-qa", "session", "profile", "--frames", "900"]);
    let Commands::Session(args) = &cli.command else {
        panic!("expected session command");
    };
    let SessionCommands::Profile(args) = &args.command else {
        panic!("expected session profile command");
    };

    assert_eq!(args.max_frame_samples, None);
    assert_eq!(
        effective_session_profile_max_frame_samples(args, args.frames),
        900
    );
    assert!(validate_session_profile_args("session profile", args).is_ok());
}

#[test]
fn session_profile_rejects_frames_above_limit() {
    let cli = Cli::parse_from(["godot-qa", "session", "profile", "--frames", "1001"]);
    let Commands::Session(args) = &cli.command else {
        panic!("expected session command");
    };
    let SessionCommands::Profile(args) = &args.command else {
        panic!("expected session profile command");
    };
    let failure = validate_session_profile_args("session profile", args).unwrap_err();
    assert_eq!(failure.kind, "invalid_params");
    assert_eq!(
        failure.message.as_deref(),
        Some("session profile requires --frames between 1 and 1000")
    );
}
