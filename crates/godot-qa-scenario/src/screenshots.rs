use godot_qa_core::config::ProjectConfig;
use godot_qa_core::results::{CommandResult, FailureDetail};
use godot_qa_runtime as runtime;
use image::{ImageBuffer, ImageError, Rgba, RgbaImage};
use serde_json::{Map, Value, json};
use std::collections::{HashMap, VecDeque};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::visual_risk::{self, ScreenshotArtifact, VisualRiskOptions};

#[path = "screenshots/burst.rs"]
mod burst;

const DEFAULT_SEQUENCE_ENGINE_FPS: f64 = 60.0;
const MAX_SEQUENCE_REQUESTED_FRAMES: u64 = 100_000;
const MAX_SEQUENCE_CAPTURED_FRAMES: usize = 1_000;

pub fn capture_screenshot(
    config: &ProjectConfig,
    output: &str,
    state_guards: ScreenshotStateGuards,
) -> CommandResult {
    let command = "screenshot capture";
    let output_path = resolve_input_path(&config.project_root, Some(output));
    let Some(output_path) = output_path else {
        return args_invalid(
            command,
            "screenshot output path is required",
            json!("--out <path>"),
            json!(output),
        );
    };

    let guard_report = match evaluate_screenshot_state_guards(config, command, &state_guards) {
        Ok(report) => report,
        Err(result) => return result,
    };
    let capture =
        match capture_live_screenshot(config, command, capture_label_from_path(&output_path)) {
            Ok(capture) => capture,
            Err(result) => return result,
        };
    let _ = capture.has_frame_metadata();
    let quality = match validate_live_screenshot_quality(
        command,
        "screenshot capture failed",
        &capture.screenshot_path,
        Some(&output_path),
    ) {
        Ok(quality) => quality,
        Err(result) => return result,
    };

    if let Some(parent) = output_path.parent()
        && let Err(error) = fs::create_dir_all(parent)
    {
        return capture_artifact_write_failure(command, &output_path, error);
    }
    if let Err(error) = fs::copy(&capture.screenshot_path, &output_path) {
        return capture_artifact_write_failure(command, &output_path, error);
    }

    let mut data = Map::from_iter([
        (
            "output".to_string(),
            json!(output_path.to_string_lossy().into_owned()),
        ),
        (
            "artifacts".to_string(),
            json!({
                "screenshot": capture.screenshot_path,
            }),
        ),
        ("screenshot_quality".to_string(), quality),
    ]);
    if !guard_report.is_null() {
        data.insert("state_guards".to_string(), guard_report);
    }

    CommandResult {
        ok: true,
        command: command.to_string(),
        message: "screenshot captured".to_string(),
        data,
        failures: Vec::new(),
        exit_code: 0,
    }
}

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

#[derive(Debug, Clone, Default)]
pub struct ScreenshotStateGuards {
    pub require_visible: Vec<String>,
    pub require_hidden: Vec<String>,
    pub require_state: Option<Value>,
}

impl ScreenshotStateGuards {
    pub fn is_empty(&self) -> bool {
        self.require_visible.is_empty()
            && self.require_hidden.is_empty()
            && self.require_state.is_none()
    }

    fn requirements_json(&self) -> Value {
        json!({
            "require_visible": self.require_visible,
            "require_hidden": self.require_hidden,
            "require_state": self.require_state,
        })
    }
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

pub fn compare_screenshots(
    config: &ProjectConfig,
    actual: Option<&str>,
    baseline: Option<&str>,
    diff: Option<&str>,
    threshold: f64,
    live: bool,
) -> CommandResult {
    let command = "screenshot compare";
    let actual_path = if live {
        None
    } else {
        resolve_input_path(&config.project_root, actual)
    };
    let baseline_path = resolve_input_path(&config.project_root, baseline);

    if live && actual.is_some_and(|value| !value.trim().is_empty()) {
        return args_invalid(
            command,
            "screenshot compare --live cannot be combined with --actual",
            json!({"live": true, "actual": actual}),
            json!("--live with --baseline and no --actual"),
        );
    }

    let Some(baseline_path) = baseline_path else {
        return args_invalid(
            command,
            if live {
                "screenshot compare --live requires --baseline"
            } else {
                "screenshot compare requires --actual and --baseline"
            },
            json!({"live": live, "actual": actual, "baseline": baseline}),
            if live {
                json!("--baseline <path>")
            } else {
                json!("--actual <path> and --baseline <path>")
            },
        );
    };

    if !live && actual_path.is_none() {
        return args_invalid(
            command,
            "screenshot compare requires --actual and --baseline",
            json!({"live": live, "actual": actual, "baseline": baseline}),
            json!("--actual <path> and --baseline <path>"),
        );
    }

    if !(0.0..=1.0).contains(&threshold) {
        return args_invalid(
            command,
            "threshold must be between 0.0 and 1.0",
            json!(threshold),
            json!("number between 0.0 and 1.0"),
        );
    }

    let actual_path = if live {
        let capture =
            match capture_live_screenshot(config, command, capture_label_from_path(&baseline_path))
            {
                Ok(capture) => capture,
                Err(result) => return result,
            };
        let _ = capture.has_frame_metadata();
        if let Err(result) = validate_live_screenshot_quality(
            command,
            "screenshot comparison failed",
            &capture.screenshot_path,
            None,
        ) {
            return result;
        }
        capture.screenshot_path
    } else {
        actual_path.expect("non-live screenshot compare requires an actual path")
    };

    if let Some(diff_path) = resolve_input_path(&config.project_root, diff) {
        let metrics_path = metrics_path_for_diff(&diff_path);
        return compare_screenshot_files(
            command,
            &actual_path,
            &baseline_path,
            threshold,
            Some(&diff_path),
            Some(&metrics_path),
        );
    }

    let first_pass =
        compare_screenshot_files(command, &actual_path, &baseline_path, threshold, None, None);
    if first_pass.ok {
        return first_pass;
    }
    if first_pass
        .failures
        .first()
        .map(|failure| {
            matches!(
                failure.kind.as_str(),
                "screenshot_file_missing"
                    | "baseline_not_found"
                    | "image_decode_failed"
                    | "artifact_write_failed"
            )
        })
        .unwrap_or(false)
    {
        return first_pass;
    }

    let compare_root = allocate_compare_root(&config.artifacts_dir);
    let diff_path = compare_root.join("diff.png");
    let metrics_path = compare_root.join("metrics.json");
    compare_screenshot_files(
        command,
        &actual_path,
        &baseline_path,
        threshold,
        Some(&diff_path),
        Some(&metrics_path),
    )
}

pub fn compare_screenshot_files(
    command: &str,
    actual_path: &Path,
    baseline_path: &Path,
    threshold: f64,
    diff_path: Option<&Path>,
    metrics_path: Option<&Path>,
) -> CommandResult {
    compare_screenshot_files_inner(
        command,
        actual_path,
        baseline_path,
        threshold,
        diff_path,
        metrics_path,
        ScreenshotComparison::default(),
    )
}

/// Opt-in per-channel tolerance for the isolated presented-startup assertion.
/// The ordinary screenshot comparator remains exact by default.
pub fn compare_presented_startup_files(
    command: &str,
    actual_path: &Path,
    baseline_path: &Path,
    threshold: f64,
    channel_tolerance: u8,
    diff_path: Option<&Path>,
    metrics_path: Option<&Path>,
) -> CommandResult {
    compare_screenshot_files_inner(
        command,
        actual_path,
        baseline_path,
        threshold,
        diff_path,
        metrics_path,
        ScreenshotComparison {
            channel_tolerance: Some(channel_tolerance),
            actual: None,
        },
    )
}

#[derive(Default)]
struct ScreenshotComparison<'a> {
    channel_tolerance: Option<u8>,
    actual: Option<Result<&'a ScreenshotAnalysis, String>>,
}

