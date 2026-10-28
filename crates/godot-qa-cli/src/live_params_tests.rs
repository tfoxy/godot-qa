#![allow(dead_code, unused_imports)]

use super::runtime;
use super::{
    AudioCommands, Cli, Commands, InputCommands, InputJoyCommands, InputMouseCommands,
    InputTouchCommands, PathCandidate, PhysicsCollisionReportArgs, PhysicsCommands,
    PhysicsInspectArgs, PhysicsOverlayArgs, PointerKind,
    SESSION_SCRIPT_WAIT_BUSY_UNKNOWN_BACKOFF_MS, ScreenshotCommands, SessionCommands,
    SessionTimeScaleArgs, SurfaceAuditArgs, SurfaceCommands, TraceCommands, TraceRecordArgs,
    VisualCommands, audio_analyze_result, binary_surface_has_incompatible_duplicates,
    binary_surface_mismatch_failure, binary_surface_warnings, coalesced_reload_count, command_path,
    discover_binary_command_surface, doctor_binary_surface_report_for_candidates,
    event_summarize_params, event_trace_params, external_harness_path, is_relevant_watch_path,
    physics_collision_report_params, physics_inspect_params, physics_overlay_params,
    public_command_surface, run_bounded_setup_script, run_setup_script_before,
    run_setup_script_before_with_timeout, screenshot_result, script_diagnostic_excerpt,
    session_frame_pacing_matches, session_frame_pacing_request,
    session_script_failure_is_retryable, session_script_frame_pacing_request,
    session_script_project_args, session_script_step_op, session_script_wait_busy_backoff_ms,
    session_time_scale_params, setup_script_timeout_result, snapshot_filter_params,
    summary_json_result, surface_audit_params, surface_audit_provider_methods, trace_record_params,
};
use clap::Parser;
use godot_qa_core::config::ProjectConfig;
use godot_qa_core::results::{CommandResult, FailureDetail};
use image::{ImageBuffer, Rgba};
use serde_json::{Map, json};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

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
fn physics_inspect_parses_and_builds_protocol_params() {
    let cli = Cli::parse_from([
        "godot-qa",
        "physics",
        "inspect",
        "qa:raft_0",
        "--radius",
        "360",
        "--frames",
        "5",
        "--state-provider",
        "scene-root",
        "--state-method",
        "get_qa_collision_footprint_state",
        "--snapshot",
        "collider-debug",
    ]);
    let Commands::Physics(args) = cli.command else {
        panic!("expected physics command");
    };
    let PhysicsCommands::Inspect(args) = args.command else {
        panic!("expected physics inspect command");
    };
    let params = physics_inspect_params(&args).unwrap();
    assert_eq!(params["target"], "qa:raft_0");
    assert_eq!(params["radius"], json!(360.0));
    assert_eq!(params["frames"], json!(5));
    assert_eq!(params["state_provider"], "scene-root");
    assert_eq!(
        params["state_methods"],
        json!(["get_qa_collision_footprint_state"])
    );
    assert_eq!(params["include_project_state"], json!(true));
    assert_eq!(params["snapshot"], "collider-debug");
}

#[test]
fn physics_inspect_rejects_out_of_bounds_and_mutating_state_methods() {
    let invalid_radius = PhysicsInspectArgs {
        target: "qa:probe".to_string(),
        radius: 4097.0,
        frames: 1,
        state_provider: "scene-root".to_string(),
        state_method: Vec::new(),
        no_project_state: false,
        snapshot: None,
    };
    assert_eq!(
        physics_inspect_params(&invalid_radius).unwrap_err().kind,
        "invalid_params"
    );

    let mutating_method = PhysicsInspectArgs {
        target: "qa:probe".to_string(),
        radius: 256.0,
        frames: 1,
        state_provider: "scene-root".to_string(),
        state_method: vec!["qa_apply_collision_probe".to_string()],
        no_project_state: false,
        snapshot: None,
    };
    let failure = physics_inspect_params(&mutating_method).unwrap_err();
    assert_eq!(failure.kind, "invalid_params");
    assert_eq!(failure.command.as_deref(), Some("physics inspect"));
}

#[test]
fn physics_overlay_parses_and_builds_protocol_params() {
    let cli = Cli::parse_from([
        "godot-qa",
        "physics",
        "overlay",
        "qa:actor",
        "--radius",
        "400",
        "--frames",
        "2",
        "--out",
        ".qa/artifacts/manual/actor.png",
        "--include-aabb",
        "--include-contacts",
        "--state-provider",
        "scene-root",
        "--state-method",
        "get_qa_collision_visual_state",
        "--max-shapes",
        "64",
        "--strict",
    ]);
    let Commands::Physics(args) = cli.command else {
        panic!("expected physics command");
    };
    let PhysicsCommands::Overlay(args) = args.command else {
        panic!("expected physics overlay command");
    };
    let params = physics_overlay_params(&args).unwrap();
    assert_eq!(params["target"], "qa:actor");
    assert_eq!(params["viewport"], json!(false));
    assert_eq!(params["radius"], json!(400.0));
    assert_eq!(params["frames"], json!(2));
    assert_eq!(params["include_aabb"], json!(true));
    assert_eq!(params["include_contacts"], json!(true));
    assert_eq!(params["state_provider"], "scene-root");
    assert_eq!(
        params["state_methods"],
        json!(["get_qa_collision_visual_state"])
    );
    assert_eq!(params["max_shapes"], json!(64));
    assert_eq!(params["strict"], json!(true));
}

#[test]
fn physics_overlay_accepts_viewport_without_target() {
    let cli = Cli::parse_from([
        "godot-qa",
        "physics",
        "overlay",
        "--viewport",
        "--out",
        ".qa/artifacts/manual/viewport.png",
    ]);
    let Commands::Physics(args) = cli.command else {
        panic!("expected physics command");
    };
    let PhysicsCommands::Overlay(args) = args.command else {
        panic!("expected physics overlay command");
    };
    let params = physics_overlay_params(&args).unwrap();
    assert_eq!(params["target"], "");
    assert_eq!(params["viewport"], json!(true));
}

#[test]
fn physics_overlay_snapshot_review_parses_protocol_params() {
    let cli = Cli::parse_from([
        "godot-qa",
        "physics",
        "overlay",
        "qa:actor",
        "--scenario",
        "physics_overlay_demo",
        "--snapshot",
        "after",
        "--radius",
        "400",
        "--out",
        ".qa/artifacts/manual/from_snapshot.png",
    ]);
    let Commands::Physics(args) = cli.command else {
        panic!("expected physics command");
    };
    let PhysicsCommands::Overlay(args) = args.command else {
        panic!("expected physics overlay command");
    };
    let params = physics_overlay_params(&args).unwrap();
    assert_eq!(args.scenario.as_deref(), Some("physics_overlay_demo"));
    assert_eq!(args.snapshot.as_deref(), Some("after"));
    assert_eq!(params["target"], "qa:actor");
    assert_eq!(params["radius"], json!(400.0));
}

#[test]
fn physics_overlay_snapshot_review_rejects_half_configured_mode() {
    let args = PhysicsOverlayArgs {
        target: Some("qa:actor".to_string()),
        viewport: false,
        scenario: Some("physics_overlay_demo".to_string()),
        snapshot: None,
        radius: 256.0,
        frames: 1,
        out: PathBuf::from("overlay.png"),
        include_aabb: false,
        include_contacts: false,
        state_provider: "scene-root".to_string(),
        state_method: Vec::new(),
        no_project_state: false,
        max_shapes: 512,
        strict: false,
    };
    let failure = physics_overlay_params(&args).unwrap_err();
    assert_eq!(failure.kind, "invalid_params");
    assert!(
        failure
            .message
            .as_deref()
            .unwrap_or_default()
            .contains("--scenario and --snapshot")
    );
}

