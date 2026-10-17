use godot_qa_core::config::ProjectConfig;
use godot_qa_core::results::{CommandResult, FailureDetail};
use image::imageops::{FilterType, resize};
use image::{GenericImageView, ImageBuffer, Rgba};
use serde_json::{Map, Value, json};
use std::fs;
use std::path::{Path, PathBuf};

const COMMAND: &str = "screenshot contact-sheet";
const LABEL_MARGIN: u32 = 18;
const CELL_GAP: u32 = 8;
const PADDING: u32 = 8;
const LABEL_SCALE: u32 = 2;

#[derive(Debug, Clone)]
pub struct ScreenshotContactSheetOptions {
    pub sequence_dir: String,
    pub columns: i64,
    pub thumb_width: i64,
    pub out: String,
}

#[derive(Debug)]
struct ContactFrame {
    manifest_index: usize,
    index: Option<i64>,
    engine_frame: Option<i64>,
    time_msec: Option<i64>,
    manifest_path: String,
    source_path: PathBuf,
    source_width: u32,
    source_height: u32,
    thumb_width: u32,
    thumb_height: u32,
    label: String,
    pixels: ImageBuffer<Rgba<u8>, Vec<u8>>,
}

pub fn build_contact_sheet(
    config: &ProjectConfig,
    options: ScreenshotContactSheetOptions,
) -> CommandResult {
    if let Err(result) = validate_options(&options) {
        return result;
    }

    let sequence_dir = resolve_input_path(&config.project_root, &options.sequence_dir);
    let output_path = resolve_input_path(&config.project_root, &options.out);
    let companion_path = companion_json_path(&output_path);
    let manifest_path = sequence_dir.join("manifest.json");
    let columns = options.columns as u32;
    let thumb_width = options.thumb_width as u32;

    let manifest = match read_manifest(&manifest_path) {
        Ok(manifest) => manifest,
        Err(result) => return result,
    };
    let frames = match load_frames(&sequence_dir, &manifest, thumb_width) {
        Ok(frames) => frames,
        Err(result) => return result,
    };

    let rows = div_ceil(frames.len() as u32, columns);
    let max_thumb_height = frames
        .iter()
        .map(|frame| frame.thumb_height)
        .max()
        .unwrap_or(0);
    if max_thumb_height == 0 {
        return contact_failure(
            "screenshot_contact_sheet_invalid_dimensions",
            Some(&manifest_path),
            "screenshot contact-sheet requires frames with positive dimensions",
            json!("positive image dimensions"),
            json!({"thumbnail_height": max_thumb_height}),
        );
    }

    let cell_width = thumb_width;
    let cell_height = max_thumb_height + LABEL_MARGIN;
    let output_width = sheet_extent(columns, cell_width);
    let output_height = sheet_extent(rows, cell_height);
    let mut sheet =
        ImageBuffer::from_pixel(output_width, output_height, Rgba([245, 245, 245, 255]));
    let mut metadata_frames = Vec::new();

    for (position, frame) in frames.iter().enumerate() {
        let column = position as u32 % columns;
        let row = position as u32 / columns;
        let cell_x = PADDING + column * (cell_width + CELL_GAP);
        let cell_y = PADDING + row * (cell_height + CELL_GAP);
        let image_x = cell_x + (cell_width - frame.thumb_width) / 2;
        let image_y = cell_y + (max_thumb_height - frame.thumb_height) / 2;
        blit(&mut sheet, &frame.pixels, image_x, image_y);

        let label_x = cell_x;
        let label_y = cell_y + max_thumb_height + 4;
        draw_label_text(
            &mut sheet,
            label_x,
            label_y,
            &frame.label,
            Rgba([20, 20, 20, 255]),
        );

        metadata_frames.push(json!({
            "manifest_index": frame.manifest_index,
            "index": frame.index,
            "engine_frame": frame.engine_frame,
            "time_msec": frame.time_msec,
            "source_path": frame.source_path.to_string_lossy().into_owned(),
            "manifest_path": frame.manifest_path,
            "source_size": {"width": frame.source_width, "height": frame.source_height},
            "cell": {
                "column": column,
                "row": row,
                "x": cell_x,
                "y": cell_y,
                "width": cell_width,
                "height": cell_height
            },
            "thumbnail": {
                "x": image_x,
                "y": image_y,
                "width": frame.thumb_width,
                "height": frame.thumb_height
            },
            "label": {
                "x": label_x,
                "y": label_y,
                "height": LABEL_MARGIN,
                "text": frame.label
            }
        }));
    }

    if let Some(parent) = output_path.parent()
        && let Err(error) = fs::create_dir_all(parent)
    {
        return write_failure(&output_path, error);
    }
    if let Err(error) = sheet.save(&output_path) {
        return image_write_failure(&output_path, error);
    }

    let metadata = json!({
        "command": "screenshot.contact_sheet",
        "sequence_dir": sequence_dir.to_string_lossy().into_owned(),
        "manifest": manifest_path.to_string_lossy().into_owned(),
        "output": output_path.to_string_lossy().into_owned(),
        "companion_json": companion_path.to_string_lossy().into_owned(),
        "columns": columns,
        "rows": rows,
        "thumbnail_size": {"width": thumb_width, "height": max_thumb_height},
        "label_margin": LABEL_MARGIN,
        "cell_gap": CELL_GAP,
        "output_size": {"width": output_width, "height": output_height},
        "frames": metadata_frames,
    });

    if let Err(error) = write_json_file(&companion_path, &metadata) {
        return write_failure(&companion_path, error);
    }

    CommandResult {
        ok: true,
        command: COMMAND.to_string(),
        message: "screenshot contact-sheet generated".to_string(),
        data: Map::from_iter([
            (
                "output".to_string(),
                json!(output_path.to_string_lossy().into_owned()),
            ),
            (
                "companion_json".to_string(),
                json!(companion_path.to_string_lossy().into_owned()),
            ),
            ("columns".to_string(), json!(columns)),
            ("rows".to_string(), json!(rows)),
            (
                "thumbnail_size".to_string(),
                json!({"width": thumb_width, "height": max_thumb_height}),
            ),
            (
                "output_size".to_string(),
                json!({"width": output_width, "height": output_height}),
            ),
            (
                "artifacts".to_string(),
                json!({
                    "contact_sheet": output_path.to_string_lossy().into_owned(),
                    "metadata": companion_path.to_string_lossy().into_owned(),
                }),
            ),
        ]),
        failures: Vec::new(),
        exit_code: 0,
    }
}

