use super::capture::{
    live_screenshot_capture_from_response, screenshot_low_information_result,
    validate_live_screenshot_quality,
};
use super::sequence::{
    screenshot_sequence_low_information_failure, screenshot_sequence_manifest,
    screenshot_sequence_plan, screenshot_sequence_quality_data, write_json_file,
};
use super::{
    ScreenshotSequenceOptions, ScreenshotStateGuards, analyze_screenshot_file,
    capture_live_screenshot, compare_presented_startup_files, compare_screenshot_files,
};
use crate::visual_risk::{self, ScreenshotArtifact, VisualRiskOptions};
use godot_qa_core::config::ProjectConfig;
use image::{ImageBuffer, Rgba};
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

fn save_png(path: &Path, size: (u32, u32), pixels: Vec<[u8; 4]>) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    let image = ImageBuffer::<Rgba<u8>, Vec<u8>>::from_vec(
        size.0,
        size.1,
        pixels
            .into_iter()
            .flat_map(|pixel| pixel.into_iter())
            .collect(),
    )
    .unwrap();
    image.save(path).unwrap();
}

fn test_config(root: &Path) -> ProjectConfig {
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
        godot_joypad_isolation: false,
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

pub(super) fn sequence_options() -> ScreenshotSequenceOptions {
    ScreenshotSequenceOptions {
        capture_mode: "serial".to_string(),
        max_bytes: None,
        max_seconds: None,
        frames: Some(30),
        every: Some(15),
        seconds: None,
        fps: None,
        out: ".qa/artifacts/manual/sequence".to_string(),
        scenario: Some("demo".to_string()),
        scenario_path: Some("qa/scenarios/examples/demo.yaml".to_string()),
        scenario_scene: Some("res://ui/demo.tscn".to_string()),
        state_guards: ScreenshotStateGuards::default(),
        stop_after_state: None,
        allow_low_information: false,
        visual_risk: false,
        visual_risk_strict: false,
        contact_sheet: false,
        contact_sheet_columns: None,
        contact_sheet_thumb_width: None,
        contact_sheet_out: None,
    }
}

#[test]
fn screenshot_sequence_manifest_shape_uses_relative_paths_and_dimensions() {
    let root = temp_dir("screenshot-sequence-manifest");
    let frame_path = root.join("sequence/frames/frame_0000.png");
    let mut pixels = vec![[10, 10, 10, 255]; 128 * 128];
    for pixel in pixels.iter_mut().take(2000) {
        *pixel = [240, 80, 40, 255];
    }
    save_png(&frame_path, (128, 128), pixels);
    let quality = analyze_screenshot_file(&frame_path).unwrap();
    let options = sequence_options();
    let plan = screenshot_sequence_plan("screenshot sequence", &options).unwrap();
    let frame = json!({
        "index": 0,
        "engine_frame": 12,
        "time_msec": 34,
        "path": "frames/frame_0000.png",
        "width": quality["width"],
        "height": quality["height"],
        "quality": screenshot_sequence_quality_data(&quality, &[]),
    });
    let manifest = screenshot_sequence_manifest(&options, &plan, vec![frame]);
    let manifest_path = root.join("sequence/manifest.json");
    write_json_file(&manifest_path, &manifest).unwrap();
    let saved: serde_json::Value =
        serde_json::from_slice(&fs::read(&manifest_path).unwrap()).unwrap();

    assert_eq!(saved["command"], json!("screenshot.sequence"));
    assert_eq!(saved["scenario"], json!("demo"));
    assert_eq!(saved["frame_count_requested"], json!(30));
    assert_eq!(saved["capture_every_frames"], json!(15));
    assert_eq!(saved["captured_count"], json!(1));
    assert_eq!(saved["frames"][0]["path"], json!("frames/frame_0000.png"));
    assert_eq!(saved["frames"][0]["width"], json!(128));
    assert_eq!(saved["frames"][0]["height"], json!(128));
    assert_eq!(
        saved["frames"][0]["quality"]["low_information"],
        json!(false)
    );
}

#[test]
fn screenshot_sequence_timing_plan_validates_frame_and_seconds_modes() {
    let mut options = sequence_options();
    let frame_plan = screenshot_sequence_plan("screenshot sequence", &options).unwrap();
    assert_eq!(frame_plan.capture_frames, vec![0, 15, 30]);

    options.frames = None;
    options.every = None;
    options.seconds = Some(2.0);
    options.fps = Some(2.0);
    let seconds_plan = screenshot_sequence_plan("screenshot sequence", &options).unwrap();
    assert_eq!(seconds_plan.mode, "seconds");
    assert_eq!(seconds_plan.frame_count_requested, 120);
    assert_eq!(seconds_plan.capture_every_frames, 30);
    assert_eq!(seconds_plan.capture_frames, vec![0, 30, 60, 90, 120]);

    options.fps = Some(120.0);
    let bounded = screenshot_sequence_plan("screenshot sequence", &options).unwrap();
    assert_eq!(bounded.capture_every_frames, 1);

    options.seconds = Some(20_000.0);
    let invalid = screenshot_sequence_plan("screenshot sequence", &options).unwrap_err();
    assert_eq!(invalid.failures[0].kind, "screenshot_sequence_args_invalid");
}

#[test]
fn screenshot_sequence_low_information_default_failure_and_allow_data() {
    let root = temp_dir("screenshot-sequence-low-information");
    let frame_path = root.join("sequence/frames/frame_0000.png");
    save_png(&frame_path, (128, 128), vec![[0, 0, 0, 255]; 128 * 128]);
    let quality = analyze_screenshot_file(&frame_path).unwrap();
    let failure = screenshot_sequence_low_information_failure(
        "screenshot sequence",
        0,
        0,
        &frame_path,
        &quality,
    );

    assert_eq!(quality["low_information"], json!(true));
    assert_eq!(failure.kind, "screenshot_low_information");
    assert_eq!(failure.details["index"], json!(0));

    let retained = screenshot_sequence_quality_data(&quality, &[]);
    assert_eq!(retained["low_information"], json!(true));
    assert!(
        !retained["low_information_reasons"]
            .as_array()
            .unwrap()
            .is_empty()
    );
}

#[test]
fn screenshot_sequence_visual_risk_warnings_are_data_and_strict_failures() {
    let root = temp_dir("screenshot-sequence-visual-risk");
    let frame_path = root.join("sequence/frames/frame_0000.png");
    let mut pixels = vec![[20, 20, 20, 255]; 128 * 128];
    for pixel in pixels.iter_mut().take(900) {
        *pixel = [255, 0, 0, 255];
    }
    save_png(&frame_path, (128, 128), pixels);
    let quality = analyze_screenshot_file(&frame_path).unwrap();
    assert_eq!(quality["low_information"], json!(false));
    let warnings = visual_risk::analyze_screenshot_artifacts(
        vec![ScreenshotArtifact {
            snapshot: Some("frame_0000".to_string()),
            screenshot: frame_path.clone(),
        }],
        &VisualRiskOptions::default(),
    );

    assert!(!warnings.is_empty());
    let quality_data = screenshot_sequence_quality_data(&quality, &warnings);
    assert_eq!(
        quality_data["visual_risk_warnings"]
            .as_array()
            .unwrap()
            .len(),
        warnings.len()
    );
    let mut strict_failure = warnings[0].clone();
    strict_failure.command = Some("screenshot sequence".to_string());
    strict_failure.severity = Some("error".to_string());
    assert_eq!(
        strict_failure.command.as_deref(),
        Some("screenshot sequence")
    );
    assert_eq!(strict_failure.severity.as_deref(), Some("error"));
}

#[test]
fn analyze_screenshot_file_reports_size_colors_and_low_information() {
    let root = temp_dir("screenshot-quality-flat");
    let screenshot_path = root.join("flat.png");
    save_png(&screenshot_path, (64, 64), vec![[32, 32, 32, 255]; 64 * 64]);

    let quality = analyze_screenshot_file(&screenshot_path).unwrap();

    assert_eq!(quality["width"], 64);
    assert_eq!(quality["height"], 64);
    assert_eq!(quality["pixel_count"], 4096);
    assert_eq!(quality["non_background_pixel_count"], 0);
    assert_eq!(quality["low_information"], true);
    assert_eq!(
        quality["dominant_colors"][0]["rgba"],
        json!([32, 32, 32, 255])
    );
    assert_eq!(quality["dominant_colors"][0]["count"], 4096);
}

#[test]
fn analyze_screenshot_file_keeps_distinct_foreground_from_background() {
    let root = temp_dir("screenshot-quality-foreground");
    let screenshot_path = root.join("foreground.png");
    let mut pixels = vec![[10, 10, 10, 255]; 128 * 128];
    for pixel in pixels.iter_mut().take(2000) {
        *pixel = [240, 80, 40, 255];
    }
    save_png(&screenshot_path, (128, 128), pixels);

    let quality = analyze_screenshot_file(&screenshot_path).unwrap();

    assert_eq!(quality["width"], 128);
    assert_eq!(quality["height"], 128);
    assert_eq!(quality["non_background_pixel_count"], 2000);
    assert_eq!(quality["low_information"], false);
}

#[test]
fn validate_live_screenshot_quality_fails_transparent_png() {
    let root = temp_dir("screenshot-quality-transparent");
    let screenshot_path = root.join("transparent.png");
    save_png(&screenshot_path, (128, 128), vec![[0, 0, 0, 0]; 128 * 128]);

    let result = validate_live_screenshot_quality(
        "screenshot capture",
        "screenshot capture failed",
        &screenshot_path,
        Some(&root.join("out.png")),
    )
    .unwrap_err();

    assert!(!result.ok);
    assert_eq!(result.exit_code, 1);
    assert_eq!(result.failures[0].kind, "screenshot_low_information");
    assert_eq!(result.data["screenshot_quality"]["low_information"], true);
    assert_eq!(
        result.data["screenshot_quality"]["dominant_colors"][0]["rgba"],
        json!([0, 0, 0, 0])
    );
}

#[test]
fn validate_live_screenshot_quality_fails_flat_black_png() {
    let root = temp_dir("screenshot-quality-black");
    let screenshot_path = root.join("black.png");
    save_png(
        &screenshot_path,
        (128, 128),
        vec![[0, 0, 0, 255]; 128 * 128],
    );

    let result = validate_live_screenshot_quality(
        "screenshot capture",
        "screenshot capture failed",
        &screenshot_path,
        None,
    )
    .unwrap_err();

    assert_eq!(result.failures[0].kind, "screenshot_low_information");
    assert_eq!(
        result.data["screenshot_quality"]["low_information_reasons"][0],
        json!("flat_or_blank")
    );
}

#[test]
fn validate_live_screenshot_quality_passes_useful_foreground() {
    let root = temp_dir("screenshot-quality-useful");
    let screenshot_path = root.join("useful.png");
    let mut pixels = vec![[10, 10, 10, 255]; 128 * 128];
    for pixel in pixels.iter_mut().take(2000) {
        *pixel = [240, 80, 40, 255];
    }
    save_png(&screenshot_path, (128, 128), pixels);

    let quality = validate_live_screenshot_quality(
        "screenshot capture",
        "screenshot capture failed",
        &screenshot_path,
        None,
    )
    .unwrap();

    assert_eq!(quality["low_information"], false);
}

#[test]
fn screenshot_low_information_result_does_not_copy_requested_output() {
    let root = temp_dir("screenshot-quality-no-copy");
    let staged_path = root.join("staged.png");
    let output_path = root.join("out.png");
    save_png(&staged_path, (128, 128), vec![[0, 0, 0, 0]; 128 * 128]);
    let quality = analyze_screenshot_file(&staged_path).unwrap();

    let result = screenshot_low_information_result(
        "screenshot capture",
        "screenshot capture failed",
        &staged_path,
        Some(&output_path),
        quality,
    );

    assert!(!result.ok);
    assert_eq!(result.data["output"], json!(output_path));
    assert!(!output_path.exists());
}

#[test]
fn live_compare_quality_failure_uses_comparison_message() {
    let root = temp_dir("screenshot-quality-live-compare");
    let screenshot_path = root.join("actual.png");
    save_png(&screenshot_path, (128, 128), vec![[0, 0, 0, 0]; 128 * 128]);

    let result = validate_live_screenshot_quality(
        "screenshot compare",
        "screenshot comparison failed",
        &screenshot_path,
        None,
    )
    .unwrap_err();

    assert_eq!(result.command, "screenshot compare");
    assert_eq!(result.message, "screenshot comparison failed");
    assert_eq!(result.failures[0].kind, "screenshot_low_information");
}

#[test]
fn live_screenshot_capture_requires_existing_session() {
    let root = temp_dir("screenshot-live-session-required");
    let config = test_config(&root);

    let result = capture_live_screenshot(&config, "screenshot compare", "main_menu".to_string())
        .unwrap_err();

    assert!(!result.ok);
    assert_eq!(result.command, "screenshot compare");
    assert_eq!(result.message, "live command failed");
    assert_eq!(result.failures[0].kind, "session_not_found");
    assert!(!godot_qa_runtime::session_json_path(&config).exists());
    assert!(!config.artifacts_dir.join("live").exists());
}

#[test]
fn live_screenshot_capture_metadata_parses_supported_response_fields() {
    let root = temp_dir("screenshot-live-metadata");
    let screenshot_path = root.join(".qa/artifacts/live/session-1/screenshots/current.png");
    save_png(
        &screenshot_path,
        (16, 16),
        vec![[32, 96, 160, 255]; 16 * 16],
    );

    let capture = live_screenshot_capture_from_response(
        "screenshot capture",
        serde_json::Map::from_iter([
            (
                "artifacts".to_string(),
                json!({
                    "screenshot": screenshot_path,
                }),
            ),
            ("engine_frame".to_string(), json!(42)),
            ("time_msec".to_string(), json!(1234)),
            ("display_server".to_string(), json!("x11")),
            ("session_id".to_string(), json!("session-1")),
            (
                "session_started_at".to_string(),
                json!("2026-05-17T00:00:00Z"),
            ),
            ("session_scene".to_string(), json!("res://main.tscn")),
        ]),
    )
    .unwrap();

    assert_eq!(capture.screenshot_path, screenshot_path);
    assert_eq!(capture.engine_frame, Some(42));
    assert_eq!(capture.time_msec, Some(1234));
    assert_eq!(capture.metadata["display_server"], json!("x11"));
    assert_eq!(capture.metadata["session_id"], json!("session-1"));
    assert_eq!(
        capture.metadata["session_started_at"],
        json!("2026-05-17T00:00:00Z")
    );
    assert_eq!(capture.metadata["session_scene"], json!("res://main.tscn"));
    assert!(capture.metadata.get("artifacts").is_none());
}

#[test]
fn live_screenshot_capture_metadata_remains_compatible_when_absent() {
    let root = temp_dir("screenshot-live-metadata-absent");
    let screenshot_path = root.join(".qa/artifacts/live/session-1/screenshots/current.png");
    save_png(
        &screenshot_path,
        (16, 16),
        vec![[32, 96, 160, 255]; 16 * 16],
    );

    let capture = live_screenshot_capture_from_response(
        "screenshot compare",
        serde_json::Map::from_iter([(
            "artifacts".to_string(),
            json!({
                "screenshot": screenshot_path,
            }),
        )]),
    )
    .unwrap();

    assert_eq!(capture.screenshot_path, screenshot_path);
    assert_eq!(capture.engine_frame, None);
    assert_eq!(capture.time_msec, None);
    assert!(capture.metadata.is_empty());
}

#[test]
fn screenshot_compare_size_mismatch_keeps_json_shape_and_artifact_paths() {
    let root = temp_dir("screenshot-compare-size-mismatch");
    let actual = root.join(".qa/artifacts/live/session-1/screenshots/main_menu.png");
    let baseline = root.join("qa/baselines/live/main_menu.png");
    let diff = root.join(".qa/artifacts/screenshot-compare/run-1/diff.png");
    let metrics = root.join(".qa/artifacts/screenshot-compare/run-1/metrics.json");
    save_png(&actual, (32, 24), vec![[255, 0, 0, 255]; 32 * 24]);
    save_png(&baseline, (32, 32), vec![[0, 0, 255, 255]; 32 * 32]);

    let result = compare_screenshot_files(
        "screenshot compare",
        &actual,
        &baseline,
        0.01,
        Some(&diff),
        Some(&metrics),
    );

    assert!(!result.ok);
    assert_eq!(result.command, "screenshot compare");
    assert_eq!(result.message, "screenshot comparison failed");
    assert_eq!(result.failures[0].kind, "screenshot_size_mismatch");
    assert_eq!(result.data["actual"], json!(actual));
    assert_eq!(result.data["baseline"], json!(baseline));
    assert_eq!(result.data["artifacts"]["diff"], json!(diff));
    assert_eq!(result.data["artifacts"]["metrics"], json!(metrics));
    assert_eq!(result.data["metrics"]["actual_size"], json!([32, 24]));
    assert_eq!(result.data["metrics"]["baseline_size"], json!([32, 32]));
    assert_eq!(
        result.failures[0].expected,
        Some(json!({
            "baseline_size": [32, 32],
            "threshold": 0.01
        }))
    );
    assert_eq!(
        result.failures[0].actual,
        Some(json!({
            "actual_size": [32, 24],
            "different_ratio": 1.0
        }))
    );
    assert!(diff.is_file());
    assert!(metrics.is_file());
    assert!(diff.starts_with(root.join(".qa/artifacts")));
    assert!(metrics.starts_with(root.join(".qa/artifacts")));
    assert!(!diff.starts_with(root.join("qa/baselines")));
    assert!(!metrics.starts_with(root.join("qa/baselines")));
}

#[test]
fn presented_channel_tolerance_is_opt_in_and_keeps_raw_diff() {
    let root = temp_dir("presented-channel-tolerance");
    let actual = root.join("presented.png");
    let baseline = root.join("viewport.png");
    let diff = root.join("diff.png");
    let metrics = root.join("metrics.json");
    save_png(&actual, (2, 1), vec![[10, 20, 30, 255], [40, 50, 60, 255]]);
    save_png(
        &baseline,
        (2, 1),
        vec![[11, 20, 30, 255], [40, 51, 60, 255]],
    );
    let original = fs::read(&actual).unwrap();
    let original_baseline = fs::read(&baseline).unwrap();
    let exact = compare_screenshot_files("screenshot compare", &actual, &baseline, 0.0, None, None);
    assert!(!exact.ok);
    assert_eq!(exact.data["metrics"]["different_ratio"], 1.0);
    assert_eq!(exact.data["metrics"]["different_pixels"], 2);
    assert!(exact.data["metrics"].get("channel_tolerance").is_none());
    let presented_exact = compare_presented_startup_files(
        "screenshot assert-startup-presented",
        &actual,
        &baseline,
        0.0,
        0,
        None,
        None,
    );
    assert!(!presented_exact.ok);
    assert_eq!(
        presented_exact.data["metrics"]["tolerated_different_ratio"],
        1.0
    );
    let tolerated = compare_presented_startup_files(
        "screenshot assert-startup-presented",
        &actual,
        &baseline,
        0.0,
        1,
        Some(&diff),
        Some(&metrics),
    );
    assert!(tolerated.ok, "{tolerated:?}");
    assert_eq!(tolerated.data["metrics"]["different_pixels"], 0);
    assert_eq!(tolerated.data["metrics"]["different_ratio"], 0.0);
    assert_eq!(tolerated.data["metrics"]["raw_different_pixels"], 2);
    assert_eq!(tolerated.data["metrics"]["raw_different_ratio"], 1.0);
    assert_eq!(tolerated.data["metrics"]["tolerated_different_pixels"], 0);
    assert_eq!(tolerated.data["metrics"]["tolerated_different_ratio"], 0.0);
    assert_eq!(tolerated.data["metrics"]["channel_tolerance"], 1);
    assert_eq!(tolerated.data["metrics"]["max_channel_delta"], 1);
    assert!(diff.is_file() && metrics.is_file());
    assert_eq!(fs::read(&actual).unwrap(), original);
    assert_eq!(fs::read(&baseline).unwrap(), original_baseline);
    let diff_image = image::open(&diff).unwrap().to_rgba8();
    assert_eq!(diff_image.get_pixel(0, 0).0, [255, 0, 0, 255]);
    assert_eq!(diff_image.get_pixel(1, 0).0, [255, 0, 0, 255]);
    save_png(
        &baseline,
        (2, 1),
        vec![[12, 20, 30, 255], [40, 51, 60, 255]],
    );
    let too_far = compare_presented_startup_files(
        "screenshot assert-startup-presented",
        &actual,
        &baseline,
        0.0,
        1,
        Some(&diff),
        Some(&metrics),
    );
    assert!(!too_far.ok);
    assert_eq!(too_far.data["metrics"]["different_pixels"], 1);
    assert_eq!(too_far.data["metrics"]["different_ratio"], 0.5);
    assert_eq!(too_far.data["metrics"]["raw_different_pixels"], 2);
    assert_eq!(too_far.data["metrics"]["raw_different_ratio"], 1.0);
    assert_eq!(too_far.data["metrics"]["tolerated_different_ratio"], 0.5);
    assert_eq!(too_far.data["metrics"]["max_channel_delta"], 2);
    fs::remove_dir_all(root).unwrap();
}
