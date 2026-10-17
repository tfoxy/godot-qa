use godot_qa_core::config::ProjectConfig;
use godot_qa_core::results::{CommandResult, FailureDetail};
use godot_qa_runtime as runtime;
use image::{GenericImageView, Rgba, RgbaImage};
use serde_json::{Map, Value, json};
use std::fs;
use std::path::{Path, PathBuf};

const COMMAND: &str = "physics.overlay";
const COMMAND_TEXT: &str = "physics overlay";
const BODY_COLOR: Rgba<u8> = Rgba([255, 0, 200, 255]);
const AREA_COLOR: Rgba<u8> = Rgba([0, 225, 255, 255]);
const AABB_COLOR: Rgba<u8> = Rgba([255, 220, 0, 255]);
const CONTACT_COLOR: Rgba<u8> = Rgba([255, 40, 40, 255]);
const CONTACT_NORMAL_COLOR: Rgba<u8> = Rgba([255, 255, 255, 255]);
const FOOTPRINT_COLOR: Rgba<u8> = Rgba([0, 235, 90, 255]);

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct OverlayArtifactPaths {
    pub(crate) base: PathBuf,
    pub(crate) overlay: PathBuf,
    pub(crate) metadata: PathBuf,
}

pub(crate) fn physics_overlay_result(
    config: &ProjectConfig,
    params: Map<String, Value>,
    out_path: PathBuf,
) -> CommandResult {
    let paths = overlay_artifact_paths(&out_path);
    if let Some(parent) = paths.overlay.parent() {
        if let Err(error) = fs::create_dir_all(parent) {
            return failed(
                "physics overlay artifact directory could not be created",
                artifact_write_failure(&paths.overlay, &error),
                Map::new(),
            );
        }
    }

    let screenshot_name = format!("physics-overlay-base-{}", unique_suffix());
    let screenshot_result = runtime::run_live_screenshot_protocol_command(
        config,
        COMMAND_TEXT,
        runtime::SCREENSHOT_CAPTURE,
        Map::from_iter([("name".to_string(), json!(screenshot_name))]),
    );
    if !screenshot_result.ok {
        return retag_failure_result(
            screenshot_result,
            "physics overlay screenshot capture failed",
        );
    }
    let Some(runtime_screenshot) = screenshot_artifact_path(&screenshot_result.data) else {
        return failed(
            "physics overlay screenshot capture failed",
            FailureDetail {
                kind: "artifact_missing".to_string(),
                command: Some(COMMAND_TEXT.to_string()),
                message: Some("screenshot capture did not return an artifact path".to_string()),
                expected: Some(json!("data.artifacts.screenshot path")),
                actual: Some(json!(screenshot_result.data)),
                ..FailureDetail::default()
            },
            Map::new(),
        );
    };
    if let Err(error) = fs::copy(&runtime_screenshot, &paths.base) {
        return failed(
            "physics overlay base screenshot copy failed",
            artifact_write_failure(&paths.base, &error),
            Map::new(),
        );
    }

    let collect_result = runtime::run_live_protocol_command(
        config,
        COMMAND_TEXT,
        runtime::PHYSICS_OVERLAY_COLLECT,
        params.clone(),
    );
    if !collect_result.ok {
        return retag_failure_result(collect_result, "physics overlay collection failed");
    }
    let image_size = image::open(&paths.base)
        .ok()
        .map(|image| image.dimensions());
    let artifact_context = artifact_context(&paths);
    let warnings = overlay_warnings(
        &collect_result.data,
        &params,
        Some(&artifact_context),
        image_size,
    );

    let render_metadata =
        match render_overlay_png(&paths.base, &paths.overlay, &collect_result.data) {
            Ok(metadata) => metadata,
            Err(failure) => {
                return failed(
                    "physics overlay render failed",
                    failure,
                    collect_result.data,
                );
            }
        };

    let metadata = overlay_metadata(&collect_result.data, render_metadata, &paths, &warnings);
    if let Err(error) = write_json(&paths.metadata, &metadata) {
        return failed(
            "physics overlay metadata write failed",
            artifact_write_failure(&paths.metadata, &error),
            collect_result.data,
        );
    }

    let summary = summary_from_collection(&collect_result.data, &warnings);
    let target = collect_result
        .data
        .get("target")
        .cloned()
        .unwrap_or_else(|| params.get("target").cloned().unwrap_or_else(|| json!("")));
    let data = Map::from_iter([
        ("target".to_string(), target),
        ("artifacts".to_string(), Value::Object(artifact_context)),
        ("summary".to_string(), summary),
    ]);
    if let Some(failure) = strict_overlay_warning_failure(&params, &warnings) {
        return failed("physics overlay strict validation failed", failure, data);
    }
    CommandResult {
        ok: true,
        command: COMMAND.to_string(),
        message: "physics overlay completed".to_string(),
        data,
        failures: Vec::new(),
        exit_code: 0,
    }
}

