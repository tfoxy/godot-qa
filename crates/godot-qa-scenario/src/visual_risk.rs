use crate::screenshots;
use godot_qa_core::results::FailureDetail;
use image::GenericImageView;
use serde_json::{Map, Value, json};
use std::collections::{HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};

pub const RULE_LOW_VISUAL_DIVERSITY: &str = "visual-risk.low_visual_diversity";
pub const RULE_HUGE_FLAT_SHAPE: &str = "visual-risk.huge_flat_shape";
pub const RULE_DEBUG_PRIMARY_COLORS: &str = "visual-risk.debug_primary_colors";
pub const RULE_MISSING_VISIBLE_TARGET_PIXELS: &str = "visual-risk.missing_visible_target_pixels";

#[derive(Debug, Clone)]
pub struct VisualRiskOptions {
    pub low_diversity_unique_color_limit: u64,
    pub low_diversity_dominant_ratio: f64,
    pub huge_flat_shape_ratio: f64,
    pub huge_flat_shape_min_pixels: u64,
    pub debug_primary_ratio: f64,
}

impl Default for VisualRiskOptions {
    fn default() -> Self {
        Self {
            low_diversity_unique_color_limit: 8,
            low_diversity_dominant_ratio: 0.90,
            huge_flat_shape_ratio: 0.70,
            huge_flat_shape_min_pixels: 16_384,
            debug_primary_ratio: 0.35,
        }
    }
}

pub type VisualRiskFinding = FailureDetail;

#[derive(Debug, Clone)]
pub struct ScreenshotArtifact {
    pub snapshot: Option<String>,
    pub screenshot: PathBuf,
}

pub fn analyze_result_artifacts(
    result_data: &Map<String, Value>,
    options: &VisualRiskOptions,
) -> Vec<VisualRiskFinding> {
    let mut findings =
        analyze_screenshot_artifacts(collect_screenshot_artifacts(result_data), options);
    findings.extend(collect_target_pixel_findings(result_data));
    findings
}

pub fn collect_target_pixel_findings(result_data: &Map<String, Value>) -> Vec<VisualRiskFinding> {
    let mut findings = Vec::new();
    let mut seen = HashSet::new();
    collect_target_pixel_findings_in_value(
        &Value::Object(result_data.clone()),
        &TargetPixelContext::default(),
        &mut seen,
        &mut findings,
    );
    findings
}

pub fn analyze_screenshot_artifacts(
    artifacts: Vec<ScreenshotArtifact>,
    options: &VisualRiskOptions,
) -> Vec<VisualRiskFinding> {
    let mut findings = Vec::new();
    for artifact in artifacts {
        if !artifact.screenshot.is_file() {
            continue;
        }
        findings.extend(analyze_screenshot_artifact(&artifact, options));
    }
    findings
}

pub fn collect_screenshot_artifacts(result_data: &Map<String, Value>) -> Vec<ScreenshotArtifact> {
    let mut artifacts = Vec::new();
    let mut seen = HashSet::new();

    if let Some(path) = result_data
        .get("artifacts")
        .and_then(Value::as_object)
        .and_then(|artifacts| artifacts.get("screenshot"))
        .and_then(Value::as_str)
    {
        push_artifact(&mut artifacts, &mut seen, None, PathBuf::from(path));
    }

    if let Some(snapshot_artifacts) = result_data
        .get("snapshot_artifacts")
        .and_then(Value::as_object)
    {
        for (snapshot, entry) in snapshot_artifacts {
            if let Some(path) = entry
                .as_object()
                .and_then(|entry| entry.get("screenshot"))
                .and_then(Value::as_str)
            {
                push_artifact(
                    &mut artifacts,
                    &mut seen,
                    Some(snapshot.clone()),
                    PathBuf::from(path),
                );
            }
        }
    }

    if let Some(snapshots_dir) = result_data
        .get("artifacts")
        .and_then(Value::as_object)
        .and_then(|artifacts| artifacts.get("snapshots_dir"))
        .and_then(Value::as_str)
    {
        if let Some(snapshots) = result_data.get("snapshots").and_then(Value::as_array) {
            for snapshot in snapshots.iter().filter_map(Value::as_str) {
                push_artifact(
                    &mut artifacts,
                    &mut seen,
                    Some(snapshot.to_string()),
                    PathBuf::from(snapshots_dir)
                        .join(snapshot)
                        .join("screenshot.png"),
                );
            }
        }
    }

    artifacts
}

