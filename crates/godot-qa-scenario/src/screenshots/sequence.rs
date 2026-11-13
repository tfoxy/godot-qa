use super::capture::{collect_qa_visual_targets, evaluate_screenshot_state_guards};
use super::{
    ScreenshotStateGuards, analyze_screenshot_file, burst, capture_live_screenshot,
    screenshot_args_invalid,
};
use crate::visual_risk::{self, ScreenshotArtifact, VisualRiskOptions};
use godot_qa_core::config::ProjectConfig;
use godot_qa_core::results::{CommandResult, FailureDetail};
use godot_qa_runtime as runtime;
use serde_json::{Map, Value, json};
use std::fs;
use std::path::{Path, PathBuf};

const DEFAULT_SEQUENCE_ENGINE_FPS: f64 = 60.0;
const MAX_SEQUENCE_REQUESTED_FRAMES: u64 = 100_000;
const MAX_SEQUENCE_CAPTURED_FRAMES: usize = 1_000;

#[derive(Debug, Clone)]
pub struct ScreenshotSequenceOptions {
    pub capture_mode: String,
    pub max_bytes: Option<u64>,
    pub max_seconds: Option<f64>,
    pub frames: Option<i64>,
    pub every: Option<i64>,
    pub seconds: Option<f64>,
    pub fps: Option<f64>,
    pub out: String,
    pub scenario: Option<String>,
    pub scenario_path: Option<String>,
    pub scenario_scene: Option<String>,
    pub state_guards: ScreenshotStateGuards,
    /// Optional `state_matches` predicate that stops the sequence early once it holds.
    /// Validated CLI-side (see `parse_stop_after_state`); `None` means capture the full plan.
    pub stop_after_state: Option<Value>,
    pub allow_low_information: bool,
    pub visual_risk: bool,
    pub visual_risk_strict: bool,
    pub contact_sheet: bool,
    pub contact_sheet_columns: Option<i64>,
    pub contact_sheet_thumb_width: Option<i64>,
    pub contact_sheet_out: Option<String>,
}

#[derive(Debug, Clone)]
pub struct ScreenshotContactSheetOptions {
    pub sequence_dir: String,
    pub columns: i64,
    pub thumb_width: i64,
    pub out: String,
}

pub fn capture_sequence(
    config: &ProjectConfig,
    options: ScreenshotSequenceOptions,
) -> CommandResult {
    let command = "screenshot sequence";
    capture_screenshot_sequence(config, command, options)
}

pub fn build_contact_sheet(
    _config: &ProjectConfig,
    options: ScreenshotContactSheetOptions,
) -> CommandResult {
    let command = "screenshot contact-sheet";
    if options.sequence_dir.trim().is_empty() {
        return screenshot_args_invalid(
            command,
            "screenshot contact-sheet failed",
            "screenshot_contact_sheet_args_invalid",
            "screenshot contact-sheet requires a sequence directory",
            json!(options.sequence_dir),
            json!("non-empty sequence directory"),
        );
    }
    if options.columns <= 0 {
        return screenshot_args_invalid(
            command,
            "screenshot contact-sheet failed",
            "screenshot_contact_sheet_args_invalid",
            "screenshot contact-sheet requires --columns > 0",
            json!(options.columns),
            json!("positive integer columns"),
        );
    }
    if options.thumb_width <= 0 {
        return screenshot_args_invalid(
            command,
            "screenshot contact-sheet failed",
            "screenshot_contact_sheet_args_invalid",
            "screenshot contact-sheet requires --thumb-width > 0",
            json!(options.thumb_width),
            json!("positive integer thumb width"),
        );
    }
    if options.out.trim().is_empty() {
        return screenshot_args_invalid(
            command,
            "screenshot contact-sheet failed",
            "screenshot_contact_sheet_args_invalid",
            "screenshot contact-sheet requires --out",
            json!(options.out),
            json!("non-empty output path"),
        );
    }

    CommandResult {
        ok: false,
        command: command.to_string(),
        message: "screenshot contact-sheet is not implemented yet".to_string(),
        data: Map::from_iter([(
            "options".to_string(),
            json!({
                "sequence_dir": options.sequence_dir,
                "columns": options.columns,
                "thumb_width": options.thumb_width,
                "out": options.out,
            }),
        )]),
        failures: vec![FailureDetail {
            kind: "screenshot_contact_sheet_not_implemented".to_string(),
            command: Some(command.to_string()),
            message: Some(
                "screenshot contact-sheet parser and dispatch are available; image generation is pending"
                    .to_string(),
            ),
            expected: Some(json!("future S72 contact sheet implementation")),
            actual: Some(json!("dispatch stub")),
            ..FailureDetail::default()
        }],
        exit_code: 1,
    }
}