pub(crate) fn physics_overlay_from_snapshot_result(
    config: &ProjectConfig,
    params: Map<String, Value>,
    scenario: &str,
    snapshot: &str,
    out_path: PathBuf,
) -> CommandResult {
    let paths = overlay_artifact_paths(&out_path);
    let run_root = match resolve_scenario_artifact_root(config, scenario) {
        Ok(path) => path,
        Err(failure) => {
            return failed(
                "physics overlay snapshot review failed",
                failure,
                Map::new(),
            );
        }
    };
    let result_path = run_root.join("result.json");
    let result = match read_json(&result_path) {
        Ok(value) => value,
        Err(error) => {
            return failed(
                "physics overlay snapshot review failed",
                FailureDetail {
                    kind: "artifact_read_failed".to_string(),
                    command: Some(COMMAND_TEXT.to_string()),
                    path: Some(result_path.to_string_lossy().into_owned()),
                    message: Some("scenario result.json could not be read".to_string()),
                    expected: Some(json!("valid scenario result.json artifact")),
                    actual: Some(json!(error.to_string())),
                    ..FailureDetail::default()
                },
                Map::new(),
            );
        }
    };
    let screenshot = run_root
        .join("snapshots")
        .join(snapshot)
        .join("screenshot.png");
    if !screenshot.is_file() {
        return failed(
            "physics overlay snapshot review failed",
            FailureDetail {
                kind: "artifact_missing".to_string(),
                command: Some(COMMAND_TEXT.to_string()),
                path: Some(screenshot.to_string_lossy().into_owned()),
                message: Some("snapshot screenshot evidence was not found".to_string()),
                expected: Some(json!(
                    "snapshots/<snapshot>/screenshot.png from the scenario run"
                )),
                actual: Some(json!("missing")),
                ..FailureDetail::default()
            },
            review_data(&run_root, snapshot, None),
        );
    }
    let evidence = match matching_overlay_evidence(&result, snapshot, &params) {
        Ok(value) => value,
        Err(failure) => {
            return failed(
                "physics overlay snapshot review failed",
                failure,
                review_data(&run_root, snapshot, None),
            );
        }
    };
    let collection = match overlay_collection_from_evidence(evidence) {
        Some(collection) => collection,
        None => {
            return failed(
                "physics overlay snapshot review failed",
                FailureDetail {
                    kind: "artifact_missing".to_string(),
                    command: Some(COMMAND_TEXT.to_string()),
                    path: Some(result_path.to_string_lossy().into_owned()),
                    message: Some(
                        "matching physics overlay evidence did not contain metadata".to_string(),
                    ),
                    expected: Some(json!(
                        "physics_overlays entry with collected overlay metadata"
                    )),
                    actual: Some(evidence.clone()),
                    ..FailureDetail::default()
                },
                review_data(&run_root, snapshot, None),
            );
        }
    };
    if let Err(error) = fs::copy(&screenshot, &paths.base) {
        return failed(
            "physics overlay base screenshot copy failed",
            artifact_write_failure(&paths.base, &error),
            review_data(&run_root, snapshot, Some(&paths)),
        );
    }
    let image_size = image::open(&paths.base)
        .ok()
        .map(|image| image.dimensions());
    let artifact_context = artifact_context(&paths);
    let warnings = overlay_warnings(&collection, &params, Some(&artifact_context), image_size);
    let render_metadata = match render_overlay_png(&paths.base, &paths.overlay, &collection) {
        Ok(metadata) => metadata,
        Err(failure) => return failed("physics overlay render failed", failure, collection),
    };
    let mut metadata = overlay_metadata(&collection, render_metadata, &paths, &warnings);
    if let Some(metadata_object) = metadata.as_object_mut() {
        metadata_object.insert(
            "review".to_string(),
            json!({
                "scenario": scenario,
                "snapshot": snapshot,
                "run_root": run_root,
                "source_screenshot": screenshot,
            }),
        );
    }
    if let Err(error) = write_json(&paths.metadata, &metadata) {
        return failed(
            "physics overlay metadata write failed",
            artifact_write_failure(&paths.metadata, &error),
            collection,
        );
    }
    let target = collection
        .get("target")
        .cloned()
        .unwrap_or_else(|| params.get("target").cloned().unwrap_or_else(|| json!("")));
    let data = Map::from_iter([
        ("target".to_string(), target),
        ("scenario".to_string(), json!(scenario)),
        ("snapshot".to_string(), json!(snapshot)),
        ("run_root".to_string(), json!(run_root)),
        ("artifacts".to_string(), Value::Object(artifact_context)),
        (
            "summary".to_string(),
            summary_from_collection(&collection, &warnings),
        ),
    ]);
    if let Some(failure) = strict_overlay_warning_failure(&params, &warnings) {
        return failed("physics overlay strict validation failed", failure, data);
    }
    CommandResult {
        ok: true,
        command: COMMAND.to_string(),
        message: "physics overlay completed".to_string(),
        data,
        failures: Vec::new(),
        exit_code: 0,
    }
}

pub(crate) fn render_overlay_png(
    base_path: &Path,
    overlay_path: &Path,
    collection: &Map<String, Value>,
) -> Result<Value, FailureDetail> {
    let mut image = image::open(base_path)
        .map_err(|error| FailureDetail {
            kind: "artifact_read_failed".to_string(),
            command: Some(COMMAND_TEXT.to_string()),
            path: Some(base_path.to_string_lossy().into_owned()),
            message: Some("base screenshot could not be decoded".to_string()),
            expected: Some(json!("readable PNG screenshot")),
            actual: Some(json!(error.to_string())),
            ..FailureDetail::default()
        })?
        .to_rgba8();

    let shapes = collection
        .get("shapes")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    for shape in &shapes {
        if let Some(shape) = shape.as_object() {
            draw_shape(&mut image, shape);
        }
    }
    draw_contacts(
        &mut image,
        collection.get("contacts").and_then(Value::as_array),
    );
    draw_visual_footprints(&mut image, collection.get("project_state"));

    if let Some(parent) = overlay_path.parent() {
        fs::create_dir_all(parent).map_err(|error| artifact_write_failure(parent, &error))?;
    }
    image.save(overlay_path).map_err(|error| FailureDetail {
        kind: "artifact_write_failed".to_string(),
        command: Some(COMMAND_TEXT.to_string()),
        path: Some(overlay_path.to_string_lossy().into_owned()),
        message: Some("overlay PNG could not be written".to_string()),
        expected: Some(json!("writable overlay PNG path")),
        actual: Some(json!(error.to_string())),
        ..FailureDetail::default()
    })?;

    Ok(json!({
        "legend": overlay_legend(),
        "rendered": {
            "shape_count": shapes.len(),
            "contact_count": collection.get("contacts").and_then(Value::as_array).map_or(0, Vec::len),
            "visual_footprint_count": visual_footprints(collection.get("project_state")).len(),
        }
    }))
}

pub(crate) fn overlay_artifact_paths(out_path: &Path) -> OverlayArtifactPaths {
    let overlay = out_path.to_path_buf();
    let parent = out_path.parent().unwrap_or_else(|| Path::new(""));
    let stem = out_path
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or("physics_overlay");
    OverlayArtifactPaths {
        base: parent.join(format!("{stem}.base.png")),
        overlay,
        metadata: parent.join(format!("{stem}.overlay.json")),
    }
}

pub(crate) fn overlay_legend() -> Value {
    json!({
        "body_collision": {"color": "#ff00c8", "style": "solid outline"},
        "area_collision": {"color": "#00e1ff", "style": "solid outline"},
        "aabb": {"color": "#ffdc00", "style": "thin rectangle"},
        "contact_point": {"color": "#ff2828", "style": "dot"},
        "contact_normal": {"color": "#ffffff", "style": "line"},
        "visual_footprint": {"color": "#00eb5a", "style": "solid outline"}
    })
}