pub(crate) fn compare_screenshot_files_cached(
    command: &str,
    actual_path: &Path,
    baseline_path: &Path,
    threshold: f64,
    diff_path: Option<&Path>,
    metrics_path: Option<&Path>,
    cache: &mut ScreenshotAnalysisCache,
) -> CommandResult {
    compare_screenshot_files_inner(
        command,
        actual_path,
        baseline_path,
        threshold,
        diff_path,
        metrics_path,
        ScreenshotComparison {
            actual: (actual_path.is_file() && baseline_path.is_file())
                .then(|| cache.get(actual_path)),
            ..ScreenshotComparison::default()
        },
    )
}

fn compare_screenshot_files_inner(
    command: &str,
    actual_path: &Path,
    baseline_path: &Path,
    threshold: f64,
    diff_path: Option<&Path>,
    metrics_path: Option<&Path>,
    comparison: ScreenshotComparison<'_>,
) -> CommandResult {
    let channel_tolerance = comparison.channel_tolerance;
    if !actual_path.is_file() {
        return result(
            false,
            command,
            "screenshot comparison failed",
            actual_path,
            baseline_path,
            threshold,
            None,
            None,
            vec![FailureDetail {
                kind: "screenshot_file_missing".to_string(),
                command: Some(command.to_string()),
                path: Some(actual_path.to_string_lossy().into_owned()),
                message: Some("actual screenshot file was not found".to_string()),
                expected: Some(json!("existing actual screenshot file")),
                actual: Some(json!("missing")),
                ..FailureDetail::default()
            }],
        );
    }
    if !baseline_path.is_file() {
        return result(
            false,
            command,
            "screenshot comparison failed",
            actual_path,
            baseline_path,
            threshold,
            None,
            None,
            vec![FailureDetail {
                kind: "baseline_not_found".to_string(),
                command: Some(command.to_string()),
                path: Some(baseline_path.to_string_lossy().into_owned()),
                message: Some("baseline screenshot file was not found".to_string()),
                expected: Some(json!("existing baseline screenshot file")),
                actual: Some(json!("missing")),
                ..FailureDetail::default()
            }],
        );
    }

    let owned_actual;
    let actual_image = match comparison.actual {
        Some(Ok(analysis)) => &analysis.pixels,
        Some(Err(message)) => {
            return decode_failure(command, actual_path, baseline_path, threshold, message);
        }
        None => {
            owned_actual = match load_rgba_image(actual_path) {
                Ok(image) => image,
                Err(message) => {
                    return decode_failure(command, actual_path, baseline_path, threshold, message);
                }
            };
            &owned_actual
        }
    };
    let baseline_image = match load_rgba_image(baseline_path) {
        Ok(image) => image,
        Err(message) => {
            return decode_failure(command, actual_path, baseline_path, threshold, message);
        }
    };

    let (metrics, ok) =
        compare_images_with_tolerance(actual_image, &baseline_image, threshold, channel_tolerance);
    let mut artifacts = Map::new();

    if let Some(path) = metrics_path {
        if let Err(error) = write_metrics(path, &metrics) {
            return artifact_write_failure(
                command,
                actual_path,
                baseline_path,
                threshold,
                path,
                error,
            );
        }
        artifacts.insert(
            "metrics".to_string(),
            json!(path.to_string_lossy().into_owned()),
        );
    }

    if !ok || channel_tolerance.is_some() {
        if let Some(path) = diff_path {
            if let Err(error) =
                write_diff_image(path, &comparison_diff_image(actual_image, &baseline_image))
            {
                return artifact_write_failure(
                    command,
                    actual_path,
                    baseline_path,
                    threshold,
                    path,
                    error,
                );
            }
            artifacts.insert(
                "diff".to_string(),
                json!(path.to_string_lossy().into_owned()),
            );
        }
    }

    if !ok {
        let failure = if metrics
            .get("size_match")
            .and_then(Value::as_bool)
            .unwrap_or(false)
        {
            FailureDetail {
                kind: "screenshot_mismatch".to_string(),
                command: Some(command.to_string()),
                message: Some(
                    "screenshot comparison exceeded the allowed difference threshold".to_string(),
                ),
                expected: Some(json!(format!(
                    "different_ratio <= {}",
                    threshold_string(threshold)
                ))),
                actual: metrics.get("different_ratio").cloned(),
                ..FailureDetail::default()
            }
        } else {
            FailureDetail {
                kind: "screenshot_size_mismatch".to_string(),
                command: Some(command.to_string()),
                message: Some("screenshot dimensions did not match the baseline".to_string()),
                expected: Some(json!({
                    "baseline_size": metrics.get("baseline_size").cloned().unwrap_or(Value::Null),
                    "threshold": threshold,
                })),
                actual: Some(json!({
                    "actual_size": metrics.get("actual_size").cloned().unwrap_or(Value::Null),
                    "different_ratio": metrics.get("different_ratio").cloned().unwrap_or(Value::Null),
                })),
                ..FailureDetail::default()
            }
        };

        return result(
            false,
            command,
            "screenshot comparison failed",
            actual_path,
            baseline_path,
            threshold,
            Some(Value::Object(metrics)),
            Some(artifacts),
            vec![failure],
        );
    }

    result(
        true,
        command,
        "screenshot comparison passed",
        actual_path,
        baseline_path,
        threshold,
        Some(Value::Object(metrics)),
        Some(artifacts),
        Vec::new(),
    )
}