fn capture_screenshot_sequence(
    config: &ProjectConfig,
    command: &str,
    options: ScreenshotSequenceOptions,
) -> CommandResult {
    let plan = match screenshot_sequence_plan(command, &options) {
        Ok(plan) => plan,
        Err(result) => return result,
    };
    let Some(sequence_dir) = resolve_sequence_output_path(Some(&options.out)) else {
        return screenshot_args_invalid(
            command,
            "screenshot sequence failed",
            "screenshot_sequence_args_invalid",
            "screenshot sequence requires --out",
            json!(options.out),
            json!("non-empty output directory"),
        );
    };
    let frames_dir = sequence_dir.join("frames");
    let manifest_path = sequence_dir.join("manifest.json");
    if let Err(error) = fs::create_dir_all(&frames_dir) {
        return sequence_artifact_write_failure(command, &frames_dir, error);
    }

    let burst = if options.capture_mode == "burst" {
        match burst::capture(config, command, &options, &plan) {
            Ok(capture) => Some(capture),
            Err(result) => return result,
        }
    } else {
        None
    };
    let mut frames = Vec::new();
    let mut failures = burst
        .as_ref()
        .map(|capture| capture.failures.clone())
        .unwrap_or_default();
    let mut low_information_frame_count = 0_u64;
    let mut failed_capture_count = burst
        .as_ref()
        .map_or(0, |capture| u64::from(!capture.failures.is_empty()));
    let mut visual_risk_warning_count = 0_u64;
    let mut first_failed_frame = burst
        .as_ref()
        .and_then(|capture| {
            capture.failures.first().map(
                |failure| json!({"sequence_index": capture.frames.len(), "type": failure.kind}),
            )
        })
        .unwrap_or(Value::Null);
    // `--stop-after-state` ends the sequence early once a `state_matches` predicate holds,
    // mirroring `audio capture`. Unlike the post-hoc `--require-state` guard it is polled
    // per captured frame while the sequence is still running.
    let mut stop_condition_matched = false;
    let mut stop_condition_sample_count = 0_u64;
    let mut stop_condition_stopped_at = Value::Null;
    let mut stop_condition_last_error = Value::Null;

    for (sequence_index, requested_frame) in plan.capture_frames.iter().copied().enumerate() {
        if burst
            .as_ref()
            .is_some_and(|capture| sequence_index >= capture.frames.len())
        {
            break;
        }
        let frame_name = format!("frame_{requested_frame:04}.png");
        let relative_path = format!("frames/{frame_name}");
        let output_path = frames_dir.join(&frame_name);

        let guard_report = if let Some(burst) = &burst {
            burst.frames[sequence_index]
                .metadata
                .get("state_guards")
                .cloned()
                .unwrap_or(Value::Null)
        } else {
            match evaluate_screenshot_state_guards(config, command, &options.state_guards) {
                Ok(report) => report,
                Err(result) => {
                    failed_capture_count += 1;
                    if first_failed_frame.is_null() {
                        first_failed_frame = json!({
                            "index": requested_frame,
                            "sequence_index": sequence_index,
                            "type": result.failures.first().map(|failure| failure.kind.clone()),
                        });
                    }
                    failures.extend(result.failures);
                    break;
                }
            }
        };

        let capture = if let Some(burst) = &burst {
            Ok(burst.frames[sequence_index].clone())
        } else {
            capture_live_screenshot(config, command, format!("sequence_{requested_frame:04}"))
        };
        let capture = match capture {
            Ok(capture) => capture,
            Err(result) => {
                failed_capture_count += 1;
                if first_failed_frame.is_null() {
                    first_failed_frame = json!({
                        "index": requested_frame,
                        "sequence_index": sequence_index,
                        "type": result.failures.first().map(|failure| failure.kind.clone()),
                    });
                }
                failures.extend(result.failures);
                break;
            }
        };

        if let Err(error) = fs::copy(&capture.screenshot_path, &output_path) {
            failures.push(sequence_artifact_write_failure_detail(
                command,
                &output_path,
                error,
            ));
            if first_failed_frame.is_null() {
                first_failed_frame = json!({
                    "index": requested_frame,
                    "sequence_index": sequence_index,
                    "type": "artifact_write_failed",
                });
            }
            break;
        }

        let quality = match analyze_screenshot_file(&output_path) {
            Ok(quality) => quality,
            Err(error) => {
                failures.push(FailureDetail {
                    kind: "image_decode_failed".to_string(),
                    command: Some(command.to_string()),
                    path: Some(output_path.to_string_lossy().into_owned()),
                    message: Some(error),
                    expected: Some(json!("decodable PNG screenshot artifact")),
                    actual: Some(json!("decode_failed")),
                    artifacts: Some(Map::from_iter([(
                        "screenshot".to_string(),
                        json!(output_path.to_string_lossy().into_owned()),
                    )])),
                    ..FailureDetail::default()
                });
                if first_failed_frame.is_null() {
                    first_failed_frame = json!({
                        "index": requested_frame,
                        "sequence_index": sequence_index,
                        "type": "image_decode_failed",
                    });
                }
                json!({
                    "path": output_path.to_string_lossy().into_owned(),
                    "low_information": true,
                    "low_information_reasons": ["decode_failed"],
                    "visual_risk_warnings": [],
                })
            }
        };

        let low_information = quality
            .get("low_information")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        if low_information {
            low_information_frame_count += 1;
            if !options.allow_low_information {
                failures.push(screenshot_sequence_low_information_failure(
                    command,
                    requested_frame,
                    sequence_index,
                    &output_path,
                    &quality,
                ));
                if first_failed_frame.is_null() {
                    first_failed_frame = json!({
                        "index": requested_frame,
                        "sequence_index": sequence_index,
                        "type": "screenshot_low_information",
                    });
                }
            }
        }

        let visual_risk_warnings = if options.visual_risk {
            visual_risk::analyze_screenshot_artifacts(
                vec![ScreenshotArtifact {
                    snapshot: Some(format!("frame_{requested_frame:04}")),
                    screenshot: output_path.clone(),
                }],
                &VisualRiskOptions::default(),
            )
        } else {
            Vec::new()
        };
        visual_risk_warning_count += visual_risk_warnings.len() as u64;
        if options.visual_risk_strict {
            for warning in &visual_risk_warnings {
                let mut failure = warning.clone();
                failure.command = Some(command.to_string());
                failure.severity = Some("error".to_string());
                if first_failed_frame.is_null() {
                    first_failed_frame = json!({
                        "index": requested_frame,
                        "sequence_index": sequence_index,
                        "type": failure.kind,
                    });
                }
                failures.push(failure);
            }
        }

        let quality_data = screenshot_sequence_quality_data(&quality, &visual_risk_warnings);
        let target_evidence = if burst.is_some() {
            None
        } else {
            screenshot_sequence_target_evidence(config, command)
        };
        let mut frame = Map::from_iter([
            ("index".to_string(), json!(requested_frame)),
            (
                "engine_frame".to_string(),
                json!(capture.engine_frame.unwrap_or(requested_frame)),
            ),
            (
                "time_msec".to_string(),
                json!(capture.time_msec.unwrap_or_else(|| {
                    ((requested_frame as f64 / DEFAULT_SEQUENCE_ENGINE_FPS) * 1000.0).round() as u64
                })),
            ),
            ("path".to_string(), json!(relative_path)),
            (
                "width".to_string(),
                json!(quality.get("width").and_then(Value::as_u64).unwrap_or(0)),
            ),
            (
                "height".to_string(),
                json!(quality.get("height").and_then(Value::as_u64).unwrap_or(0)),
            ),
            ("quality".to_string(), quality_data),
        ]);
        if let Some(target_evidence) = target_evidence {
            frame.insert("target_evidence".to_string(), target_evidence);
        }
        if !guard_report.is_null() {
            frame.insert("state_guards".to_string(), guard_report);
        }
        if burst.is_some() {
            for key in [
                "draw_ordinal",
                "elapsed_msec",
                "elapsed_usec",
                "requested_time_usec",
                "requested_time_msec",
                "capture_source",
                "draw_forced",
            ] {
                if let Some(value) = capture.metadata.get(key) {
                    frame.insert(key.to_string(), value.clone());
                }
            }
        }
        frames.push(Value::Object(frame));

        if burst.is_some() {
            continue;
        }

        if let Some(predicate) = options.stop_after_state.as_ref().and_then(Value::as_object) {
            stop_condition_sample_count += 1;
            let stop_result = runtime::run_live_protocol_command(
                config,
                command,
                runtime::ASSERT_STATE_MATCHES,
                predicate.clone(),
            );
            if stop_result.ok {
                stop_condition_matched = true;
                stop_condition_stopped_at = json!({
                    "index": requested_frame,
                    "sequence_index": sequence_index,
                });
                break;
            }
            stop_condition_last_error = json!(stop_result.failures);
            let is_mismatch = stop_result
                .failures
                .first()
                .map(|failure| failure.kind == "state_matches_mismatch")
                .unwrap_or(false);
            if !is_mismatch {
                if first_failed_frame.is_null() {
                    first_failed_frame = json!({
                        "index": requested_frame,
                        "sequence_index": sequence_index,
                        "type": stop_result.failures.first().map(|failure| failure.kind.clone()),
                    });
                }
                failures.extend(stop_result.failures);
                break;
            }
        }

        if let Some(next_frame) = plan.capture_frames.get(sequence_index + 1) {
            let wait_frames = next_frame.saturating_sub(requested_frame);
            if wait_frames > 0 {
                let wait_result = runtime::run_live_screenshot_protocol_command(
                    config,
                    command,
                    runtime::SESSION_WAIT_FRAMES,
                    Map::from_iter([("frames".to_string(), json!(wait_frames))]),
                );
                if !wait_result.ok {
                    failed_capture_count += 1;
                    if first_failed_frame.is_null() {
                        first_failed_frame = json!({
                            "index": *next_frame,
                            "sequence_index": sequence_index + 1,
                            "type": wait_result.failures.first().map(|failure| failure.kind.clone()),
                        });
                    }
                    failures.extend(wait_result.failures);
                    break;
                }
            }
        }
    }

    if let Some(capture) = &burst {
        stop_condition_matched = capture
            .metadata
            .get("stop_condition_matched")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        stop_condition_sample_count = capture
            .metadata
            .get("stop_condition_sample_count")
            .and_then(Value::as_u64)
            .unwrap_or(0);
        stop_condition_stopped_at = capture
            .metadata
            .get("stop_condition_stopped_at")
            .cloned()
            .unwrap_or(Value::Null);
        stop_condition_last_error = capture
            .metadata
            .get("stop_condition_last_error")
            .cloned()
            .unwrap_or(Value::Null);
    }
    let mut manifest = screenshot_sequence_manifest(&options, &plan, frames);
    if let Some(capture) = &burst {
        manifest
            .as_object_mut()
            .unwrap()
            .extend(capture.metadata.clone());
    }
    if let Err(error) = write_json_file(&manifest_path, &manifest) {
        return CommandResult {
            ok: false,
            command: command.to_string(),
            message: "screenshot sequence failed".to_string(),
            data: sequence_result_data(
                &sequence_dir,
                &frames_dir,
                &manifest_path,
                &plan,
                0,
                low_information_frame_count,
                failed_capture_count,
                visual_risk_warning_count,
                first_failed_frame,
            ),
            failures: {
                failures.push(sequence_artifact_write_failure_detail(
                    command,
                    &manifest_path,
                    error,
                ));
                failures
            },
            exit_code: 1,
        };
    }

    // Mirror `audio capture`: an unmatched `stop_after_state` on an otherwise clean sequence
    // is a structured timeout failure; other failures already explain why capture stopped.
    if options.stop_after_state.is_some() && !stop_condition_matched && failures.is_empty() {
        failures.push(FailureDetail {
            kind: "screenshot_sequence.stop_condition_timeout".to_string(),
            command: Some(command.to_string()),
            message: Some(
                "screenshot sequence stop_after_state did not match before the capture plan ended"
                    .to_string(),
            ),
            expected: Some(json!(
                "stop_after_state matched before the sequence completed"
            )),
            actual: Some(
                options
                    .stop_after_state
                    .clone()
                    .unwrap_or_else(|| json!("not_matched")),
            ),
            ..FailureDetail::default()
        });
    }
    let ok = failures.is_empty();
    let captured_count = manifest
        .get("captured_count")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let mut data = sequence_result_data(
        &sequence_dir,
        &frames_dir,
        &manifest_path,
        &plan,
        captured_count,
        low_information_frame_count,
        failed_capture_count,
        visual_risk_warning_count,
        first_failed_frame,
    );
    if let Some(capture) = &burst {
        data.extend(capture.metadata.clone());
    }
    if let Some(requested) = options.stop_after_state.as_ref() {
        data.insert(
            "stop_after_state".to_string(),
            json!({
                "requested": requested,
                "matched": stop_condition_matched,
                "sample_count": stop_condition_sample_count,
                "stopped_at": stop_condition_stopped_at,
                "last_error": stop_condition_last_error,
            }),
        );
        data.insert(
            "stop_condition_matched".to_string(),
            json!(stop_condition_matched),
        );
        data.insert(
            "stop_condition_timed_out".to_string(),
            json!(!stop_condition_matched),
        );
    }
    CommandResult {
        ok,
        command: command.to_string(),
        message: if ok {
            "screenshot sequence captured".to_string()
        } else {
            "screenshot sequence failed".to_string()
        },
        data,
        failures,
        exit_code: if ok { 0 } else { 1 },
    }
}