#[test]
fn physics_overlay_rejects_invalid_target_viewport_and_mutating_state_methods() {
    let both = PhysicsOverlayArgs {
        target: Some("qa:actor".to_string()),
        viewport: true,
        scenario: None,
        snapshot: None,
        radius: 256.0,
        frames: 1,
        out: PathBuf::from("overlay.png"),
        include_aabb: false,
        include_contacts: false,
        state_provider: "scene-root".to_string(),
        state_method: Vec::new(),
        no_project_state: false,
        max_shapes: 512,
        strict: false,
    };
    assert_eq!(
        physics_overlay_params(&both).unwrap_err().kind,
        "invalid_params"
    );

    let missing = PhysicsOverlayArgs {
        target: None,
        viewport: false,
        ..both
    };
    assert_eq!(
        physics_overlay_params(&missing)
            .unwrap_err()
            .command
            .as_deref(),
        Some("physics overlay")
    );

    let mutating_method = PhysicsOverlayArgs {
        target: Some("qa:actor".to_string()),
        viewport: false,
        scenario: None,
        snapshot: None,
        radius: 256.0,
        frames: 1,
        out: PathBuf::from("overlay.png"),
        include_aabb: false,
        include_contacts: false,
        state_provider: "scene-root".to_string(),
        state_method: vec!["qa_set_collision_shape".to_string()],
        no_project_state: false,
        max_shapes: 512,
        strict: false,
    };
    let failure = physics_overlay_params(&mutating_method).unwrap_err();
    assert_eq!(failure.kind, "invalid_params");
    assert_eq!(failure.command.as_deref(), Some("physics overlay"));
}

#[test]
fn physics_overlay_rejects_bad_out_and_max_shapes() {
    let bad_out = PhysicsOverlayArgs {
        target: Some("qa:actor".to_string()),
        viewport: false,
        scenario: None,
        snapshot: None,
        radius: 256.0,
        frames: 1,
        out: PathBuf::from("overlay.jpg"),
        include_aabb: false,
        include_contacts: false,
        state_provider: "scene-root".to_string(),
        state_method: Vec::new(),
        no_project_state: false,
        max_shapes: 512,
        strict: false,
    };
    assert_eq!(
        physics_overlay_params(&bad_out).unwrap_err().kind,
        "invalid_params"
    );

    let bad_max = PhysicsOverlayArgs {
        out: PathBuf::from("overlay.png"),
        max_shapes: 4097,
        ..bad_out
    };
    assert_eq!(
        physics_overlay_params(&bad_max).unwrap_err().kind,
        "invalid_params"
    );
}

#[test]
fn physics_overlay_output_paths_are_derived_from_overlay_png() {
    let paths = super::physics_overlay::overlay_artifact_paths(Path::new(
        ".qa/artifacts/manual/physics_overlay_actor.png",
    ));
    assert_eq!(
        paths.base,
        PathBuf::from(".qa/artifacts/manual/physics_overlay_actor.base.png")
    );
    assert_eq!(
        paths.metadata,
        PathBuf::from(".qa/artifacts/manual/physics_overlay_actor.overlay.json")
    );
}

#[test]
fn physics_overlay_metadata_legend_names_rendered_evidence() {
    let legend = super::physics_overlay::overlay_legend();
    assert_eq!(legend["body_collision"]["color"], "#ff00c8");
    assert_eq!(legend["area_collision"]["color"], "#00e1ff");
    assert_eq!(legend["aabb"]["color"], "#ffdc00");
    assert_eq!(legend["contact_point"]["style"], "dot");
    assert_eq!(legend["visual_footprint"]["style"], "solid outline");
}

