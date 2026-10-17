use crate::{contact_sheet, screenshots};
use godot_qa_core::config::ProjectConfig;
use godot_qa_core::results::{CommandResult, FailureDetail};
use godot_qa_runtime as runtime;
use image::{GenericImageView, ImageBuffer, Rgba};
use serde_json::{Map, Value, json};
use std::fs;
use std::path::{Path, PathBuf};

const COMMAND: &str = "visual isolate";

#[derive(Debug, Clone)]
pub struct VisualIsolateOptions {
    pub out: String,
    pub provider: String,
    pub driver: String,
    pub action: String,
    pub layers: Vec<String>,
    pub columns: i64,
    pub thumb_width: i64,
    pub contact_sheet_out: Option<String>,
}

pub fn isolate_visual_layers(
    config: &ProjectConfig,
    options: VisualIsolateOptions,
) -> CommandResult {
    let out_dir = resolve_output_path(&config.project_root, &options.out);
    if options.out.trim().is_empty() {
        return invalid_params(
            "--out",
            json!(options.out),
            json!("non-empty output directory"),
        );
    }
    if options.provider.trim().is_empty()
        || options.driver.trim().is_empty()
        || options.action.trim().is_empty()
    {
        return invalid_params(
            "provider/driver/action",
            json!({"provider": options.provider, "driver": options.driver, "action": options.action}),
            json!("non-empty provider, driver, and action"),
        );
    }

    let state = match read_visual_layer_state(config, &options.provider) {
        Ok(state) => state,
        Err(result) => return result,
    };
    let layer_ids = selected_layer_ids(&state, &options.layers);
    if layer_ids.is_empty() {
        return invalid_params(
            "--layer",
            json!({"requested": options.layers, "provider_state": state}),
            json!("one or more visual layer ids"),
        );
    }

    let frames_dir = out_dir.join("frames");
    if let Err(error) = fs::create_dir_all(&frames_dir) {
        return write_failure(&frames_dir, error);
    }

    if let Err(result) = set_visual_layer_mode(config, &options, "normal", None) {
        return result;
    }
    let baseline_path = frames_dir.join("baseline.png");
    let baseline = match capture_named_frame(config, "baseline", &baseline_path) {
        Ok(capture) => capture,
        Err(result) => return result,
    };

    let mut frames = vec![manifest_frame(0, "baseline", &baseline_path, &baseline)];
    let mut layer_reports = Vec::new();
    let mut frame_index = 1_i64;

    for layer_id in &layer_ids {
        if let Err(result) = set_visual_layer_mode(config, &options, "only", Some(layer_id)) {
            return result;
        }
        let only_path = frames_dir.join(format!("{layer_id}_only.png"));
        let only = match capture_named_frame(config, &format!("{layer_id}_only"), &only_path) {
            Ok(capture) => capture,
            Err(result) => return result,
        };
        frames.push(manifest_frame(
            frame_index,
            &format!("{layer_id}_only"),
            &only_path,
            &only,
        ));
        frame_index += 1;

        if let Err(result) = set_visual_layer_mode(config, &options, "without", Some(layer_id)) {
            return result;
        }
        let without_path = frames_dir.join(format!("{layer_id}_without.png"));
        let without =
            match capture_named_frame(config, &format!("{layer_id}_without"), &without_path) {
                Ok(capture) => capture,
                Err(result) => return result,
            };
        frames.push(manifest_frame(
            frame_index,
            &format!("{layer_id}_without"),
            &without_path,
            &without,
        ));
        frame_index += 1;

        let diff_path = frames_dir.join(format!("{layer_id}_diff.png"));
        let diff = match write_diff_image(&baseline_path, &without_path, &diff_path) {
            Ok(diff) => diff,
            Err(result) => return result,
        };
        let diff_frame = image_manifest_frame(
            frame_index,
            &format!("{layer_id}_diff"),
            &diff_path,
            diff.get("width"),
            diff.get("height"),
        );
        frames.push(diff_frame);
        frame_index += 1;
        layer_reports.push(json!({
            "layer": layer_id,
            "only": path_string(&only_path),
            "without": path_string(&without_path),
            "diff": diff,
        }));
    }

    if let Err(result) = set_visual_layer_mode(config, &options, "normal", None) {
        return result;
    }

    let manifest_path = out_dir.join("manifest.json");
    let manifest = json!({
        "command": "visual.isolate",
        "provider": options.provider,
        "driver": options.driver,
        "action": options.action,
        "layers": layer_ids,
        "provider_state": state,
        "captured_count": frames.len(),
        "frames": frames,
        "layer_reports": layer_reports,
        "no_visible_debug_overlay": true,
    });
    if let Err(error) = write_json_file(&manifest_path, &manifest) {
        return write_failure(&manifest_path, error);
    }

    let contact_sheet_out = options.contact_sheet_out.clone().unwrap_or_else(|| {
        out_dir
            .join("contact_sheet.png")
            .to_string_lossy()
            .into_owned()
    });
    let contact_result = contact_sheet::build_contact_sheet(
        config,
        contact_sheet::ScreenshotContactSheetOptions {
            sequence_dir: out_dir.to_string_lossy().into_owned(),
            columns: options.columns,
            thumb_width: options.thumb_width,
            out: contact_sheet_out,
        },
    );
    if !contact_result.ok {
        return contact_result;
    }

    CommandResult {
        ok: true,
        command: COMMAND.to_string(),
        message: "visual isolation captured".to_string(),
        data: Map::from_iter([
            ("out".to_string(), json!(path_string(&out_dir))),
            ("provider".to_string(), json!(options.provider)),
            ("driver".to_string(), json!(options.driver)),
            ("manifest".to_string(), json!(path_string(&manifest_path))),
            ("layers".to_string(), json!(layer_reports)),
            (
                "contact_sheet".to_string(),
                contact_result
                    .data
                    .get("artifacts")
                    .cloned()
                    .unwrap_or_else(|| json!({})),
            ),
            (
                "artifacts".to_string(),
                json!({
                    "sequence_dir": path_string(&out_dir),
                    "frames_dir": path_string(&frames_dir),
                    "manifest": path_string(&manifest_path),
                    "contact_sheet": contact_result.data.get("artifacts").and_then(|artifacts| artifacts.get("contact_sheet")).cloned(),
                }),
            ),
        ]),
        failures: Vec::new(),
        exit_code: 0,
    }
}