fn push_artifact(
    artifacts: &mut Vec<ScreenshotArtifact>,
    seen: &mut HashSet<String>,
    snapshot: Option<String>,
    screenshot: PathBuf,
) {
    let key = screenshot.to_string_lossy().into_owned();
    if seen.insert(key) {
        artifacts.push(ScreenshotArtifact {
            snapshot,
            screenshot,
        });
    }
}

fn analyze_screenshot_artifact(
    artifact: &ScreenshotArtifact,
    options: &VisualRiskOptions,
) -> Vec<VisualRiskFinding> {
    let Ok(quality) = screenshots::analyze_screenshot_file(&artifact.screenshot) else {
        return Vec::new();
    };
    if quality
        .get("low_information")
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        return Vec::new();
    }

    let Ok(measurements) = measure_visual_risk(&artifact.screenshot) else {
        return Vec::new();
    };
    let mut findings = Vec::new();

    if measurements.unique_color_count <= options.low_diversity_unique_color_limit
        && measurements.dominant_color_ratio >= options.low_diversity_dominant_ratio
    {
        findings.push(finding(
            RULE_LOW_VISUAL_DIVERSITY,
            artifact,
            json!({
                "width": measurements.width,
                "height": measurements.height,
                "pixel_count": measurements.pixel_count,
                "unique_color_count": measurements.unique_color_count,
                "dominant_color_ratio": measurements.dominant_color_ratio,
                "low_information": false,
            }),
            "screenshot uses very few visible colors over most pixels",
            "confirm the screenshot is not a placeholder, loading state, or unfinished blockout",
        ));
    }

    if measurements.largest_flat_region_ratio >= options.huge_flat_shape_ratio
        && measurements.largest_flat_region_pixel_count >= options.huge_flat_shape_min_pixels
    {
        findings.push(finding(
            RULE_HUGE_FLAT_SHAPE,
            artifact,
            json!({
                "width": measurements.width,
                "height": measurements.height,
                "pixel_count": measurements.pixel_count,
                "largest_flat_region_pixel_count": measurements.largest_flat_region_pixel_count,
                "largest_flat_region_ratio": measurements.largest_flat_region_ratio,
                "largest_flat_region_rgba": measurements.largest_flat_region_rgba,
            }),
            "one contiguous flat-color region covers most of the screenshot",
            "confirm the region is intentional UI or scene composition rather than a blockout surface",
        ));
    }

    if measurements.debug_primary_ratio >= options.debug_primary_ratio {
        findings.push(finding(
            RULE_DEBUG_PRIMARY_COLORS,
            artifact,
            json!({
                "width": measurements.width,
                "height": measurements.height,
                "pixel_count": measurements.pixel_count,
                "debug_primary_pixel_count": measurements.debug_primary_pixel_count,
                "debug_primary_ratio": measurements.debug_primary_ratio,
            }),
            "debug-primary colors cover a large share of visible pixels",
            "confirm high-saturation red, green, blue, yellow, cyan, or magenta regions are intentional",
        ));
    }

    findings
}

fn finding(
    rule_id: &str,
    artifact: &ScreenshotArtifact,
    actual: Value,
    reason: &str,
    suggested_fix: &str,
) -> VisualRiskFinding {
    let screenshot = artifact.screenshot.to_string_lossy().into_owned();
    let mut details = Map::from_iter([
        ("rule_id".to_string(), json!(rule_id)),
        ("reason".to_string(), json!(reason)),
        ("suggested_fix".to_string(), json!(suggested_fix)),
    ]);
    if let Some(snapshot) = &artifact.snapshot {
        details.insert("snapshot".to_string(), json!(snapshot));
    }

    FailureDetail {
        kind: rule_id.to_string(),
        path: Some(screenshot.clone()),
        message: Some(reason.to_string()),
        actual: Some(actual),
        severity: Some("warning".to_string()),
        artifacts: Some(Map::from_iter([(
            "screenshot".to_string(),
            json!(screenshot),
        )])),
        details,
        ..FailureDetail::default()
    }
}