fn resolve_sequence_output_path(raw_path: Option<&str>) -> Option<PathBuf> {
    let raw_path = raw_path?.trim();
    if raw_path.is_empty() {
        return None;
    }
    let path = Path::new(raw_path);
    if path.is_absolute() {
        return Some(path.to_path_buf());
    }
    Some(
        std::env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .join(path),
    )
}

fn screenshot_sequence_target_evidence(config: &ProjectConfig, command: &str) -> Option<Value> {
    let result = runtime::run_live_screenshot_protocol_command(
        config,
        command,
        runtime::TREE_DUMP,
        Map::from_iter([
            ("qa_only".to_string(), json!(true)),
            ("include_metadata".to_string(), json!(true)),
        ]),
    );
    if !result.ok {
        return None;
    }
    let mut targets = Vec::new();
    collect_qa_visual_targets(&Value::Object(result.data), &mut targets);
    if targets.is_empty() {
        None
    } else {
        Some(json!({ "targets": targets }))
    }
}

#[derive(Debug, Clone)]
pub(super) struct ScreenshotSequencePlan {
    pub(super) mode: &'static str,
    pub(super) frame_count_requested: u64,
    pub(super) capture_every_frames: u64,
    pub(super) capture_frames: Vec<u64>,
    pub(super) seconds: Option<f64>,
    pub(super) fps: Option<f64>,
}