fn draw_shape(image: &mut RgbaImage, shape: &Map<String, Value>) {
    let color = if shape
        .get("is_area")
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        AREA_COLOR
    } else {
        BODY_COLOR
    };
    let points = points_from_value(shape.get("screen_points"));
    if points.len() >= 2 {
        draw_polyline(image, &points, color, true, 2);
    }
    if let Some(aabb) = rect_from_value(shape.get("screen_aabb")) {
        let include_aabb = shape.contains_key("aabb") || shape.contains_key("screen_aabb");
        if include_aabb {
            draw_rect(image, aabb, AABB_COLOR, 1);
        }
    }
}

fn draw_contacts(image: &mut RgbaImage, contacts: Option<&Vec<Value>>) {
    for contact in contacts.into_iter().flatten() {
        let Some(contact) = contact.as_object() else {
            continue;
        };
        if let Some(point) = point_from_value(
            contact
                .get("screen_position")
                .or_else(|| contact.get("position")),
        ) {
            draw_disc(image, point, 4.0, CONTACT_COLOR);
            if let Some(normal) = point_from_value(
                contact
                    .get("screen_normal")
                    .or_else(|| contact.get("normal")),
            ) {
                let end = (point.0 + normal.0 * 24.0, point.1 + normal.1 * 24.0);
                draw_line(image, point, end, CONTACT_NORMAL_COLOR, 2);
            }
        }
    }
}

fn draw_visual_footprints(image: &mut RgbaImage, project_state: Option<&Value>) {
    for footprint in visual_footprints(project_state) {
        let Some(footprint) = footprint.as_object() else {
            continue;
        };
        let points = points_from_value(footprint.get("screen_points"));
        if points.len() >= 2 {
            draw_polyline(image, &points, FOOTPRINT_COLOR, true, 2);
        } else if let Some(rect) = rect_from_value(footprint.get("screen_aabb")) {
            draw_rect(image, rect, FOOTPRINT_COLOR, 2);
        }
    }
}

fn draw_polyline(
    image: &mut RgbaImage,
    points: &[(f64, f64)],
    color: Rgba<u8>,
    closed: bool,
    width: i32,
) {
    for segment in points.windows(2) {
        draw_line(image, segment[0], segment[1], color, width);
    }
    if closed && points.len() > 2 {
        draw_line(image, points[points.len() - 1], points[0], color, width);
    }
}

fn draw_rect(image: &mut RgbaImage, rect: (f64, f64, f64, f64), color: Rgba<u8>, width: i32) {
    let (x, y, w, h) = rect;
    let points = [(x, y), (x + w, y), (x + w, y + h), (x, y + h)];
    draw_polyline(image, &points, color, true, width);
}

fn draw_disc(image: &mut RgbaImage, center: (f64, f64), radius: f64, color: Rgba<u8>) {
    let r = radius.ceil() as i32;
    let cx = center.0.round() as i32;
    let cy = center.1.round() as i32;
    for y in (cy - r)..=(cy + r) {
        for x in (cx - r)..=(cx + r) {
            let dx = x - cx;
            let dy = y - cy;
            if (dx * dx + dy * dy) as f64 <= radius * radius {
                put_pixel_checked(image, x, y, color);
            }
        }
    }
}

fn draw_line(
    image: &mut RgbaImage,
    start: (f64, f64),
    end: (f64, f64),
    color: Rgba<u8>,
    width: i32,
) {
    let mut x0 = start.0.round() as i32;
    let mut y0 = start.1.round() as i32;
    let x1 = end.0.round() as i32;
    let y1 = end.1.round() as i32;
    let dx = (x1 - x0).abs();
    let sx = if x0 < x1 { 1 } else { -1 };
    let dy = -(y1 - y0).abs();
    let sy = if y0 < y1 { 1 } else { -1 };
    let mut err = dx + dy;
    loop {
        for oy in -width / 2..=width / 2 {
            for ox in -width / 2..=width / 2 {
                put_pixel_checked(image, x0 + ox, y0 + oy, color);
            }
        }
        if x0 == x1 && y0 == y1 {
            break;
        }
        let e2 = 2 * err;
        if e2 >= dy {
            err += dy;
            x0 += sx;
        }
        if e2 <= dx {
            err += dx;
            y0 += sy;
        }
    }
}

fn put_pixel_checked(image: &mut RgbaImage, x: i32, y: i32, color: Rgba<u8>) {
    if x >= 0 && y >= 0 && (x as u32) < image.width() && (y as u32) < image.height() {
        image.put_pixel(x as u32, y as u32, color);
    }
}

fn points_from_value(value: Option<&Value>) -> Vec<(f64, f64)> {
    value
        .and_then(Value::as_array)
        .map(|points| {
            points
                .iter()
                .filter_map(|point| point_from_value(Some(point)))
                .collect()
        })
        .unwrap_or_default()
}

fn point_from_value(value: Option<&Value>) -> Option<(f64, f64)> {
    let object = value?.as_object()?;
    Some((number_field(object, "x")?, number_field(object, "y")?))
}

fn rect_from_value(value: Option<&Value>) -> Option<(f64, f64, f64, f64)> {
    let object = value?.as_object()?;
    let x = number_field(object, "x")?;
    let y = number_field(object, "y")?;
    let w = number_field(object, "w").or_else(|| number_field(object, "width"))?;
    let h = number_field(object, "h").or_else(|| number_field(object, "height"))?;
    Some((x, y, w, h))
}

fn number_field(object: &Map<String, Value>, key: &str) -> Option<f64> {
    object.get(key).and_then(Value::as_f64)
}

fn visual_footprints(project_state: Option<&Value>) -> Vec<Value> {
    let mut footprints = Vec::new();
    collect_visual_footprints(project_state.unwrap_or(&Value::Null), &mut footprints);
    footprints
}

fn collect_visual_footprints(value: &Value, footprints: &mut Vec<Value>) {
    match value {
        Value::Object(object) => {
            for (key, value) in object {
                if key == "visual_footprints" {
                    if let Some(values) = value.as_array() {
                        footprints.extend(values.iter().cloned());
                    }
                } else {
                    collect_visual_footprints(value, footprints);
                }
            }
        }
        Value::Array(values) => {
            for value in values {
                collect_visual_footprints(value, footprints);
            }
        }
        _ => {}
    }
}