/// Immutable decoded screenshot shared by finalization checks. Quality is computed only
/// when requested; standalone comparisons need only the pixels.
#[derive(Debug)]
pub(crate) struct ScreenshotAnalysis {
    pixels: RgbaImage,
    path: PathBuf,
    quality: OnceLock<Value>,
}

impl ScreenshotAnalysis {
    fn load(path: &Path) -> Result<Self, String> {
        Ok(Self {
            pixels: load_rgba_image(path)?,
            path: path.to_path_buf(),
            quality: OnceLock::new(),
        })
    }

    pub(crate) fn quality(&self) -> &Value {
        self.quality
            .get_or_init(|| analyze_screenshot_pixels(&self.path, &self.pixels))
    }

    pub(crate) fn measure_target(&self, rect: [f64; 4]) -> Value {
        measure_saved_target_analysis(self, rect)
    }
}

/// Lives only for one scenario's finalization. Retain at most two decoded
/// frames, even in long runs; quality summaries survive eviction for later
/// assertions without repeating whole-image color analysis.
#[derive(Default)]
pub(crate) struct ScreenshotAnalysisCache {
    entries: HashMap<PathBuf, Result<ScreenshotAnalysis, String>>,
    recent: VecDeque<PathBuf>,
    quality: HashMap<PathBuf, Value>,
}

impl ScreenshotAnalysisCache {
    pub(crate) fn get(&mut self, path: &Path) -> Result<&ScreenshotAnalysis, String> {
        const MAX_DECODED_SCREENSHOTS: usize = 2;
        if !self.entries.contains_key(path) {
            if self.entries.len() == MAX_DECODED_SCREENSHOTS
                && let Some(oldest) = self.recent.pop_front()
                && let Some(Ok(analysis)) = self.entries.remove(&oldest)
                && let Some(quality) = analysis.quality.get()
            {
                self.quality.insert(oldest, quality.clone());
            }
            let analysis = ScreenshotAnalysis::load(path).inspect(|analysis| {
                if let Some(quality) = self.quality.get(path) {
                    // This frame's immutable quality was computed before eviction.
                    let _ = analysis.quality.set(quality.clone());
                }
            });
            self.entries.insert(path.to_path_buf(), analysis);
        }
        self.recent.retain(|recent| recent != path);
        self.recent.push_back(path.to_path_buf());
        self.entries[path].as_ref().map_err(Clone::clone)
    }
}

pub fn analyze_screenshot_file(path: &Path) -> Result<Value, String> {
    Ok(ScreenshotAnalysis::load(path)?.quality().clone())
}

fn analyze_screenshot_pixels(path: &Path, pixels: &RgbaImage) -> Value {
    let (width, height) = pixels.dimensions();
    let pixel_count = u64::from(width) * u64::from(height);
    let mut counts: HashMap<[u8; 4], u64> = HashMap::new();
    for pixel in pixels.pixels() {
        *counts.entry(pixel.0).or_insert(0) += 1;
    }

    let mut dominant_colors: Vec<([u8; 4], u64)> = counts.into_iter().collect();
    dominant_colors.sort_by(|(left_color, left_count), (right_color, right_count)| {
        right_count
            .cmp(left_count)
            .then_with(|| left_color.cmp(right_color))
    });
    let background_count = dominant_colors
        .first()
        .map(|(_, count)| *count)
        .unwrap_or(0);
    let non_background_pixel_count = pixel_count.saturating_sub(background_count);
    let non_background_ratio = if pixel_count == 0 {
        0.0
    } else {
        non_background_pixel_count as f64 / pixel_count as f64
    };
    let dominant_ratio = if pixel_count == 0 {
        0.0
    } else {
        background_count as f64 / pixel_count as f64
    };

    let mut reasons = Vec::new();
    if width < 128 || height < 128 {
        reasons.push(json!("small_dimensions"));
    }
    if pixel_count == 0 {
        reasons.push(json!("empty_image"));
    }
    if pixel_count > 0 && non_background_ratio < 0.01 {
        reasons.push(json!("flat_or_blank"));
    }
    if pixel_count > 0 && dominant_ratio >= 0.98 {
        reasons.push(json!("dominant_color_overwhelms"));
    }

    let dominant_color_values = dominant_colors
        .into_iter()
        .take(5)
        .map(|(color, count)| {
            json!({
                "rgba": color,
                "count": count,
                "ratio": if pixel_count == 0 {
                    0.0
                } else {
                    count as f64 / pixel_count as f64
                },
            })
        })
        .collect::<Vec<_>>();

    json!({
        "path": path.to_string_lossy().into_owned(),
        "width": width,
        "height": height,
        "pixel_count": pixel_count,
        "dominant_colors": dominant_color_values,
        "non_background_pixel_count": non_background_pixel_count,
        "non_background_ratio": non_background_ratio,
        "low_information": !reasons.is_empty(),
        "low_information_reasons": reasons,
    })
}