pub(super) fn screenshot_sequence_plan(
    command: &str,
    options: &ScreenshotSequenceOptions,
) -> Result<ScreenshotSequencePlan, CommandResult> {
    validate_sequence_options(command, options)?;
    burst::validate(command, options)?;
    if let (Some(frames), Some(every)) = (options.frames, options.every) {
        return bounded_sequence_plan(command, "frames", frames as u64, every as u64, None, None);
    }

    let seconds = options.seconds.unwrap_or_default();
    let fps = options.fps.unwrap_or_default();
    if options.capture_mode == "burst" {
        let samples = (seconds * fps).floor();
        if !samples.is_finite() || samples >= MAX_SEQUENCE_CAPTURED_FRAMES as f64 {
            return Err(screenshot_args_invalid(
                command,
                "screenshot sequence failed",
                "screenshot_sequence_args_invalid",
                "burst capture plan exceeds supported capture count",
                json!({"seconds": seconds, "fps": fps}),
                json!(MAX_SEQUENCE_CAPTURED_FRAMES),
            ));
        }
        let count = samples as u64 + 1;
        return Ok(ScreenshotSequencePlan {
            mode: "seconds",
            frame_count_requested: (seconds * DEFAULT_SEQUENCE_ENGINE_FPS).ceil() as u64,
            capture_every_frames: 1,
            capture_frames: (0..count).collect(),
            seconds: Some(seconds),
            fps: Some(fps),
        });
    }
    let frame_count_requested = (seconds * DEFAULT_SEQUENCE_ENGINE_FPS).ceil() as u64;
    let capture_every_frames = (DEFAULT_SEQUENCE_ENGINE_FPS / fps).ceil().max(1.0) as u64;
    bounded_sequence_plan(
        command,
        "seconds",
        frame_count_requested,
        capture_every_frames,
        Some(seconds),
        Some(fps),
    )
}