pub fn draw_label_text(
    image: &mut ImageBuffer<Rgba<u8>, Vec<u8>>,
    x: u32,
    y: u32,
    text: &str,
    color: Rgba<u8>,
) {
    let mut cursor_x = x;
    for ch in text.chars() {
        if ch == ' ' {
            cursor_x += 4 * LABEL_SCALE;
            continue;
        }
        if let Some(glyph) = glyph(ch) {
            draw_glyph(image, cursor_x, y, glyph, color);
        }
        cursor_x += 4 * LABEL_SCALE;
    }
}

fn validate_options(options: &ScreenshotContactSheetOptions) -> Result<(), CommandResult> {
    if options.sequence_dir.trim().is_empty() {
        return Err(args_invalid(
            "screenshot contact-sheet requires a sequence directory",
            json!(options.sequence_dir),
            json!("non-empty sequence directory"),
        ));
    }
    if options.columns <= 0 {
        return Err(args_invalid(
            "screenshot contact-sheet requires --columns > 0",
            json!(options.columns),
            json!("positive integer columns"),
        ));
    }
    if options.thumb_width <= 0 {
        return Err(args_invalid(
            "screenshot contact-sheet requires --thumb-width > 0",
            json!(options.thumb_width),
            json!("positive integer thumb width"),
        ));
    }
    if options.out.trim().is_empty() {
        return Err(args_invalid(
            "screenshot contact-sheet requires --out",
            json!(options.out),
            json!("non-empty output path"),
        ));
    }
    Ok(())
}