pub fn measure_saved_target_pixels(path: &Path, rect: [f64; 4]) -> Result<Value, String> {
    Ok(ScreenshotAnalysis::load(path)?.measure_target(rect))
}

fn measure_saved_target_analysis(analysis: &ScreenshotAnalysis, rect: [f64; 4]) -> Value {
    let path = &analysis.path;
    let quality = analysis.quality();
    if quality
        .get("low_information")
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        return json!({
            "source_screenshot": path.to_string_lossy().into_owned(),
            "screenshot_quality": quality,
            "low_information": true,
            "low_information_reasons": quality
                .get("low_information_reasons")
                .cloned()
                .unwrap_or_else(|| json!([])),
            "target_rect": rect,
            "useful_pixel_count": 0_u64,
            "useful_pixel_ratio": 0.0,
        });
    }

    let pixels = &analysis.pixels;
    let (width, height) = pixels.dimensions();
    let x0 = rect[0].floor().max(0.0).min(f64::from(width)) as u32;
    let y0 = rect[1].floor().max(0.0).min(f64::from(height)) as u32;
    let x1 = (rect[0] + rect[2]).ceil().max(0.0).min(f64::from(width)) as u32;
    let y1 = (rect[1] + rect[3]).ceil().max(0.0).min(f64::from(height)) as u32;
    let measured_pixel_count = u64::from(x1.saturating_sub(x0)) * u64::from(y1.saturating_sub(y0));

    let dominant = quality
        .get("dominant_colors")
        .and_then(Value::as_array)
        .and_then(|colors| colors.first())
        .and_then(|entry| entry.get("rgba"))
        .and_then(Value::as_array)
        .and_then(|rgba| {
            Some([
                rgba.first()?.as_u64()? as u8,
                rgba.get(1)?.as_u64()? as u8,
                rgba.get(2)?.as_u64()? as u8,
                rgba.get(3)?.as_u64()? as u8,
            ])
        });

    let mut useful_pixel_count = 0_u64;
    for y in y0..y1 {
        for x in x0..x1 {
            let rgba = pixels.get_pixel(x, y).0;
            let visible = rgba[3] > 0;
            let non_black = rgba[0] > 0 || rgba[1] > 0 || rgba[2] > 0;
            let differs_from_background = dominant != Some(rgba);
            if visible && non_black && differs_from_background {
                useful_pixel_count += 1;
            }
        }
    }
    let useful_pixel_ratio = if measured_pixel_count == 0 {
        0.0
    } else {
        useful_pixel_count as f64 / measured_pixel_count as f64
    };

    json!({
        "source_screenshot": path.to_string_lossy().into_owned(),
        "screenshot_quality": quality,
        "low_information": false,
        "low_information_reasons": [],
        "target_rect": rect,
        "measured_pixel_count": measured_pixel_count,
        "useful_pixel_count": useful_pixel_count,
        "useful_pixel_ratio": useful_pixel_ratio,
    })
}

fn result(
    ok: bool,
    command: &str,
    message: &str,
    actual_path: &Path,
    baseline_path: &Path,
    threshold: f64,
    metrics: Option<Value>,
    artifacts: Option<Map<String, Value>>,
    failures: Vec<FailureDetail>,
) -> CommandResult {
    let mut data = Map::new();
    data.insert(
        "actual".to_string(),
        json!(actual_path.to_string_lossy().into_owned()),
    );
    data.insert(
        "baseline".to_string(),
        json!(baseline_path.to_string_lossy().into_owned()),
    );
    data.insert("threshold".to_string(), json!(threshold));
    data.insert("metrics".to_string(), metrics.unwrap_or_else(|| json!({})));
    data.insert(
        "artifacts".to_string(),
        Value::Object(artifacts.unwrap_or_default()),
    );
    CommandResult {
        ok,
        command: command.to_string(),
        message: message.to_string(),
        data,
        failures,
        exit_code: if ok { 0 } else { 1 },
    }
}

fn decode_failure(
    command: &str,
    actual_path: &Path,
    baseline_path: &Path,
    threshold: f64,
    message: String,
) -> CommandResult {
    result(
        false,
        command,
        "screenshot comparison failed",
        actual_path,
        baseline_path,
        threshold,
        None,
        None,
        vec![FailureDetail {
            kind: "image_decode_failed".to_string(),
            command: Some(command.to_string()),
            message: Some(message),
            expected: Some(json!("decodable PNG image")),
            actual: Some(json!("decode_failed")),
            ..FailureDetail::default()
        }],
    )
}

fn artifact_write_failure(
    command: &str,
    actual_path: &Path,
    baseline_path: &Path,
    threshold: f64,
    path: &Path,
    error: std::io::Error,
) -> CommandResult {
    result(
        false,
        command,
        "screenshot comparison failed",
        actual_path,
        baseline_path,
        threshold,
        None,
        None,
        vec![FailureDetail {
            kind: "artifact_write_failed".to_string(),
            command: Some(command.to_string()),
            path: Some(path.to_string_lossy().into_owned()),
            message: Some(error.to_string()),
            expected: Some(json!(
                "writable output path for screenshot comparison artifacts"
            )),
            actual: Some(json!("OSError")),
            ..FailureDetail::default()
        }],
    )
}

