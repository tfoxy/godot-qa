#![allow(dead_code, unused_imports)]

use super::runtime;
use super::{
    AudioCommands, Cli, Commands, InputCommands, InputJoyCommands, PathCandidate,
    PhysicsCollisionReportArgs, PhysicsCommands, PhysicsInspectArgs, PhysicsOverlayArgs,
    SESSION_SCRIPT_WAIT_BUSY_UNKNOWN_BACKOFF_MS, ScreenshotCommands, SessionCommands,
    SessionTimeScaleArgs, SurfaceAuditArgs, SurfaceCommands, TraceCommands, TraceRecordArgs,
    append_config_warnings, audio_analyze_result, binary_surface_has_incompatible_duplicates,
    binary_surface_mismatch_failure, binary_surface_warnings, check_script_command_string,
    coalesced_reload_count, command_path, discover_binary_command_surface,
    doctor_binary_surface_report_for_candidates, event_summarize_params, event_trace_params,
    external_harness_path, is_relevant_watch_path, physics_collision_report_params,
    physics_inspect_params, physics_overlay_params, public_command_surface, screenshot_result,
    script_diagnostic_excerpt, session_frame_pacing_matches, session_frame_pacing_request,
    session_script_failure_is_retryable, session_script_frame_pacing_request,
    session_script_project_args, session_script_step_op, session_script_wait_busy_backoff_ms,
    session_time_scale_params, snapshot_filter_params, surface_audit_params,
    surface_audit_provider_methods, trace_record_params,
};
use clap::Parser;
use godot_qa_core::config::ProjectConfig;
use godot_qa_core::results::{CommandResult, FailureDetail};

#[test]
fn presented_startup_channel_tolerance_defaults_exact_and_is_scoped() {
    let base = [
        "godot-qa",
        "screenshot",
        "assert-startup-presented",
        "--scene",
        "res://main.tscn",
    ];
    let parsed = Cli::try_parse_from(base).unwrap();
    let Commands::Screenshot(screenshot) = parsed.command else {
        panic!("expected screenshot");
    };
    let ScreenshotCommands::AssertStartupPresented(args) = screenshot.command else {
        panic!("expected presented startup");
    };
    assert_eq!(args.channel_tolerance, 0);

    let parsed = Cli::try_parse_from([
        "godot-qa",
        "screenshot",
        "assert-startup-presented",
        "--scene",
        "res://main.tscn",
        "--channel-tolerance",
        "1",
    ])
    .unwrap();
    let Commands::Screenshot(screenshot) = parsed.command else {
        panic!("expected screenshot");
    };
    let ScreenshotCommands::AssertStartupPresented(args) = screenshot.command else {
        panic!("expected presented startup");
    };
    assert_eq!(args.channel_tolerance, 1);
    assert!(
        Cli::try_parse_from([
            "godot-qa",
            "screenshot",
            "assert-startup-presented",
            "--scene",
            "res://main.tscn",
            "--channel-tolerance",
            "256"
        ])
        .is_err()
    );
    assert!(
        Cli::try_parse_from([
            "godot-qa",
            "screenshot",
            "compare",
            "--actual",
            "a.png",
            "--baseline",
            "b.png",
            "--channel-tolerance",
            "1"
        ])
        .is_err()
    );
}
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

fn sample_command_result(warnings: Option<serde_json::Value>) -> CommandResult {
    let mut data = Map::new();
    data.insert("mode".to_string(), json!("tests"));
    if let Some(warnings) = warnings {
        data.insert("warnings".to_string(), warnings);
    }
    CommandResult {
        ok: true,
        command: "scenario run".to_string(),
        message: "scenario run passed".to_string(),
        data,
        failures: Vec::new(),
        exit_code: 0,
    }
}

fn unsupported_config_warning(path: &str) -> serde_json::Value {
    json!({
        "type": "unsupported_config_file",
        "severity": "warning",
        "path": path,
        "expected": "qa/config.yaml (or qa/config.local.yaml)",
        "remediation": "rename the file to qa/config.yaml",
    })
}

#[test]
fn append_config_warnings_creates_warnings_array_when_absent() {
    let mut config = test_project_config(temp_dir("append-config-warn-absent"));
    config.config_warnings = vec![unsupported_config_warning("qa/config.json")];
    let _ = fs::remove_dir_all(&config.project_root);

    let result = append_config_warnings(sample_command_result(None), &config);

    // The warning is surfaced without touching the verdict axes.
    assert!(result.ok);
    assert_eq!(result.exit_code, 0);
    assert!(result.failures.is_empty());
    let warnings = result.data["warnings"].as_array().unwrap();
    assert_eq!(warnings.len(), 1);
    assert_eq!(warnings[0]["type"], "unsupported_config_file");
    assert_eq!(warnings[0]["path"], "qa/config.json");
}

#[test]
fn append_config_warnings_extends_existing_warnings_array() {
    let mut config = test_project_config(temp_dir("append-config-warn-extend"));
    config.config_warnings = vec![unsupported_config_warning("qa/config.yml")];
    let _ = fs::remove_dir_all(&config.project_root);

    let existing = json!([{"type": "some_other_warning", "severity": "warning"}]);
    let result = append_config_warnings(sample_command_result(Some(existing)), &config);

    let warnings = result.data["warnings"].as_array().unwrap();
    assert_eq!(warnings.len(), 2);
    assert_eq!(warnings[0]["type"], "some_other_warning");
    assert_eq!(warnings[1]["type"], "unsupported_config_file");
    assert_eq!(warnings[1]["path"], "qa/config.yml");
}

#[test]
fn append_config_warnings_is_noop_without_config_warnings() {
    let config = test_project_config(temp_dir("append-config-warn-none"));
    let _ = fs::remove_dir_all(&config.project_root);

    let result = append_config_warnings(sample_command_result(None), &config);

    // No config warnings means no `warnings` key is invented.
    assert!(!result.data.contains_key("warnings"));
    assert!(result.ok);
    assert_eq!(result.exit_code, 0);
}

fn save_test_png(path: &Path, size: (u32, u32), color: [u8; 4]) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    let image = ImageBuffer::<Rgba<u8>, Vec<u8>>::from_pixel(size.0, size.1, Rgba(color));
    image.save(path).unwrap();
}

#[test]
fn audio_cli_analyze_parses_window_options() {
    let cli = Cli::parse_from([
        "godot-qa",
        "audio",
        "analyze",
        "clip.wav",
        "--window-seconds",
        "0.25",
        "--hop-seconds",
        "0.125",
        "--strict-warnings",
        "--json",
    ]);

    assert!(cli.json);
    assert_eq!(command_path(&cli.command), "audio analyze");
    let Commands::Audio(args) = cli.command else {
        panic!("expected audio command");
    };
    let AudioCommands::Analyze(args) = args.command else {
        panic!("expected audio analyze command");
    };
    assert_eq!(args.wav, PathBuf::from("clip.wav"));
    assert_eq!(args.window_seconds, Some(0.25));
    assert_eq!(args.hop_seconds, Some(0.125));
    assert!(args.strict_warnings);
}

#[test]
fn audio_cli_capture_parses_live_contract() {
    let cli = Cli::parse_from([
        "godot-qa",
        "--json",
        "audio",
        "capture",
        "--live",
        "--seconds",
        "3",
        "--max-seconds",
        "10",
        "--bus",
        "Master",
        "--name",
        "live_demo",
        "--out",
        ".qa/artifacts/manual/live_demo.wav",
        "--analyze",
        "--strict-warnings",
        "--stop-after-state",
        r#"{"target":"qa:race","source":"method","method":"get_qa_race_state","matches":[{"path":"done","operator":"equals","expected":true}],"sample_every_frames":2}"#,
    ]);

    assert!(cli.json);
    assert_eq!(command_path(&cli.command), "audio capture");
    let Commands::Audio(args) = cli.command else {
        panic!("expected audio command");
    };
    let AudioCommands::Capture(args) = args.command else {
        panic!("expected audio capture command");
    };
    assert!(args.live);
    assert_eq!(args.seconds, 3.0);
    assert_eq!(args.max_seconds, 10.0);
    assert_eq!(args.bus.as_deref(), Some("Master"));
    assert_eq!(args.name, "live_demo");
    assert_eq!(
        args.out,
        Some(PathBuf::from(".qa/artifacts/manual/live_demo.wav"))
    );
    assert!(args.analyze);
    assert!(args.strict_warnings);
    assert!(
        args.stop_after_state
            .as_deref()
            .is_some_and(|raw| raw.contains("get_qa_race_state"))
    );
}