fn read_manifest(path: &Path) -> Result<Value, CommandResult> {
    let bytes = fs::read(path).map_err(|error| {
        contact_result_from_detail(contact_failure_detail(
            "screenshot_contact_sheet_manifest_missing",
            Some(path),
            &format!("failed to read sequence manifest: {error}"),
            json!("readable <sequence-dir>/manifest.json"),
            json!("missing or unreadable manifest"),
        ))
    })?;
    serde_json::from_slice(&bytes).map_err(|error| {
        contact_result_from_detail(contact_failure_detail(
            "screenshot_contact_sheet_manifest_malformed",
            Some(path),
            &format!("failed to parse sequence manifest JSON: {error}"),
            json!("valid sequence manifest JSON"),
            json!("malformed JSON"),
        ))
    })
}

fn load_frames(
    sequence_dir: &Path,
    manifest: &Value,
    thumb_width: u32,
) -> Result<Vec<ContactFrame>, CommandResult> {
    let Some(frames) = manifest.get("frames").and_then(Value::as_array) else {
        return Err(contact_result_from_detail(contact_failure_detail(
            "screenshot_contact_sheet_manifest_malformed",
            Some(&sequence_dir.join("manifest.json")),
            "sequence manifest must contain a frames array",
            json!("manifest.frames array"),
            manifest.get("frames").cloned().unwrap_or(Value::Null),
        )));
    };
    if frames.is_empty() {
        return Err(contact_result_from_detail(contact_failure_detail(
            "screenshot_contact_sheet_manifest_malformed",
            Some(&sequence_dir.join("manifest.json")),
            "sequence manifest must contain at least one frame",
            json!("one or more frames"),
            json!(0),
        )));
    }

    let mut loaded = Vec::new();
    for (manifest_index, frame) in frames.iter().enumerate() {
        let Some(path_value) = frame.get("path").and_then(Value::as_str) else {
            return Err(contact_result_from_detail(contact_failure_detail(
                "screenshot_contact_sheet_manifest_malformed",
                Some(&sequence_dir.join("manifest.json")),
                "sequence frame is missing a string path",
                json!("frame.path string"),
                frame.get("path").cloned().unwrap_or(Value::Null),
            )));
        };
        if path_value.trim().is_empty() {
            return Err(contact_result_from_detail(contact_failure_detail(
                "screenshot_contact_sheet_manifest_malformed",
                Some(&sequence_dir.join("manifest.json")),
                "sequence frame path must not be empty",
                json!("non-empty frame.path"),
                json!(path_value),
            )));
        }

        let source_path = resolve_frame_path(sequence_dir, path_value);
        let image = image::open(&source_path).map_err(|error| {
            let kind = if source_path.exists() {
                "screenshot_contact_sheet_image_unreadable"
            } else {
                "screenshot_contact_sheet_frame_missing"
            };
            contact_result_from_detail(contact_failure_detail(
                kind,
                Some(&source_path),
                &format!("failed to read sequence frame image: {error}"),
                json!("readable frame image from manifest path"),
                json!(path_value),
            ))
        })?;
        let (source_width, source_height) = image.dimensions();
        if source_width == 0 || source_height == 0 {
            return Err(contact_result_from_detail(contact_failure_detail(
                "screenshot_contact_sheet_invalid_dimensions",
                Some(&source_path),
                "sequence frame image has invalid dimensions",
                json!("positive width and height"),
                json!({"width": source_width, "height": source_height}),
            )));
        }
        validate_manifest_dimensions(frame, &source_path, source_width, source_height)?;

        let thumb_height = scaled_height(source_width, source_height, thumb_width)?;
        let pixels = resize(
            &image.to_rgba8(),
            thumb_width,
            thumb_height,
            FilterType::Triangle,
        );
        let engine_frame = frame.get("engine_frame").and_then(Value::as_i64);
        let time_msec = frame.get("time_msec").and_then(Value::as_i64);
        let index = frame.get("index").and_then(Value::as_i64);
        loaded.push(ContactFrame {
            manifest_index,
            index,
            engine_frame,
            time_msec,
            manifest_path: path_value.to_string(),
            source_path,
            source_width,
            source_height,
            thumb_width,
            thumb_height,
            label: frame_label(index, engine_frame, time_msec),
            pixels,
        });
    }
    Ok(loaded)
}