fn overlay_metadata(
    collection: &Map<String, Value>,
    render_metadata: Value,
    paths: &OverlayArtifactPaths,
    warnings: &[Value],
) -> Value {
    json!({
        "command": COMMAND,
        "target": collection.get("target").cloned().unwrap_or_else(|| json!("")),
        "viewport": collection.get("viewport").cloned().unwrap_or_else(|| json!(false)),
        "legend": overlay_legend(),
        "render": render_metadata,
        "artifacts": {
            "screenshot": absolute_path(&paths.base),
            "overlay": absolute_path(&paths.overlay),
            "metadata": absolute_path(&paths.metadata),
        },
        "shapes": collection.get("shapes").cloned().unwrap_or_else(|| json!([])),
        "contacts": collection.get("contacts").cloned().unwrap_or_else(|| json!([])),
        "project_state": collection.get("project_state").cloned().unwrap_or_else(|| json!({})),
        "runtime_metadata": collection.get("metadata").cloned().unwrap_or_else(|| json!({})),
        "warnings": warnings,
    })
}

fn summary_from_collection(collection: &Map<String, Value>, warnings: &[Value]) -> Value {
    let metadata = collection.get("metadata").and_then(Value::as_object);
    json!({
        "shape_count": metadata.and_then(|m| m.get("shape_count")).and_then(Value::as_i64).unwrap_or_else(|| collection.get("shapes").and_then(Value::as_array).map_or(0, |values| values.len() as i64)),
        "body_count": metadata.and_then(|m| m.get("body_count")).and_then(Value::as_i64).unwrap_or(0),
        "area_count": metadata.and_then(|m| m.get("area_count")).and_then(Value::as_i64).unwrap_or(0),
        "contact_count": metadata.and_then(|m| m.get("contact_count")).and_then(Value::as_i64).unwrap_or_else(|| collection.get("contacts").and_then(Value::as_array).map_or(0, |values| values.len() as i64)),
        "visual_footprint_count": visual_footprints(collection.get("project_state")).len(),
        "warnings": warnings,
    })
}

fn overlay_warnings(
    collection: &Map<String, Value>,
    params: &Map<String, Value>,
    artifacts: Option<&Map<String, Value>>,
    image_size: Option<(u32, u32)>,
) -> Vec<Value> {
    let mut warnings = Vec::new();
    let shapes = collection
        .get("shapes")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    if shapes.is_empty() {
        warnings.push(warning(
            "physics_overlay.no_collision_shapes_found",
            "no collision shapes were found for the physics overlay target",
            collection,
            params,
            artifacts,
            Map::new(),
        ));
    }

    if collection
        .get("metadata")
        .and_then(Value::as_object)
        .is_some_and(metadata_reports_truncation)
    {
        let metadata = collection.get("metadata").and_then(Value::as_object);
        let mut details = Map::new();
        for key in [
            "max_shapes",
            "shape_count_total",
            "shape_count_returned",
            "shape_count",
        ] {
            if let Some(value) = metadata.and_then(|m| m.get(key)).cloned() {
                details.insert(key.to_string(), value);
            }
        }
        warnings.push(warning(
            "physics_overlay.unbounded_shape_count_truncated",
            "collision shape collection exceeded the configured maximum and was truncated",
            collection,
            params,
            artifacts,
            details,
        ));
    }

    warnings.extend(visual_footprint_comparisons(
        collection, params, artifacts, &shapes,
    ));

    if target_is_offscreen(collection, image_size) {
        warnings.push(warning(
            "physics_overlay.target_offscreen",
            "the physics overlay target is outside the captured viewport",
            collection,
            params,
            artifacts,
            Map::new(),
        ));
    }
    warnings
}

fn visual_footprint_comparisons(
    collection: &Map<String, Value>,
    params: &Map<String, Value>,
    artifacts: Option<&Map<String, Value>>,
    shapes: &[Value],
) -> Vec<Value> {
    if !visual_footprint_provider_available(collection.get("project_state")) {
        return Vec::new();
    }

    let shape_rects: Vec<_> = shapes.iter().filter_map(shape_aabb).collect();
    let footprints = visual_footprints(collection.get("project_state"));
    let footprint_rects: Vec<_> = footprints.iter().filter_map(footprint_aabb).collect();
    let mut warnings = Vec::new();

    if shape_rects.is_empty() && !footprint_rects.is_empty() {
        warnings.push(warning(
            "physics_overlay.visual_footprint_without_collision",
            "project visual footprints were reported without matching collision shapes",
            collection,
            params,
            artifacts,
            Map::new(),
        ));
        return warnings;
    }
    if !shape_rects.is_empty() && footprint_rects.is_empty() {
        warnings.push(warning(
            "physics_overlay.collision_without_visual_footprint",
            "collision shapes were reported without project visual footprint data",
            collection,
            params,
            artifacts,
            Map::new(),
        ));
        return warnings;
    }
    if shape_rects.is_empty() || footprint_rects.is_empty() {
        return warnings;
    }

    let collision = union_rects(&shape_rects).expect("shape rects is non-empty");
    let visual = union_rects(&footprint_rects).expect("footprint rects is non-empty");
    if shape_rects.iter().any(|shape| {
        !footprint_rects
            .iter()
            .any(|footprint| rects_intersect(*shape, *footprint))
    }) {
        warnings.push(warning(
            "physics_overlay.collision_without_visual_footprint",
            "at least one collision shape does not overlap a project visual footprint",
            collection,
            params,
            artifacts,
            comparison_details(collision, visual),
        ));
    }
    if footprint_rects.iter().any(|footprint| {
        !shape_rects
            .iter()
            .any(|shape| rects_intersect(*shape, *footprint))
    }) {
        warnings.push(warning(
            "physics_overlay.visual_footprint_without_collision",
            "at least one project visual footprint does not overlap a collision shape",
            collection,
            params,
            artifacts,
            comparison_details(collision, visual),
        ));
    }
    if collider_extends_far_beyond_visual(collision, visual) {
        warnings.push(warning(
            "physics_overlay.collider_extends_far_beyond_visual_footprint",
            "collision bounds extend far beyond the project visual footprint",
            collection,
            params,
            artifacts,
            comparison_details(collision, visual),
        ));
    }
    if collider_center_offset_from_visual(collision, visual) {
        warnings.push(warning(
            "physics_overlay.collider_center_offset_from_visual_footprint",
            "collision bounds center is offset from the project visual footprint center",
            collection,
            params,
            artifacts,
            comparison_details(collision, visual),
        ));
    }
    warnings
}