#[test]
fn screenshot_sequence_cli_contract() {
    let project_root = temp_dir("screenshot-sequence-cli-contract");
    let scenario_dir = project_root.join("qa/scenarios/examples");
    fs::create_dir_all(&scenario_dir).unwrap();
    fs::write(
        scenario_dir.join("race_start.yaml"),
        "name: race_start\nscene: res://ui/fixtures/race_start.tscn\nsteps: []\n",
    )
    .unwrap();
    let project = project_root.to_string_lossy().into_owned();
    let sequence = Cli::parse_from([
        "godot-qa",
        "--project",
        &project,
        "--json",
        "screenshot",
        "sequence",
        "--frames",
        "300",
        "--every",
        "15",
        "--out",
        ".qa/artifacts/manual/race_sequence",
        "--scenario",
        "race_start",
        "--allow-low-information",
        "--visual-risk-strict",
        "--contact-sheet",
        "--contact-sheet-columns",
        "5",
        "--contact-sheet-thumb-width",
        "320",
        "--contact-sheet-out",
        ".qa/artifacts/manual/race_contact_sheet.png",
    ]);
    assert!(sequence.json);
    assert_eq!(command_path(&sequence.command), "screenshot sequence");
    let Commands::Screenshot(args) = &sequence.command else {
        panic!("expected screenshot command");
    };
    let ScreenshotCommands::Sequence(args) = &args.command else {
        panic!("expected screenshot sequence command");
    };
    assert_eq!(args.frames, Some(300));
    assert_eq!(args.every, Some(15));
    assert_eq!(args.out, ".qa/artifacts/manual/race_sequence");
    assert_eq!(args.scenario.as_deref(), Some("race_start"));
    assert!(args.allow_low_information);
    assert!(args.visual_risk_strict);

    let result = screenshot_result(
        &sequence,
        match &sequence.command {
            Commands::Screenshot(args) => args,
            _ => unreachable!(),
        },
    );
    assert_eq!(result.command, "screenshot sequence");
    assert!(!result.ok);
    assert_eq!(result.failures[0].kind, "session_not_found");
    assert_eq!(result.data["frame_count_requested"], json!(300));
    assert_eq!(result.data["capture_every_frames"], json!(15));
    assert_eq!(result.data["captured_count"], json!(0));
    assert!(result.data["artifacts"]["manifest"].as_str().is_some());

    let seconds = Cli::parse_from([
        "godot-qa",
        "--project",
        &project,
        "screenshot",
        "sequence",
        "--seconds",
        "20",
        "--fps",
        "2",
        "--out",
        ".qa/artifacts/manual/review_sequence",
    ]);
    assert_eq!(command_path(&seconds.command), "screenshot sequence");

    let invalid = Cli::parse_from([
        "godot-qa",
        "--project",
        &project,
        "screenshot",
        "sequence",
        "--frames",
        "120",
        "--seconds",
        "2",
        "--out",
        ".qa/artifacts/manual/bad_sequence",
    ]);
    let invalid_result = screenshot_result(
        &invalid,
        match &invalid.command {
            Commands::Screenshot(args) => args,
            _ => unreachable!(),
        },
    );
    assert_eq!(invalid_result.command, "screenshot sequence");
    assert_eq!(invalid_result.exit_code, 1);
    assert_eq!(
        invalid_result.failures[0].kind,
        "screenshot_sequence_args_invalid"
    );
    assert_eq!(
        invalid_result.failures[0].command.as_deref(),
        Some("screenshot sequence")
    );

    let contact_sheet = Cli::parse_from([
        "godot-qa",
        "--project",
        &project,
        "screenshot",
        "contact-sheet",
        ".qa/artifacts/manual/race_sequence",
        "--columns",
        "5",
        "--thumb-width",
        "320",
        "--out",
        ".qa/artifacts/manual/race_contact_sheet.png",
    ]);
    assert_eq!(
        command_path(&contact_sheet.command),
        "screenshot contact-sheet"
    );
    let contact_result = screenshot_result(
        &contact_sheet,
        match &contact_sheet.command {
            Commands::Screenshot(args) => args,
            _ => unreachable!(),
        },
    );
    assert_eq!(contact_result.command, "screenshot contact-sheet");
    assert!(!contact_result.ok);
    assert_eq!(
        contact_result.failures[0].kind,
        "screenshot_contact_sheet_manifest_missing"
    );

    let capture = Cli::parse_from([
        "godot-qa",
        "screenshot",
        "capture",
        "--out",
        ".qa/artifacts/manual/current.png",
    ]);
    assert_eq!(command_path(&capture.command), "screenshot capture");
    let compare = Cli::parse_from([
        "godot-qa",
        "screenshot",
        "compare",
        "--actual",
        ".qa/artifacts/manual/current.png",
        "--baseline",
        "qa/baselines/current.png",
    ]);
    assert_eq!(command_path(&compare.command), "screenshot compare");
}

#[test]
fn screenshot_sequence_stop_after_state_parses_and_rejects_bad_json() {
    let valid = Cli::parse_from([
        "godot-qa",
        "screenshot",
        "sequence",
        "--frames",
        "300",
        "--every",
        "15",
        "--out",
        ".qa/artifacts/manual/stop_sequence",
        "--stop-after-state",
        r#"{"target":"qa:race","source":"method","method":"get_qa_race_state","matches":[{"path":"done","operator":"equals","expected":true}]}"#,
    ]);
    let Commands::Screenshot(args) = &valid.command else {
        panic!("expected screenshot command");
    };
    let ScreenshotCommands::Sequence(sequence_args) = &args.command else {
        panic!("expected screenshot sequence command");
    };
    assert!(
        sequence_args
            .stop_after_state
            .as_deref()
            .is_some_and(|raw| raw.contains("get_qa_race_state"))
    );

    let invalid = Cli::parse_from([
        "godot-qa",
        "screenshot",
        "sequence",
        "--frames",
        "300",
        "--every",
        "15",
        "--out",
        ".qa/artifacts/manual/stop_sequence",
        "--stop-after-state",
        "{not valid json",
    ]);
    let result = screenshot_result(
        &invalid,
        match &invalid.command {
            Commands::Screenshot(args) => args,
            _ => unreachable!(),
        },
    );
    assert!(!result.ok);
    assert_eq!(result.command, "screenshot sequence");
    assert_eq!(result.failures[0].kind, "invalid_params");
    assert_eq!(
        result.failures[0].command.as_deref(),
        Some("screenshot sequence")
    );
}

#[test]
fn screenshot_capture_setup_script_failure_runs_before_capture() {
    let project_root = temp_dir("screenshot-capture-setup-script");
    let script = project_root.join("qa/review-scripts/broken.yaml");
    fs::create_dir_all(script.parent().unwrap()).unwrap();
    fs::write(&script, "name: broken\n").unwrap();
    let project = project_root.to_string_lossy().into_owned();
    let cli = Cli::parse_from([
        "godot-qa",
        "--project",
        &project,
        "screenshot",
        "capture",
        "--out",
        ".qa/artifacts/manual/current.png",
        "--setup-script",
        "qa/review-scripts/broken.yaml",
    ]);

    let result = screenshot_result(
        &cli,
        match &cli.command {
            Commands::Screenshot(args) => args,
            _ => unreachable!(),
        },
    );

    assert!(!result.ok);
    assert_eq!(result.command, "screenshot capture");
    assert_eq!(
        result.data["setup_script"],
        json!("qa/review-scripts/broken.yaml")
    );
    assert_eq!(
        result.data["setup_result"]["command"],
        json!("session run-script")
    );

    let _ = fs::remove_dir_all(project_root);
}