fn validate_manifest_dimensions(
    frame: &Value,
    source_path: &Path,
    source_width: u32,
    source_height: u32,
) -> Result<(), CommandResult> {
    let manifest_width = frame.get("width").and_then(Value::as_u64);
    let manifest_height = frame.get("height").and_then(Value::as_u64);
    if let Some(width) = manifest_width
        && width != u64::from(source_width)
    {
        return Err(contact_result_from_detail(contact_failure_detail(
            "screenshot_contact_sheet_invalid_dimensions",
            Some(source_path),
            "manifest frame width does not match the image",
            json!({"width": source_width, "height": source_height}),
            json!({"width": width, "height": manifest_height}),
        )));
    }
    if let Some(height) = manifest_height
        && height != u64::from(source_height)
    {
        return Err(contact_result_from_detail(contact_failure_detail(
            "screenshot_contact_sheet_invalid_dimensions",
            Some(source_path),
            "manifest frame height does not match the image",
            json!({"width": source_width, "height": source_height}),
            json!({"width": manifest_width, "height": height}),
        )));
    }
    Ok(())
}

fn scaled_height(
    source_width: u32,
    source_height: u32,
    thumb_width: u32,
) -> Result<u32, CommandResult> {
    let height = (u64::from(source_height) * u64::from(thumb_width))
        .div_ceil(u64::from(source_width))
        .max(1);
    u32::try_from(height).map_err(|_| {
        contact_result_from_detail(contact_failure_detail(
            "screenshot_contact_sheet_invalid_dimensions",
            None,
            "scaled thumbnail height exceeds supported image dimensions",
            json!("thumbnail height that fits in u32"),
            json!(height),
        ))
    })
}

fn frame_label(index: Option<i64>, engine_frame: Option<i64>, time_msec: Option<i64>) -> String {
    let frame = engine_frame.or(index).unwrap_or(0);
    match time_msec {
        Some(time_msec) => format!("frame {frame} {time_msec}ms"),
        None => format!("frame {frame}"),
    }
}

fn blit(
    target: &mut ImageBuffer<Rgba<u8>, Vec<u8>>,
    source: &ImageBuffer<Rgba<u8>, Vec<u8>>,
    x: u32,
    y: u32,
) {
    for (source_x, source_y, pixel) in source.enumerate_pixels() {
        let target_x = x + source_x;
        let target_y = y + source_y;
        if target_x < target.width() && target_y < target.height() {
            target.put_pixel(target_x, target_y, *pixel);
        }
    }
}

fn draw_glyph(
    image: &mut ImageBuffer<Rgba<u8>, Vec<u8>>,
    x: u32,
    y: u32,
    glyph: [&'static str; 5],
    color: Rgba<u8>,
) {
    for (row, pattern) in glyph.iter().enumerate() {
        for (column, value) in pattern.chars().enumerate() {
            if value != '1' {
                continue;
            }
            let pixel_x = x + column as u32 * LABEL_SCALE;
            let pixel_y = y + row as u32 * LABEL_SCALE;
            for dy in 0..LABEL_SCALE {
                for dx in 0..LABEL_SCALE {
                    let target_x = pixel_x + dx;
                    let target_y = pixel_y + dy;
                    if target_x < image.width() && target_y < image.height() {
                        image.put_pixel(target_x, target_y, color);
                    }
                }
            }
        }
    }
}

fn glyph(ch: char) -> Option<[&'static str; 5]> {
    Some(match ch.to_ascii_lowercase() {
        '0' => ["111", "101", "101", "101", "111"],
        '1' => ["010", "110", "010", "010", "111"],
        '2' => ["111", "001", "111", "100", "111"],
        '3' => ["111", "001", "111", "001", "111"],
        '4' => ["101", "101", "111", "001", "001"],
        '5' => ["111", "100", "111", "001", "111"],
        '6' => ["111", "100", "111", "101", "111"],
        '7' => ["111", "001", "001", "010", "010"],
        '8' => ["111", "101", "111", "101", "111"],
        '9' => ["111", "101", "111", "001", "111"],
        'a' => ["010", "101", "111", "101", "101"],
        'e' => ["111", "100", "110", "100", "111"],
        'f' => ["111", "100", "110", "100", "100"],
        'm' => ["101", "111", "111", "101", "101"],
        'r' => ["110", "101", "110", "101", "101"],
        's' => ["111", "100", "111", "001", "111"],
        '-' => ["000", "000", "111", "000", "000"],
        '_' => ["000", "000", "000", "000", "111"],
        _ => return None,
    })
}

fn sheet_extent(count: u32, item: u32) -> u32 {
    PADDING * 2 + count * item + count.saturating_sub(1) * CELL_GAP
}

fn div_ceil(value: u32, divisor: u32) -> u32 {
    value.div_ceil(divisor)
}

fn resolve_input_path(project_root: &Path, raw_path: &str) -> PathBuf {
    let path = Path::new(raw_path.trim());
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        project_root.join(path)
    }
}