fn load_rgba_image(path: &Path) -> Result<RgbaImage, String> {
    image::open(path)
        .map(image::DynamicImage::into_rgba8)
        .map_err(image_error_message)
}

fn image_error_message(error: ImageError) -> String {
    error.to_string()
}

fn compare_images_with_tolerance(
    actual: &RgbaImage,
    baseline: &RgbaImage,
    threshold: f64,
    channel_tolerance: Option<u8>,
) -> (Map<String, Value>, bool) {
    let (actual_width, actual_height) = actual.dimensions();
    let (baseline_width, baseline_height) = baseline.dimensions();
    let width = actual_width.max(baseline_width);
    let height = actual_height.max(baseline_height);
    let total_pixels = u64::from(width) * u64::from(height);

    let mut different_pixels = 0u64;
    let mut tolerated_different_pixels = 0u64;
    let mut max_channel_delta = 0u8;
    let mut channel_delta_total = 0u64;

    for y in 0..height {
        for x in 0..width {
            let actual_pixel = pixel_from_buffer(actual, x, y);
            let baseline_pixel = pixel_from_buffer(baseline, x, y);
            let deltas = [
                actual_pixel[0].abs_diff(baseline_pixel[0]),
                actual_pixel[1].abs_diff(baseline_pixel[1]),
                actual_pixel[2].abs_diff(baseline_pixel[2]),
                actual_pixel[3].abs_diff(baseline_pixel[3]),
            ];
            channel_delta_total += deltas.iter().map(|value| u64::from(*value)).sum::<u64>();
            let pixel_max_delta = *deltas.iter().max().unwrap_or(&0);
            if pixel_max_delta > max_channel_delta {
                max_channel_delta = pixel_max_delta;
            }
            if actual_pixel != baseline_pixel {
                different_pixels += 1;
            }
            if pixel_max_delta > channel_tolerance.unwrap_or(0) {
                tolerated_different_pixels += 1;
            }
        }
    }

    let different_ratio = if total_pixels == 0 {
        0.0
    } else {
        different_pixels as f64 / total_pixels as f64
    };
    let tolerated_different_ratio = if total_pixels == 0 {
        0.0
    } else {
        tolerated_different_pixels as f64 / total_pixels as f64
    };
    let size_match = actual_width == baseline_width && actual_height == baseline_height;
    let mut metrics = Map::new();
    metrics.insert(
        "actual_size".to_string(),
        json!([actual_width, actual_height]),
    );
    metrics.insert(
        "baseline_size".to_string(),
        json!([baseline_width, baseline_height]),
    );
    metrics.insert("size_match".to_string(), json!(size_match));
    metrics.insert("total_pixels".to_string(), json!(total_pixels));
    metrics.insert(
        "different_pixels".to_string(),
        json!(if channel_tolerance.is_some() {
            tolerated_different_pixels
        } else {
            different_pixels
        }),
    );
    metrics.insert(
        "different_ratio".to_string(),
        json!(if channel_tolerance.is_some() {
            tolerated_different_ratio
        } else {
            different_ratio
        }),
    );
    if let Some(tolerance) = channel_tolerance {
        metrics.insert("raw_different_pixels".to_string(), json!(different_pixels));
        metrics.insert("raw_different_ratio".to_string(), json!(different_ratio));
        metrics.insert(
            "tolerated_different_pixels".to_string(),
            json!(tolerated_different_pixels),
        );
        metrics.insert(
            "tolerated_different_ratio".to_string(),
            json!(tolerated_different_ratio),
        );
        metrics.insert("channel_tolerance".to_string(), json!(tolerance));
    }
    metrics.insert("max_channel_delta".to_string(), json!(max_channel_delta));
    metrics.insert(
        "mean_channel_delta".to_string(),
        json!(if total_pixels == 0 {
            0.0
        } else {
            channel_delta_total as f64 / (total_pixels as f64 * 4.0)
        }),
    );
    metrics.insert("threshold".to_string(), json!(threshold));

    (
        metrics,
        size_match && tolerated_different_ratio <= threshold,
    )
}

fn comparison_diff_image(actual: &RgbaImage, baseline: &RgbaImage) -> RgbaImage {
    let width = actual.width().max(baseline.width());
    let height = actual.height().max(baseline.height());
    RgbaImage::from_fn(width, height, |x, y| {
        Rgba(
            if pixel_from_buffer(actual, x, y) != pixel_from_buffer(baseline, x, y) {
                [255, 0, 0, 255]
            } else {
                [0, 0, 0, 0]
            },
        )
    })
}

fn pixel_from_buffer(image: &ImageBuffer<Rgba<u8>, Vec<u8>>, x: u32, y: u32) -> [u8; 4] {
    if x >= image.width() || y >= image.height() {
        return [0, 0, 0, 0];
    }
    image.get_pixel(x, y).0
}

fn write_metrics(path: &Path, metrics: &Map<String, Value>) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(
        path,
        serde_json::to_string_pretty(metrics).unwrap_or_else(|_| "{}".to_string()),
    )
}

fn write_diff_image(path: &Path, image: &RgbaImage) -> Result<(), std::io::Error> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    image
        .save(path)
        .map_err(|error| std::io::Error::other(error.to_string()))
}

fn threshold_string(threshold: f64) -> String {
    if threshold.fract() == 0.0 {
        format!("{threshold:.1}")
    } else {
        threshold.to_string()
    }
}

