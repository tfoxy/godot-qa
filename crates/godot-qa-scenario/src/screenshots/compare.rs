use super::analysis::load_rgba_image;
use super::capture::{capture_label_from_path, validate_live_screenshot_quality};
use super::output_paths::resolve_input_path;
use super::{
    ScreenshotAnalysis, ScreenshotAnalysisCache, args_invalid, capture_live_screenshot,
    output_paths,
};
use godot_qa_core::config::ProjectConfig;
use godot_qa_core::results::{CommandResult, FailureDetail};
use image::{ImageBuffer, Rgba, RgbaImage};
use serde_json::{Map, Value, json};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

const MAX_COMPARISON_PIXELS: u64 = 64_000_000;

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
                    | "screenshot_compare_resource_limit"
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

/// Retain a raw diff for review even when the comparison passes.
pub(crate) fn compare_screenshot_files_for_review(
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
        ScreenshotComparison {
            always_write_diff: true,
            ..ScreenshotComparison::default()
        },
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
            ..ScreenshotComparison::default()
        },
    )
}

#[derive(Default)]
struct ScreenshotComparison<'a> {
    always_write_diff: bool,
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

    // Validate every destination before either metrics or the diff can be written.
    for output in [diff_path, metrics_path].into_iter().flatten() {
        for input in [actual_path, baseline_path] {
            match output_paths::aliases_input(output, input) {
                Ok(false) => {}
                Ok(true) => {
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
                            kind: "screenshot_output_alias".to_string(),
                            command: Some(command.to_string()),
                            path: Some(output.to_string_lossy().into_owned()),
                            message: Some(
                                "comparison output aliases an input screenshot".to_string(),
                            ),
                            expected: Some(json!(
                                "diff and metrics paths distinct from both input files"
                            )),
                            actual: Some(json!({"output": output, "input": input})),
                            ..FailureDetail::default()
                        }],
                    );
                }
                Err(error) => {
                    return artifact_write_failure(
                        command,
                        actual_path,
                        baseline_path,
                        threshold,
                        output,
                        error,
                    );
                }
            }
        }
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

    // Mismatched thin inputs can have a huge union canvas despite each input
    // being small. Bound both the comparison scan and the RGBA diff allocation.
    let width = actual_image.width().max(baseline_image.width());
    let height = actual_image.height().max(baseline_image.height());
    let pixels = u64::from(width) * u64::from(height);
    if pixels > MAX_COMPARISON_PIXELS {
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
                kind: "screenshot_compare_resource_limit".to_string(),
                command: Some(command.to_string()),
                path: Some(actual_path.to_string_lossy().into_owned()),
                message: Some("comparison canvas exceeds the screenshot pixel limit".to_string()),
                expected: Some(json!({"max_comparison_pixels": MAX_COMPARISON_PIXELS})),
                actual: Some(json!({
                    "actual_size": [actual_image.width(), actual_image.height()],
                    "baseline_size": [baseline_image.width(), baseline_image.height()],
                    "comparison_size": [width, height],
                    "comparison_pixels": pixels,
                })),
                ..FailureDetail::default()
            }],
        );
    }

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

    if !ok || channel_tolerance.is_some() || comparison.always_write_diff {
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