fn resolve_frame_path(sequence_dir: &Path, raw_path: &str) -> PathBuf {
    let path = Path::new(raw_path);
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        sequence_dir.join(path)
    }
}

fn companion_json_path(output_path: &Path) -> PathBuf {
    output_path.with_extension("json")
}

fn write_json_file(path: &Path, value: &Value) -> Result<(), std::io::Error> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let content = serde_json::to_vec_pretty(value)?;
    fs::write(path, content)
}

fn args_invalid(message: &str, actual: Value, expected: Value) -> CommandResult {
    contact_failure(
        "screenshot_contact_sheet_args_invalid",
        None,
        message,
        expected,
        actual,
    )
}

fn contact_failure(
    kind: &str,
    path: Option<&Path>,
    message: &str,
    expected: Value,
    actual: Value,
) -> CommandResult {
    CommandResult {
        ok: false,
        command: COMMAND.to_string(),
        message: "screenshot contact-sheet failed".to_string(),
        data: Map::new(),
        failures: vec![contact_failure_detail(
            kind, path, message, expected, actual,
        )],
        exit_code: 1,
    }
}

fn contact_result_from_detail(detail: FailureDetail) -> CommandResult {
    CommandResult {
        ok: false,
        command: COMMAND.to_string(),
        message: "screenshot contact-sheet failed".to_string(),
        data: Map::new(),
        failures: vec![detail],
        exit_code: 1,
    }
}

fn contact_failure_detail(
    kind: &str,
    path: Option<&Path>,
    message: &str,
    expected: Value,
    actual: Value,
) -> FailureDetail {
    FailureDetail {
        kind: kind.to_string(),
        command: Some(COMMAND.to_string()),
        path: path.map(|path| path.to_string_lossy().into_owned()),
        message: Some(message.to_string()),
        expected: Some(expected),
        actual: Some(actual),
        ..FailureDetail::default()
    }
}

fn write_failure(path: &Path, error: std::io::Error) -> CommandResult {
    contact_failure(
        "artifact_write_failed",
        Some(path),
        &error.to_string(),
        json!("writable contact sheet artifact path"),
        json!("OSError"),
    )
}

fn image_write_failure(path: &Path, error: image::ImageError) -> CommandResult {
    contact_failure(
        "artifact_write_failed",
        Some(path),
        &error.to_string(),
        json!("writable contact sheet PNG path"),
        json!("ImageError"),
    )
}

#[cfg(test)]
mod tests {
    use super::{ScreenshotContactSheetOptions, build_contact_sheet};
    use godot_qa_core::config::ProjectConfig;
    use image::{ImageBuffer, Rgba};
    use serde_json::{Value, json};
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

    fn save_png(path: &Path, size: (u32, u32), color: [u8; 4]) {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        let image = ImageBuffer::<Rgba<u8>, Vec<u8>>::from_pixel(size.0, size.1, Rgba(color));
        image.save(path).unwrap();
    }

    fn write_manifest(sequence_dir: &Path, frames: Value) {
        fs::write(
            sequence_dir.join("manifest.json"),
            serde_json::to_vec_pretty(&json!({
                "command": "screenshot.sequence",
                "captured_count": frames.as_array().unwrap().len(),
                "frames": frames,
            }))
            .unwrap(),
        )
        .unwrap();
    }