#[derive(Debug, Clone)]
pub struct LiveScreenshotCapture {
    pub screenshot_path: PathBuf,
    pub engine_frame: Option<u64>,
    pub time_msec: Option<u64>,
    pub metadata: Map<String, Value>,
}

impl LiveScreenshotCapture {
    pub fn has_frame_metadata(&self) -> bool {
        self.engine_frame.is_some() || self.time_msec.is_some() || !self.metadata.is_empty()
    }
}

fn resolve_input_path(project_root: &Path, raw_path: Option<&str>) -> Option<PathBuf> {
    let raw_path = raw_path?.trim();
    if raw_path.is_empty() {
        return None;
    }
    let path = Path::new(raw_path);
    Some(if path.is_absolute() {
        path.to_path_buf()
    } else {
        project_root.join(path)
    })
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

fn collect_qa_visual_targets(value: &Value, targets: &mut Vec<Value>) {
    let Some(object) = value.as_object() else {
        return;
    };
    if let Some(qa_id) = object.get("qa_id").and_then(Value::as_str) {
        let qa_id = qa_id.trim();
        if !qa_id.is_empty() {
            let locator = if qa_id.starts_with("qa:") {
                qa_id.to_string()
            } else {
                format!("qa:{qa_id}")
            };
            let mut target = Map::from_iter([
                ("target".to_string(), json!(locator)),
                ("target_id".to_string(), json!(locator)),
                ("qa_id".to_string(), json!(qa_id)),
            ]);
            if let Some(path) = object.get("path").and_then(Value::as_str) {
                target.insert("path".to_string(), json!(path));
            }
            if let Some(visible) = object.get("visible").and_then(Value::as_bool) {
                target.insert("visible".to_string(), json!(visible));
            }
            if let Some(evidence) = object.get("visual_evidence") {
                target.insert("visual_evidence".to_string(), evidence.clone());
            }
            targets.push(Value::Object(target));
        }
    }
    if let Some(children) = object.get("children").and_then(Value::as_array) {
        for child in children {
            collect_qa_visual_targets(child, targets);
        }
    }
    if let Some(root) = object.get("root") {
        collect_qa_visual_targets(root, targets);
    }
}

fn evaluate_screenshot_state_guards(
    config: &ProjectConfig,
    command: &str,
    guards: &ScreenshotStateGuards,
) -> Result<Value, CommandResult> {
    if guards.is_empty() {
        return Ok(Value::Null);
    }

    let mut checks = Vec::new();
    for target in guards.require_visible.iter().map(String::as_str) {
        let target = target.trim();
        if target.is_empty() {
            continue;
        }
        let result = runtime::run_live_protocol_command(
            config,
            command,
            runtime::ASSERT_VISIBLE,
            Map::from_iter([("target".to_string(), json!(target))]),
        );
        let check = json!({
            "type": "require_visible",
            "target": target,
            "ok": result.ok,
            "data": result.data,
            "failures": result.failures,
        });
        if !result.ok {
            checks.push(check);
            return Err(screenshot_state_guard_failure(
                config, command, guards, checks,
            ));
        }
        checks.push(check);
    }

    for target in guards.require_hidden.iter().map(String::as_str) {
        let target = target.trim();
        if target.is_empty() {
            continue;
        }
        let result = runtime::run_live_protocol_command(
            config,
            command,
            runtime::ASSERT_HIDDEN,
            Map::from_iter([("target".to_string(), json!(target))]),
        );
        let check = json!({
            "type": "require_hidden",
            "target": target,
            "ok": result.ok,
            "data": result.data,
            "failures": result.failures,
        });
        if !result.ok {
            checks.push(check);
            return Err(screenshot_state_guard_failure(
                config, command, guards, checks,
            ));
        }
        checks.push(check);
    }

    if let Some(require_state) = &guards.require_state {
        let Some(params) = require_state.as_object() else {
            return Err(screenshot_args_invalid(
                command,
                &format!("{command} failed"),
                "screenshot_state_guard_args_invalid",
                "screenshot state guard requires --require-state to be a JSON object",
                require_state.clone(),
                json!("state_matches JSON object"),
            ));
        };
        let result = runtime::run_live_protocol_command(
            config,
            command,
            runtime::ASSERT_STATE_MATCHES,
            params.clone(),
        );
        let check = json!({
            "type": "require_state",
            "ok": result.ok,
            "data": result.data,
            "failures": result.failures,
        });
        if !result.ok {
            checks.push(check);
            return Err(screenshot_state_guard_failure(
                config, command, guards, checks,
            ));
        }
        checks.push(check);
    }

    Ok(json!({
        "ok": true,
        "requirements": guards.requirements_json(),
        "checks": checks,
    }))
}

fn screenshot_state_guard_failure(
    config: &ProjectConfig,
    command: &str,
    guards: &ScreenshotStateGuards,
    checks: Vec<Value>,
) -> CommandResult {
    let context = screenshot_guard_context(config, command);
    let actual = json!({
        "requirements": guards.requirements_json(),
        "checks": checks.clone(),
        "context": context,
    });
    CommandResult {
        ok: false,
        command: command.to_string(),
        message: format!("{command} state guard failed"),
        data: Map::from_iter([
            ("state_guards".to_string(), guards.requirements_json()),
            ("guard_checks".to_string(), json!(checks)),
            ("context".to_string(), context),
        ]),
        failures: vec![FailureDetail {
            kind: "screenshot_state_guard_failed".to_string(),
            command: Some(command.to_string()),
            message: Some(
                "screenshot state guard failed before capture; no new screenshot was accepted"
                    .to_string(),
            ),
            expected: Some(json!("all screenshot state guards pass before capture")),
            actual: Some(actual),
            severity: Some("error".to_string()),
            ..FailureDetail::default()
        }],
        exit_code: 1,
    }
}

fn screenshot_guard_context(config: &ProjectConfig, command: &str) -> Value {
    let status =
        runtime::run_live_protocol_command(config, command, runtime::SESSION_STATUS, Map::new());
    let tree = runtime::run_live_protocol_command(
        config,
        command,
        runtime::TREE_DUMP,
        Map::from_iter([
            ("qa_only".to_string(), json!(true)),
            ("include_metadata".to_string(), json!(true)),
        ]),
    );
    let mut targets = Vec::new();
    if tree.ok {
        collect_qa_visual_targets(&Value::Object(tree.data.clone()), &mut targets);
    }
    json!({
        "status_ok": status.ok,
        "scene": status.data.get("runtime").and_then(|runtime| runtime.get("scene")).cloned(),
        "visible_qa_targets": targets
            .into_iter()
            .filter(|target| target.get("visible").and_then(Value::as_bool).unwrap_or(false))
            .collect::<Vec<_>>(),
        "status_failures": status.failures,
        "tree_ok": tree.ok,
        "tree_failures": tree.failures,
    })
}

fn metrics_path_for_diff(diff_path: &Path) -> PathBuf {
    match (
        diff_path.parent(),
        diff_path.file_stem(),
        diff_path
            .extension()
            .and_then(|extension| extension.to_str()),
    ) {
        (Some(parent), Some(stem), Some(_)) => {
            parent.join(format!("{}.metrics.json", stem.to_string_lossy()))
        }
        (Some(parent), Some(stem), None) => {
            parent.join(format!("{}.metrics.json", stem.to_string_lossy()))
        }
        _ => diff_path.with_extension("metrics.json"),
    }
}

fn allocate_compare_root(artifacts_dir: &Path) -> PathBuf {
    let scenario_root = artifacts_dir.join("screenshot-compare");
    let _ = fs::create_dir_all(&scenario_root);
    let base_run_id = default_run_id();
    let mut candidate = base_run_id.clone();
    let mut suffix = 1u32;
    loop {
        let root = scenario_root.join(&candidate);
        match fs::create_dir(&root) {
            Ok(()) => return root,
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                candidate = format!("{base_run_id}-{suffix:02}");
                suffix += 1;
            }
            Err(_) => return root,
        }
    }
}