fn bounded_sequence_plan(
    command: &str,
    mode: &'static str,
    frame_count_requested: u64,
    capture_every_frames: u64,
    seconds: Option<f64>,
    fps: Option<f64>,
) -> Result<ScreenshotSequencePlan, CommandResult> {
    if frame_count_requested == 0 || frame_count_requested > MAX_SEQUENCE_REQUESTED_FRAMES {
        return Err(screenshot_args_invalid(
            command,
            "screenshot sequence failed",
            "screenshot_sequence_args_invalid",
            "screenshot sequence requested frame window is outside supported bounds",
            json!(frame_count_requested),
            json!({"min": 1, "max": MAX_SEQUENCE_REQUESTED_FRAMES}),
        ));
    }

    let mut capture_frames = Vec::new();
    let mut frame = 0_u64;
    while frame <= frame_count_requested {
        capture_frames.push(frame);
        if capture_frames.len() > MAX_SEQUENCE_CAPTURED_FRAMES {
            return Err(screenshot_args_invalid(
                command,
                "screenshot sequence failed",
                "screenshot_sequence_args_invalid",
                "screenshot sequence capture count exceeds supported bounds",
                json!({
                    "frame_count_requested": frame_count_requested,
                    "capture_every_frames": capture_every_frames,
                    "captured_count": capture_frames.len(),
                }),
                json!({"max_captured_count": MAX_SEQUENCE_CAPTURED_FRAMES}),
            ));
        }
        let next = frame.saturating_add(capture_every_frames);
        if next == frame {
            break;
        }
        frame = next;
    }

    Ok(ScreenshotSequencePlan {
        mode,
        frame_count_requested,
        capture_every_frames,
        capture_frames,
        seconds,
        fps,
    })
}