#[derive(Debug, Default, Clone)]
struct TargetPixelContext {
    target: Option<String>,
    snapshot: Option<String>,
    artifacts: Option<Map<String, Value>>,
}

fn collect_target_pixel_findings_in_value(
    value: &Value,
    context: &TargetPixelContext,
    seen: &mut HashSet<String>,
    findings: &mut Vec<VisualRiskFinding>,
) {
    match value {
        Value::Object(object) => {
            let mut context = context.clone();
            if context.target.is_none() {
                context.target = string_field(object, "target");
            }
            if context.snapshot.is_none() {
                context.snapshot = string_field(object, "snapshot");
            }
            if context.artifacts.is_none() {
                context.artifacts = object.get("artifacts").and_then(Value::as_object).cloned();
            }

            maybe_push_target_pixel_finding(object, &context, seen, findings);

            if let Some(evidence) = object.get("visual_evidence").and_then(Value::as_object) {
                let mut evidence_context = context.clone();
                if evidence_context.target.is_none() {
                    evidence_context.target = string_field(evidence, "target");
                }
                maybe_push_target_pixel_finding(evidence, &evidence_context, seen, findings);
            }

            for child in object.values() {
                collect_target_pixel_findings_in_value(child, &context, seen, findings);
            }
        }
        Value::Array(items) => {
            for item in items {
                collect_target_pixel_findings_in_value(item, context, seen, findings);
            }
        }
        _ => {}
    }
}

fn maybe_push_target_pixel_finding(
    object: &Map<String, Value>,
    context: &TargetPixelContext,
    seen: &mut HashSet<String>,
    findings: &mut Vec<VisualRiskFinding>,
) {
    if object
        .get("low_information")
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        return;
    }
    let count = object
        .get("useful_pixel_count")
        .or_else(|| object.get("visible_pixel_count"))
        .and_then(Value::as_u64);
    let ratio = object
        .get("useful_pixel_ratio")
        .or_else(|| object.get("visible_pixel_ratio"))
        .and_then(Value::as_f64);
    if count.is_none() && ratio.is_none() {
        return;
    }
    if count.unwrap_or(0) > 0 || ratio.unwrap_or(0.0) > 0.0 {
        return;
    }

    let target = string_field(object, "target").or_else(|| context.target.clone());
    let Some(target) = target.filter(|target| !target.is_empty()) else {
        return;
    };
    let target_rect = object
        .get("target_rect")
        .or_else(|| object.get("rect"))
        .cloned();
    if target_rect.is_none() {
        return;
    }
    let snapshot = string_field(object, "snapshot").or_else(|| context.snapshot.clone());
    let screenshot =
        string_field(object, "source_screenshot").or_else(|| string_field(object, "screenshot"));
    let artifacts = object
        .get("artifacts")
        .and_then(Value::as_object)
        .cloned()
        .or_else(|| context.artifacts.clone());
    let key = json!({
        "target": target.clone(),
        "snapshot": snapshot,
        "screenshot": screenshot,
        "target_rect": target_rect,
    })
    .to_string();
    if !seen.insert(key) {
        return;
    }

    let mut actual = Map::from_iter([
        ("target".to_string(), json!(target.clone())),
        (
            "target_rect".to_string(),
            target_rect.unwrap_or(Value::Null),
        ),
        (
            "visible_pixel_count".to_string(),
            count.map_or(Value::Null, Value::from),
        ),
        (
            "visible_pixel_ratio".to_string(),
            ratio.map_or(Value::Null, Value::from),
        ),
    ]);
    if let Some(snapshot) = &snapshot {
        actual.insert("snapshot".to_string(), json!(snapshot));
    }
    if let Some(screenshot) = &screenshot {
        actual.insert("screenshot".to_string(), json!(screenshot));
    }
    if object.contains_key("useful_pixel_count") {
        actual.insert(
            "useful_pixel_count".to_string(),
            object
                .get("useful_pixel_count")
                .cloned()
                .unwrap_or(Value::Null),
        );
    }
    if object.contains_key("useful_pixel_ratio") {
        actual.insert(
            "useful_pixel_ratio".to_string(),
            object
                .get("useful_pixel_ratio")
                .cloned()
                .unwrap_or(Value::Null),
        );
    }

    let mut details = Map::from_iter([
        (
            "rule_id".to_string(),
            json!(RULE_MISSING_VISIBLE_TARGET_PIXELS),
        ),
        (
            "reason".to_string(),
            json!("target was expected to be visible but contributed no measured target pixels"),
        ),
        (
            "suggested_fix".to_string(),
            json!(
                "confirm the target is rendered in the referenced screenshot and is not fully transparent, hidden, or visually identical to the background"
            ),
        ),
    ]);
    if let Some(snapshot) = &snapshot {
        details.insert("snapshot".to_string(), json!(snapshot));
    }

    findings.push(FailureDetail {
        kind: RULE_MISSING_VISIBLE_TARGET_PIXELS.to_string(),
        path: screenshot.clone(),
        message: Some(
            "target was expected to be visible but contributed no measured target pixels"
                .to_string(),
        ),
        actual: Some(Value::Object(actual)),
        target: Some(target),
        severity: Some("warning".to_string()),
        artifacts,
        details,
        ..FailureDetail::default()
    });
}