fn strict_overlay_warning_failure(
    params: &Map<String, Value>,
    warnings: &[Value],
) -> Option<FailureDetail> {
    if !params
        .get("strict")
        .and_then(Value::as_bool)
        .unwrap_or(false)
        || warnings.is_empty()
    {
        return None;
    }
    Some(FailureDetail {
        kind: "physics_overlay_strict_warning".to_string(),
        command: Some(COMMAND_TEXT.to_string()),
        message: Some("physics overlay produced warnings while --strict was enabled".to_string()),
        expected: Some(json!("no physics overlay warnings")),
        actual: Some(Value::Array(warnings.to_vec())),
        severity: Some("error".to_string()),
        ..FailureDetail::default()
    })
}

fn metadata_reports_truncation(metadata: &Map<String, Value>) -> bool {
    metadata
        .get("truncated")
        .or_else(|| metadata.get("shapes_truncated"))
        .and_then(Value::as_bool)
        .unwrap_or(false)
}

fn visual_footprint_provider_available(project_state: Option<&Value>) -> bool {
    let Some(state) = project_state.and_then(Value::as_object) else {
        return false;
    };
    if state
        .get("available")
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        return true;
    }
    visual_footprints(project_state).iter().any(|value| {
        value
            .as_object()
            .is_some_and(|footprint| footprint_aabb_object(footprint).is_some())
    })
}

fn warning(
    id: &str,
    message: &str,
    collection: &Map<String, Value>,
    params: &Map<String, Value>,
    artifacts: Option<&Map<String, Value>>,
    mut details: Map<String, Value>,
) -> Value {
    details.insert("id".to_string(), json!(id));
    details.insert("severity".to_string(), json!("warning"));
    details.insert("message".to_string(), json!(message));
    details.insert("target".to_string(), warning_target(collection, params));
    details.insert("radius".to_string(), warning_radius(collection, params));
    if let Some(artifacts) = artifacts {
        details.insert("artifacts".to_string(), Value::Object(artifacts.clone()));
    }
    Value::Object(details)
}

fn warning_target(collection: &Map<String, Value>, params: &Map<String, Value>) -> Value {
    collection
        .get("target")
        .cloned()
        .or_else(|| params.get("target").cloned())
        .unwrap_or_else(|| json!(""))
}

fn warning_radius(collection: &Map<String, Value>, params: &Map<String, Value>) -> Value {
    collection
        .get("sample")
        .and_then(Value::as_object)
        .and_then(|sample| sample.get("radius"))
        .cloned()
        .or_else(|| params.get("radius").cloned())
        .unwrap_or(Value::Null)
}

fn artifact_context(paths: &OverlayArtifactPaths) -> Map<String, Value> {
    Map::from_iter([
        ("screenshot".to_string(), json!(absolute_path(&paths.base))),
        ("overlay".to_string(), json!(absolute_path(&paths.overlay))),
        (
            "metadata".to_string(),
            json!(absolute_path(&paths.metadata)),
        ),
    ])
}

fn target_is_offscreen(collection: &Map<String, Value>, image_size: Option<(u32, u32)>) -> bool {
    let Some((width, height)) = image_size else {
        return false;
    };
    let viewport = (0.0, 0.0, f64::from(width), f64::from(height));
    collection
        .get("target_node")
        .or_else(|| collection.get("target_info"))
        .and_then(|value| value.as_object())
        .and_then(|target| {
            target
                .get("screen_aabb")
                .and_then(|value| rect_from_value(Some(value)))
                .or_else(|| {
                    target
                        .get("visual_rect")
                        .and_then(|value| rect_from_value(Some(value)))
                })
        })
        .is_some_and(|rect| !rects_intersect(rect, viewport))
}

fn shape_aabb(shape: &Value) -> Option<(f64, f64, f64, f64)> {
    let shape = shape.as_object()?;
    rect_from_value(shape.get("world_aabb"))
        .or_else(|| rect_from_value(shape.get("aabb")))
        .or_else(|| rect_from_value(shape.get("screen_aabb")))
}

fn footprint_aabb(footprint: &Value) -> Option<(f64, f64, f64, f64)> {
    footprint.as_object().and_then(footprint_aabb_object)
}

fn footprint_aabb_object(footprint: &Map<String, Value>) -> Option<(f64, f64, f64, f64)> {
    rect_from_value(footprint.get("world_aabb"))
        .or_else(|| rect_from_value(footprint.get("screen_aabb")))
        .or_else(|| rect_from_points(&points_from_value(footprint.get("world_points"))))
        .or_else(|| rect_from_points(&points_from_value(footprint.get("screen_points"))))
        .or_else(|| circle_footprint_rect(footprint))
        .or_else(|| sized_footprint_rect(footprint))
}

fn rect_from_points(points: &[(f64, f64)]) -> Option<(f64, f64, f64, f64)> {
    let first = points.first()?;
    let mut min_x = first.0;
    let mut min_y = first.1;
    let mut max_x = first.0;
    let mut max_y = first.1;
    for (x, y) in points.iter().copied().skip(1) {
        min_x = min_x.min(x);
        min_y = min_y.min(y);
        max_x = max_x.max(x);
        max_y = max_y.max(y);
    }
    Some((min_x, min_y, max_x - min_x, max_y - min_y))
}

fn circle_footprint_rect(footprint: &Map<String, Value>) -> Option<(f64, f64, f64, f64)> {
    let radius = footprint.get("radius").and_then(Value::as_f64)?;
    let center = point_from_value(footprint.get("world_position"))
        .or_else(|| point_from_value(footprint.get("screen_position")))?;
    Some((
        center.0 - radius,
        center.1 - radius,
        radius * 2.0,
        radius * 2.0,
    ))
}

fn sized_footprint_rect(footprint: &Map<String, Value>) -> Option<(f64, f64, f64, f64)> {
    let size = footprint.get("size")?.as_object()?;
    let width = number_field(size, "x").or_else(|| number_field(size, "width"))?;
    let height = number_field(size, "y").or_else(|| number_field(size, "height"))?;
    let center = point_from_value(footprint.get("world_position"))
        .or_else(|| point_from_value(footprint.get("screen_position")))?;
    Some((
        center.0 - width / 2.0,
        center.1 - height / 2.0,
        width,
        height,
    ))
}