fn read_visual_layer_state(config: &ProjectConfig, provider: &str) -> Result<Value, CommandResult> {
    let result = runtime::run_live_protocol_command(
        config,
        COMMAND,
        runtime::QA_STATE_READ,
        Map::from_iter([("provider".to_string(), json!(provider))]),
    );
    if !result.ok {
        return Err(result);
    }
    Ok(result.data.get("state").cloned().unwrap_or(Value::Null))
}

fn selected_layer_ids(state: &Value, requested: &[String]) -> Vec<String> {
    let requested = requested
        .iter()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .collect::<Vec<_>>();
    if !requested.is_empty() {
        return requested;
    }
    state
        .get("layers")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|layer| {
            layer
                .get("id")
                .or_else(|| layer.get("name"))
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(ToString::to_string)
        })
        .collect()
}

fn set_visual_layer_mode(
    config: &ProjectConfig,
    options: &VisualIsolateOptions,
    mode: &str,
    layer: Option<&str>,
) -> Result<(), CommandResult> {
    let mut params = Map::from_iter([("mode".to_string(), json!(mode))]);
    if let Some(layer) = layer {
        params.insert("layer".to_string(), json!(layer));
    }
    let result = runtime::run_live_protocol_command(
        config,
        COMMAND,
        runtime::FIXTURE_CALL,
        Map::from_iter([
            ("driver".to_string(), json!(options.driver)),
            ("action".to_string(), json!(options.action)),
            ("params".to_string(), Value::Object(params)),
        ]),
    );
    if result.ok { Ok(()) } else { Err(result) }
}

fn capture_named_frame(
    config: &ProjectConfig,
    label: &str,
    output_path: &Path,
) -> Result<screenshots::LiveScreenshotCapture, CommandResult> {
    let capture = screenshots::capture_live_screenshot(config, COMMAND, label.to_string())?;
    if let Some(parent) = output_path.parent()
        && let Err(error) = fs::create_dir_all(parent)
    {
        return Err(write_failure(output_path, error));
    }
    if let Err(error) = fs::copy(&capture.screenshot_path, output_path) {
        return Err(write_failure(output_path, error));
    }
    Ok(capture)
}

fn manifest_frame(
    index: i64,
    label: &str,
    output_path: &Path,
    capture: &screenshots::LiveScreenshotCapture,
) -> Value {
    let (width, height) = image_dimensions(output_path);
    json!({
        "index": index,
        "label": label,
        "engine_frame": capture.engine_frame,
        "time_msec": capture.time_msec,
        "path": relative_frame_path(output_path),
        "width": width,
        "height": height,
    })
}

fn image_manifest_frame(
    index: i64,
    label: &str,
    output_path: &Path,
    width: Option<&Value>,
    height: Option<&Value>,
) -> Value {
    json!({
        "index": index,
        "label": label,
        "path": relative_frame_path(output_path),
        "width": width.and_then(Value::as_u64).unwrap_or(0),
        "height": height.and_then(Value::as_u64).unwrap_or(0),
    })
}