pub(super) fn screenshot_sequence_quality_data(
    quality: &Value,
    visual_risk_warnings: &[FailureDetail],
) -> Value {
    json!({
        "low_information": quality
            .get("low_information")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        "low_information_reasons": quality
            .get("low_information_reasons")
            .cloned()
            .unwrap_or_else(|| json!([])),
        "pixel_count": quality
            .get("pixel_count")
            .cloned()
            .unwrap_or_else(|| json!(0)),
        "non_background_pixel_count": quality
            .get("non_background_pixel_count")
            .cloned()
            .unwrap_or_else(|| json!(0)),
        "non_background_ratio": quality
            .get("non_background_ratio")
            .cloned()
            .unwrap_or_else(|| json!(0.0)),
        "visual_risk_warnings": visual_risk_warnings
            .iter()
            .map(|warning| serde_json::to_value(warning).unwrap_or_else(|_| json!({})))
            .collect::<Vec<_>>(),
    })
}

pub(super) fn screenshot_sequence_manifest(
    options: &ScreenshotSequenceOptions,
    plan: &ScreenshotSequencePlan,
    frames: Vec<Value>,
) -> Value {
    json!({
        "command": "screenshot.sequence",
        "scenario": options.scenario,
        "scenario_path": options.scenario_path,
        "scenario_scene": options.scenario_scene,
        "scenario_steps_replayed": false,
        "state_guard_requirements": options.state_guards.requirements_json(),
        "timing_mode": plan.mode,
        "frame_count_requested": plan.frame_count_requested,
        "capture_every_frames": plan.capture_every_frames,
        "seconds": plan.seconds,
        "fps": plan.fps,
        "captured_count": frames.len(),
        "frames": frames,
    })
}