fn union_rects(rects: &[(f64, f64, f64, f64)]) -> Option<(f64, f64, f64, f64)> {
    let first = *rects.first()?;
    let (mut min_x, mut min_y, mut max_x, mut max_y) =
        (first.0, first.1, first.0 + first.2, first.1 + first.3);
    for rect in rects.iter().copied().skip(1) {
        min_x = min_x.min(rect.0);
        min_y = min_y.min(rect.1);
        max_x = max_x.max(rect.0 + rect.2);
        max_y = max_y.max(rect.1 + rect.3);
    }
    Some((min_x, min_y, max_x - min_x, max_y - min_y))
}

fn rects_intersect(a: (f64, f64, f64, f64), b: (f64, f64, f64, f64)) -> bool {
    let (ax2, ay2) = (a.0 + a.2, a.1 + a.3);
    let (bx2, by2) = (b.0 + b.2, b.1 + b.3);
    a.0 <= bx2 && ax2 >= b.0 && a.1 <= by2 && ay2 >= b.1
}

fn collider_extends_far_beyond_visual(
    collision: (f64, f64, f64, f64),
    visual: (f64, f64, f64, f64),
) -> bool {
    let threshold = 8.0_f64.max(visual.2.max(visual.3) * 0.25);
    let left = (visual.0 - collision.0).max(0.0);
    let top = (visual.1 - collision.1).max(0.0);
    let right = (collision.0 + collision.2 - visual.0 - visual.2).max(0.0);
    let bottom = (collision.1 + collision.3 - visual.1 - visual.3).max(0.0);
    [left, top, right, bottom]
        .into_iter()
        .any(|extension| extension > threshold)
}

fn collider_center_offset_from_visual(
    collision: (f64, f64, f64, f64),
    visual: (f64, f64, f64, f64),
) -> bool {
    let collision_center = (
        collision.0 + collision.2 / 2.0,
        collision.1 + collision.3 / 2.0,
    );
    let visual_center = (visual.0 + visual.2 / 2.0, visual.1 + visual.3 / 2.0);
    let distance = ((collision_center.0 - visual_center.0).powi(2)
        + (collision_center.1 - visual_center.1).powi(2))
    .sqrt();
    let visual_diagonal = (visual.2.powi(2) + visual.3.powi(2)).sqrt();
    distance > 8.0_f64.max(visual_diagonal * 0.25)
}

fn comparison_details(
    collision: (f64, f64, f64, f64),
    visual: (f64, f64, f64, f64),
) -> Map<String, Value> {
    Map::from_iter([
        ("collision_aabb".to_string(), rect_json(collision)),
        ("visual_footprint_aabb".to_string(), rect_json(visual)),
    ])
}

fn rect_json(rect: (f64, f64, f64, f64)) -> Value {
    json!({"x": rect.0, "y": rect.1, "w": rect.2, "h": rect.3})
}

fn screenshot_artifact_path(data: &Map<String, Value>) -> Option<PathBuf> {
    let artifacts = data.get("artifacts")?.as_object()?;
    for key in ["screenshot", "png", "path"] {
        if let Some(path) = artifacts.get(key).and_then(Value::as_str) {
            return Some(PathBuf::from(path));
        }
    }
    None
}

fn resolve_scenario_artifact_root(
    config: &ProjectConfig,
    scenario: &str,
) -> Result<PathBuf, FailureDetail> {
    let scenario = scenario.trim();
    if scenario.is_empty() {
        return Err(FailureDetail {
            kind: "invalid_params".to_string(),
            command: Some(COMMAND_TEXT.to_string()),
            message: Some("physics overlay requires --scenario to be non-empty".to_string()),
            expected: Some(json!("scenario artifact name")),
            actual: Some(json!(scenario)),
            ..FailureDetail::default()
        });
    }
    let scenario_root = config.artifacts_dir.join(scenario);
    let latest = scenario_root.join("latest");
    if latest.join("result.json").is_file() {
        return Ok(latest);
    }
    let mut candidates = Vec::new();
    if let Ok(entries) = fs::read_dir(&scenario_root) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() && path.join("result.json").is_file() {
                candidates.push(path);
            }
        }
    }
    candidates.sort_by(|a, b| a.file_name().cmp(&b.file_name()));
    candidates.pop().ok_or_else(|| FailureDetail {
        kind: "artifact_missing".to_string(),
        command: Some(COMMAND_TEXT.to_string()),
        path: Some(scenario_root.to_string_lossy().into_owned()),
        message: Some("scenario artifact root could not be resolved".to_string()),
        expected: Some(json!(
            "latest or lexicographically newest run with result.json"
        )),
        actual: Some(json!("missing")),
        ..FailureDetail::default()
    })
}

fn matching_overlay_evidence<'a>(
    result: &'a Value,
    snapshot: &str,
    params: &Map<String, Value>,
) -> Result<&'a Value, FailureDetail> {
    let overlays = result
        .get("physics_overlays")
        .and_then(Value::as_object)
        .ok_or_else(|| FailureDetail {
            kind: "artifact_missing".to_string(),
            command: Some(COMMAND_TEXT.to_string()),
            message: Some("scenario result did not contain physics overlay metadata".to_string()),
            expected: Some(json!("result.json physics_overlays object")),
            actual: Some(json!("missing")),
            ..FailureDetail::default()
        })?;
    let requested_target = params.get("target").and_then(Value::as_str).unwrap_or("");
    let requested_viewport = params
        .get("viewport")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let mut keys: Vec<_> = overlays.keys().collect();
    keys.sort();
    for key in keys {
        let Some(entry) = overlays.get(key) else {
            continue;
        };
        let Some(entry_object) = entry.as_object() else {
            continue;
        };
        if entry_object.get("snapshot").and_then(Value::as_str) != Some(snapshot) {
            continue;
        }
        if requested_viewport {
            if entry_object
                .get("viewport")
                .and_then(Value::as_bool)
                .unwrap_or(false)
            {
                return Ok(entry);
            }
            continue;
        }
        if entry_object.get("target").and_then(Value::as_str) == Some(requested_target) {
            return Ok(entry);
        }
    }
    Err(FailureDetail {
        kind: "artifact_missing".to_string(),
        command: Some(COMMAND_TEXT.to_string()),
        message: Some(
            "matching physics overlay metadata was not found for the requested snapshot"
                .to_string(),
        ),
        expected: Some(json!({
            "snapshot": snapshot,
            "target": requested_target,
            "viewport": requested_viewport
        })),
        actual: Some(json!(overlays)),
        ..FailureDetail::default()
    })
}