fn default_run_id() -> String {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs().to_string())
        .unwrap_or_else(|_| "0".to_string())
}

fn capture_label_from_path(path: &Path) -> String {
    let stem = path
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or("current")
        .trim()
        .to_lowercase();
    if stem.is_empty() {
        return "current".to_string();
    }
    let normalized: String = stem
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || character == '_' {
                character
            } else {
                '-'
            }
        })
        .collect();
    let label = normalized
        .split('-')
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("-");
    if label.is_empty() {
        "current".to_string()
    } else {
        label
    }
}

pub fn capture_live_screenshot(
    config: &ProjectConfig,
    command: &str,
    label: String,
) -> Result<LiveScreenshotCapture, CommandResult> {
    let result = runtime::run_live_screenshot_protocol_command(
        config,
        command,
        runtime::SCREENSHOT_CAPTURE,
        Map::from_iter([("name".to_string(), json!(label))]),
    );
    if !result.ok {
        return Err(result);
    }
    live_screenshot_capture_from_response(command, result.data)
}

fn live_screenshot_capture_from_response(
    command: &str,
    mut data: Map<String, Value>,
) -> Result<LiveScreenshotCapture, CommandResult> {
    let Some(artifacts) = data.get("artifacts").and_then(Value::as_object) else {
        return Err(protocol_error(
            command,
            "screenshot capture failed",
            "live screenshot response must include an artifacts object",
            json!("artifacts object with screenshot path"),
            json!("missing"),
        ));
    };
    let Some(screenshot_raw) = artifacts.get("screenshot").and_then(Value::as_str) else {
        return Err(protocol_error(
            command,
            "screenshot capture failed",
            "live screenshot response must include a screenshot artifact path",
            json!("string artifacts.screenshot path"),
            json!("missing"),
        ));
    };
    let screenshot_path = PathBuf::from(screenshot_raw);
    if !screenshot_path.is_file() {
        return Err(CommandResult {
            ok: false,
            command: command.to_string(),
            message: "screenshot capture failed".to_string(),
            data: Map::new(),
            failures: vec![FailureDetail {
                kind: "screenshot_file_missing".to_string(),
                command: Some(command.to_string()),
                path: Some(screenshot_path.to_string_lossy().into_owned()),
                message: Some("live screenshot artifact file was not found".to_string()),
                expected: Some(json!("existing staged screenshot artifact file")),
                actual: Some(json!("missing")),
                ..FailureDetail::default()
            }],
            exit_code: 1,
        });
    }
    let engine_frame = data.get("engine_frame").and_then(json_u64);
    let time_msec = data.get("time_msec").and_then(json_u64);
    data.remove("artifacts");
    Ok(LiveScreenshotCapture {
        screenshot_path,
        engine_frame,
        time_msec,
        metadata: data,
    })
}

fn json_u64(value: &Value) -> Option<u64> {
    value
        .as_u64()
        .or_else(|| value.as_i64().and_then(|value| u64::try_from(value).ok()))
}