fn write_diff_image(
    baseline_path: &Path,
    without_path: &Path,
    diff_path: &Path,
) -> Result<Value, CommandResult> {
    let baseline =
        image::open(baseline_path).map_err(|error| image_failure(baseline_path, error))?;
    let without = image::open(without_path).map_err(|error| image_failure(without_path, error))?;
    if baseline.dimensions() != without.dimensions() {
        return Err(invalid_params(
            "diff dimensions",
            json!({"baseline": baseline.dimensions(), "without": without.dimensions()}),
            json!("matching image dimensions"),
        ));
    }
    let base = baseline.to_rgba8();
    let next = without.to_rgba8();
    let (width, height) = base.dimensions();
    let mut diff = ImageBuffer::from_pixel(width, height, Rgba([0_u8, 0_u8, 0_u8, 255_u8]));
    let mut changed = 0_u64;
    let mut min_x = width;
    let mut min_y = height;
    let mut max_x = 0_u32;
    let mut max_y = 0_u32;
    for y in 0..height {
        for x in 0..width {
            let a = base.get_pixel(x, y).0;
            let b = next.get_pixel(x, y).0;
            let delta = a
                .iter()
                .zip(b.iter())
                .map(|(left, right)| left.abs_diff(*right) as u16)
                .max()
                .unwrap_or(0);
            if delta > 12 {
                changed += 1;
                min_x = min_x.min(x);
                min_y = min_y.min(y);
                max_x = max_x.max(x);
                max_y = max_y.max(y);
                diff.put_pixel(x, y, Rgba([255_u8, 64_u8, 64_u8, 255_u8]));
            }
        }
    }
    if let Some(parent) = diff_path.parent()
        && let Err(error) = fs::create_dir_all(parent)
    {
        return Err(write_failure(diff_path, error));
    }
    if let Err(error) = diff.save(diff_path) {
        return Err(write_failure(
            diff_path,
            std::io::Error::other(error.to_string()),
        ));
    }
    let bounds = if changed == 0 {
        Value::Null
    } else {
        json!({"x": min_x, "y": min_y, "width": max_x - min_x + 1, "height": max_y - min_y + 1})
    };
    let changed_width_ratio = (max_x - min_x + 1) as f64 / width as f64;
    let changed_height_ratio = (max_y - min_y + 1) as f64 / height as f64;
    let rectangular_warning =
        changed > 0 && changed_height_ratio > 0.75 && !(0.35..=0.65).contains(&changed_width_ratio);
    Ok(json!({
        "path": path_string(diff_path),
        "width": width,
        "height": height,
        "changed_pixel_count": changed,
        "changed_pixel_ratio": changed as f64 / (width as f64 * height as f64),
        "changed_pixel_bounds": bounds,
        "straight_edge_or_rectangular_region_warning": rectangular_warning,
    }))
}

fn image_failure(path: &Path, error: image::ImageError) -> CommandResult {
    CommandResult {
        ok: false,
        command: COMMAND.to_string(),
        message: "visual isolate failed".to_string(),
        data: Map::new(),
        failures: vec![FailureDetail {
            kind: "image_decode_failed".to_string(),
            command: Some(COMMAND.to_string()),
            path: Some(path_string(path)),
            message: Some(error.to_string()),
            expected: Some(json!("decodable PNG screenshot artifact")),
            actual: Some(json!("decode_failed")),
            ..FailureDetail::default()
        }],
        exit_code: 1,
    }
}

fn image_dimensions(path: &Path) -> (u32, u32) {
    image::open(path)
        .map(|image| image.dimensions())
        .unwrap_or((0, 0))
}

fn relative_frame_path(path: &Path) -> String {
    path.file_name()
        .and_then(|name| name.to_str())
        .map(|name| format!("frames/{name}"))
        .unwrap_or_else(|| path_string(path))
}

fn resolve_output_path(project_root: &Path, raw: &str) -> PathBuf {
    let path = Path::new(raw.trim());
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        project_root.join(path)
    }
}

fn path_string(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

fn write_json_file(path: &Path, value: &Value) -> Result<(), std::io::Error> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, serde_json::to_vec_pretty(value)?)
}

fn write_failure(path: &Path, error: std::io::Error) -> CommandResult {
    CommandResult {
        ok: false,
        command: COMMAND.to_string(),
        message: "visual isolate failed".to_string(),
        data: Map::new(),
        failures: vec![FailureDetail {
            kind: "artifact_write_failed".to_string(),
            command: Some(COMMAND.to_string()),
            path: Some(path_string(path)),
            message: Some(error.to_string()),
            expected: Some(json!("writable visual isolation artifact path")),
            actual: Some(json!("OSError")),
            ..FailureDetail::default()
        }],
        exit_code: 1,
    }
}

fn invalid_params(field: &str, actual: Value, expected: Value) -> CommandResult {
    CommandResult {
        ok: false,
        command: COMMAND.to_string(),
        message: "visual isolate failed".to_string(),
        data: Map::new(),
        failures: vec![FailureDetail {
            kind: "invalid_params".to_string(),
            command: Some(COMMAND.to_string()),
            message: Some(format!("visual isolate invalid {field}")),
            expected: Some(expected),
            actual: Some(actual),
            ..FailureDetail::default()
        }],
        exit_code: 2,
    }
}