    #[test]
    fn contact_sheet_renders_manifest_order_and_writes_geometry_json() {
        let root = temp_dir("contact-sheet-order");
        let sequence_dir = root.join("seq");
        save_png(
            &sequence_dir.join("frames/b.png"),
            (20, 10),
            [0, 255, 0, 255],
        );
        save_png(
            &sequence_dir.join("frames/a.png"),
            (10, 20),
            [255, 0, 0, 255],
        );
        save_png(
            &sequence_dir.join("frames/c.png"),
            (20, 20),
            [0, 0, 255, 255],
        );
        write_manifest(
            &sequence_dir,
            json!([
                {"index": 2, "engine_frame": 20, "time_msec": 200, "path": "frames/b.png", "width": 20, "height": 10},
                {"index": 0, "engine_frame": 0, "time_msec": 0, "path": "frames/a.png", "width": 10, "height": 20},
                {"index": 1, "engine_frame": 10, "time_msec": 100, "path": "frames/c.png", "width": 20, "height": 20}
            ]),
        );

        let result = build_contact_sheet(
            &test_config(&root),
            ScreenshotContactSheetOptions {
                sequence_dir: "seq".to_string(),
                columns: 2,
                thumb_width: 40,
                out: "out/contact.png".to_string(),
            },
        );

        assert!(result.ok, "{:?}", result.failures);
        let metadata_path = root.join("out/contact.json");
        let metadata: Value = serde_json::from_slice(&fs::read(metadata_path).unwrap()).unwrap();
        assert_eq!(metadata["columns"], json!(2));
        assert_eq!(metadata["rows"], json!(2));
        assert_eq!(
            metadata["thumbnail_size"],
            json!({"width": 40, "height": 80})
        );
        assert_eq!(
            metadata["frames"][0]["manifest_path"],
            json!("frames/b.png")
        );
        assert_eq!(
            metadata["frames"][1]["manifest_path"],
            json!("frames/a.png")
        );
        assert_eq!(metadata["frames"][0]["thumbnail"]["height"], json!(20));
        assert_eq!(metadata["frames"][1]["thumbnail"]["height"], json!(80));
        assert!(metadata["frames"][0]["label"]["y"].as_u64().unwrap() >= 88);
        assert_eq!(
            result.data["artifacts"]["contact_sheet"],
            json!(root.join("out/contact.png").to_string_lossy().into_owned())
        );
        assert_eq!(
            result.data["artifacts"]["metadata"],
            json!(root.join("out/contact.json").to_string_lossy().into_owned())
        );
    }

    #[test]
    fn contact_sheet_fails_for_missing_manifest() {
        let root = temp_dir("contact-sheet-missing-manifest");
        let result = build_contact_sheet(
            &test_config(&root),
            ScreenshotContactSheetOptions {
                sequence_dir: "seq".to_string(),
                columns: 2,
                thumb_width: 40,
                out: "out/contact.png".to_string(),
            },
        );

        assert!(!result.ok);
        assert_eq!(
            result.failures[0].kind,
            "screenshot_contact_sheet_manifest_missing"
        );
        assert!(
            Path::new(result.failures[0].path.as_deref().unwrap())
                .ends_with(Path::new("seq").join("manifest.json"))
        );
    }

    #[test]
    fn contact_sheet_fails_for_missing_or_unreadable_images() {
        let root = temp_dir("contact-sheet-bad-images");
        let sequence_dir = root.join("seq");
        fs::create_dir_all(sequence_dir.join("frames")).unwrap();
        write_manifest(
            &sequence_dir,
            json!([
                {"index": 0, "path": "frames/missing.png"}
            ]),
        );

        let missing = build_contact_sheet(
            &test_config(&root),
            ScreenshotContactSheetOptions {
                sequence_dir: "seq".to_string(),
                columns: 1,
                thumb_width: 40,
                out: "out/contact.png".to_string(),
            },
        );
        assert!(!missing.ok);
        assert_eq!(
            missing.failures[0].kind,
            "screenshot_contact_sheet_frame_missing"
        );

        fs::write(sequence_dir.join("frames/not_png.png"), b"not a png").unwrap();
        write_manifest(
            &sequence_dir,
            json!([
                {"index": 0, "path": "frames/not_png.png"}
            ]),
        );
        let unreadable = build_contact_sheet(
            &test_config(&root),
            ScreenshotContactSheetOptions {
                sequence_dir: "seq".to_string(),
                columns: 1,
                thumb_width: 40,
                out: "out/contact.png".to_string(),
            },
        );
        assert!(!unreadable.ok);
        assert_eq!(
            unreadable.failures[0].kind,
            "screenshot_contact_sheet_image_unreadable"
        );
    }
}
