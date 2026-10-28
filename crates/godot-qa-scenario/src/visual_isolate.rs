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

    let (frames, layer_reports) = match capture_isolation_frames(
        &mut LiveIsolationCommands {
            config,
            options: &options,
        },
        &frames_dir,
        &layer_ids,
    ) {
        Ok(captures) => captures,
        Err(result) => return result,
    };

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

// Keep all scene mutations inside this operation so every error path attempts cleanup.
trait IsolationCommands {
    fn set_mode(&mut self, mode: &str, layer: Option<&str>) -> Result<(), CommandResult>;
    fn capture(&mut self, index: i64, label: &str, path: &Path) -> Result<Value, CommandResult>;
    fn diff(
        &mut self,
        baseline: &Path,
        without: &Path,
        path: &Path,
    ) -> Result<Value, CommandResult>;
}

struct LiveIsolationCommands<'a> {
    config: &'a ProjectConfig,
    options: &'a VisualIsolateOptions,
}

impl IsolationCommands for LiveIsolationCommands<'_> {
    fn set_mode(&mut self, mode: &str, layer: Option<&str>) -> Result<(), CommandResult> {
        set_visual_layer_mode(self.config, self.options, mode, layer)
    }

    fn capture(&mut self, index: i64, label: &str, path: &Path) -> Result<Value, CommandResult> {
        let capture = capture_named_frame(self.config, label, path)?;
        Ok(manifest_frame(index, label, path, &capture))
    }

    fn diff(
        &mut self,
        baseline: &Path,
        without: &Path,
        path: &Path,
    ) -> Result<Value, CommandResult> {
        write_diff_image(baseline, without, path)
    }
}

fn capture_isolation_frames(
    commands: &mut impl IsolationCommands,
    frames_dir: &Path,
    layer_ids: &[String],
) -> Result<(Vec<Value>, Vec<Value>), CommandResult> {
    let outcome = (|| {
        // A failed fixture call may already have mutated the scene, so even this first
        // failure must go through the final normal-mode cleanup attempt.
        commands.set_mode("normal", None)?;
        let baseline_path = frames_dir.join("baseline.png");
        let baseline = commands.capture(0, "baseline", &baseline_path)?;
        let mut frames = vec![baseline];
        let mut layer_reports = Vec::new();
        let mut frame_index = 1_i64;

        for layer_id in layer_ids {
            commands.set_mode("only", Some(layer_id))?;
            let only_label = format!("{layer_id}_only");
            let only_path = frames_dir.join(format!("{only_label}.png"));
            frames.push(commands.capture(frame_index, &only_label, &only_path)?);
            frame_index += 1;

            commands.set_mode("without", Some(layer_id))?;
            let without_label = format!("{layer_id}_without");
            let without_path = frames_dir.join(format!("{without_label}.png"));
            frames.push(commands.capture(frame_index, &without_label, &without_path)?);
            frame_index += 1;

            let diff_label = format!("{layer_id}_diff");
            let diff_path = frames_dir.join(format!("{diff_label}.png"));
            let diff = commands.diff(&baseline_path, &without_path, &diff_path)?;
            frames.push(image_manifest_frame(
                frame_index,
                &diff_label,
                &diff_path,
                diff.get("width"),
                diff.get("height"),
            ));
            frame_index += 1;
            layer_reports.push(json!({
                "layer": layer_id,
                "only": path_string(&only_path),
                "without": path_string(&without_path),
                "diff": diff,
            }));
        }
        Ok((frames, layer_reports))
    })();

    match (outcome, commands.set_mode("normal", None)) {
        (outcome, Ok(())) => outcome,
        (Err(mut original), Err(cleanup)) => {
            original.failures.push(cleanup_failure(&cleanup));
            Err(original)
        }
        (Ok(_), Err(mut cleanup)) => {
            cleanup.failures.push(cleanup_failure(&cleanup));
            Err(cleanup)
        }
    }
}