#[test]
fn physics_overlay_renderer_draws_shapes_contacts_and_footprints() {
    let dir = temp_dir("physics-overlay-render");
    let base = dir.join("base.png");
    let overlay = dir.join("overlay.png");
    image::RgbaImage::from_pixel(64, 64, image::Rgba([0, 0, 0, 255]))
        .save(&base)
        .unwrap();
    let collection = Map::from_iter([
        (
            "shapes".to_string(),
            json!([{
                "screen_points": [{"x": 10, "y": 10}, {"x": 40, "y": 10}, {"x": 40, "y": 40}, {"x": 10, "y": 40}],
                "screen_aabb": {"x": 10, "y": 10, "w": 30, "h": 30},
                "aabb": {"x": 10, "y": 10, "w": 30, "h": 30},
                "is_body": true,
                "is_area": false
            }]),
        ),
        (
            "contacts".to_string(),
            json!([{"screen_position": {"x": 20, "y": 20}, "screen_normal": {"x": 1, "y": 0}}]),
        ),
        (
            "project_state".to_string(),
            json!({"visual_footprints": [{"screen_points": [{"x": 5, "y": 5}, {"x": 15, "y": 5}, {"x": 15, "y": 15}, {"x": 5, "y": 15}]}]}),
        ),
    ]);

    let metadata = super::physics_overlay::render_overlay_png(&base, &overlay, &collection)
        .expect("overlay renders");
    assert_eq!(metadata["rendered"]["shape_count"], json!(1));
    assert_eq!(metadata["rendered"]["contact_count"], json!(1));
    assert_eq!(metadata["rendered"]["visual_footprint_count"], json!(1));
    let rendered = image::open(&overlay).unwrap().to_rgba8();
    assert_eq!(rendered.get_pixel(10, 10).0, [255, 220, 0, 255]);
    assert_eq!(rendered.get_pixel(20, 24).0, [255, 40, 40, 255]);
    assert_eq!(rendered.get_pixel(5, 5).0, [0, 235, 90, 255]);
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn physics_collision_report_live_parses_and_builds_trace_params() {
    let cli = Cli::parse_from([
        "godot-qa",
        "physics",
        "collision-report",
        "qa:actor",
        "--frames",
        "180",
        "--sample-every-frames",
        "2",
        "--physics",
        "--state-provider",
        "scene-root",
        "--state-method",
        "get_qa_collision_state",
        "--max-normal-retention",
        "0.35",
        "--strict-warnings",
    ]);
    let Commands::Physics(args) = cli.command else {
        panic!("expected physics command");
    };
    let PhysicsCommands::CollisionReport(args) = args.command else {
        panic!("expected collision-report command");
    };
    let params = physics_collision_report_params(&args).unwrap();
    assert_eq!(params["targets"], json!(["qa:actor"]));
    assert_eq!(params["frames"], json!(180));
    assert_eq!(params["sample_every_frames"], json!(2));
    assert_eq!(params["include_physics"], json!(true));
    assert_eq!(params["include_project_state"], json!(true));
    assert_eq!(params["state_methods"], json!(["get_qa_collision_state"]));
    assert!(args.strict_warnings);
    assert_eq!(args.max_normal_retention, Some(0.35));
}

#[test]
fn physics_collision_report_defaults_to_primary_state_method_only() {
    let cli = Cli::parse_from([
        "godot-qa",
        "physics",
        "collision-report",
        "qa:actor",
        "--frames",
        "300",
        "--sample-every-frames",
        "1",
        "--physics",
    ]);
    let Commands::Physics(args) = cli.command else {
        panic!("expected physics command");
    };
    let PhysicsCommands::CollisionReport(args) = args.command else {
        panic!("expected collision-report command");
    };
    let params = physics_collision_report_params(&args).unwrap();
    assert_eq!(params["include_project_state"], json!(true));
    assert_eq!(params["state_methods"], json!(["get_qa_collision_state"]));
}

#[test]
fn physics_collision_report_from_trace_parses_without_target() {
    let cli = Cli::parse_from([
        "godot-qa",
        "physics",
        "collision-report",
        "--from-trace",
        ".qa/artifacts/latest/trace.json",
        "--max-velocity-reversals",
        "2",
    ]);
    let Commands::Physics(args) = cli.command else {
        panic!("expected physics command");
    };
    let PhysicsCommands::CollisionReport(args) = args.command else {
        panic!("expected collision-report command");
    };
    let params = physics_collision_report_params(&args).unwrap();
    assert_eq!(
        params["from_trace"],
        json!(".qa/artifacts/latest/trace.json")
    );
    assert_eq!(params["targets"], json!([]));
    assert_eq!(args.max_velocity_reversals, Some(2));
}

#[test]
fn physics_collision_report_rejects_invalid_bounds_and_methods() {
    let both = PhysicsCollisionReportArgs {
        target: Some("qa:actor".to_string()),
        from_trace: Some(PathBuf::from("trace.json")),
        setup_script: None,
        summary_json: false,
        frames: 60,
        sample_every_frames: 1,
        physics: false,
        state_provider: "scene-root".to_string(),
        state_method: Vec::new(),
        max_normal_retention: None,
        max_velocity_reversals: None,
        max_same_contact_frames: None,
        max_angular_velocity_spike: None,
        max_overlap_depth: None,
        max_low_progress_contact_frames: None,
        strict_warnings: false,
    };
    assert_eq!(
        physics_collision_report_params(&both).unwrap_err().kind,
        "invalid_params"
    );

    let bad_frames = PhysicsCollisionReportArgs {
        target: Some("qa:actor".to_string()),
        from_trace: None,
        frames: 0,
        ..both
    };
    assert_eq!(
        physics_collision_report_params(&bad_frames)
            .unwrap_err()
            .command
            .as_deref(),
        Some("physics collision-report")
    );

    let bad_threshold = PhysicsCollisionReportArgs {
        target: Some("qa:actor".to_string()),
        from_trace: None,
        frames: 60,
        max_overlap_depth: Some(-1.0),
        ..bad_frames
    };
    assert_eq!(
        physics_collision_report_params(&bad_threshold)
            .unwrap_err()
            .kind,
        "invalid_params"
    );

    let bad_method = PhysicsCollisionReportArgs {
        target: Some("qa:actor".to_string()),
        from_trace: None,
        frames: 60,
        max_overlap_depth: None,
        state_method: vec!["get_qa_visual_footprint_state".to_string()],
        ..bad_threshold
    };
    assert_eq!(
        physics_collision_report_params(&bad_method)
            .unwrap_err()
            .kind,
        "invalid_params"
    );
}

#[test]
fn physics_collision_report_command_path_is_report_name() {
    let cli = Cli::parse_from([
        "godot-qa",
        "physics",
        "collision-report",
        "--from-trace",
        "trace.json",
    ]);
    assert_eq!(command_path(&cli.command), "physics collision-report");
}

#[test]
fn setup_script_flags_parse_for_live_evidence_commands() {
    let setup = "qa/review-scripts/prepare_live.yaml";

    let cli = Cli::parse_from([
        "godot-qa",
        "trace",
        "record",
        "--target",
        "qa:raft_0",
        "--setup-script",
        setup,
        "--summary-json",
    ]);
    let Commands::Trace(args) = cli.command else {
        panic!("expected trace command");
    };
    let TraceCommands::Record(args) = args.command;
    assert_eq!(args.setup_script.as_deref(), Some(setup));
    assert!(args.summary_json);

    let cli = Cli::parse_from([
        "godot-qa",
        "event",
        "trace",
        "--target",
        "qa:race",
        "--provider",
        "get_qa_event_stream_state",
        "--setup-script",
        setup,
        "--summary-json",
    ]);
    let Commands::Event(args) = cli.command else {
        panic!("expected event command");
    };
    let super::EventCommands::Trace(args) = args.command else {
        panic!("expected event trace command");
    };
    assert_eq!(args.setup_script.as_deref(), Some(setup));
    assert!(args.summary_json);

    let cli = Cli::parse_from([
        "godot-qa",
        "physics",
        "collision-report",
        "qa:raft_0",
        "--setup-script",
        setup,
        "--summary-json",
    ]);
    let Commands::Physics(args) = cli.command else {
        panic!("expected physics command");
    };
    let PhysicsCommands::CollisionReport(args) = args.command else {
        panic!("expected collision-report command");
    };
    assert_eq!(args.setup_script.as_deref(), Some(setup));
    assert!(args.summary_json);

    let cli = Cli::parse_from([
        "godot-qa",
        "surface",
        "audit",
        "qa:surface_provider",
        "--center",
        "qa:raft_0",
        "--region-size",
        "320x240",
        "--sample-step",
        "16",
        "--actor-radius",
        "24",
        "--setup-script",
        setup,
    ]);
    let Commands::Surface(args) = cli.command else {
        panic!("expected surface command");
    };
    let SurfaceCommands::Audit(args) = args.command;
    assert_eq!(args.setup_script.as_deref(), Some(setup));

    let cli = Cli::parse_from([
        "godot-qa",
        "audio",
        "capture",
        "--live",
        "--seconds",
        "2",
        "--setup-script",
        setup,
    ]);
    let Commands::Audio(args) = cli.command else {
        panic!("expected audio command");
    };
    let AudioCommands::Capture(args) = args.command else {
        panic!("expected audio capture command");
    };
    assert_eq!(args.setup_script.as_deref(), Some(setup));

    let cli = Cli::parse_from([
        "godot-qa",
        "session",
        "profile",
        "--frames",
        "60",
        "--setup-script",
        setup,
        "--summary-json",
    ]);
    let Commands::Session(args) = cli.command else {
        panic!("expected session command");
    };
    let SessionCommands::Profile(args) = args.command else {
        panic!("expected session profile command");
    };
    assert_eq!(args.setup_script.as_deref(), Some(setup));
    assert!(args.summary_json);

    let cli = Cli::parse_from([
        "godot-qa",
        "screenshot",
        "capture",
        "--out",
        ".qa/artifacts/manual/current.png",
        "--setup-script",
        setup,
    ]);
    let Commands::Screenshot(args) = cli.command else {
        panic!("expected screenshot command");
    };
    let ScreenshotCommands::Capture(args) = args.command else {
        panic!("expected screenshot capture command");
    };
    assert_eq!(args.setup_script.as_deref(), Some(setup));

    let cli = Cli::parse_from([
        "godot-qa",
        "screenshot",
        "sequence",
        "--frames",
        "60",
        "--every",
        "15",
        "--out",
        ".qa/artifacts/manual/sequence",
        "--setup-script",
        setup,
    ]);
    let Commands::Screenshot(args) = cli.command else {
        panic!("expected screenshot command");
    };
    let ScreenshotCommands::Sequence(args) = args.command else {
        panic!("expected screenshot sequence command");
    };
    assert_eq!(args.setup_script.as_deref(), Some(setup));
}

#[test]
fn screenshot_state_guards_parse_for_capture_and_sequence() {
    let require_state = r#"{"source":"provider","provider":"countdown","matches":[{"path":"current_surface","operator":"equals","expected":"race"}]}"#;

    let cli = Cli::parse_from([
        "godot-qa",
        "screenshot",
        "capture",
        "--out",
        ".qa/artifacts/manual/current.png",
        "--require-visible",
        "qa:race",
        "--require-hidden",
        "qa:lobby",
        "--require-state",
        require_state,
    ]);
    let Commands::Screenshot(args) = cli.command else {
        panic!("expected screenshot command");
    };
    let ScreenshotCommands::Capture(args) = args.command else {
        panic!("expected screenshot capture command");
    };
    assert_eq!(args.require_visible, vec!["qa:race"]);
    assert_eq!(args.require_hidden, vec!["qa:lobby"]);
    assert_eq!(args.require_state.as_deref(), Some(require_state));

    let cli = Cli::parse_from([
        "godot-qa",
        "screenshot",
        "sequence",
        "--frames",
        "60",
        "--every",
        "15",
        "--out",
        ".qa/artifacts/manual/sequence",
        "--require-visible",
        "qa:race",
        "--require-hidden",
        "qa:lobby",
        "--require-state",
        require_state,
    ]);
    let Commands::Screenshot(args) = cli.command else {
        panic!("expected screenshot command");
    };
    let ScreenshotCommands::Sequence(args) = args.command else {
        panic!("expected screenshot sequence command");
    };
    assert_eq!(args.require_visible, vec!["qa:race"]);
    assert_eq!(args.require_hidden, vec!["qa:lobby"]);
    assert_eq!(args.require_state.as_deref(), Some(require_state));
}

#[test]
fn visual_isolate_parses_provider_driver_and_output_args() {
    let cli = Cli::parse_from([
        "godot-qa",
        "visual",
        "isolate",
        "--setup-script",
        "qa/review-scripts/prepare_live.yaml",
        "--provider",
        "race_visual_layers",
        "--driver",
        "race_review",
        "--action",
        "set_visual_layer_mode",
        "--layer",
        "water_surface",
        "--layer",
        "bank_shallow_gradient",
        "--out",
        ".qa/artifacts/manual/race_visual_isolation",
        "--columns",
        "3",
        "--thumb-width",
        "300",
        "--contact-sheet-out",
        ".qa/artifacts/manual/race_visual_isolation.png",
    ]);
    let Commands::Visual(args) = cli.command else {
        panic!("expected visual command");
    };
    let VisualCommands::Isolate(args) = args.command else {
        panic!("expected visual isolate command");
    };
    assert_eq!(
        args.setup_script.as_deref(),
        Some("qa/review-scripts/prepare_live.yaml")
    );
    assert_eq!(args.provider, "race_visual_layers");
    assert_eq!(args.driver, "race_review");
    assert_eq!(args.action, "set_visual_layer_mode");
    assert_eq!(args.layers, vec!["water_surface", "bank_shallow_gradient"]);
    assert_eq!(args.out, ".qa/artifacts/manual/race_visual_isolation");
    assert_eq!(args.columns, 3);
    assert_eq!(args.thumb_width, 300);
    assert_eq!(
        args.contact_sheet_out.as_deref(),
        Some(".qa/artifacts/manual/race_visual_isolation.png")
    );
}

#[test]
fn summary_json_result_prunes_large_payloads_but_keeps_artifact_paths() {
    let result = CommandResult {
        ok: true,
        command: "event trace".to_string(),
        message: "event trace completed".to_string(),
        data: Map::from_iter([
            ("events".to_string(), json!([{"event_id": "a"}])),
            ("samples".to_string(), json!([{"frame": 1}])),
            (
                "profile".to_string(),
                json!({
                    "frame_times_ms": [16.0],
                    "monitor_samples": [{"time": 0}],
                    "frame_time_ms": {"sample_count": 1}
                }),
            ),
            (
                "artifacts".to_string(),
                json!({"events": ".qa/events.json", "summary": ".qa/summary.json"}),
            ),
            ("summary".to_string(), json!({"total_event_count": 1})),
        ]),
        failures: Vec::new(),
        exit_code: 0,
    };

    let pruned = summary_json_result(result);

    assert_eq!(pruned.data["summary_json"], json!(true));
    assert!(pruned.data.get("events").is_none());
    assert!(pruned.data.get("samples").is_none());
    assert!(pruned.data["profile"].get("frame_times_ms").is_none());
    assert!(pruned.data["profile"].get("monitor_samples").is_none());
    assert_eq!(pruned.data["artifacts"]["events"], json!(".qa/events.json"));
    assert_eq!(pruned.data["summary"]["total_event_count"], json!(1));
}

#[test]
fn setup_script_failure_is_reported_before_main_live_command() {
    let root = temp_dir("setup-script-before-main");
    let config = test_project_config(root.clone());
    let script = root.join("qa/review-scripts/broken.yaml");
    fs::create_dir_all(script.parent().unwrap()).unwrap();
    fs::write(&script, "name: broken\n").unwrap();

    let result = run_setup_script_before(
        &config,
        "event trace",
        Some("qa/review-scripts/broken.yaml"),
    )
    .expect("broken setup script should stop command");

    assert!(!result.ok);
    assert_eq!(result.command, "event trace");
    assert_eq!(
        result.data["setup_script"],
        json!("qa/review-scripts/broken.yaml")
    );
    assert_eq!(
        result.data["setup_result"]["command"],
        json!("session run-script")
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn run_bounded_setup_script_returns_result_when_worker_finishes() {
    let completed = run_bounded_setup_script(Duration::from_secs(5), || CommandResult {
        ok: true,
        command: "session run-script".to_string(),
        message: "done".to_string(),
        data: Map::new(),
        failures: Vec::new(),
        exit_code: 0,
    })
    .expect("fast worker should complete before deadline");
    assert!(completed.ok);
    assert_eq!(completed.message, "done");
}

#[test]
fn run_bounded_setup_script_times_out_when_worker_overruns() {
    let started = Instant::now();
    let outcome = run_bounded_setup_script(Duration::from_millis(100), || {
        std::thread::sleep(Duration::from_secs(30));
        CommandResult {
            ok: true,
            command: "session run-script".to_string(),
            message: "should never arrive".to_string(),
            data: Map::new(),
            failures: Vec::new(),
            exit_code: 0,
        }
    });
    assert!(matches!(
        outcome,
        Err(std::sync::mpsc::RecvTimeoutError::Timeout)
    ));
    // The wait is bounded by the deadline, not the (30s) worker.
    assert!(started.elapsed() < Duration::from_secs(5));
}

#[test]
fn setup_script_timeout_result_is_structured_and_names_the_script() {
    let result = setup_script_timeout_result(
        "trace record",
        "qa/review-scripts/hangs.yaml",
        7,
        Duration::from_millis(7200),
    );
    assert!(!result.ok);
    assert_eq!(result.command, "trace record");
    assert_eq!(result.failures.len(), 1);
    assert_eq!(result.failures[0].kind, "setup_script_timeout");
    assert_eq!(
        result.data["setup_script"],
        json!("qa/review-scripts/hangs.yaml")
    );
    assert_eq!(result.data["setup_timeout_seconds"], json!(7));
    assert!(
        result.failures[0]
            .message
            .as_deref()
            .unwrap()
            .contains("qa/review-scripts/hangs.yaml")
    );
}

#[test]
fn run_setup_script_before_with_timeout_passes_through_fast_failures() {
    let root = temp_dir("setup-script-timeout-passthrough");
    let config = test_project_config(root.clone());
    let script = root.join("qa/review-scripts/broken.yaml");
    fs::create_dir_all(script.parent().unwrap()).unwrap();
    fs::write(&script, "name: broken\n").unwrap();

    // A generous timeout must not interfere with a script that fails fast.
    let result = run_setup_script_before_with_timeout(
        &config,
        "trace record",
        Some("qa/review-scripts/broken.yaml"),
        Some(120),
    )
    .expect("broken setup script should stop command");
    assert!(!result.ok);
    assert_ne!(
        result.failures.first().map(|f| f.kind.as_str()),
        Some("setup_script_timeout")
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn setup_script_fixture_call_validation_failure_stops_main_live_command() {
    let root = temp_dir("setup-script-fixture-call-before-main");
    let config = test_project_config(root.clone());
    let script = root.join("qa/review-scripts/broken_fixture_call.yaml");
    fs::create_dir_all(script.parent().unwrap()).unwrap();
    fs::write(
        &script,
        "name: broken_fixture_call\nsteps:\n  - fixture_call:\n      driver: schema_driver\n      action: set_status\n      params: []\n",
    )
    .unwrap();

    let result = run_setup_script_before(
        &config,
        "trace record",
        Some("qa/review-scripts/broken_fixture_call.yaml"),
    )
    .expect("invalid fixture_call setup script should stop command");

    assert!(!result.ok);
    assert_eq!(result.command, "trace record");
    assert_eq!(
        result.data["setup_script"],
        json!("qa/review-scripts/broken_fixture_call.yaml")
    );
    assert_eq!(
        result.data["setup_result"]["command"],
        json!("session run-script")
    );
    assert_eq!(result.failures[0].kind, "invalid_session_script");
    assert_eq!(
        result.failures[0].message.as_deref(),
        Some("fixture_call step field 'params' must be an object when present")
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn surface_audit_cli_contract() {
    assert!(public_command_surface().contains(&"surface audit"));

    let cli = Cli::parse_from([
        "godot-qa",
        "surface",
        "audit",
        "qa:surface_provider",
        "--center",
        "qa:actor_spawn",
        "--region-size",
        "600x400",
        "--sample-step",
        "16",
        "--actor-radius",
        "32",
        "--min-spawn-clearance",
        "96",
        "--out",
        ".qa/artifacts/manual/surface_audit.json",
        "--provider-method",
        "get_qa_surface_state",
        "--strict-warnings",
    ]);
    assert_eq!(command_path(&cli.command), "surface audit");
    let Commands::Surface(args) = cli.command else {
        panic!("expected surface command");
    };
    let SurfaceCommands::Audit(args) = args.command;
    let params = surface_audit_params(&args).unwrap();
    assert_eq!(params["provider"], "qa:surface_provider");
    assert_eq!(params["mode"], "region");
    assert_eq!(params["center"], "qa:actor_spawn");
    assert_eq!(
        params["region_size"],
        json!({"width": 600.0, "height": 400.0})
    );
    assert_eq!(params["sample_step"], json!(16.0));
    assert_eq!(params["actor_radius"], json!(32.0));
    assert_eq!(params["min_spawn_clearance"], json!(96.0));
    assert_eq!(
        params["out"],
        json!(".qa/artifacts/manual/surface_audit.json")
    );
    assert_eq!(params["provider_method"], "get_qa_surface_state");
    assert_eq!(params["strict_warnings"], json!(true));

    let route_cli = Cli::parse_from([
        "godot-qa",
        "surface",
        "audit",
        "qa:surface_provider",
        "--route",
        "qa:start_anchor,qa:mid_anchor,qa:finish_anchor",
        "--sample-step",
        "16",
        "--actor-radius",
        "32",
        "--min-corridor-width",
        "96",
        "--heatmap-out",
        ".qa/artifacts/manual/surface_corridor_heatmap.png",
        "--provider-method",
        "qa_probe_surface",
    ]);
    let Commands::Surface(route_args) = route_cli.command else {
        panic!("expected surface command");
    };
    let SurfaceCommands::Audit(route_args) = route_args.command;
    let route_params = surface_audit_params(&route_args).unwrap();
    assert_eq!(route_params["mode"], "route");
    assert_eq!(
        route_params["route"],
        json!(["qa:start_anchor", "qa:mid_anchor", "qa:finish_anchor"])
    );
    assert_eq!(route_params["min_corridor_width"], json!(96.0));
    assert_eq!(
        route_params["heatmap_out"],
        json!(".qa/artifacts/manual/surface_corridor_heatmap.png")
    );

    let mirror_cli = Cli::parse_from([
        "godot-qa",
        "surface",
        "audit",
        "qa:surface_provider",
        "--left-region",
        "qa:left_lane",
        "--right-region",
        "qa:right_lane",
        "--mirror-axis",
        "qa:centerline",
        "--sample-step",
        "16",
        "--actor-radius",
        "32",
        "--provider-method",
        "get_surface_qa_state",
    ]);
    let Commands::Surface(mirror_args) = mirror_cli.command else {
        panic!("expected surface command");
    };
    let SurfaceCommands::Audit(mirror_args) = mirror_args.command;
    let mirror_params = surface_audit_params(&mirror_args).unwrap();
    assert_eq!(mirror_params["mode"], "mirror");
    assert_eq!(mirror_params["left_region"], "qa:left_lane");
    assert_eq!(mirror_params["right_region"], "qa:right_lane");
    assert_eq!(mirror_params["mirror_axis"], "qa:centerline");

    for method in [
        "qa_surface_probe",
        "get_qa_surface_state",
        "get_surface_qa_state",
        "qa_probe_surface",
    ] {
        let args = SurfaceAuditArgs {
            provider: Some("qa:surface_provider".to_string()),
            provider_id: None,
            setup_script: None,
            center: Some("qa:spawn".to_string()),
            region_size: Some("320x240".to_string()),
            route: None,
            left_region: None,
            right_region: None,
            mirror_axis: None,
            sample_step: 16.0,
            actor_radius: 24.0,
            min_spawn_clearance: None,
            min_corridor_width: None,
            out: None,
            heatmap_out: None,
            provider_method: method.to_string(),
            strict_warnings: false,
        };
        assert_eq!(
            surface_audit_params(&args).unwrap()["provider_method"],
            method
        );
    }

    let invalid_method = SurfaceAuditArgs {
        provider: Some("qa:surface_provider".to_string()),
        provider_id: None,
        setup_script: None,
        center: Some("qa:spawn".to_string()),
        region_size: Some("320x240".to_string()),
        route: None,
        left_region: None,
        right_region: None,
        mirror_axis: None,
        sample_step: 16.0,
        actor_radius: 24.0,
        min_spawn_clearance: None,
        min_corridor_width: None,
        out: None,
        heatmap_out: None,
        provider_method: "get_qa_visual_footprint_state".to_string(),
        strict_warnings: false,
    };
    assert_eq!(
        surface_audit_params(&invalid_method).unwrap_err().kind,
        "invalid_params"
    );

    let unbounded = SurfaceAuditArgs {
        provider: Some("qa:surface_provider".to_string()),
        provider_id: None,
        setup_script: None,
        center: None,
        region_size: None,
        route: None,
        left_region: None,
        right_region: None,
        mirror_axis: None,
        sample_step: 16.0,
        actor_radius: 24.0,
        min_spawn_clearance: None,
        min_corridor_width: None,
        out: None,
        heatmap_out: None,
        provider_method: "qa_surface_probe".to_string(),
        strict_warnings: false,
    };
    assert_eq!(
        surface_audit_params(&unbounded)
            .unwrap_err()
            .command
            .as_deref(),
        Some("surface audit")
    );

    let incompatible = SurfaceAuditArgs {
        center: Some("qa:spawn".to_string()),
        region_size: Some("320x240".to_string()),
        route: Some("qa:a,qa:b".to_string()),
        ..unbounded
    };
    assert_eq!(
        surface_audit_params(&incompatible).unwrap_err().kind,
        "invalid_params"
    );

    let bad_region = SurfaceAuditArgs {
        center: Some("qa:spawn".to_string()),
        region_size: Some("320-by-240".to_string()),
        route: None,
        ..incompatible
    };
    assert_eq!(
        surface_audit_params(&bad_region).unwrap_err().kind,
        "invalid_params"
    );

    let bad_threshold = SurfaceAuditArgs {
        region_size: Some("320x240".to_string()),
        min_spawn_clearance: Some(0.0),
        ..bad_region
    };
    assert_eq!(
        surface_audit_params(&bad_threshold).unwrap_err().kind,
        "invalid_params"
    );
}

#[test]
fn surface_audit_heatmap_out_requires_png_path() {
    let args = SurfaceAuditArgs {
        provider: Some("qa:surface_provider".to_string()),
        provider_id: None,
        setup_script: None,
        center: Some("qa:spawn".to_string()),
        region_size: Some("320x240".to_string()),
        route: None,
        left_region: None,
        right_region: None,
        mirror_axis: None,
        sample_step: 16.0,
        actor_radius: 24.0,
        min_spawn_clearance: None,
        min_corridor_width: None,
        out: None,
        heatmap_out: Some(PathBuf::from(".qa/artifacts/manual/surface_heatmap.jpg")),
        provider_method: "qa_surface_probe".to_string(),
        strict_warnings: false,
    };
    let failure = surface_audit_params(&args).unwrap_err();
    assert_eq!(failure.kind, "invalid_params");
    assert_eq!(
        failure.message.as_deref(),
        Some("surface audit --heatmap-out must end in .png")
    );
}

#[test]
fn trace_record_parses_and_builds_protocol_params() {
    let cli = Cli::parse_from([
        "godot-qa",
        "trace",
        "record",
        "--target",
        "qa:raft_0",
        "--target",
        "path:/root/Other",
        "--frames",
        "180",
        "--sample-every-frames",
        "2",
        "--physics",
        "--state-provider",
        "scene-root",
        "--state-method",
        "get_qa_physics_state",
        "--snapshot-start",
        "--snapshot-end",
    ]);
    let Commands::Trace(args) = cli.command else {
        panic!("expected trace command");
    };
    let TraceCommands::Record(args) = args.command;
    let params = trace_record_params(&args).unwrap();
    assert_eq!(params["targets"], json!(["qa:raft_0", "path:/root/Other"]));
    assert_eq!(params["frames"], json!(180));
    assert_eq!(params["sample_every_frames"], json!(2));
    assert_eq!(params["include_physics"], json!(true));
    assert_eq!(params["state_provider"], "scene-root");
    assert_eq!(params["state_methods"], json!(["get_qa_physics_state"]));
    assert_eq!(params["include_project_state"], json!(true));
    assert_eq!(params["snapshot_start"], json!(true));
    assert_eq!(params["snapshot_end"], json!(true));
}

#[test]
fn trace_record_rejects_out_of_bounds_and_mutating_state_methods() {
    let invalid_frames = TraceRecordArgs {
        target: vec!["qa:probe".to_string()],
        setup_script: None,
        setup_timeout_seconds: None,
        summary_json: false,
        frames: 3601,
        sample_every_frames: 1,
        physics: false,
        state_provider: "scene-root".to_string(),
        state_method: Vec::new(),
        snapshot_start: false,
        snapshot_end: false,
        stop_after_state: None,
    };
    assert_eq!(
        trace_record_params(&invalid_frames).unwrap_err().kind,
        "invalid_params"
    );

    let invalid_interval = TraceRecordArgs {
        target: vec!["qa:probe".to_string()],
        setup_script: None,
        setup_timeout_seconds: None,
        summary_json: false,
        frames: 60,
        sample_every_frames: 0,
        physics: false,
        state_provider: "scene-root".to_string(),
        state_method: Vec::new(),
        snapshot_start: false,
        snapshot_end: false,
        stop_after_state: None,
    };
    assert_eq!(
        trace_record_params(&invalid_interval).unwrap_err().kind,
        "invalid_params"
    );

    let mutating_method = TraceRecordArgs {
        target: vec!["qa:probe".to_string()],
        setup_script: None,
        setup_timeout_seconds: None,
        summary_json: false,
        frames: 60,
        sample_every_frames: 1,
        physics: false,
        state_provider: "scene-root".to_string(),
        state_method: vec!["qa_prepare_collision_probe".to_string()],
        snapshot_start: false,
        snapshot_end: false,
        stop_after_state: None,
    };
    let failure = trace_record_params(&mutating_method).unwrap_err();
    assert_eq!(failure.kind, "invalid_params");
    assert_eq!(failure.command.as_deref(), Some("trace record"));
}

#[test]
fn trace_record_stop_after_state_parses_and_rejects_bad_json() {
    let valid = Cli::parse_from([
        "godot-qa",
        "trace",
        "record",
        "--target",
        "qa:race",
        "--frames",
        "120",
        "--stop-after-state",
        r#"{"target":"qa:race","source":"method","method":"get_qa_race_state","matches":[{"path":"done","operator":"equals","expected":true}],"sample_every_frames":2}"#,
    ]);
    let Commands::Trace(args) = valid.command else {
        panic!("expected trace command");
    };
    let TraceCommands::Record(args) = args.command;
    let params = trace_record_params(&args).unwrap();
    assert_eq!(params["stop_after_state"]["target"], json!("qa:race"));
    assert_eq!(params["stop_after_state"]["sample_every_frames"], json!(2));

    let invalid = Cli::parse_from([
        "godot-qa",
        "trace",
        "record",
        "--target",
        "qa:race",
        "--stop-after-state",
        "{not valid json",
    ]);
    let Commands::Trace(args) = invalid.command else {
        panic!("expected trace command");
    };
    let TraceCommands::Record(args) = args.command;
    let failure = trace_record_params(&args).unwrap_err();
    assert_eq!(failure.kind, "invalid_params");
    assert_eq!(failure.command.as_deref(), Some("trace record"));

    let missing_target = Cli::parse_from([
        "godot-qa",
        "trace",
        "record",
        "--target",
        "qa:race",
        "--stop-after-state",
        r#"{"matches":[{"path":"done","operator":"equals","expected":true}]}"#,
    ]);
    let Commands::Trace(args) = missing_target.command else {
        panic!("expected trace command");
    };
    let TraceCommands::Record(args) = args.command;
    assert_eq!(
        trace_record_params(&args).unwrap_err().kind,
        "invalid_params"
    );
}

#[test]
fn event_trace_cli_parses_and_builds_protocol_params() {
    assert!(public_command_surface().contains(&"event trace"));
    assert!(public_command_surface().contains(&"event summarize"));

    let cli = Cli::parse_from([
        "godot-qa",
        "event",
        "trace",
        "--target",
        "qa:event_source",
        "--provider",
        "get_qa_event_stream_state",
        "--frames",
        "120",
        "--sample-every-frames",
        "2",
        "--strict-warnings",
        "--json",
    ]);

    assert!(cli.json);
    assert_eq!(command_path(&cli.command), "event trace");
    let Commands::Event(args) = cli.command else {
        panic!("expected event command");
    };
    let super::EventCommands::Trace(args) = args.command else {
        panic!("expected event trace command");
    };
    let params = event_trace_params(&args).unwrap();
    assert_eq!(params["target"], json!("qa:event_source"));
    assert_eq!(params["provider"], json!("get_qa_event_stream_state"));
    assert_eq!(params["frames"], json!(120));
    assert_eq!(params["sample_every_frames"], json!(2));
    assert_eq!(params["strict_warnings"], json!(true));
}

#[test]
fn event_trace_accepts_target_locator_provider_alias() {
    let cli = Cli::parse_from([
        "godot-qa",
        "event",
        "trace",
        "--target",
        "qa:race",
        "--provider",
        "qa:race",
    ]);
    let Commands::Event(args) = cli.command else {
        panic!("expected event command");
    };
    let super::EventCommands::Trace(args) = args.command else {
        panic!("expected event trace command");
    };

    let params = event_trace_params(&args).unwrap();

    assert_eq!(params["target"], json!("qa:race"));
    assert_eq!(params["provider"], json!("get_qa_event_stream_state"));
}

#[test]
fn event_trace_rejects_invalid_provider_bounds_and_pairing_config() {
    let bad_provider = super::EventTraceArgs {
        target: "qa:event_source".to_string(),
        provider: " ".to_string(),
        setup_script: None,
        summary_json: false,
        frames: 120,
        sample_every_frames: 1,
        pairing_config: None,
        strict_warnings: false,
    };
    assert_eq!(
        event_trace_params(&bad_provider).unwrap_err().kind,
        "invalid_params"
    );

    let bad_frames = super::EventTraceArgs {
        provider: "get_qa_event_stream_state".to_string(),
        frames: 3601,
        ..bad_provider
    };
    assert_eq!(
        event_trace_params(&bad_frames).unwrap_err().kind,
        "invalid_params"
    );

    let bad_sample_interval = super::EventTraceArgs {
        frames: 120,
        sample_every_frames: 0,
        ..bad_frames
    };
    assert_eq!(
        event_trace_params(&bad_sample_interval).unwrap_err().kind,
        "invalid_params"
    );

    let dir = temp_dir("event-trace-bad-pairing");
    let pairing = dir.join("pairing.json");
    fs::write(
        &pairing,
        r#"{"pairs":[{"source":"","requires_within_msec":1,"any_of":["sfx.ok"]}]}"#,
    )
    .unwrap();
    let bad_pairing = super::EventTraceArgs {
        sample_every_frames: 1,
        pairing_config: Some(pairing),
        ..bad_sample_interval
    };
    assert_eq!(
        event_trace_params(&bad_pairing).unwrap_err().kind,
        "event_trace.invalid_pairing_config"
    );
}

#[test]
fn event_trace_summarize_from_file_uses_event_summary_contract() {
    let dir = temp_dir("event-trace-summarize");
    let events = dir.join("events.json");
    fs::write(
        &events,
        serde_json::to_string_pretty(&json!({
            "events": [
                {
                    "event_id": "a",
                    "frame": 1,
                    "time_msec": 0.0,
                    "category": "gameplay",
                    "event_type": "gameplay.spawn",
                    "payload": {}
                }
            ]
        }))
        .unwrap(),
    )
    .unwrap();
    let args = super::EventSummarizeArgs {
        events: events.clone(),
        pairing_config: None,
        strict_warnings: false,
    };
    let params = event_summarize_params(&args).unwrap();
    let result = runtime::event_summarize_result(
        &params.events,
        params.pairing_config.as_deref(),
        params.strict_warnings,
    );

    assert!(result.ok, "{:?}", result.failures);
    assert_eq!(result.data["total_event_count"], json!(1));
    assert_eq!(
        result.data["event_count_by_type"]["gameplay.spawn"],
        json!(1)
    );
    assert_eq!(result.data["artifacts"]["events"], json!(events));
}

#[test]
fn event_trace_strict_warning_failure_from_summarize() {
    let dir = temp_dir("event-trace-strict");
    let events = dir.join("events.json");
    fs::write(&events, r#"{"events":[]}"#).unwrap();

    let result = runtime::event_summarize_result(&events, None, true);

    assert!(!result.ok);
    assert_eq!(result.exit_code, 1);
    assert_eq!(result.failures[0].kind, "event_trace.strict_warnings");
    assert_eq!(
        result.data["warning_ids"],
        json!(["event_trace.no_events_recorded"])
    );
}

#[test]
fn snapshot_filter_params_accept_comma_and_repeat_metadata_keys() {
    let cli = Cli::parse_from([
        "godot-qa",
        "tree",
        "--target",
        " qa:start_button ",
        "--max-depth",
        "2",
        "--qa-only",
        "--include-metadata",
        "--metadata-key",
        "qa_id,role",
        "--metadata-keys",
        "state:current",
    ]);
    let Commands::Tree(args) = cli.command else {
        panic!("expected tree command");
    };
    let params = snapshot_filter_params("tree", &args.filters).unwrap();
    assert_eq!(params["target"], "qa:start_button");
    assert_eq!(params["max_depth"], 2);
    assert_eq!(params["qa_only"], true);
    assert_eq!(params["include_metadata"], true);
    assert_eq!(
        params["metadata_keys"],
        serde_json::json!(["qa_id", "role", "state:current"])
    );
}

#[test]
fn snapshot_filter_params_preserve_empty_params_when_unfiltered() {
    let cli = Cli::parse_from(["godot-qa", "tree"]);
    let Commands::Tree(args) = cli.command else {
        panic!("expected tree command");
    };
    let params = snapshot_filter_params("tree", &args.filters).unwrap();
    assert!(params.is_empty());
}

#[test]
fn input_joy_motion_parses_axis_value_device_and_frames() {
    let cli = Cli::parse_from([
        "godot-qa", "input", "joy", "motion", "right_x", "1.0", "--frames", "2", "--device", "0",
    ]);
    assert_eq!(command_path(&cli.command), "input joy motion");
    let Commands::Input(args) = cli.command else {
        panic!("expected input command");
    };
    let InputCommands::Joy(joy) = args.command else {
        panic!("expected joy command");
    };
    let InputJoyCommands::Motion(motion) = joy.command else {
        panic!("expected joy motion command");
    };
    assert_eq!(motion.axis, "right_x");
    assert_eq!(motion.value, 1.0);
    assert_eq!(motion.frames, 2);
    assert_eq!(motion.device, 0);
}

#[test]
fn input_joy_button_parses_button_device_and_frames() {
    let cli = Cli::parse_from([
        "godot-qa", "input", "joy", "button", "start", "--frames", "2", "--device", "3",
    ]);
    assert_eq!(command_path(&cli.command), "input joy button");
    let Commands::Input(args) = cli.command else {
        panic!("expected input command");
    };
    let InputCommands::Joy(joy) = args.command else {
        panic!("expected joy command");
    };
    let InputJoyCommands::Button(button) = joy.command else {
        panic!("expected joy button command");
    };
    assert_eq!(button.button, "start");
    assert_eq!(button.frames, 2);
    assert_eq!(button.device, 3);
}

#[test]
fn input_joy_button_defaults_device_and_frames() {
    let cli = Cli::parse_from(["godot-qa", "input", "joy", "button", "start"]);
    let Commands::Input(args) = cli.command else {
        panic!("expected input command");
    };
    let InputCommands::Joy(joy) = args.command else {
        panic!("expected joy command");
    };
    let InputJoyCommands::Button(button) = joy.command else {
        panic!("expected joy button command");
    };
    assert_eq!(button.button, "start");
    assert_eq!(button.frames, 1);
    assert_eq!(button.device, 0);
}

#[test]
fn input_joy_motion_defaults_device_and_frames() {
    let cli = Cli::parse_from(["godot-qa", "input", "joy", "motion", "trigger_left", "0.49"]);
    let Commands::Input(args) = cli.command else {
        panic!("expected input command");
    };
    let InputCommands::Joy(joy) = args.command else {
        panic!("expected joy command");
    };
    let InputJoyCommands::Motion(motion) = joy.command else {
        panic!("expected joy motion command");
    };
    assert_eq!(motion.axis, "trigger_left");
    assert_eq!(motion.value, 0.49);
    assert_eq!(motion.frames, 1);
    assert_eq!(motion.device, 0);
}

#[test]
fn input_mouse_wheel_defaults_dx_and_dy() {
    let cli = Cli::parse_from([
        "godot-qa", "input", "mouse", "wheel", "--x", "500", "--y", "300",
    ]);
    assert_eq!(command_path(&cli.command), "input mouse wheel");
    let Commands::Input(args) = cli.command else {
        panic!("expected input command");
    };
    let InputCommands::Mouse(mouse) = args.command else {
        panic!("expected mouse command");
    };
    let InputMouseCommands::Wheel(wheel) = mouse.command else {
        panic!("expected mouse wheel command");
    };
    assert_eq!(wheel.x, 500);
    assert_eq!(wheel.y, 300);
    assert_eq!(wheel.dy, 0.0);
    assert_eq!(wheel.dx, 0.0);
}

#[test]
fn input_mouse_wheel_parses_explicit_dx_and_dy() {
    let cli = Cli::parse_from([
        "godot-qa", "input", "mouse", "wheel", "--x", "500", "--y", "300", "--dy", "-3.0", "--dx",
        "1.0",
    ]);
    assert_eq!(command_path(&cli.command), "input mouse wheel");
    let Commands::Input(args) = cli.command else {
        panic!("expected input command");
    };
    let InputCommands::Mouse(mouse) = args.command else {
        panic!("expected mouse command");
    };
    let InputMouseCommands::Wheel(wheel) = mouse.command else {
        panic!("expected mouse wheel command");
    };
    assert_eq!(wheel.x, 500);
    assert_eq!(wheel.y, 300);
    assert_eq!(wheel.dy, -3.0);
    assert_eq!(wheel.dx, 1.0);
}

#[test]
fn input_touch_down_defaults_index() {
    let cli = Cli::parse_from([
        "godot-qa", "input", "touch", "down", "--x", "640", "--y", "360",
    ]);
    assert_eq!(command_path(&cli.command), "input touch down");
    let Commands::Input(args) = cli.command else {
        panic!("expected input command");
    };
    let InputCommands::Touch(touch) = args.command else {
        panic!("expected touch command");
    };
    let InputTouchCommands::Down(down) = touch.command else {
        panic!("expected touch down command");
    };
    assert_eq!(down.x, 640);
    assert_eq!(down.y, 360);
    assert_eq!(down.index, 0);
}

#[test]
fn input_touch_down_parses_explicit_index() {
    let cli = Cli::parse_from([
        "godot-qa", "input", "touch", "down", "--x", "640", "--y", "360", "--index", "1",
    ]);
    assert_eq!(command_path(&cli.command), "input touch down");
    let Commands::Input(args) = cli.command else {
        panic!("expected input command");
    };
    let InputCommands::Touch(touch) = args.command else {
        panic!("expected touch command");
    };
    let InputTouchCommands::Down(down) = touch.command else {
        panic!("expected touch down command");
    };
    assert_eq!(down.x, 640);
    assert_eq!(down.y, 360);
    assert_eq!(down.index, 1);
}

#[test]
fn input_touch_move_defaults_frames_and_index() {
    let cli = Cli::parse_from([
        "godot-qa", "input", "touch", "move", "--x", "400", "--y", "360",
    ]);
    assert_eq!(command_path(&cli.command), "input touch move");
    let Commands::Input(args) = cli.command else {
        panic!("expected input command");
    };
    let InputCommands::Touch(touch) = args.command else {
        panic!("expected touch command");
    };
    let InputTouchCommands::Move(move_args) = touch.command else {
        panic!("expected touch move command");
    };
    assert_eq!(move_args.x, 400);
    assert_eq!(move_args.y, 360);
    assert_eq!(move_args.frames, 1);
    assert_eq!(move_args.index, 0);
}

#[test]
fn input_touch_move_parses_explicit_frames_and_index() {
    let cli = Cli::parse_from([
        "godot-qa", "input", "touch", "move", "--x", "400", "--y", "360", "--frames", "10",
        "--index", "1",
    ]);
    assert_eq!(command_path(&cli.command), "input touch move");
    let Commands::Input(args) = cli.command else {
        panic!("expected input command");
    };
    let InputCommands::Touch(touch) = args.command else {
        panic!("expected touch command");
    };
    let InputTouchCommands::Move(move_args) = touch.command else {
        panic!("expected touch move command");
    };
    assert_eq!(move_args.x, 400);
    assert_eq!(move_args.y, 360);
    assert_eq!(move_args.frames, 10);
    assert_eq!(move_args.index, 1);
}

#[test]
fn input_touch_up_without_coords_leaves_x_and_y_none() {
    let cli = Cli::parse_from(["godot-qa", "input", "touch", "up"]);
    assert_eq!(command_path(&cli.command), "input touch up");
    let Commands::Input(args) = cli.command else {
        panic!("expected input command");
    };
    let InputCommands::Touch(touch) = args.command else {
        panic!("expected touch command");
    };
    let InputTouchCommands::Up(up) = touch.command else {
        panic!("expected touch up command");
    };
    assert_eq!(up.x, None);
    assert_eq!(up.y, None);
    assert_eq!(up.index, 0);
}

#[test]
fn input_touch_up_parses_explicit_coords_and_index() {
    let cli = Cli::parse_from([
        "godot-qa", "input", "touch", "up", "--x", "400", "--y", "360", "--index", "2",
    ]);
    assert_eq!(command_path(&cli.command), "input touch up");
    let Commands::Input(args) = cli.command else {
        panic!("expected input command");
    };
    let InputCommands::Touch(touch) = args.command else {
        panic!("expected touch command");
    };
    let InputTouchCommands::Up(up) = touch.command else {
        panic!("expected touch up command");
    };
    assert_eq!(up.x, Some(400));
    assert_eq!(up.y, Some(360));
    assert_eq!(up.index, 2);
}

#[test]
fn input_touch_tap_defaults_index() {
    let cli = Cli::parse_from([
        "godot-qa", "input", "touch", "tap", "--x", "640", "--y", "360",
    ]);
    assert_eq!(command_path(&cli.command), "input touch tap");
    let Commands::Input(args) = cli.command else {
        panic!("expected input command");
    };
    let InputCommands::Touch(touch) = args.command else {
        panic!("expected touch command");
    };
    let InputTouchCommands::Tap(tap) = touch.command else {
        panic!("expected touch tap command");
    };
    assert_eq!(tap.x, 640);
    assert_eq!(tap.y, 360);
    assert_eq!(tap.index, 0);
}

#[test]
fn input_drag_defaults_frames_pointer_and_index() {
    let cli = Cli::parse_from([
        "godot-qa", "input", "drag", "--from-x", "600", "--from-y", "300", "--to-x", "200",
        "--to-y", "300",
    ]);
    assert_eq!(command_path(&cli.command), "input drag");
    let Commands::Input(args) = cli.command else {
        panic!("expected input command");
    };
    let InputCommands::Drag(drag) = args.command else {
        panic!("expected drag command");
    };
    assert_eq!(drag.from_x, 600);
    assert_eq!(drag.from_y, 300);
    assert_eq!(drag.to_x, 200);
    assert_eq!(drag.to_y, 300);
    assert_eq!(drag.frames, 8);
    assert!(matches!(drag.pointer, PointerKind::Touch));
    assert_eq!(drag.index, 0);
}

#[test]
fn input_drag_parses_explicit_pointer_frames_and_index() {
    let cli = Cli::parse_from([
        "godot-qa",
        "input",
        "drag",
        "--from-x",
        "600",
        "--from-y",
        "300",
        "--to-x",
        "200",
        "--to-y",
        "300",
        "--pointer",
        "mouse",
        "--frames",
        "12",
        "--index",
        "1",
    ]);
    assert_eq!(command_path(&cli.command), "input drag");
    let Commands::Input(args) = cli.command else {
        panic!("expected input command");
    };
    let InputCommands::Drag(drag) = args.command else {
        panic!("expected drag command");
    };
    assert_eq!(drag.from_x, 600);
    assert_eq!(drag.from_y, 300);
    assert_eq!(drag.to_x, 200);
    assert_eq!(drag.to_y, 300);
    assert_eq!(drag.frames, 12);
    assert!(matches!(drag.pointer, PointerKind::Mouse));
    assert_eq!(drag.index, 1);
}

#[test]
fn input_type_parses_positional_text_with_spaces() {
    let cli = Cli::parse_from(["godot-qa", "input", "type", "Hello, 127.0.0.1!"]);
    assert_eq!(command_path(&cli.command), "input type");
    let Commands::Input(args) = cli.command else {
        panic!("expected input command");
    };
    let InputCommands::Type(text) = args.command else {
        panic!("expected input type command");
    };
    assert_eq!(text.text, "Hello, 127.0.0.1!");
}

#[test]
fn input_type_allows_hyphen_leading_text() {
    let cli = Cli::parse_from(["godot-qa", "input", "type", "-3 degrees"]);
    assert_eq!(command_path(&cli.command), "input type");
    let Commands::Input(args) = cli.command else {
        panic!("expected input command");
    };
    let InputCommands::Type(text) = args.command else {
        panic!("expected input type command");
    };
    assert_eq!(text.text, "-3 degrees");
}

#[test]
fn snapshot_filter_params_reject_negative_depth_and_unsafe_key() {
    let cli = Cli::parse_from(["godot-qa", "snapshot", "--max-depth=-1"]);
    let Commands::Snapshot(args) = cli.command else {
        panic!("expected snapshot command");
    };
    let failure = snapshot_filter_params("snapshot", &args.filters).unwrap_err();
    assert_eq!(failure.kind, "invalid_params");
    assert_eq!(failure.actual, Some(serde_json::json!(-1)));

    let cli = Cli::parse_from(["godot-qa", "tree", "--metadata-key", "../secret"]);
    let Commands::Tree(args) = cli.command else {
        panic!("expected tree command");
    };
    let failure = snapshot_filter_params("tree", &args.filters).unwrap_err();
    assert_eq!(failure.kind, "invalid_params");
    assert_eq!(failure.actual, Some(serde_json::json!("../secret")));
}