pub(super) fn screenshot_sequence_low_information_failure(
    command: &str,
    requested_frame: u64,
    sequence_index: usize,
    screenshot_path: &Path,
    quality: &Value,
) -> FailureDetail {
    let mut details = Map::new();
    details.insert("index".to_string(), json!(requested_frame));
    details.insert("sequence_index".to_string(), json!(sequence_index));
    FailureDetail {
        kind: "screenshot_low_information".to_string(),
        command: Some(command.to_string()),
        path: Some(screenshot_path.to_string_lossy().into_owned()),
        message: Some(
            "sequence frame appears blank, too small, or visually low-information".to_string(),
        ),
        expected: Some(json!(
            "screenshot sequence frame with useful visual evidence"
        )),
        actual: Some(quality.clone()),
        severity: Some("error".to_string()),
        artifacts: Some(Map::from_iter([(
            "screenshot".to_string(),
            json!(screenshot_path.to_string_lossy().into_owned()),
        )])),
        details,
        ..FailureDetail::default()
    }
}

fn sequence_result_data(
    sequence_dir: &Path,
    frames_dir: &Path,
    manifest_path: &Path,
    plan: &ScreenshotSequencePlan,
    captured_count: u64,
    low_information_frame_count: u64,
    failed_capture_count: u64,
    visual_risk_warning_count: u64,
    first_failed_frame: Value,
) -> Map<String, Value> {
    Map::from_iter([
        (
            "out".to_string(),
            json!(sequence_dir.to_string_lossy().into_owned()),
        ),
        (
            "frame_count_requested".to_string(),
            json!(plan.frame_count_requested),
        ),
        (
            "capture_every_frames".to_string(),
            json!(plan.capture_every_frames),
        ),
        ("captured_count".to_string(), json!(captured_count)),
        (
            "low_information_frame_count".to_string(),
            json!(low_information_frame_count),
        ),
        (
            "failed_capture_count".to_string(),
            json!(failed_capture_count),
        ),
        (
            "visual_risk_warning_count".to_string(),
            json!(visual_risk_warning_count),
        ),
        ("first_failed_frame".to_string(), first_failed_frame),
        (
            "artifacts".to_string(),
            json!({
                "sequence_dir": sequence_dir.to_string_lossy().into_owned(),
                "frames_dir": frames_dir.to_string_lossy().into_owned(),
                "manifest": manifest_path.to_string_lossy().into_owned(),
            }),
        ),
    ])
}

pub(super) fn write_json_file(path: &Path, value: &Value) -> Result<(), std::io::Error> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let content = serde_json::to_vec_pretty(value)?;
    fs::write(path, content)
}

fn sequence_artifact_write_failure(
    command: &str,
    path: &Path,
    error: std::io::Error,
) -> CommandResult {
    CommandResult {
        ok: false,
        command: command.to_string(),
        message: "screenshot sequence failed".to_string(),
        data: Map::new(),
        failures: vec![sequence_artifact_write_failure_detail(command, path, error)],
        exit_code: 1,
    }
}