fn cleanup_failure(result: &CommandResult) -> FailureDetail {
    FailureDetail {
        kind: "visual_isolation_cleanup_failed".to_string(),
        command: Some(COMMAND.to_string()),
        message: Some("could not restore visual isolation to normal mode".to_string()),
        expected: Some(json!({"mode": "normal"})),
        actual: Some(json!({
            "command": result.command,
            "message": result.message,
            "data": result.data,
            "failures": result.failures,
        })),
        ..FailureDetail::default()
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
    let rectangular_warning = if changed == 0 {
        false
    } else {
        let changed_width_ratio = (max_x - min_x + 1) as f64 / width as f64;
        let changed_height_ratio = (max_y - min_y + 1) as f64 / height as f64;
        changed_height_ratio > 0.75 && !(0.35..=0.65).contains(&changed_width_ratio)
    };
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    struct MockCommands {
        calls: Vec<String>,
        fail_at: Option<usize>,
        fail_cleanup: bool,
        mode: String,
    }

    impl MockCommands {
        fn step(&mut self, call: String) -> Result<(), CommandResult> {
            let index = self.calls.len();
            let cleanup = call == "mode:normal" && index > 0;
            self.calls.push(call.clone());
            if self.fail_at == Some(index) || (cleanup && self.fail_cleanup) {
                return Err(invalid_params(
                    "injected phase",
                    json!(call),
                    json!("success"),
                ));
            }
            Ok(())
        }
    }

    impl IsolationCommands for MockCommands {
        fn set_mode(&mut self, mode: &str, _layer: Option<&str>) -> Result<(), CommandResult> {
            // A primary fixture call may mutate before failing; failed cleanup
            // may instead leave the last isolation mode active.
            if !(mode == "normal" && !self.calls.is_empty() && self.fail_cleanup) {
                self.mode = mode.to_string();
            }
            self.step(format!("mode:{mode}"))
        }

        fn capture(
            &mut self,
            index: i64,
            label: &str,
            path: &Path,
        ) -> Result<Value, CommandResult> {
            self.step(format!("capture:{label}"))?;
            Ok(json!({"index": index, "label": label, "path": relative_frame_path(path)}))
        }

        fn diff(
            &mut self,
            _baseline: &Path,
            _without: &Path,
            path: &Path,
        ) -> Result<Value, CommandResult> {
            self.step("diff".to_string())?;
            Ok(json!({"path": path_string(path), "width": 8, "height": 8}))
        }
    }

    fn mock_commands(fail_at: Option<usize>, fail_cleanup: bool) -> MockCommands {
        MockCommands {
            calls: Vec::new(),
            fail_at,
            fail_cleanup,
            mode: "normal".to_string(),
        }
    }

    #[test]
    fn every_failed_capture_phase_attempts_normal_cleanup_and_preserves_primary_failure() {
        let phases = [
            "mode:normal",
            "capture:baseline",
            "mode:only",
            "capture:layer_only",
            "mode:without",
            "capture:layer_without",
            "diff",
        ];
        for (index, phase) in phases.iter().enumerate() {
            for fail_cleanup in [false, true] {
                let mut commands = mock_commands(Some(index), fail_cleanup);
                let result = capture_isolation_frames(
                    &mut commands,
                    Path::new("frames"),
                    &["layer".to_string()],
                )
                .unwrap_err();
                assert_eq!(
                    commands.calls.last().unwrap(),
                    "mode:normal",
                    "phase {phase}"
                );
                assert_eq!(commands.calls.len(), index + 2, "phase {phase}");
                if !fail_cleanup {
                    assert_eq!(commands.mode, "normal");
                } else if index >= 2 {
                    assert_ne!(commands.mode, "normal");
                }
                assert_eq!(result.failures[0].actual, Some(json!(phase)));
                assert_eq!(result.exit_code, 2);
                assert_eq!(result.failures.len(), if fail_cleanup { 2 } else { 1 });
                if fail_cleanup {
                    let cleanup = &result.failures[1];
                    assert_eq!(cleanup.kind, "visual_isolation_cleanup_failed");
                    assert_eq!(cleanup.expected, Some(json!({"mode": "normal"})));
                    assert_eq!(
                        cleanup.actual.as_ref().unwrap()["failures"][0]["actual"],
                        "mode:normal"
                    );
                }
            }
        }
    }

    #[test]
    fn successful_captures_restore_normal_before_artifact_finalization() {
        let mut commands = mock_commands(None, false);
        let (frames, reports) =
            capture_isolation_frames(&mut commands, Path::new("frames"), &["layer".to_string()])
                .unwrap();
        assert_eq!(commands.calls.len(), 8);
        assert_eq!(commands.calls.last().unwrap(), "mode:normal");
        assert_eq!(frames.len(), 4);
        assert_eq!(frames[0]["label"], "baseline");
        assert_eq!(frames[3]["label"], "layer_diff");
        assert_eq!(reports[0]["layer"], "layer");
    }

    #[test]
    fn cleanup_failure_prevents_success_and_retains_native_failure() {
        let mut commands = mock_commands(None, true);
        let result =
            capture_isolation_frames(&mut commands, Path::new("frames"), &["layer".to_string()])
                .unwrap_err();
        assert_eq!(result.failures[0].kind, "invalid_params");
        assert_eq!(result.failures[1].kind, "visual_isolation_cleanup_failed");
        assert!(!result.ok);
    }

    #[test]
    fn diff_handles_identical_images_and_preserves_changed_bounds_and_pixels() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "godot-qa-visual-isolate-{}-{unique}",
            std::process::id()
        ));
        fs::create_dir_all(&root).unwrap();
        let baseline_path = root.join("baseline.png");
        let without_path = root.join("without.png");
        let diff_path = root.join("diff.png");
        let baseline = ImageBuffer::from_pixel(4, 4, Rgba([20_u8, 30, 40, 255]));
        baseline.save(&baseline_path).unwrap();
        baseline.save(&without_path).unwrap();
        let identical = write_diff_image(&baseline_path, &without_path, &diff_path).unwrap();
        assert_eq!(identical["changed_pixel_count"], 0);
        assert_eq!(identical["changed_pixel_ratio"], 0.0);
        assert_eq!(identical["changed_pixel_bounds"], Value::Null);
        assert_eq!(
            identical["straight_edge_or_rectangular_region_warning"],
            false
        );
        let decoded = image::open(&diff_path).unwrap().to_rgba8();
        assert!(decoded.pixels().all(|pixel| pixel.0 == [0, 0, 0, 255]));

        let mut without = baseline;
        for y in 0..4 {
            without.put_pixel(3, y, Rgba([33, 30, 40, 255]));
        }
        // Differences at or below the existing threshold stay excluded.
        without.put_pixel(0, 0, Rgba([32, 30, 40, 255]));
        without.save(&without_path).unwrap();
        let changed = write_diff_image(&baseline_path, &without_path, &diff_path).unwrap();
        assert_eq!(changed["changed_pixel_count"], 4);
        assert_eq!(changed["changed_pixel_ratio"], 0.25);
        assert_eq!(
            changed["changed_pixel_bounds"],
            json!({"x": 3, "y": 0, "width": 1, "height": 4})
        );
        assert_eq!(changed["straight_edge_or_rectangular_region_warning"], true);
        let decoded = image::open(&diff_path).unwrap().to_rgba8();
        assert_eq!(decoded.get_pixel(3, 0).0, [255, 64, 64, 255]);
        assert_eq!(decoded.get_pixel(0, 0).0, [0, 0, 0, 255]);
        fs::remove_dir_all(root).unwrap();
    }
}