fn overlay_collection_from_evidence(evidence: &Value) -> Option<Map<String, Value>> {
    let object = evidence.as_object()?;
    for key in ["metadata", "collection", "overlay"] {
        if let Some(collection) = object.get(key).and_then(Value::as_object) {
            return Some(collection.clone());
        }
    }
    None
}

fn review_data(
    run_root: &Path,
    snapshot: &str,
    paths: Option<&OverlayArtifactPaths>,
) -> Map<String, Value> {
    let mut data = Map::from_iter([
        ("run_root".to_string(), json!(run_root)),
        ("snapshot".to_string(), json!(snapshot)),
    ]);
    if let Some(paths) = paths {
        data.insert(
            "artifacts".to_string(),
            Value::Object(artifact_context(paths)),
        );
    }
    data
}

fn read_json(path: &Path) -> Result<Value, std::io::Error> {
    let raw = fs::read_to_string(path)?;
    serde_json::from_str(&raw).map_err(std::io::Error::other)
}

fn retag_failure_result(mut result: CommandResult, message: &str) -> CommandResult {
    result.command = COMMAND.to_string();
    result.message = message.to_string();
    for failure in &mut result.failures {
        failure.command = Some(COMMAND_TEXT.to_string());
    }
    result
}

fn failed(message: &str, mut failure: FailureDetail, data: Map<String, Value>) -> CommandResult {
    failure.command.get_or_insert(COMMAND_TEXT.to_string());
    CommandResult {
        ok: false,
        command: COMMAND.to_string(),
        message: message.to_string(),
        data,
        failures: vec![failure],
        exit_code: 1,
    }
}

fn write_json(path: &Path, value: &Value) -> Result<(), std::io::Error> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(
        path,
        serde_json::to_string_pretty(value).unwrap_or_else(|_| "{}".to_string()),
    )
}

fn artifact_write_failure(path: &Path, error: &dyn std::fmt::Display) -> FailureDetail {
    FailureDetail {
        kind: "artifact_write_failed".to_string(),
        command: Some(COMMAND_TEXT.to_string()),
        path: Some(path.to_string_lossy().into_owned()),
        message: Some("physics overlay artifact could not be written".to_string()),
        expected: Some(json!("writable artifact path")),
        actual: Some(json!(error.to_string())),
        ..FailureDetail::default()
    }
}

fn absolute_path(path: &Path) -> String {
    if path.is_absolute() {
        return path.to_string_lossy().into_owned();
    }
    std::env::current_dir()
        .unwrap_or_else(|_| PathBuf::from("."))
        .join(path)
        .to_string_lossy()
        .into_owned()
}