fn validate_live_screenshot_quality(
    command: &str,
    message: &str,
    screenshot_path: &Path,
    output_path: Option<&Path>,
) -> Result<Value, CommandResult> {
    match analyze_screenshot_file(screenshot_path) {
        Ok(quality) => {
            if quality
                .get("low_information")
                .and_then(Value::as_bool)
                .unwrap_or(false)
            {
                Err(screenshot_low_information_result(
                    command,
                    message,
                    screenshot_path,
                    output_path,
                    quality,
                ))
            } else {
                Ok(quality)
            }
        }
        Err(error) => Err(CommandResult {
            ok: false,
            command: command.to_string(),
            message: message.to_string(),
            data: live_screenshot_quality_data(screenshot_path, output_path, None),
            failures: vec![FailureDetail {
                kind: "image_decode_failed".to_string(),
                command: Some(command.to_string()),
                path: Some(screenshot_path.to_string_lossy().into_owned()),
                message: Some(error),
                expected: Some(json!("decodable PNG screenshot artifact")),
                actual: Some(json!("decode_failed")),
                artifacts: Some(Map::from_iter([(
                    "screenshot".to_string(),
                    json!(screenshot_path.to_string_lossy().into_owned()),
                )])),
                ..FailureDetail::default()
            }],
            exit_code: 1,
        }),
    }
}

fn screenshot_low_information_result(
    command: &str,
    message: &str,
    screenshot_path: &Path,
    output_path: Option<&Path>,
    quality: Value,
) -> CommandResult {
    CommandResult {
        ok: false,
        command: command.to_string(),
        message: message.to_string(),
        data: live_screenshot_quality_data(screenshot_path, output_path, Some(quality.clone())),
        failures: vec![FailureDetail {
            kind: "screenshot_low_information".to_string(),
            command: Some(command.to_string()),
            path: Some(screenshot_path.to_string_lossy().into_owned()),
            message: Some(
                "screenshot appears blank, too small, or visually low-information".to_string(),
            ),
            expected: Some(json!("screenshot with useful visual evidence")),
            actual: Some(quality),
            severity: Some("error".to_string()),
            artifacts: Some(Map::from_iter([(
                "screenshot".to_string(),
                json!(screenshot_path.to_string_lossy().into_owned()),
            )])),
            ..FailureDetail::default()
        }],
        exit_code: 1,
    }
}

fn live_screenshot_quality_data(
    screenshot_path: &Path,
    output_path: Option<&Path>,
    quality: Option<Value>,
) -> Map<String, Value> {
    let mut data = Map::from_iter([(
        "artifacts".to_string(),
        json!({
            "screenshot": screenshot_path.to_string_lossy().into_owned(),
        }),
    )]);
    if let Some(output_path) = output_path {
        data.insert(
            "output".to_string(),
            json!(output_path.to_string_lossy().into_owned()),
        );
    }
    if let Some(quality) = quality {
        data.insert("screenshot_quality".to_string(), quality);
    }
    data
}

fn protocol_error(
    command: &str,
    message: &str,
    failure_message: &str,
    expected: Value,
    actual: Value,
) -> CommandResult {
    CommandResult {
        ok: false,
        command: command.to_string(),
        message: message.to_string(),
        data: Map::new(),
        failures: vec![FailureDetail {
            kind: "protocol_error".to_string(),
            command: Some(command.to_string()),
            message: Some(failure_message.to_string()),
            expected: Some(expected),
            actual: Some(actual),
            ..FailureDetail::default()
        }],
        exit_code: 1,
    }
}

fn capture_artifact_write_failure(
    command: &str,
    path: &Path,
    error: std::io::Error,
) -> CommandResult {
    CommandResult {
        ok: false,
        command: command.to_string(),
        message: "screenshot capture failed".to_string(),
        data: Map::new(),
        failures: vec![FailureDetail {
            kind: "artifact_write_failed".to_string(),
            command: Some(command.to_string()),
            path: Some(path.to_string_lossy().into_owned()),
            message: Some(error.to_string()),
            expected: Some(json!("writable output path for copied screenshot artifact")),
            actual: Some(json!("OSError")),
            ..FailureDetail::default()
        }],
        exit_code: 1,
    }
}

#[derive(Debug, Clone)]
struct ScreenshotSequencePlan {
    mode: &'static str,
    frame_count_requested: u64,
    capture_every_frames: u64,
    capture_frames: Vec<u64>,
    seconds: Option<f64>,
    fps: Option<f64>,
}

fn screenshot_sequence_plan(
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

fn screenshot_sequence_quality_data(
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

fn screenshot_sequence_manifest(
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

fn screenshot_sequence_low_information_failure(
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

fn write_json_file(path: &Path, value: &Value) -> Result<(), std::io::Error> {
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

fn args_invalid(command: &str, message: &str, actual: Value, expected: Value) -> CommandResult {
    screenshot_args_invalid(
        command,
        "screenshot comparison failed",
        "screenshot_compare_args_invalid",
        message,
        actual,
        expected,
    )
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

fn screenshot_args_invalid(
    command: &str,
    result_message: &str,
    kind: &str,
    message: &str,
    actual: Value,
    expected: Value,
) -> CommandResult {
    CommandResult {
        ok: false,
        command: command.to_string(),
        message: result_message.to_string(),
        data: Map::new(),
        failures: vec![FailureDetail {
            kind: kind.to_string(),
            command: Some(command.to_string()),
            message: Some(message.to_string()),
            expected: Some(expected),
            actual: Some(actual),
            ..FailureDetail::default()
        }],
        exit_code: 1,
    }
}

#[cfg(test)]
mod tests {
    use super::{
        ScreenshotSequenceOptions, ScreenshotStateGuards, analyze_screenshot_file,
        capture_live_screenshot, compare_presented_startup_files, compare_screenshot_files,
        live_screenshot_capture_from_response, screenshot_low_information_result,
        screenshot_sequence_low_information_failure, screenshot_sequence_manifest,
        screenshot_sequence_plan, screenshot_sequence_quality_data,
        validate_live_screenshot_quality, write_json_file,
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

        let result =
            capture_live_screenshot(&config, "screenshot compare", "main_menu".to_string())
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
        let exact =
            compare_screenshot_files("screenshot compare", &actual, &baseline, 0.0, None, None);
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
}

#[cfg(test)]
#[path = "screenshots_analysis_tests.rs"]
mod analysis_tests;