fn string_field(object: &Map<String, Value>, key: &str) -> Option<String> {
    object
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

#[derive(Debug)]
struct VisualRiskMeasurements {
    width: u32,
    height: u32,
    pixel_count: u64,
    unique_color_count: u64,
    dominant_color_ratio: f64,
    largest_flat_region_pixel_count: u64,
    largest_flat_region_ratio: f64,
    largest_flat_region_rgba: [u8; 4],
    debug_primary_pixel_count: u64,
    debug_primary_ratio: f64,
}

fn measure_visual_risk(path: &Path) -> Result<VisualRiskMeasurements, String> {
    let image = image::ImageReader::open(path)
        .map_err(|error| error.to_string())?
        .decode()
        .map_err(|error| error.to_string())?;
    let (width, height) = image.dimensions();
    let pixels = image.to_rgba8();
    let pixel_count = u64::from(width) * u64::from(height);
    let mut counts: HashMap<[u8; 4], u64> = HashMap::new();
    let mut debug_primary_pixel_count = 0;
    for pixel in pixels.pixels() {
        *counts.entry(pixel.0).or_insert(0) += 1;
        if is_debug_primary(pixel.0) {
            debug_primary_pixel_count += 1;
        }
    }
    let dominant_count = counts.values().copied().max().unwrap_or(0);
    let (largest_flat_region_pixel_count, largest_flat_region_rgba) =
        largest_flat_region(&pixels, width, height);

    Ok(VisualRiskMeasurements {
        width,
        height,
        pixel_count,
        unique_color_count: counts.len() as u64,
        dominant_color_ratio: ratio(dominant_count, pixel_count),
        largest_flat_region_pixel_count,
        largest_flat_region_ratio: ratio(largest_flat_region_pixel_count, pixel_count),
        largest_flat_region_rgba,
        debug_primary_pixel_count,
        debug_primary_ratio: ratio(debug_primary_pixel_count, pixel_count),
    })
}

fn ratio(count: u64, total: u64) -> f64 {
    if total == 0 {
        0.0
    } else {
        count as f64 / total as f64
    }
}

fn is_debug_primary(rgba: [u8; 4]) -> bool {
    if rgba[3] < 16 {
        return false;
    }
    matches!(
        [rgba[0], rgba[1], rgba[2]],
        [255, 0, 0] | [0, 255, 0] | [0, 0, 255] | [255, 255, 0] | [0, 255, 255] | [255, 0, 255]
    )
}

fn largest_flat_region(
    pixels: &image::ImageBuffer<image::Rgba<u8>, Vec<u8>>,
    width: u32,
    height: u32,
) -> (u64, [u8; 4]) {
    let len = (u64::from(width) * u64::from(height)) as usize;
    let mut visited = vec![false; len];
    let mut largest_count = 0;
    let mut largest_color = [0, 0, 0, 0];

    for y in 0..height {
        for x in 0..width {
            let index = pixel_index(x, y, width);
            if visited[index] {
                continue;
            }
            let color = pixels.get_pixel(x, y).0;
            let count = flood_flat_region(pixels, width, height, x, y, color, &mut visited);
            if count > largest_count {
                largest_count = count;
                largest_color = color;
            }
        }
    }

    (largest_count, largest_color)
}

fn flood_flat_region(
    pixels: &image::ImageBuffer<image::Rgba<u8>, Vec<u8>>,
    width: u32,
    height: u32,
    start_x: u32,
    start_y: u32,
    color: [u8; 4],
    visited: &mut [bool],
) -> u64 {
    let mut count = 0;
    let mut queue = VecDeque::from([(start_x, start_y)]);
    while let Some((x, y)) = queue.pop_front() {
        let index = pixel_index(x, y, width);
        if visited[index] || pixels.get_pixel(x, y).0 != color {
            continue;
        }
        visited[index] = true;
        count += 1;
        if x > 0 {
            queue.push_back((x - 1, y));
        }
        if x + 1 < width {
            queue.push_back((x + 1, y));
        }
        if y > 0 {
            queue.push_back((x, y - 1));
        }
        if y + 1 < height {
            queue.push_back((x, y + 1));
        }
    }
    count
}

fn pixel_index(x: u32, y: u32, width: u32) -> usize {
    (u64::from(y) * u64::from(width) + u64::from(x)) as usize
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{ImageBuffer, Rgba};
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_dir(name: &str) -> PathBuf {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("godot-qa-{name}-{stamp}"));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn save_png(path: &Path, width: u32, height: u32, color_at: impl Fn(u32, u32) -> [u8; 4]) {
        let image = ImageBuffer::from_fn(width, height, |x, y| Rgba(color_at(x, y)));
        image.save(path).unwrap();
    }

    fn analyze_one(path: PathBuf) -> Vec<VisualRiskFinding> {
        analyze_screenshot_artifacts(
            vec![ScreenshotArtifact {
                snapshot: Some("after".to_string()),
                screenshot: path,
            }],
            &VisualRiskOptions::default(),
        )
    }

    #[test]
    fn visual_risk_reports_no_findings_for_varied_screenshot() {
        let root = temp_dir("visual-risk-varied");
        let path = root.join("varied.png");
        save_png(&path, 160, 160, |x, y| {
            [
                ((x * 3 + y) % 251) as u8,
                ((x + y * 5) % 241) as u8,
                ((x * 7 + y * 11) % 239) as u8,
                255,
            ]
        });

        assert!(analyze_one(path).is_empty());
    }

    #[test]
    fn visual_risk_reports_low_diversity_with_artifact_path_and_facts() {
        let root = temp_dir("visual-risk-low-diversity");
        let path = root.join("low.png");
        save_png(&path, 160, 160, |x, y| {
            if x < 148 || y % 2 == 0 {
                [40, 40, 48, 255]
            } else {
                [80, 80, 88, 255]
            }
        });

        let findings = analyze_one(path.clone());
        let finding = findings
            .iter()
            .find(|finding| finding.kind == RULE_LOW_VISUAL_DIVERSITY)
            .unwrap();
        assert_eq!(finding.severity.as_deref(), Some("warning"));
        assert_eq!(finding.details["rule_id"], RULE_LOW_VISUAL_DIVERSITY);
        assert_eq!(finding.details["snapshot"], "after");
        assert_eq!(
            finding.artifacts.as_ref().unwrap()["screenshot"],
            json!(path.to_string_lossy().into_owned())
        );
        assert_eq!(finding.actual.as_ref().unwrap()["unique_color_count"], 2);
        assert_eq!(
            finding.actual.as_ref().unwrap()["low_information"],
            Value::Bool(false)
        );
    }

    #[test]
    fn visual_risk_reports_huge_flat_shape() {
        let root = temp_dir("visual-risk-flat-shape");
        let path = root.join("shape.png");
        save_png(&path, 160, 160, |x, y| {
            if x < 138 {
                [24, 32, 40, 255]
            } else {
                [
                    ((x * 13 + y * 17) % 253) as u8,
                    ((x * 19 + y * 23) % 251) as u8,
                    ((x * 29 + y * 31) % 249) as u8,
                    255,
                ]
            }
        });

        let findings = analyze_one(path);
        let finding = findings
            .iter()
            .find(|finding| finding.kind == RULE_HUGE_FLAT_SHAPE)
            .unwrap();
        assert!(
            finding.actual.as_ref().unwrap()["largest_flat_region_ratio"]
                .as_f64()
                .unwrap()
                >= 0.70
        );
    }

    #[test]
    fn visual_risk_reports_excessive_debug_primary_colors() {
        let root = temp_dir("visual-risk-debug-primary");
        let path = root.join("debug.png");
        save_png(&path, 160, 160, |x, y| {
            if x < 64 {
                [255, 0, 0, 255]
            } else if x < 96 {
                [0, 255, 0, 255]
            } else {
                [
                    ((x * 13 + y * 17) % 253) as u8,
                    ((x * 19 + y * 23) % 251) as u8,
                    ((x * 29 + y * 31) % 249) as u8,
                    255,
                ]
            }
        });

        let findings = analyze_one(path);
        let finding = findings
            .iter()
            .find(|finding| finding.kind == RULE_DEBUG_PRIMARY_COLORS)
            .unwrap();
        assert!(
            finding.actual.as_ref().unwrap()["debug_primary_ratio"]
                .as_f64()
                .unwrap()
                >= 0.35
        );
    }

    #[test]
    fn visual_risk_does_not_reclassify_low_information_screenshots() {
        let root = temp_dir("visual-risk-low-information");
        let path = root.join("blank.png");
        save_png(&path, 160, 160, |_x, _y| [0, 0, 0, 255]);

        assert!(analyze_one(path).is_empty());
    }

    #[test]
    fn visual_risk_missing_visible_target_pixels() {
        let root = temp_dir("visual-risk-missing-target-pixels");
        let screenshot = root.join("snapshots/after/screenshot.png");
        let overlay = root.join("snapshots/after/visual/hero_overlay.png");
        let screenshot = screenshot.to_string_lossy().into_owned();
        let overlay = overlay.to_string_lossy().into_owned();
        let result_data = Map::from_iter([(
            "saved_snapshot_pixels".to_string(),
            json!([
                {
                    "snapshot": "after",
                    "assertion": {
                        "saved_snapshot_pixels": {
                            "target": "qa:hero",
                            "snapshot": "after",
                            "min_count": 0
                        }
                    },
                    "artifacts": {
                        "screenshot": screenshot.clone(),
                        "overlay": overlay
                    },
                    "visual_evidence": {
                        "target": "qa:hero",
                        "source_screenshot": screenshot.clone(),
                        "target_rect": [12.0, 20.0, 32.0, 16.0],
                        "measured_pixel_count": 512,
                        "useful_pixel_count": 0,
                        "useful_pixel_ratio": 0.0,
                        "low_information": false
                    }
                }
            ]),
        )]);

        let findings = collect_target_pixel_findings(&result_data);

        assert_eq!(findings.len(), 1);
        let finding = &findings[0];
        assert_eq!(finding.kind, RULE_MISSING_VISIBLE_TARGET_PIXELS);
        assert_eq!(finding.severity.as_deref(), Some("warning"));
        assert_eq!(finding.target.as_deref(), Some("qa:hero"));
        assert_eq!(finding.details["snapshot"], json!("after"));
        assert_eq!(finding.path.as_deref(), Some(screenshot.as_str()));
        assert_eq!(
            finding.artifacts.as_ref().unwrap()["screenshot"],
            json!(screenshot)
        );
        assert_eq!(
            finding.actual.as_ref().unwrap()["useful_pixel_count"],
            json!(0)
        );
        assert_eq!(
            finding.actual.as_ref().unwrap()["useful_pixel_ratio"],
            json!(0.0)
        );
        assert_eq!(
            finding.actual.as_ref().unwrap()["target_rect"],
            json!([12.0, 20.0, 32.0, 16.0])
        );
    }
}