#[test]
fn screenshot_contact_sheet_cli_contract() {
    let project_root = temp_dir("screenshot-contact-sheet-cli-contract");
    let project = project_root.to_string_lossy().into_owned();
    let sequence_dir = project_root.join(".qa/artifacts/manual/race_sequence");
    save_test_png(
        &sequence_dir.join("frames/frame_0010.png"),
        (20, 10),
        [0, 255, 0, 255],
    );
    save_test_png(
        &sequence_dir.join("frames/frame_0000.png"),
        (10, 20),
        [255, 0, 0, 255],
    );
    fs::write(
            sequence_dir.join("manifest.json"),
            serde_json::to_vec_pretty(&json!({
                "command": "screenshot.sequence",
                "captured_count": 2,
                "frames": [
                    {"index": 1, "engine_frame": 10, "time_msec": 100, "path": "frames/frame_0010.png", "width": 20, "height": 10},
                    {"index": 0, "engine_frame": 0, "time_msec": 0, "path": "frames/frame_0000.png", "width": 10, "height": 20}
                ]
            }))
            .unwrap(),
        )
        .unwrap();

    let cli = Cli::parse_from([
        "godot-qa",
        "--project",
        &project,
        "--json",
        "screenshot",
        "contact-sheet",
        ".qa/artifacts/manual/race_sequence",
        "--columns",
        "2",
        "--thumb-width",
        "40",
        "--out",
        ".qa/artifacts/manual/race_contact_sheet.png",
    ]);
    let result = screenshot_result(
        &cli,
        match &cli.command {
            Commands::Screenshot(args) => args,
            _ => unreachable!(),
        },
    );

    assert!(result.ok, "{:?}", result.failures);
    assert_eq!(result.command, "screenshot contact-sheet");
    assert_eq!(
        result.data["artifacts"]["contact_sheet"],
        json!(
            project_root
                .join(".qa/artifacts/manual/race_contact_sheet.png")
                .to_string_lossy()
                .into_owned()
        )
    );
    assert_eq!(
        result.data["artifacts"]["metadata"],
        json!(
            project_root
                .join(".qa/artifacts/manual/race_contact_sheet.json")
                .to_string_lossy()
                .into_owned()
        )
    );

    let metadata: serde_json::Value = serde_json::from_slice(
        &fs::read(project_root.join(".qa/artifacts/manual/race_contact_sheet.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(
        metadata["frames"][0]["manifest_path"],
        json!("frames/frame_0010.png")
    );
    assert_eq!(
        metadata["frames"][1]["manifest_path"],
        json!("frames/frame_0000.png")
    );
}

#[test]
fn audio_cli_capture_rejects_non_live_and_unbounded_duration() {
    let config = test_project_config(PathBuf::from("/tmp/project"));
    let args = super::AudioCaptureArgs {
        live: false,
        setup_script: None,
        seconds: 3.0,
        max_seconds: 60.0,
        bus: None,
        name: "clip".to_string(),
        out: None,
        analyze: false,
        strict_warnings: false,
        stop_after_state: None,
    };
    let failure = super::audio_capture_request(&config, &args).unwrap_err();
    assert_eq!(failure.kind, "invalid_params");

    let args = super::AudioCaptureArgs {
        live: true,
        setup_script: None,
        seconds: 61.0,
        max_seconds: 60.0,
        bus: None,
        name: "clip".to_string(),
        out: None,
        analyze: false,
        strict_warnings: false,
        stop_after_state: None,
    };
    let failure = super::audio_capture_request(&config, &args).unwrap_err();
    assert_eq!(failure.kind, "audio_capture.duration_too_long");
}

#[test]
fn audio_cli_export_validates_json_args_and_safe_output() {
    let config = test_project_config(PathBuf::from("/tmp/project"));
    let args = super::AudioExportArgs {
        target: "qa:audio_controller".to_string(),
        method: "qa_export_audio_track".to_string(),
        args: Some("[\"deterministic_transients\"]".to_string()),
        name: "exported".to_string(),
        out: Some(PathBuf::from(".qa/artifacts/manual/exported.wav")),
        analyze: true,
        strict_warnings: true,
    };
    let request = super::audio_export_request(&config, &args).unwrap();
    assert_eq!(request.args, json!(["deterministic_transients"]));
    assert_eq!(
        request.out,
        Some(PathBuf::from(
            "/tmp/project/.qa/artifacts/manual/exported.wav"
        ))
    );

    let bad_args = super::AudioExportArgs {
        args: Some("{\"not\":\"array\"}".to_string()),
        ..args
    };
    let failure = super::audio_export_request(&config, &bad_args).unwrap_err();
    assert_eq!(failure.kind, "invalid_params");
}

fn render_args(method: &str, seconds: f64) -> super::AudioRenderArgs {
    super::AudioRenderArgs {
        target: "qa:audio_controller".to_string(),
        method: method.to_string(),
        seconds,
        max_seconds: 60.0,
        mix_rate: 44_100,
        args: Some("[]".to_string()),
        name: "rendered".to_string(),
        out: Some(PathBuf::from(".qa/artifacts/manual/rendered.wav")),
        analyze: false,
        strict_warnings: false,
    }
}

#[test]
fn audio_cli_render_validates_method_duration_and_output() {
    let config = test_project_config(PathBuf::from("/tmp/project"));

    let request = super::audio_render_request(&config, &render_args("qa_fill_audio", 8.0)).unwrap();
    assert_eq!(request.seconds, 8.0);
    assert_eq!(request.mix_rate, 44_100);
    assert_eq!(
        request.out,
        Some(PathBuf::from(
            "/tmp/project/.qa/artifacts/manual/rendered.wav"
        ))
    );

    // Only project-owned qa_ methods are allowed.
    let bad_method = super::audio_render_request(&config, &render_args("fill_audio", 8.0));
    assert_eq!(bad_method.unwrap_err().kind, "invalid_params");

    // Duration must be positive and finite.
    let bad_duration = super::audio_render_request(&config, &render_args("qa_fill_audio", 0.0));
    assert_eq!(bad_duration.unwrap_err().kind, "invalid_params");
}

#[test]
fn audio_cli_analyze_writes_spectrogram_png() {
    let dir = temp_dir("audio-spectrogram");
    let wav = dir.join("clip.wav");
    let samples: Vec<f64> = (0..8_000)
        .map(|i| (2.0 * std::f64::consts::PI * 440.0 * i as f64 / 8_000.0).sin() * 0.5)
        .collect();
    fs::write(&wav, wav_pcm16_for_audio_cli(8_000, &samples)).unwrap();
    let png = dir.join("clip.png");

    let args = super::AudioAnalyzeArgs {
        wav: wav.clone(),
        window_seconds: None,
        hop_seconds: None,
        spectrogram: Some(png.clone()),
        strict_warnings: false,
    };
    let result = audio_analyze_result(&args);

    assert!(result.ok, "{:?}", result.failures);
    assert_eq!(result.data["artifacts"]["spectrogram"], json!(png));
    let bytes = fs::read(&png).unwrap();
    assert_eq!(
        &bytes[0..8],
        &[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A]
    );

    // A non-.png spectrogram path is rejected without aborting the analysis.
    let bad = super::AudioAnalyzeArgs {
        wav,
        window_seconds: None,
        hop_seconds: None,
        spectrogram: Some(dir.join("clip.txt")),
        strict_warnings: false,
    };
    let bad_result = audio_analyze_result(&bad);
    assert!(!bad_result.ok);
    assert_eq!(bad_result.failures[0].kind, "invalid_params");
}

#[test]
fn audio_cli_analyze_outputs_json_envelope_without_live_session() {
    let dir = temp_dir("audio-analyze");
    let wav = dir.join("clip.wav");
    fs::write(&wav, wav_pcm16_for_audio_cli(8_000, &[0.0, 0.2, -0.2, 0.0])).unwrap();
    let args = super::AudioAnalyzeArgs {
        wav: wav.clone(),
        window_seconds: Some(0.01),
        hop_seconds: Some(0.01),
        spectrogram: None,
        strict_warnings: false,
    };

    let result = audio_analyze_result(&args);

    assert!(result.ok, "{:?}", result.failures);
    assert_eq!(result.command, "audio analyze");
    assert_eq!(result.exit_code, 0);
    assert_eq!(result.data["artifacts"]["wav"], json!(wav));
    assert_eq!(result.data["analysis"]["sample_rate"], 8_000);
    assert!(result.data["analysis"].get("samples").is_none());
    assert!(result.data["analysis"]["peak_by_window"].is_array());
    assert!(result.failures.is_empty());
}

#[test]
fn audio_cli_analyze_invalid_wav_fails_with_stable_type() {
    let dir = temp_dir("audio-invalid");
    let wav = dir.join("invalid.wav");
    fs::write(&wav, b"not a wav").unwrap();
    let args = super::AudioAnalyzeArgs {
        wav: wav.clone(),
        window_seconds: None,
        hop_seconds: None,
        spectrogram: None,
        strict_warnings: false,
    };

    let result = audio_analyze_result(&args);

    assert!(!result.ok);
    assert_eq!(result.exit_code, 1);
    assert_eq!(result.failures[0].kind, "audio_analysis.invalid_wav");
    assert_eq!(
        result.failures[0].path.as_deref(),
        Some(wav.to_str().unwrap())
    );
}

#[test]
fn audio_cli_analyze_strict_warnings_promotes_warning_facts() {
    let dir = temp_dir("audio-strict");
    let wav = dir.join("silent.wav");
    fs::write(&wav, wav_pcm16_for_audio_cli(8_000, &vec![0.0; 800])).unwrap();
    let args = super::AudioAnalyzeArgs {
        wav,
        window_seconds: None,
        hop_seconds: None,
        spectrogram: None,
        strict_warnings: true,
    };

    let result = audio_analyze_result(&args);

    assert!(!result.ok);
    assert_eq!(result.exit_code, 1);
    assert_eq!(result.failures[0].kind, "audio_analysis.near_silent");
    assert_eq!(
        result.data["warnings"][0]["id"],
        json!("audio_analysis.near_silent")
    );
}

fn wav_pcm16_for_audio_cli(sample_rate: u32, mono_samples: &[f64]) -> Vec<u8> {
    let mut data = Vec::new();
    for sample in mono_samples {
        let clamped = sample.clamp(-1.0, 1.0);
        let value = if clamped >= 1.0 {
            i16::MAX
        } else if clamped <= -1.0 {
            i16::MIN
        } else {
            (clamped * f64::from(i16::MAX)) as i16
        };
        data.extend_from_slice(&value.to_le_bytes());
    }

    let block_align = 2u16;
    let byte_rate = sample_rate * u32::from(block_align);
    let riff_size = 4 + 8 + 16 + 8 + data.len() as u32;
    let mut bytes = Vec::new();
    bytes.extend_from_slice(b"RIFF");
    bytes.extend_from_slice(&riff_size.to_le_bytes());
    bytes.extend_from_slice(b"WAVE");
    bytes.extend_from_slice(b"fmt ");
    bytes.extend_from_slice(&16u32.to_le_bytes());
    bytes.extend_from_slice(&1u16.to_le_bytes());
    bytes.extend_from_slice(&1u16.to_le_bytes());
    bytes.extend_from_slice(&sample_rate.to_le_bytes());
    bytes.extend_from_slice(&byte_rate.to_le_bytes());
    bytes.extend_from_slice(&block_align.to_le_bytes());
    bytes.extend_from_slice(&16u16.to_le_bytes());
    bytes.extend_from_slice(b"data");
    bytes.extend_from_slice(&(data.len() as u32).to_le_bytes());
    bytes.extend_from_slice(&data);
    bytes
}

fn repo_contract_surface_audit_docs() {
    let cli_spec = include_str!("../../../docs/cli-spec.md");
    let protocol = include_str!("../../../docs/runtime-bridge-protocol.md");
    let scenario_format = include_str!("../../../docs/scenario-format.md");
    let artifact_format = include_str!("../../../docs/artifact-format.md");
    let test_strategy = include_str!("../../../docs/test-strategy.md");
    let sample_readme = include_str!("../../../examples/sample_project/README.md");

    for text in [
        cli_spec,
        protocol,
        artifact_format,
        test_strategy,
        sample_readme,
    ] {
        assert!(text.contains("surface audit"));
        assert!(text.contains("qa:surface_provider"));
    }

    for syntax in [
        "godot-qa surface audit qa:surface_provider",
        "--center qa:clear_spawn",
        "--region-size 320x240",
        "--sample-step 16",
        "--actor-radius 24",
        "--min-spawn-clearance 72",
        "--route qa:route_start,qa:route_mid,qa:route_finish",
        "--min-corridor-width 72",
        "--left-region qa:left_lane",
        "--right-region qa:right_lane",
        "--mirror-axis qa:centerline",
        "--strict-warnings",
    ] {
        assert!(cli_spec.contains(syntax), "missing CLI syntax: {syntax}");
    }

    for metric in [
        "sample_count",
        "walkable_count",
        "blocked_count",
        "soft_count",
        "unknown_count",
        "spawn_clearance_min_distance",
        "spawn_clearance_pass",
        "minimum_corridor_width",
        "corridor_width_pass",
        "hard_block_cluster_count",
        "soft_block_cluster_count",
        "dead_end_pocket_count",
        "largest_dead_end_area",
        "route_clearance_fail_points",
        "symmetry_delta_score",
        "left_right_passability_delta",
        "provider_missing_or_invalid",
    ] {
        assert!(cli_spec.contains(metric), "missing CLI metric: {metric}");
        assert!(
            artifact_format.contains(metric),
            "missing artifact metric: {metric}"
        );
    }

    for warning_id in [
        "surface_audit.provider_missing",
        "surface_audit.provider_invalid",
        "surface_audit.unknown_surface_label",
        "surface_audit.sample_limit_exceeded",
        "surface_audit.spawn_clearance_failed",
        "surface_audit.corridor_width_failed",
        "surface_audit.dead_end_pocket_detected",
        "surface_audit.hard_block_cluster_blocks_route",
        "surface_audit.soft_blocker_turns_corridor_risky",
        "surface_audit.symmetry_delta_high",
        "surface_audit.not_enough_data",
    ] {
        assert!(
            cli_spec.contains(warning_id),
            "missing CLI warning id: {warning_id}"
        );
    }

    for method in [
        "qa_surface_probe",
        "get_qa_surface_state",
        "get_surface_qa_state",
        "qa_probe_surface",
    ] {
        assert!(
            cli_spec.contains(method),
            "missing CLI provider method: {method}"
        );
        assert!(
            protocol.contains(method),
            "missing protocol provider method: {method}"
        );
        assert!(
            surface_audit_provider_methods().contains(&method),
            "provider method not accepted by parser: {method}"
        );
    }

    for artifact_text in [
        ".qa/artifacts/manual/surface_audit.json",
        ".qa/artifacts/manual/surface_corridor_heatmap.png",
        "surface_audit.json",
        "heatmap",
        "not baselines",
        "not auto-approved",
        "not runtime overlays",
    ] {
        let found = cli_spec.contains(artifact_text)
            || artifact_format.contains(artifact_text)
            || test_strategy.contains(artifact_text);
        assert!(found, "missing artifact wording: {artifact_text}");
    }

    for fixture in [
        "res://ui/fixtures/surface_audit_demo.tscn",
        "qa:clear_spawn",
        "qa:blocked_spawn",
        "qa:route_start",
        "qa:route_mid",
        "qa:route_finish",
        "qa:left_lane",
        "qa:right_lane",
        "qa:centerline",
        "clear spawn",
        "blocked spawn",
        "narrow corridor",
        "dead-end pocket",
        "soft blocker",
        "hard blocker",
        "mirrored-delta layout",
    ] {
        assert!(
            sample_readme.contains(fixture) || test_strategy.contains(fixture),
            "missing fixture anchor: {fixture}"
        );
    }

    for validator in [
        "cargo test -p godot-qa-cli repo_contract_surface_audit_docs",
        "cargo run --bin godot-qa -- scenario run surface_probe_demo --project examples/sample_project --json",
        "cargo run --bin godot-qa -- assert surface-matches qa:demo_actor --project examples/sample_project --snapshot after_move --json",
    ] {
        assert!(
            test_strategy.contains(validator) || sample_readme.contains(validator),
            "missing validator or regression command: {validator}"
        );
    }

    for boundary in [
        "does not infer meaning from labels, node names, colors, textures, sprites, collision layers, or navigation resources",
        "does not infer gameplay meaning from labels, node names, colors, textures, or collision layers",
        "Project semantics stay in the provider",
        "must not mutate scene state, navigation data, collision data, or project geometry",
        "must not draw or enable a debug overlay",
        "existing scenario assertions must remain unchanged",
    ] {
        let found = cli_spec.contains(boundary)
            || protocol.contains(boundary)
            || scenario_format.contains(boundary)
            || test_strategy.contains(boundary)
            || sample_readme.contains(boundary);
        assert!(found, "missing boundary wording: {boundary}");
    }

    assert!(public_command_surface().contains(&"surface audit"));
    assert_eq!(runtime::SURFACE_AUDIT_PROBE, "surface.audit_probe");
}

#[test]
fn s76_artifact_summary_docs_contract() {
    let cli_spec = include_str!("../../../docs/cli-spec.md");
    let artifact_format = include_str!("../../../docs/artifact-format.md");
    let test_strategy = include_str!("../../../docs/test-strategy.md");
    let readme = include_str!("../../../docs/workflows.md");

    for syntax in [
        "godot-qa artifact summarize <run-root>",
        "[--type <type>]",
        "[--warnings-only]",
        "[--expect <type>]",
        "[--markdown-out <path>]",
        "[--no-write]",
        "--json",
        "\"command\": \"artifact.summarize\"",
    ] {
        assert!(cli_spec.contains(syntax), "missing CLI syntax: {syntax}");
    }

    for category in [
        "scenario_results",
        "snapshots",
        "screenshots",
        "screenshot_sequences",
        "contact_sheets",
        "screenshot_diffs",
        "layout_audits",
        "traces",
        "physics_inspections",
        "physics_overlays",
        "collision_reports",
        "surface_audits",
        "audio_artifacts",
        "audio_analyses",
        "event_traces",
        "visual_audits",
        "profiles",
        "logs",
        "errors",
        "warnings",
        "unknown_artifacts",
    ] {
        assert!(
            cli_spec.contains(category),
            "missing CLI category: {category}"
        );
        assert!(
            artifact_format.contains(category),
            "missing artifact category: {category}"
        );
    }

    for warning_id in [
        "artifact_summary.run_root_missing",
        "artifact_summary.unrecognized_artifact",
        "artifact_summary.result_failure_detected",
        "artifact_summary.runtime_errors_detected",
        "artifact_summary.audio_warning_detected",
        "artifact_summary.visual_warning_detected",
        "artifact_summary.physics_warning_detected",
        "artifact_summary.event_warning_detected",
        "artifact_summary.missing_expected_artifact",
        "artifact_summary.large_artifact_skipped",
    ] {
        assert!(
            artifact_format.contains(warning_id),
            "missing artifact summary warning id: {warning_id}"
        );
    }

    for heading in [
        "# Artifact Summary",
        "## Run root",
        "## High-priority findings",
        "## Screenshots and visual artifacts",
        "## Runtime traces and physics",
        "## Audio",
        "## Events",
        "## Logs and errors",
        "## Unknown artifacts",
    ] {
        assert!(
            artifact_format.contains(heading),
            "missing Markdown heading: {heading}"
        );
    }

    for wording in [
        "artifact_summary.json",
        "before manually opening evidence folders",
        "before manually reading evidence folders",
        "does not embed base64 screenshots, audio, traces, or other binary payloads",
        "must not copy screenshots, audio, traces, contact sheets, overlays, diffs, or other binary payloads",
        "cargo test -p godot-qa-cli s76_artifact_summary_docs_contract",
        "cargo run --bin godot-qa -- artifact summarize .qa/artifacts/sample_main_menu_start/latest --json",
        "cargo run --bin godot-qa -- artifact summarize .qa/artifacts/bundles/sample_regression_gate/latest --expect scenario_results --json",
    ] {
        let found = cli_spec.contains(wording)
            || artifact_format.contains(wording)
            || test_strategy.contains(wording)
            || readme.contains(wording);
        assert!(found, "missing artifact summary wording: {wording}");
    }

    assert!(public_command_surface().contains(&"artifact summarize"));
}

#[test]
fn docs_visual_audit_contract() {
    let cli_spec = include_str!("../../../docs/cli-spec.md");
    let artifact_format = include_str!("../../../docs/artifact-format.md");
    let test_strategy = include_str!("../../../docs/test-strategy.md");
    let readme = include_str!("../../../docs/workflows.md");

    for syntax in [
        "godot-qa visual audit --project <project> --live",
        "--screenshot .qa/artifacts/manual/current.png",
        "--sequence .qa/artifacts/manual/race_sequence/manifest.json",
        "--scenario visual_audit_demo",
        "--targets .qa/artifacts/manual/current.targets.json",
        "--target qa:player",
        "--annotate-out .qa/artifacts/manual/visual_audit.png",
        "--min-pixel-area <value>",
        "--min-width <px>",
        "--min-height <px>",
        "--min-contrast-ratio <float>",
        "--min-color-distance <float>",
        "--strict-warnings",
    ] {
        assert!(
            cli_spec.contains(syntax),
            "missing visual audit syntax: {syntax}"
        );
    }

    for metric in [
        "target_id",
        "screenshot_path",
        "inside_viewport",
        "pixel_area",
        "bounding_box_width",
        "bounding_box_height",
        "visible_pixel_count",
        "estimated_local_background_color",
        "estimated_target_color_or_median",
        "relative_luminance_target",
        "relative_luminance_background",
        "contrast_ratio_estimate",
        "local_color_distance",
        "edge_contrast_estimate",
        "small_target_warning",
        "low_contrast_warning",
        "offscreen_warning",
        "occlusion_or_missing_pixels_warning",
        "frames_checked",
        "frames_with_target_missing",
        "minimum_pixel_area_across_frames",
        "minimum_contrast_across_frames",
        "worst_frame_path_by_rule",
        "thresholds",
        "min_edge_contrast",
    ] {
        assert!(cli_spec.contains(metric), "missing CLI metric: {metric}");
        assert!(
            artifact_format.contains(metric),
            "missing artifact metric: {metric}"
        );
    }

    for warning_id in [
        "visual_audit.target_missing",
        "visual_audit.target_offscreen",
        "visual_audit.target_too_small",
        "visual_audit.low_luminance_contrast",
        "visual_audit.low_color_distance",
        "visual_audit.low_edge_contrast",
        "visual_audit.target_bounds_unavailable",
        "visual_audit.sequence_target_missing_frames",
        "visual_audit.annotated_artifact_failed",
    ] {
        assert!(
            cli_spec.contains(warning_id),
            "missing CLI warning id: {warning_id}"
        );
        assert!(
            artifact_format.contains(warning_id),
            "missing artifact warning id: {warning_id}"
        );
    }

    for default_or_boundary in [
        "min_pixel_area: 256",
        "min_width: 16",
        "min_height: 16",
        "min_contrast_ratio: 3.0",
        "min_color_distance: 24.0",
        "min_edge_contrast: 0.08",
        "warning-only by default",
        "not an aesthetic pass/fail oracle",
        "non-aesthetic",
        "manual review",
        "does not update or approve screenshot baselines",
        "does not trigger fake target inference",
        "must not infer target bounds from arbitrary colors",
        "annotation PNG",
    ] {
        let found = cli_spec.contains(default_or_boundary)
            || artifact_format.contains(default_or_boundary)
            || test_strategy.contains(default_or_boundary)
            || readme.contains(default_or_boundary);
        assert!(
            found,
            "missing visual audit contract language: {default_or_boundary}"
        );
    }

    assert!(public_command_surface().contains(&"visual audit"));
}

#[test]
fn scenario_run_accepts_external_file_without_name() {
    let cli = Cli::parse_from([
        "godot-qa",
        "scenario",
        "run",
        "--file",
        ".qa/review-scenarios/s2.yaml",
        "--json",
    ]);
    assert_eq!(command_path(&cli.command), "scenario run");
    let Commands::Scenario(args) = cli.command else {
        panic!("expected scenario command");
    };
    let super::ScenarioCommands::Run(run) = args.command else {
        panic!("expected scenario run command");
    };
    assert_eq!(run.name, None);
    assert_eq!(run.file.as_deref(), Some(".qa/review-scenarios/s2.yaml"));
}

#[test]
fn scenario_run_audio_flags_resolve_to_override() {
    // --audio -> Some(true), --no-audio -> Some(false), neither -> None.
    let audio = Cli::try_parse_from(["godot-qa", "scenario", "run", "demo", "--audio"])
        .expect("scenario run --audio must parse");
    let Commands::Scenario(args) = audio.command else {
        panic!("expected scenario command");
    };
    let super::ScenarioCommands::Run(run) = args.command else {
        panic!("expected scenario run command");
    };
    assert!(run.audio);
    assert!(!run.no_audio);
    assert_eq!(
        crate::scenario::scenario_run_audio_override(&run),
        Some(true)
    );

    let no_audio = Cli::try_parse_from(["godot-qa", "scenario", "run", "demo", "--no-audio"])
        .expect("scenario run --no-audio must parse");
    let Commands::Scenario(args) = no_audio.command else {
        panic!("expected scenario command");
    };
    let super::ScenarioCommands::Run(run) = args.command else {
        panic!("expected scenario run command");
    };
    assert_eq!(
        crate::scenario::scenario_run_audio_override(&run),
        Some(false)
    );

    let plain = Cli::try_parse_from(["godot-qa", "scenario", "run", "demo"])
        .expect("scenario run must parse");
    let Commands::Scenario(args) = plain.command else {
        panic!("expected scenario command");
    };
    let super::ScenarioCommands::Run(run) = args.command else {
        panic!("expected scenario run command");
    };
    assert_eq!(crate::scenario::scenario_run_audio_override(&run), None);

    // The pair is mutually exclusive, mirroring --headless/--windowed.
    assert!(
        Cli::try_parse_from([
            "godot-qa",
            "scenario",
            "run",
            "demo",
            "--audio",
            "--no-audio"
        ])
        .is_err()
    );
}

#[test]
fn scenario_run_android_target_flags_parse() {
    let cli = Cli::try_parse_from([
        "godot-qa",
        "scenario",
        "run",
        "demo",
        "--target",
        "android",
        "--preset",
        "Android QA",
        "--device",
        "serial-1",
        "--package",
        "dev.qa.sample",
        "--no-apk-cache",
    ])
    .expect("scenario run --target android must parse");
    let Commands::Scenario(args) = cli.command else {
        panic!("expected scenario command");
    };
    let super::ScenarioCommands::Run(run) = args.command else {
        panic!("expected scenario run command");
    };
    assert_eq!(run.target, super::SessionTargetArg::Android);
    assert_eq!(run.preset.as_deref(), Some("Android QA"));
    assert_eq!(run.device.as_deref(), Some("serial-1"));
    assert_eq!(run.package_id.as_deref(), Some("dev.qa.sample"));
    assert!(run.no_apk_cache);
}

#[test]
fn scenario_run_defaults_to_desktop_target() {
    let cli = Cli::try_parse_from(["godot-qa", "scenario", "run", "demo"])
        .expect("scenario run must parse");
    let Commands::Scenario(args) = cli.command else {
        panic!("expected scenario command");
    };
    let super::ScenarioCommands::Run(run) = args.command else {
        panic!("expected scenario run command");
    };
    assert_eq!(run.target, super::SessionTargetArg::Desktop);
    assert!(run.preset.is_none());
    assert!(run.device.is_none());
    assert!(run.package_id.is_none());
    assert!(!run.no_apk_cache);
}

#[test]
fn session_start_no_apk_cache_flag_parses() {
    let with_flag = Cli::try_parse_from([
        "godot-qa",
        "session",
        "start",
        "--target",
        "android",
        "--preset",
        "Android QA",
        "--no-apk-cache",
    ])
    .expect("session start --no-apk-cache must parse");
    let Commands::Session(args) = with_flag.command else {
        panic!("expected session command");
    };
    let SessionCommands::Start(start) = args.command else {
        panic!("expected session start command");
    };
    assert!(start.no_apk_cache);

    let plain =
        Cli::try_parse_from(["godot-qa", "session", "start"]).expect("session start must parse");
    let Commands::Session(args) = plain.command else {
        panic!("expected session command");
    };
    let SessionCommands::Start(start) = args.command else {
        panic!("expected session start command");
    };
    assert!(!start.no_apk_cache, "the flag defaults to off");
}

#[test]
fn external_harness_path_rejects_missing_directory_and_unsupported_extension() {
    let root = temp_dir("external-harness");
    let config = ProjectConfig {
        project_root: root.clone(),
        repo_root: root.clone(),
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
    };

    let missing = external_harness_path(&config, "scenario run", ".qa/missing.yaml")
        .expect_err("missing file should fail");
    assert_eq!(missing.kind, "file_not_found");

    fs::create_dir_all(root.join(".qa")).unwrap();
    fs::write(root.join(".qa/script.txt"), "name: bad").unwrap();
    let unsupported = external_harness_path(&config, "session run-script", ".qa/script.txt")
        .expect_err("unsupported extension should fail");
    assert_eq!(unsupported.kind, "unsupported_file_type");

    fs::create_dir_all(root.join(".qa/dir.yaml")).unwrap();
    let directory = external_harness_path(&config, "session run-script", ".qa/dir.yaml")
        .expect_err("directory should fail");
    assert_eq!(directory.kind, "path_is_directory");
}

#[test]
fn check_tests_filter_flags_parse_contract() {
    let cli = Cli::parse_from([
        "godot-qa",
        "check",
        "--tests",
        "--test",
        "sample_script_test",
        "--test",
        "sample_scene_test",
    ]);
    let Commands::Check(args) = &cli.command else {
        panic!("expected check command");
    };
    assert!(args.tests);
    assert_eq!(
        args.test,
        vec![
            "sample_script_test".to_string(),
            "sample_scene_test".to_string()
        ]
    );

    let cli = Cli::parse_from([
        "godot-qa",
        "check",
        "--tests",
        "--tag",
        "smoke",
        "--tag",
        "ui",
        "--project",
        "examples/sample project",
        "--json",
    ]);
    let Commands::Check(args) = &cli.command else {
        panic!("expected check command");
    };
    assert_eq!(args.tag, vec!["smoke".to_string(), "ui".to_string()]);
    assert_eq!(
        super::check_tests_command_string(&cli, args),
        "godot-qa check --tests --tag smoke --tag ui --project 'examples/sample project' --json"
    );

    let cli = Cli::parse_from(["godot-qa", "check", "--tests", "--profile", "smoke"]);
    let Commands::Check(args) = &cli.command else {
        panic!("expected check command");
    };
    assert_eq!(
        super::check_tests_profile_args(args)
            .unwrap()
            .manifest_profile,
        Some("smoke".to_string())
    );

    let argv = vec![
        "godot-qa", "check", "--tests", "--test", "a", "--tag", "smoke",
    ];
    let error = Cli::try_parse_from(argv).unwrap_err();
    assert_eq!(error.kind(), clap::error::ErrorKind::ArgumentConflict);

    for argv in [
        vec!["godot-qa", "check", "--test", "a"],
        vec!["godot-qa", "check", "--tag", "smoke"],
        vec!["godot-qa", "check", "--profile", "smoke"],
    ] {
        let error = Cli::try_parse_from(argv).unwrap_err();
        assert_eq!(
            error.kind(),
            clap::error::ErrorKind::MissingRequiredArgument
        );
    }
}

#[test]
fn check_tests_profile_flag_parser_contract() {
    let cli = Cli::parse_from([
        "godot-qa",
        "check",
        "--tests",
        "--profile",
        "--test",
        "sample_script_test",
    ]);
    let Commands::Check(args) = &cli.command else {
        panic!("expected check command");
    };
    let parsed = super::check_tests_profile_args(args).unwrap();
    assert!(parsed.profile_enabled);
    assert_eq!(parsed.manifest_profile, None);
    assert_eq!(args.test, vec!["sample_script_test".to_string()]);
    assert_eq!(
        super::check_tests_command_string(&cli, args),
        "godot-qa check --tests --test sample_script_test --profile"
    );

    let cli = Cli::parse_from([
        "godot-qa",
        "check",
        "--tests",
        "--profile",
        "--tag",
        "smoke",
    ]);
    let Commands::Check(args) = &cli.command else {
        panic!("expected check command");
    };
    let parsed = super::check_tests_profile_args(args).unwrap();
    assert!(parsed.profile_enabled);
    assert_eq!(parsed.manifest_profile, None);
    assert_eq!(args.tag, vec!["smoke".to_string()]);

    let cli = Cli::parse_from([
        "godot-qa",
        "check",
        "--tests",
        "--profile",
        "--profile",
        "smoke",
    ]);
    let Commands::Check(args) = &cli.command else {
        panic!("expected check command");
    };
    let parsed = super::check_tests_profile_args(args).unwrap();
    assert!(parsed.profile_enabled);
    assert_eq!(parsed.manifest_profile, Some("smoke".to_string()));
    assert_eq!(
        super::check_tests_command_string(&cli, args),
        "godot-qa check --tests --profile --profile smoke"
    );

    let cli = Cli::parse_from(["godot-qa", "check", "--tests", "--profile", "smoke"]);
    let Commands::Check(args) = &cli.command else {
        panic!("expected check command");
    };
    let parsed = super::check_tests_profile_args(args).unwrap();
    assert!(!parsed.profile_enabled);
    assert_eq!(parsed.manifest_profile, Some("smoke".to_string()));

    for (argv, expected_kind) in [
        (
            vec![
                "godot-qa",
                "check",
                "--tests",
                "--profile",
                "--profile",
                "--test",
                "sample_script_test",
            ],
            "duplicate_profile_flag",
        ),
        (
            vec![
                "godot-qa",
                "check",
                "--tests",
                "--profile",
                "smoke",
                "--profile",
                "full",
            ],
            "duplicate_profile_selector",
        ),
        (
            vec![
                "godot-qa",
                "check",
                "--tests",
                "--test",
                "sample_script_test",
                "--profile",
                "smoke",
            ],
            "selection_conflict",
        ),
        (
            vec![
                "godot-qa",
                "check",
                "--tests",
                "--tag",
                "smoke",
                "--profile",
                "smoke",
            ],
            "selection_conflict",
        ),
    ] {
        let cli = Cli::parse_from(argv);
        let Commands::Check(args) = &cli.command else {
            panic!("expected check command");
        };
        let failure = super::check_tests_profile_args(args).unwrap_err();
        assert_eq!(failure.kind, expected_kind);
    }
}

#[test]
fn check_tests_batch_flag_parse_contract() {
    let cli = Cli::parse_from([
        "godot-qa", "check", "--tests", "--batch", "--tag", "smoke", "--json",
    ]);
    let Commands::Check(args) = &cli.command else {
        panic!("expected check command");
    };
    assert!(args.tests);
    assert!(args.batch);
    assert_eq!(
        super::check_tests_command_string(&cli, args),
        "godot-qa check --tests --batch --tag smoke --json"
    );

    let error = Cli::try_parse_from(["godot-qa", "check", "--batch"]).unwrap_err();
    assert_eq!(
        error.kind(),
        clap::error::ErrorKind::MissingRequiredArgument
    );

    for argv in [
        vec!["godot-qa", "check", "--all-scenes", "--batch"],
        vec!["godot-qa", "check", "--scene", "res://main.tscn", "--batch"],
    ] {
        let error = Cli::try_parse_from(argv).unwrap_err();
        assert_eq!(error.kind(), clap::error::ErrorKind::ArgumentConflict);
    }
}

#[test]
fn check_tests_timeout_flag_parse_contract() {
    let cli = Cli::parse_from([
        "godot-qa",
        "check",
        "--tests",
        "--test",
        "sample_script_test",
        "--timeout-seconds",
        "1.5",
        "--json",
    ]);
    let Commands::Check(args) = &cli.command else {
        panic!("expected check command");
    };
    assert_eq!(args.timeout_seconds, Some(1.5));
    assert_eq!(
        super::check_tests_command_string(&cli, args),
        "godot-qa check --tests --test sample_script_test --timeout-seconds 1.5 --json"
    );

    let cli = Cli::parse_from(["godot-qa", "check", "--timeout-seconds", "1"]);
    let Commands::Check(args) = &cli.command else {
        panic!("expected check command");
    };
    let failure = super::check_timeout_for_mode("check", args.timeout_seconds, false).unwrap_err();
    assert_eq!(failure.kind, "invalid_params");
}

#[test]
fn check_script_flags_parse_contract() {
    let cli = Cli::parse_from([
        "godot-qa",
        "check",
        "--script",
        "res://src/main.gd",
        "--timeout-seconds",
        "2.5",
        "--project",
        "examples/sample project",
        "--json",
    ]);
    let Commands::Check(args) = &cli.command else {
        panic!("expected check command");
    };
    assert_eq!(args.script.as_deref(), Some("res://src/main.gd"));
    assert!(!args.all_scripts);
    assert_eq!(args.timeout_seconds, Some(2.5));
    assert_eq!(
        check_script_command_string(&cli, args),
        "godot-qa check --script 'res://src/main.gd' --timeout-seconds 2.5 --project 'examples/sample project' --json"
    );

    let cli = Cli::parse_from(["godot-qa", "check", "--all-scripts", "--json"]);
    let Commands::Check(args) = &cli.command else {
        panic!("expected check command");
    };
    assert!(args.all_scripts);
    assert_eq!(
        check_script_command_string(&cli, args),
        "godot-qa check --all-scripts --json"
    );

    for argv in [
        vec![
            "godot-qa",
            "check",
            "--script",
            "res://main.gd",
            "--all-scripts",
        ],
        vec!["godot-qa", "check", "--script", "res://main.gd", "--tests"],
        vec![
            "godot-qa",
            "check",
            "--script",
            "res://main.gd",
            "--scene",
            "res://main.tscn",
        ],
        vec!["godot-qa", "check", "--all-scripts", "--all-scenes"],
    ] {
        let error = Cli::try_parse_from(argv).unwrap_err();
        assert_eq!(error.kind(), clap::error::ErrorKind::ArgumentConflict);
    }
}

#[test]
fn debug_host_sidecar_is_hidden_from_public_command_surface() {
    use clap::CommandFactory;

    // The internal `__debug-host` sidecar must never leak into the doctor command
    // surface, or the binary-surface parity tests would flag every real binary.
    assert!(
        !public_command_surface()
            .iter()
            .any(|leaf| leaf.contains("debug-host")),
        "public command surface must not include the hidden __debug-host sidecar"
    );

    // It must be defined but hidden from `--help` (so `discover_binary_command_surface`,
    // which walks help output, never sees it) ...
    let command = Cli::command();
    let sidecar = command
        .get_subcommands()
        .find(|sub| sub.get_name() == "__debug-host")
        .expect("__debug-host subcommand should be defined");
    assert!(
        sidecar.is_hide_set(),
        "__debug-host must stay hidden from --help"
    );

    // ... yet still parseable, proving it is hidden rather than removed.
    let cli = Cli::parse_from([
        "godot-qa",
        "__debug-host",
        "--record",
        "/tmp/debug_host.json",
    ]);
    assert!(matches!(cli.command, Commands::DebugHost(_)));
}

#[test]
fn doctor_surface_discovery_uses_help_without_running_commands() {
    let root = temp_dir("surface-discovery");
    let binary = godot_qa_test_support::fixture(
        &root.join("godot-qa"),
        &[
            (
                "response:--help",
                "Commands:\n  doctor\n  session\n  legacy\n",
            ),
            ("response:doctor --help", "Usage: godot-qa doctor\n"),
            ("response:legacy --help", "Usage: godot-qa legacy\n"),
            (
                "response:session --help",
                "Commands:\n  status\n  profile\n",
            ),
            (
                "response:session status --help",
                "Usage: godot-qa session status\n",
            ),
            (
                "response:session profile --help",
                "Usage: godot-qa session profile\n",
            ),
            ("exit", "2"),
        ],
    );

    let surface = discover_binary_command_surface(&binary).unwrap();

    fs::remove_dir_all(root).unwrap();
    assert!(surface.contains("doctor"));
    assert!(surface.contains("session status"));
    assert!(surface.contains("session profile"));
    assert!(surface.contains("legacy"));
}

#[test]
fn doctor_surface_report_includes_current_exe_path_order_and_warnings() {
    let root = temp_dir("surface-report");
    let first = godot_qa_test_support::fixture(
        &root.join("first/godot-qa"),
        &[
            ("version", "godot-qa 0.1.0 stale"),
            ("response:--help", "Commands:\n  doctor\n  session\n"),
            ("response:doctor --help", "Usage: godot-qa doctor\n"),
            ("response:session --help", "Commands:\n  status\n"),
            (
                "response:session status --help",
                "Usage: godot-qa session status\n",
            ),
            ("exit", "2"),
        ],
    );
    let second = godot_qa_test_support::fixture(
        &root.join("second/godot-qa"),
        &[
            ("version", "godot-qa 0.2.0 current"),
            ("response:--help", "Commands:\n  doctor\n  session\n"),
            ("response:doctor --help", "Usage: godot-qa doctor\n"),
            (
                "response:session --help",
                "Commands:\n  status\n  profile\n",
            ),
            (
                "response:session status --help",
                "Usage: godot-qa session status\n",
            ),
            (
                "response:session profile --help",
                "Usage: godot-qa session profile\n",
            ),
            ("exit", "2"),
        ],
    );
    let candidates = vec![
        PathCandidate {
            path_order: 0,
            raw_path: first.clone(),
            canonical_path: first.canonicalize().unwrap(),
        },
        PathCandidate {
            path_order: 1,
            raw_path: second.clone(),
            canonical_path: second.canonicalize().unwrap(),
        },
    ];
    let first_canonical = first.canonicalize().unwrap();

    let report = doctor_binary_surface_report_for_candidates(candidates);
    let warnings = binary_surface_warnings(&report);

    fs::remove_dir_all(root).unwrap();
    assert!(report["current_exe"].is_string() || report["current_exe"].is_null());
    assert_eq!(
        report["first_path_binary"].as_str().unwrap(),
        first_canonical.to_string_lossy().as_ref()
    );
    assert_eq!(report["candidates"].as_array().unwrap().len(), 2);
    assert_eq!(report["candidates"][0]["path_order"], 0);
    assert_eq!(report["candidates"][0]["is_first_on_path"], true);
    assert_eq!(
        report["candidates"][0]["version"]["version"],
        "godot-qa 0.1.0 stale"
    );
    assert!(
        report["candidates"][0]["missing_commands"]
            .as_array()
            .unwrap()
            .contains(&serde_json::json!("scenario run"))
    );
    assert!(!warnings.is_empty());
    assert_eq!(warnings[0]["type"], "multiple_godot_qa_binaries");
    assert!(
        warnings[0]["remediation"]
            .as_str()
            .unwrap()
            .contains("PATH")
    );
}

#[test]
fn doctor_strict_binary_surface_promotes_duplicate_mismatch_only() {
    let cli = Cli::parse_from(["godot-qa", "doctor", "--strict-binary-surface"]);
    let Commands::Doctor(args) = &cli.command else {
        panic!("expected doctor command");
    };
    assert!(args.strict_binary_surface);

    let report = serde_json::json!({
        "current_exe": "/tmp/current/godot-qa",
        "first_path_binary": "/tmp/stale/godot-qa",
        "expected_source": "rust_public_command_surface",
        "expected_commands": ["doctor", "scenario run"],
        "summary": {
            "candidate_count": 2,
            "incompatible_candidate_count": 1,
            "probe_error_count": 0,
        },
        "candidates": [
            {
                "path_order": 0,
                "raw_path": "/tmp/stale/godot-qa",
                "canonical_path": "/tmp/stale/godot-qa",
                "is_current_exe": false,
                "is_first_on_path": true,
                "version": {"version": "stale"},
                "command_count": 1,
                "missing_commands": ["scenario run"],
                "extra_commands": [],
                "probe_errors": [],
                "remediation": "remove stale PATH entry"
            },
            {
                "path_order": 1,
                "raw_path": "/tmp/current/godot-qa",
                "canonical_path": "/tmp/current/godot-qa",
                "is_current_exe": true,
                "is_first_on_path": false,
                "version": {"version": "current"},
                "command_count": 2,
                "missing_commands": [],
                "extra_commands": [],
                "probe_errors": [],
                "remediation": "matches"
            }
        ]
    });

    assert!(binary_surface_has_incompatible_duplicates(&report));
    let failure = binary_surface_mismatch_failure(&report);
    assert_eq!(failure.kind, "binary_surface_mismatch");
    assert_eq!(failure.command.as_deref(), Some("doctor"));
    assert_eq!(
        failure.actual.unwrap()["first_path_binary"],
        "/tmp/stale/godot-qa"
    );

    let single_candidate = serde_json::json!({
        "summary": {"candidate_count": 1},
        "candidates": [{
            "missing_commands": ["scenario run"],
            "extra_commands": [],
        }]
    });
    assert!(!binary_surface_has_incompatible_duplicates(
        &single_candidate
    ));
}

#[test]
fn doctor_strict_addon_version_flag_parses() {
    let cli = Cli::parse_from(["godot-qa", "doctor", "--strict-addon-version"]);
    let Commands::Doctor(args) = &cli.command else {
        panic!("expected doctor command");
    };
    assert!(args.strict_addon_version);
    assert!(!args.strict_binary_surface);

    let default = Cli::parse_from(["godot-qa", "doctor"]);
    let Commands::Doctor(default_args) = &default.command else {
        panic!("expected doctor command");
    };
    assert!(!default_args.strict_addon_version);
}

#[test]
fn addon_version_mismatch_reports_structured_warning_and_failure() {
    let addon_dir = std::path::Path::new("/tmp/project/addons/godot_qa");
    let warning = super::addon_version_mismatch_warning(addon_dir, "0.0.0", Some("0.1.0"), "0.1.0");
    assert_eq!(warning["type"], "addon_version_mismatch");
    assert_eq!(warning["severity"], "warning");
    assert_eq!(warning["addon_version"], "0.0.0");
    assert_eq!(warning["source_version"], "0.1.0");
    assert_eq!(warning["cli_version"], "0.1.0");

    let failure = super::addon_version_mismatch_failure(addon_dir, "0.0.0", "0.1.0");
    assert_eq!(failure.kind, "addon_version_mismatch");
    assert_eq!(failure.command.as_deref(), Some("doctor"));
    assert_eq!(failure.expected, Some(serde_json::json!("0.1.0")));
    assert_eq!(failure.actual, Some(serde_json::json!("0.0.0")));
}

#[test]
fn scenario_sweep_cli_parse_contract() {
    let cli = Cli::try_parse_from([
        "godot-qa",
        "scenario",
        "sweep",
        "sample_main_menu_start",
        "--seeds",
        "-1,2,3",
        "--repeat",
        "2",
        "--require-all-pass",
        "--headless",
        "--project-arg",
        "--qa-difficulty=hard",
    ])
    .expect("scenario sweep must parse");
    let Commands::Scenario(scenario) = &cli.command else {
        panic!("expected scenario command");
    };
    let super::ScenarioCommands::Sweep(args) = &scenario.command else {
        panic!("expected sweep subcommand");
    };
    assert_eq!(args.name.as_deref(), Some("sample_main_menu_start"));
    assert_eq!(args.seeds, vec![-1, 2, 3]);
    assert_eq!(args.repeat, 2);
    assert!(args.require_all_pass);
    assert!(args.headless);
    assert_eq!(args.project_args, vec!["--qa-difficulty=hard".to_string()]);

    let cli = Cli::try_parse_from([
        "godot-qa",
        "scenario",
        "sweep",
        "demo",
        "--seed-start",
        "10",
        "--seed-count",
        "5",
    ])
    .expect("seed range must parse");
    let Commands::Scenario(scenario) = &cli.command else {
        panic!("expected scenario command");
    };
    let super::ScenarioCommands::Sweep(args) = &scenario.command else {
        panic!("expected sweep subcommand");
    };
    assert_eq!(args.seed_start, Some(10));
    assert_eq!(args.seed_count, Some(5));

    // --seeds conflicts with the range selection, and --seed-start requires
    // --seed-count (and vice versa).
    for argv in [
        vec![
            "godot-qa",
            "scenario",
            "sweep",
            "demo",
            "--seeds",
            "1",
            "--seed-start",
            "2",
            "--seed-count",
            "3",
        ],
        vec!["godot-qa", "scenario", "sweep", "demo", "--seed-start", "2"],
        vec!["godot-qa", "scenario", "sweep", "demo", "--seed-count", "3"],
        vec![
            "godot-qa",
            "scenario",
            "sweep",
            "demo",
            "--file",
            "sweep.json",
        ],
    ] {
        let error = Cli::try_parse_from(argv).unwrap_err();
        assert!(
            matches!(
                error.kind(),
                clap::error::ErrorKind::ArgumentConflict
                    | clap::error::ErrorKind::MissingRequiredArgument
            ),
            "unexpected parse error kind: {:?}",
            error.kind()
        );
    }
}

#[test]
fn scenario_sweep_seed_selection_is_validated_before_preflight() {
    let missing = godot_qa_scenario::sweep::expand_sweep_seeds(&[], None, None, 1).unwrap_err();
    assert_eq!(missing.kind, "invalid_params");
    let duplicate =
        godot_qa_scenario::sweep::expand_sweep_seeds(&[7, 7], None, None, 1).unwrap_err();
    assert_eq!(duplicate.kind, "invalid_params");
    let expanded = godot_qa_scenario::sweep::expand_sweep_seeds(&[], Some(3), Some(2), 3).unwrap();
    assert_eq!(expanded, vec![3, 3, 3, 4, 4, 4]);
}

#[test]
fn telemetry_record_cli_parse_and_params_contract() {
    let cli = Cli::try_parse_from([
        "godot-qa",
        "telemetry",
        "record",
        "--target",
        "qa:hud",
        "--method",
        "qa_pacing_metrics",
        "--frames",
        "120",
        "--every",
        "4",
        "--out",
        ".qa/artifacts/manual/pacing.jsonl",
    ])
    .expect("telemetry record must parse");
    let Commands::Telemetry(telemetry) = &cli.command else {
        panic!("expected telemetry command");
    };
    let super::TelemetryCommands::Record(args) = &telemetry.command;
    let params = super::telemetry_record_params(args).expect("valid params");
    assert_eq!(params.get("target"), Some(&json!("qa:hud")));
    assert_eq!(params.get("method"), Some(&json!("qa_pacing_metrics")));
    assert_eq!(params.get("frames"), Some(&json!(120)));
    assert_eq!(params.get("every"), Some(&json!(4)));
    assert!(!params.contains_key("provider"));

    let cli = Cli::try_parse_from([
        "godot-qa",
        "telemetry",
        "record",
        "--provider",
        "difficulty",
    ])
    .expect("provider mode must parse");
    let Commands::Telemetry(telemetry) = &cli.command else {
        panic!("expected telemetry command");
    };
    let super::TelemetryCommands::Record(args) = &telemetry.command;
    let params = super::telemetry_record_params(args).expect("valid provider params");
    assert_eq!(params.get("provider"), Some(&json!("difficulty")));
    assert_eq!(params.get("frames"), Some(&json!(600)));
    assert_eq!(params.get("every"), Some(&json!(1)));

    // Provider and target/method are mutually exclusive; --target requires
    // --method.
    for argv in [
        vec![
            "godot-qa",
            "telemetry",
            "record",
            "--provider",
            "difficulty",
            "--target",
            "qa:hud",
            "--method",
            "qa_pacing_metrics",
        ],
        vec!["godot-qa", "telemetry", "record", "--target", "qa:hud"],
        vec![
            "godot-qa",
            "telemetry",
            "record",
            "--method",
            "qa_pacing_metrics",
        ],
    ] {
        assert!(Cli::try_parse_from(argv).is_err());
    }
}

#[test]
fn telemetry_record_params_reject_missing_source_bad_method_and_bounds() {
    let base = |mutate: fn(&mut super::TelemetryRecordArgs)| {
        let mut args = super::TelemetryRecordArgs {
            provider: None,
            target: Some("qa:hud".to_string()),
            method: Some("qa_pacing_metrics".to_string()),
            frames: 600,
            every: 1,
            out: None,
        };
        mutate(&mut args);
        args
    };

    let missing = super::telemetry_record_params(&base(|args| {
        args.target = None;
        args.method = None;
    }))
    .unwrap_err();
    assert_eq!(missing.kind, "invalid_params");

    for method in ["qa_set_score", "qa_apply_state", "free", "qa bad"] {
        let args = super::TelemetryRecordArgs {
            provider: None,
            target: Some("qa:hud".to_string()),
            method: Some(method.to_string()),
            frames: 600,
            every: 1,
            out: None,
        };
        let failure = super::telemetry_record_params(&args).unwrap_err();
        assert_eq!(
            failure.kind, "invalid_params",
            "method {method:?} must be rejected"
        );
    }

    let frames = super::telemetry_record_params(&base(|args| args.frames = 0)).unwrap_err();
    assert_eq!(frames.kind, "invalid_params");
    let every = super::telemetry_record_params(&base(|args| args.every = 300)).unwrap_err();
    assert_eq!(every.kind, "invalid_params");
    let provider = super::telemetry_record_params(&super::TelemetryRecordArgs {
        provider: Some("bad provider!".to_string()),
        target: None,
        method: None,
        frames: 600,
        every: 1,
        out: None,
    })
    .unwrap_err();
    assert_eq!(provider.kind, "invalid_params");
}