fn sequence_artifact_write_failure_detail(
    command: &str,
    path: &Path,
    error: std::io::Error,
) -> FailureDetail {
    FailureDetail {
        kind: "artifact_write_failed".to_string(),
        command: Some(command.to_string()),
        path: Some(path.to_string_lossy().into_owned()),
        message: Some(error.to_string()),
        expected: Some(json!("writable screenshot sequence artifact path")),
        actual: Some(json!("OSError")),
        ..FailureDetail::default()
    }
}

fn validate_sequence_options(
    command: &str,
    options: &ScreenshotSequenceOptions,
) -> Result<(), CommandResult> {
    let frame_mode = options.frames.is_some() || options.every.is_some();
    let time_mode = options.seconds.is_some() || options.fps.is_some();
    if frame_mode == time_mode {
        return Err(screenshot_args_invalid(
            command,
            "screenshot sequence failed",
            "screenshot_sequence_args_invalid",
            "screenshot sequence requires exactly one timing mode: --frames with --every, or --seconds with --fps",
            json!({
                "frames": options.frames,
                "every": options.every,
                "seconds": options.seconds,
                "fps": options.fps,
            }),
            json!("either --frames <n> --every <n>, or --seconds <n> --fps <n>"),
        ));
    }
    if frame_mode {
        match (options.frames, options.every) {
            (Some(frames), Some(every)) if frames > 0 && every > 0 => {}
            _ => {
                return Err(screenshot_args_invalid(
                    command,
                    "screenshot sequence failed",
                    "screenshot_sequence_args_invalid",
                    "screenshot sequence frame timing requires positive --frames and --every",
                    json!({"frames": options.frames, "every": options.every}),
                    json!("positive integer --frames and --every"),
                ));
            }
        }
    }
    if time_mode {
        match (options.seconds, options.fps) {
            (Some(seconds), Some(fps))
                if seconds.is_finite() && fps.is_finite() && seconds > 0.0 && fps > 0.0 => {}
            _ => {
                return Err(screenshot_args_invalid(
                    command,
                    "screenshot sequence failed",
                    "screenshot_sequence_args_invalid",
                    "screenshot sequence time timing requires positive finite --seconds and --fps",
                    json!({"seconds": options.seconds, "fps": options.fps}),
                    json!("positive finite --seconds and --fps"),
                ));
            }
        }
    }
    if options.out.trim().is_empty() {
        return Err(screenshot_args_invalid(
            command,
            "screenshot sequence failed",
            "screenshot_sequence_args_invalid",
            "screenshot sequence requires --out",
            json!(options.out),
            json!("non-empty output directory"),
        ));
    }
    if let Some(scenario) = &options.scenario
        && scenario.trim().is_empty()
    {
        return Err(screenshot_args_invalid(
            command,
            "screenshot sequence failed",
            "screenshot_sequence_args_invalid",
            "screenshot sequence requires a non-empty --scenario when supplied",
            json!(scenario),
            json!("non-empty scenario name"),
        ));
    }
    if let Some(columns) = options.contact_sheet_columns
        && columns <= 0
    {
        return Err(screenshot_args_invalid(
            command,
            "screenshot sequence failed",
            "screenshot_sequence_args_invalid",
            "screenshot sequence requires --contact-sheet-columns > 0",
            json!(columns),
            json!("positive integer contact sheet columns"),
        ));
    }
    if let Some(thumb_width) = options.contact_sheet_thumb_width
        && thumb_width <= 0
    {
        return Err(screenshot_args_invalid(
            command,
            "screenshot sequence failed",
            "screenshot_sequence_args_invalid",
            "screenshot sequence requires --contact-sheet-thumb-width > 0",
            json!(thumb_width),
            json!("positive integer contact sheet thumb width"),
        ));
    }
    if let Some(out) = &options.contact_sheet_out
        && out.trim().is_empty()
    {
        return Err(screenshot_args_invalid(
            command,
            "screenshot sequence failed",
            "screenshot_sequence_args_invalid",
            "screenshot sequence requires a non-empty --contact-sheet-out when supplied",
            json!(out),
            json!("non-empty contact sheet output path"),
        ));
    }
    Ok(())
}