fn unique_suffix() -> String {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_nanos().to_string())
        .unwrap_or_else(|_| "0".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use godot_qa_core::config::{GodotAudioMode, GodotLaunchMode, ProjectConfig};
    use serde_json::json;
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
            godot_launch_mode: GodotLaunchMode::Auto,
            godot_audio_mode: GodotAudioMode::Auto,
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

    fn params(strict: bool) -> Map<String, Value> {
        Map::from_iter([
            ("target".to_string(), json!("qa:actor")),
            ("radius".to_string(), json!(400.0)),
            ("strict".to_string(), json!(strict)),
        ])
    }

    fn shape_rect(x: f64, y: f64, w: f64, h: f64) -> Value {
        json!({
            "logical_id": "shape",
            "world_aabb": {"x": x, "y": y, "w": w, "h": h},
            "is_body": true,
            "is_area": false
        })
    }

    fn footprint_rect(x: f64, y: f64, w: f64, h: f64) -> Value {
        json!({
            "qa_id": "actor",
            "world_aabb": {"x": x, "y": y, "w": w, "h": h}
        })
    }

    fn collection(shapes: Vec<Value>, footprints: Option<Vec<Value>>) -> Map<String, Value> {
        let project_state = match footprints {
            Some(footprints) => json!({
                "enabled": true,
                "available": true,
                "visual_footprints": footprints,
            }),
            None => json!({
                "enabled": true,
                "available": false,
            }),
        };
        Map::from_iter([
            ("target".to_string(), json!("qa:actor")),
            ("sample".to_string(), json!({"radius": 400.0})),
            ("metadata".to_string(), json!({})),
            ("shapes".to_string(), Value::Array(shapes)),
            ("project_state".to_string(), project_state),
        ])
    }

    #[test]
    fn physics_overlay_snapshot_review_renders_from_saved_screenshot_and_metadata() {
        let root = temp_dir("physics-overlay-snapshot-review");
        let config = test_config(&root);
        let run_root = config.artifacts_dir.join("physics_demo/20260517T000000Z");
        let screenshot = run_root.join("snapshots/after/screenshot.png");
        fs::create_dir_all(screenshot.parent().unwrap()).unwrap();
        image::RgbaImage::from_pixel(64, 64, image::Rgba([0, 0, 0, 255]))
            .save(&screenshot)
            .unwrap();
        write_json(
            &run_root.join("result.json"),
            &json!({
                "ok": true,
                "snapshots": ["after"],
                "physics_overlays": {
                    "1:actor": {
                        "step_index": 1,
                        "name": "actor",
                        "target": "qa:actor",
                        "snapshot": "after",
                        "metadata": {
                            "target": "qa:actor",
                            "sample": {"radius": 400.0},
                            "metadata": {"shape_count": 1, "body_count": 1, "area_count": 0, "contact_count": 0},
                            "shapes": [{
                                "screen_points": [{"x": 10, "y": 10}, {"x": 40, "y": 10}, {"x": 40, "y": 40}, {"x": 10, "y": 40}],
                                "screen_aabb": {"x": 10, "y": 10, "w": 30, "h": 30},
                                "is_body": true,
                                "is_area": false
                            }],
                            "contacts": [],
                            "project_state": {"available": false}
                        }
                    }
                }
            }),
        )
        .unwrap();
        let out = root.join("review.png");
        let result = physics_overlay_from_snapshot_result(
            &config,
            params(false),
            "physics_demo",
            "after",
            out.clone(),
        );

        assert!(result.ok, "{:#?}", result.failures);
        assert!(out.is_file());
        assert!(root.join("review.base.png").is_file());
        assert!(root.join("review.overlay.json").is_file());
        assert_eq!(result.data["snapshot"], json!("after"));
        assert_eq!(result.data["summary"]["shape_count"], json!(1));
        let rendered = image::open(&out).unwrap().to_rgba8();
        assert_eq!(rendered.get_pixel(10, 10).0, [255, 220, 0, 255]);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn physics_overlay_snapshot_review_fails_without_matching_metadata() {
        let root = temp_dir("physics-overlay-snapshot-missing-metadata");
        let config = test_config(&root);
        let run_root = config.artifacts_dir.join("physics_demo/latest");
        let screenshot = run_root.join("snapshots/after/screenshot.png");
        fs::create_dir_all(screenshot.parent().unwrap()).unwrap();
        image::RgbaImage::from_pixel(8, 8, image::Rgba([0, 0, 0, 255]))
            .save(&screenshot)
            .unwrap();
        write_json(
            &run_root.join("result.json"),
            &json!({"ok": true, "snapshots": ["after"]}),
        )
        .unwrap();
        let result = physics_overlay_from_snapshot_result(
            &config,
            params(false),
            "physics_demo",
            "after",
            root.join("review.png"),
        );

        assert!(!result.ok);
        assert_eq!(result.failures[0].kind, "artifact_missing");
        assert!(
            result.failures[0]
                .message
                .as_deref()
                .unwrap_or_default()
                .contains("physics overlay metadata")
        );
        let _ = fs::remove_dir_all(root);
    }

    fn warning_ids(warnings: &[Value]) -> Vec<String> {
        warnings
            .iter()
            .filter_map(|warning| warning.get("id").and_then(Value::as_str))
            .map(str::to_string)
            .collect()
    }

    fn collect_warnings(
        collection: &Map<String, Value>,
        params: &Map<String, Value>,
    ) -> Vec<Value> {
        overlay_warnings(collection, params, None, None)
    }

    #[test]
    fn physics_overlay_warning_no_shape_includes_target_and_radius() {
        let warnings = collect_warnings(&collection(Vec::new(), None), &params(false));
        assert!(
            warning_ids(&warnings)
                .contains(&"physics_overlay.no_collision_shapes_found".to_string())
        );
        let warning = warnings
            .iter()
            .find(|warning| warning["id"] == "physics_overlay.no_collision_shapes_found")
            .unwrap();
        assert_eq!(warning["target"], "qa:actor");
        assert_eq!(warning["radius"], json!(400.0));
    }

    #[test]
    fn physics_overlay_warning_visual_comparisons_omitted_when_provider_unavailable() {
        let warnings = collect_warnings(
            &collection(vec![shape_rect(0.0, 0.0, 20.0, 20.0)], None),
            &params(false),
        );
        let ids = warning_ids(&warnings);
        assert!(!ids.contains(&"physics_overlay.collision_without_visual_footprint".to_string()));
        assert!(!ids.contains(&"physics_overlay.visual_footprint_without_collision".to_string()));
    }

    #[test]
    fn physics_overlay_warning_collision_without_visual_footprint() {
        let warnings = collect_warnings(
            &collection(vec![shape_rect(0.0, 0.0, 20.0, 20.0)], Some(Vec::new())),
            &params(false),
        );
        assert!(
            warning_ids(&warnings)
                .contains(&"physics_overlay.collision_without_visual_footprint".to_string())
        );
    }

    #[test]
    fn physics_overlay_warning_visual_footprint_without_collision() {
        let warnings = collect_warnings(
            &collection(Vec::new(), Some(vec![footprint_rect(0.0, 0.0, 20.0, 20.0)])),
            &params(false),
        );
        let ids = warning_ids(&warnings);
        assert!(ids.contains(&"physics_overlay.no_collision_shapes_found".to_string()));
        assert!(ids.contains(&"physics_overlay.visual_footprint_without_collision".to_string()));
    }

    #[test]
    fn physics_overlay_warning_oversized_collider() {
        let warnings = collect_warnings(
            &collection(
                vec![shape_rect(-40.0, -40.0, 100.0, 100.0)],
                Some(vec![footprint_rect(0.0, 0.0, 20.0, 20.0)]),
            ),
            &params(false),
        );
        assert!(
            warning_ids(&warnings).contains(
                &"physics_overlay.collider_extends_far_beyond_visual_footprint".to_string()
            )
        );
    }

    #[test]
    fn physics_overlay_warning_center_offset() {
        let warnings = collect_warnings(
            &collection(
                vec![shape_rect(40.0, 0.0, 20.0, 20.0)],
                Some(vec![footprint_rect(0.0, 0.0, 20.0, 20.0)]),
            ),
            &params(false),
        );
        assert!(
            warning_ids(&warnings).contains(
                &"physics_overlay.collider_center_offset_from_visual_footprint".to_string()
            )
        );
    }

    #[test]
    fn physics_overlay_warning_target_offscreen() {
        let mut collection = collection(
            vec![shape_rect(0.0, 0.0, 20.0, 20.0)],
            Some(vec![footprint_rect(0.0, 0.0, 20.0, 20.0)]),
        );
        collection.insert(
            "target_node".to_string(),
            json!({"screen_aabb": {"x": 200, "y": 200, "w": 20, "h": 20}}),
        );
        let warnings = overlay_warnings(&collection, &params(false), None, Some((100, 100)));
        assert!(warning_ids(&warnings).contains(&"physics_overlay.target_offscreen".to_string()));
    }

    #[test]
    fn physics_overlay_warning_truncation() {
        let mut collection = collection(vec![shape_rect(0.0, 0.0, 20.0, 20.0)], None);
        collection.insert(
            "metadata".to_string(),
            json!({
                "truncated": true,
                "max_shapes": 1,
                "shape_count_total": 2,
                "shape_count_returned": 1
            }),
        );
        let warnings = collect_warnings(&collection, &params(false));
        assert!(
            warning_ids(&warnings)
                .contains(&"physics_overlay.unbounded_shape_count_truncated".to_string())
        );
    }

    #[test]
    fn physics_overlay_warning_strict_mode_converts_warnings_to_failure() {
        let warnings = collect_warnings(&collection(Vec::new(), None), &params(false));
        assert!(strict_overlay_warning_failure(&params(false), &warnings).is_none());
        let failure = strict_overlay_warning_failure(&params(true), &warnings).unwrap();
        assert_eq!(failure.kind, "physics_overlay_strict_warning");
        assert_eq!(failure.severity.as_deref(), Some("error"));
        assert_eq!(failure.actual, Some(Value::Array(warnings)));
    }
}
