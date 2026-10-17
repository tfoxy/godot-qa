use godot_qa_core::results::FailureDetail;
use godot_qa_core::{config::ProjectConfig, results::CommandResult};
use godot_qa_runtime as runtime;
use image::RgbaImage;
use serde::Serialize;
use serde_json::{Map, Value, json};
use std::cmp::min;
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use crate::screenshots::{LiveScreenshotCapture, capture_live_screenshot};

pub const RULE_TARGET_MISSING: &str = "visual_audit.target_missing";
pub const RULE_TARGET_OFFSCREEN: &str = "visual_audit.target_offscreen";
pub const RULE_TARGET_TOO_SMALL: &str = "visual_audit.target_too_small";
pub const RULE_LOW_LUMINANCE_CONTRAST: &str = "visual_audit.low_luminance_contrast";
pub const RULE_LOW_COLOR_DISTANCE: &str = "visual_audit.low_color_distance";
pub const RULE_LOW_EDGE_CONTRAST: &str = "visual_audit.low_edge_contrast";
pub const RULE_TARGET_BOUNDS_UNAVAILABLE: &str = "visual_audit.target_bounds_unavailable";
pub const RULE_SEQUENCE_TARGET_MISSING_FRAMES: &str = "visual_audit.sequence_target_missing_frames";
pub const RULE_ANNOTATED_ARTIFACT_FAILED: &str = "visual_audit.annotated_artifact_failed";

#[derive(Debug, Clone)]
pub struct VisualAuditOptions {
    pub min_pixel_area: u64,
    pub min_width: u32,
    pub min_height: u32,
    pub min_contrast_ratio: f64,
    pub min_color_distance: f64,
    pub min_edge_contrast: f64,
    pub strict_warnings: bool,
}

impl Default for VisualAuditOptions {
    fn default() -> Self {
        Self {
            min_pixel_area: 256,
            min_width: 16,
            min_height: 16,
            min_contrast_ratio: 3.0,
            min_color_distance: 24.0,
            min_edge_contrast: 0.08,
            strict_warnings: false,
        }
    }
}

pub type VisualAuditFinding = FailureDetail;

#[derive(Debug, Clone, Serialize)]
pub struct VisualAuditReport {
    pub screenshot_path: String,
    pub width: u32,
    pub height: u32,
    pub targets: Vec<VisualAuditTargetMetrics>,
    pub findings: Vec<VisualAuditFinding>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub frames_checked: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub frames_with_target_missing: Option<BTreeMap<String, u64>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub minimum_pixel_area_across_frames: Option<BTreeMap<String, u64>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub minimum_contrast_across_frames: Option<BTreeMap<String, f64>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub worst_frame_path_by_rule: Option<BTreeMap<String, BTreeMap<String, String>>>,
}

#[derive(Debug)]
pub struct LiveVisualAuditReport {
    pub report: VisualAuditReport,
    pub capture: LiveScreenshotCapture,
    pub target_evidence: Value,
}

#[derive(Debug, Clone, Serialize)]
pub struct VisualAuditTargetMetrics {
    pub target_id: String,
    pub screenshot_path: String,
    pub inside_viewport: bool,
    pub pixel_area: u64,
    pub bounding_box_width: u32,
    pub bounding_box_height: u32,
    pub visible_pixel_count: Option<u64>,
    pub estimated_local_background_color: Option<[u8; 4]>,
    pub estimated_target_color_or_median: Option<[u8; 4]>,
    pub relative_luminance_target: Option<f64>,
    pub relative_luminance_background: Option<f64>,
    pub contrast_ratio_estimate: Option<f64>,
    pub local_color_distance: Option<f64>,
    pub edge_contrast_estimate: Option<f64>,
    pub small_target_warning: bool,
    pub low_contrast_warning: bool,
    pub offscreen_warning: bool,
    pub occlusion_or_missing_pixels_warning: Option<bool>,
    pub target_rect: [f64; 4],
    pub clamped_rect: Option<[u32; 4]>,
}

#[derive(Debug, Clone, Copy)]
struct Rect {
    x: f64,
    y: f64,
    width: f64,
    height: f64,
}

#[derive(Debug)]
struct TargetEvidence {
    target_id: String,
    rect: Option<Rect>,
    visible_pixel_count: Option<u64>,
}

pub fn audit_screenshot(
    screenshot_path: impl AsRef<Path>,
    target_bounds_evidence: &Value,
    options: &VisualAuditOptions,
) -> Result<VisualAuditReport, String> {
    let screenshot_path = screenshot_path.as_ref();
    let image = image::open(screenshot_path)
        .map_err(|err| format!("failed to load screenshot PNG: {err}"))?
        .to_rgba8();
    let (width, height) = image.dimensions();
    let screenshot = screenshot_path.to_string_lossy().into_owned();
    let mut report = VisualAuditReport {
        screenshot_path: screenshot.clone(),
        width,
        height,
        targets: Vec::new(),
        findings: Vec::new(),
        frames_checked: None,
        frames_with_target_missing: None,
        minimum_pixel_area_across_frames: None,
        minimum_contrast_across_frames: None,
        worst_frame_path_by_rule: None,
    };

    for target in normalize_targets(target_bounds_evidence) {
        let Some(rect) = target.rect.filter(valid_rect) else {
            report.findings.push(finding(
                RULE_TARGET_BOUNDS_UNAVAILABLE,
                &screenshot,
                Some(&target.target_id),
                json!({
                    "target_id": target.target_id,
                    "screenshot_path": screenshot,
                    "bounds_available": false
                }),
                "target bounds were missing or malformed",
                "provide viewport-space target bounds as [x, y, width, height] or {x, y, width, height}",
                options,
            ));
            continue;
        };

        let clamped = clamp_rect(rect, width, height);
        let inside_viewport = clamped.is_some();
        if let Some(clamped) = clamped {
            let metrics = measure_target(
                &image,
                &screenshot,
                &target.target_id,
                rect,
                clamped,
                target.visible_pixel_count,
                options,
            );
            push_metric_findings(&metrics, options, &mut report.findings);
            report.targets.push(metrics);
        } else {
            let metrics = VisualAuditTargetMetrics {
                target_id: target.target_id.clone(),
                screenshot_path: screenshot.clone(),
                inside_viewport,
                pixel_area: 0,
                bounding_box_width: 0,
                bounding_box_height: 0,
                visible_pixel_count: target.visible_pixel_count,
                estimated_local_background_color: None,
                estimated_target_color_or_median: None,
                relative_luminance_target: None,
                relative_luminance_background: None,
                contrast_ratio_estimate: None,
                local_color_distance: None,
                edge_contrast_estimate: None,
                small_target_warning: true,
                low_contrast_warning: false,
                offscreen_warning: true,
                occlusion_or_missing_pixels_warning: target
                    .visible_pixel_count
                    .map(|count| count == 0),
                target_rect: rect_array(rect),
                clamped_rect: None,
            };
            push_metric_findings(&metrics, options, &mut report.findings);
            report.targets.push(metrics);
        }
    }

    Ok(report)
}

pub fn audit_sequence_manifest(
    manifest_path: impl AsRef<Path>,
    requested_targets: &[String],
    options: &VisualAuditOptions,
) -> Result<VisualAuditReport, String> {
    let manifest_path = manifest_path.as_ref();
    let manifest = read_json_value(manifest_path)?;
    let sequence_dir = manifest_path
        .parent()
        .ok_or_else(|| "sequence manifest path has no parent directory".to_string())?;
    let frames = manifest
        .get("frames")
        .and_then(Value::as_array)
        .ok_or_else(|| "sequence manifest field 'frames' must be an array".to_string())?;
    let wanted = requested_target_set(requested_targets);
    let mut aggregate = AggregateAudit::new(manifest_path);

    for frame in frames {
        let Some(relative_path) = frame.get("path").and_then(Value::as_str) else {
            return Err("sequence manifest frame entries must include a string path".to_string());
        };
        let screenshot_path = resolve_relative_path(sequence_dir, relative_path);
        if !screenshot_path.is_file() {
            return Err(format!(
                "sequence frame screenshot was not found: {}",
                screenshot_path.to_string_lossy()
            ));
        }
        let evidence = filter_evidence_targets(frame_target_evidence(frame), &wanted);
        aggregate.record_missing_targets(&evidence, &wanted);
        let report = audit_screenshot(&screenshot_path, &evidence, options)?;
        aggregate.push_frame_report(report);
    }

    Ok(aggregate.finish(options))
}

pub fn audit_scenario_artifacts(
    artifacts_dir: impl AsRef<Path>,
    scenario: Option<&str>,
    run_id: Option<&str>,
    requested_targets: &[String],
    options: &VisualAuditOptions,
) -> Result<VisualAuditReport, String> {
    let run_root = resolve_scenario_run_root(artifacts_dir.as_ref(), scenario, run_id)?;
    let result_path = run_root.join("result.json");
    let result = read_json_value(&result_path)?;
    let snapshot_artifacts = result
        .get("snapshot_artifacts")
        .and_then(Value::as_object)
        .ok_or_else(|| {
            "scenario result.json field 'snapshot_artifacts' must be an object".to_string()
        })?;
    let wanted = requested_target_set(requested_targets);
    let mut aggregate = AggregateAudit::new(&result_path);

    for snapshot in snapshot_artifacts.values() {
        let Some(screenshot) = snapshot.get("screenshot").and_then(Value::as_str) else {
            continue;
        };
        let screenshot_path = resolve_relative_path(&run_root, screenshot);
        if !screenshot_path.is_file() {
            return Err(format!(
                "scenario snapshot screenshot was not found: {}",
                screenshot_path.to_string_lossy()
            ));
        }
        let evidence = filter_evidence_targets(frame_target_evidence(snapshot), &wanted);
        aggregate.record_missing_targets(&evidence, &wanted);
        let report = audit_screenshot(&screenshot_path, &evidence, options)?;
        aggregate.push_frame_report(report);
    }

    if aggregate.frames_checked == 0 {
        return Err("scenario result.json did not contain any snapshot screenshots".to_string());
    }

    Ok(aggregate.finish(options))
}

pub fn audit_live_session(
    config: &ProjectConfig,
    command: &str,
    targets: &[String],
    options: &VisualAuditOptions,
) -> Result<LiveVisualAuditReport, CommandResult> {
    let capture = capture_live_screenshot(config, command, "visual_audit".to_string())?;
    let result = runtime::run_live_protocol_command(
        config,
        command,
        runtime::VISUAL_AUDIT_TARGETS,
        Map::from_iter([
            ("targets".to_string(), json!(targets)),
            ("include_state".to_string(), json!(true)),
            (
                "state_methods".to_string(),
                json!([
                    "get_qa_visual_state",
                    "get_visual_qa_state",
                    "get_qa_readability_state"
                ]),
            ),
        ]),
    );
    if !result.ok {
        return Err(result);
    }
    let target_evidence = normalize_live_target_evidence(Value::Object(result.data));
    let report =
        audit_screenshot(&capture.screenshot_path, &target_evidence, options).map_err(|error| {
            CommandResult {
                ok: false,
                command: command.to_string(),
                message: "visual audit failed".to_string(),
                data: Map::new(),
                failures: vec![FailureDetail {
                    kind: "visual_audit_failed".to_string(),
                    command: Some(command.to_string()),
                    path: Some(capture.screenshot_path.to_string_lossy().into_owned()),
                    message: Some("visual audit could not analyze the live screenshot".to_string()),
                    expected: Some(json!(
                        "loadable staged live screenshot PNG and valid runtime target evidence"
                    )),
                    actual: Some(json!(error)),
                    ..FailureDetail::default()
                }],
                exit_code: 1,
            }
        })?;
    Ok(LiveVisualAuditReport {
        report,
        capture,
        target_evidence,
    })
}

pub fn normalize_live_target_evidence(value: Value) -> Value {
    let Value::Object(mut object) = value else {
        return value;
    };
    let Some(Value::Array(items)) = object.remove("targets") else {
        return Value::Object(object);
    };
    object.insert(
        "targets".to_string(),
        Value::Array(items.into_iter().map(normalize_live_target).collect()),
    );
    Value::Object(object)
}

fn normalize_live_target(value: Value) -> Value {
    let Value::Object(mut object) = value else {
        return value;
    };
    if !object.contains_key("target") {
        if let Some(locator) = object.get("locator").and_then(Value::as_str) {
            object.insert("target".to_string(), json!(locator));
        }
    }
    if !object.contains_key("target_rect") {
        if let Some(rect) = object
            .get("visual_evidence")
            .and_then(Value::as_object)
            .and_then(|evidence| evidence.get("rect"))
            .cloned()
            .filter(|rect| !rect.is_null())
        {
            object.insert("target_rect".to_string(), rect);
        }
    }
    Value::Object(object)
}

struct AggregateAudit {
    source_path: String,
    width: u32,
    height: u32,
    frames_checked: u64,
    targets: Vec<VisualAuditTargetMetrics>,
    findings: Vec<VisualAuditFinding>,
    frames_with_target_missing: BTreeMap<String, u64>,
    minimum_pixel_area_across_frames: BTreeMap<String, u64>,
    minimum_contrast_across_frames: BTreeMap<String, f64>,
    worst_frame_path_by_rule: BTreeMap<String, BTreeMap<String, String>>,
}

impl AggregateAudit {
    fn new(source_path: &Path) -> Self {
        Self {
            source_path: source_path.to_string_lossy().into_owned(),
            width: 0,
            height: 0,
            frames_checked: 0,
            targets: Vec::new(),
            findings: Vec::new(),
            frames_with_target_missing: BTreeMap::new(),
            minimum_pixel_area_across_frames: BTreeMap::new(),
            minimum_contrast_across_frames: BTreeMap::new(),
            worst_frame_path_by_rule: BTreeMap::new(),
        }
    }

    fn record_missing_targets(&mut self, evidence: &Value, wanted: &BTreeSet<String>) {
        if wanted.is_empty() {
            return;
        }
        let present = normalize_targets(evidence)
            .into_iter()
            .map(|target| target.target_id)
            .collect::<BTreeSet<_>>();
        for target in wanted {
            if !present.contains(target) {
                *self
                    .frames_with_target_missing
                    .entry(target.clone())
                    .or_insert(0) += 1;
            }
        }
    }

    fn push_frame_report(&mut self, report: VisualAuditReport) {
        if self.frames_checked == 0 {
            self.width = report.width;
            self.height = report.height;
        }
        self.frames_checked += 1;
        for target in &report.targets {
            self.minimum_pixel_area_across_frames
                .entry(target.target_id.clone())
                .and_modify(|minimum| *minimum = (*minimum).min(target.pixel_area))
                .or_insert(target.pixel_area);
            if let Some(contrast) = target.contrast_ratio_estimate {
                self.minimum_contrast_across_frames
                    .entry(target.target_id.clone())
                    .and_modify(|minimum| *minimum = minimum.min(contrast))
                    .or_insert(contrast);
            }
        }
        for finding in &report.findings {
            let rule_id = finding
                .details
                .get("rule_id")
                .and_then(Value::as_str)
                .unwrap_or(finding.kind.as_str());
            let target_id = finding
                .details
                .get("target_id")
                .and_then(Value::as_str)
                .or(finding.target.as_deref())
                .unwrap_or("unknown");
            let path = finding
                .path
                .clone()
                .unwrap_or_else(|| report.screenshot_path.clone());
            self.worst_frame_path_by_rule
                .entry(target_id.to_string())
                .or_default()
                .entry(rule_id.to_string())
                .or_insert(path);
        }
        self.targets.extend(report.targets);
        self.findings.extend(report.findings);
    }

    fn finish(mut self, options: &VisualAuditOptions) -> VisualAuditReport {
        for (target_id, missing_count) in &self.frames_with_target_missing {
            let mut warning = finding(
                RULE_SEQUENCE_TARGET_MISSING_FRAMES,
                &self.source_path,
                Some(target_id),
                json!({
                    "target_id": target_id,
                    "frames_checked": self.frames_checked,
                    "frames_with_target_missing": missing_count
                }),
                "target evidence was missing from one or more sequence frames",
                "capture target visual evidence for each frame or omit targets that are not expected in every frame",
                options,
            );
            warning
                .details
                .insert("frames_checked".to_string(), json!(self.frames_checked));
            warning.details.insert(
                "frames_with_target_missing".to_string(),
                json!(missing_count),
            );
            self.worst_frame_path_by_rule
                .entry(target_id.clone())
                .or_default()
                .entry(RULE_SEQUENCE_TARGET_MISSING_FRAMES.to_string())
                .or_insert(self.source_path.clone());
            self.findings.push(warning);
        }
        VisualAuditReport {
            screenshot_path: self.source_path,
            width: self.width,
            height: self.height,
            targets: self.targets,
            findings: self.findings,
            frames_checked: Some(self.frames_checked),
            frames_with_target_missing: Some(self.frames_with_target_missing),
            minimum_pixel_area_across_frames: Some(self.minimum_pixel_area_across_frames),
            minimum_contrast_across_frames: Some(self.minimum_contrast_across_frames),
            worst_frame_path_by_rule: Some(self.worst_frame_path_by_rule),
        }
    }
}

fn read_json_value(path: &Path) -> Result<Value, String> {
    let raw = fs::read_to_string(path)
        .map_err(|error| format!("failed to read {}: {error}", path.to_string_lossy()))?;
    serde_json::from_str(&raw)
        .map_err(|error| format!("failed to parse {}: {error}", path.to_string_lossy()))
}

fn resolve_relative_path(root: &Path, raw_path: &str) -> PathBuf {
    let path = PathBuf::from(raw_path);
    if path.is_absolute() {
        path
    } else {
        root.join(path)
    }
}

fn requested_target_set(targets: &[String]) -> BTreeSet<String> {
    targets
        .iter()
        .map(|target| target.trim())
        .filter(|target| !target.is_empty())
        .map(ToOwned::to_owned)
        .collect()
}

fn frame_target_evidence(frame: &Value) -> Value {
    let Some(object) = frame.as_object() else {
        return json!({"targets": []});
    };
    for key in [
        "targets",
        "target_bounds",
        "target_evidence",
        "visual_evidence",
        "visual_targets",
    ] {
        if let Some(value) = object.get(key) {
            return wrap_targets_value(value.clone());
        }
    }
    json!({"targets": []})
}

fn wrap_targets_value(value: Value) -> Value {
    match value {
        Value::Object(object) if object.contains_key("targets") => Value::Object(object),
        Value::Array(_) => json!({ "targets": value }),
        other => other,
    }
}

fn filter_evidence_targets(value: Value, wanted: &BTreeSet<String>) -> Value {
    if wanted.is_empty() {
        return value;
    }
    match value {
        Value::Array(items) => Value::Array(
            items
                .into_iter()
                .filter(|item| evidence_target_matches(item, wanted))
                .collect(),
        ),
        Value::Object(mut object) => {
            if let Some(Value::Array(items)) = object.remove("targets") {
                object.insert(
                    "targets".to_string(),
                    Value::Array(
                        items
                            .into_iter()
                            .filter(|item| evidence_target_matches(item, wanted))
                            .collect(),
                    ),
                );
                return Value::Object(object);
            }
            Value::Object(
                object
                    .into_iter()
                    .filter(|(key, item)| {
                        wanted.contains(key) || evidence_target_matches(item, wanted)
                    })
                    .collect(),
            )
        }
        other => other,
    }
}

fn evidence_target_matches(item: &Value, wanted: &BTreeSet<String>) -> bool {
    let Some(object) = item.as_object() else {
        return false;
    };
    ["target", "target_id", "qa_id", "locator"]
        .into_iter()
        .filter_map(|key| object.get(key).and_then(Value::as_str))
        .any(|value| wanted.contains(value))
        || object
            .get("visual_evidence")
            .and_then(Value::as_object)
            .and_then(|evidence| evidence.get("target").and_then(Value::as_str))
            .is_some_and(|value| wanted.contains(value))
}

fn resolve_scenario_run_root(
    artifacts_dir: &Path,
    scenario: Option<&str>,
    run_id: Option<&str>,
) -> Result<PathBuf, String> {
    if let Some(scenario) = scenario.map(str::trim).filter(|value| !value.is_empty()) {
        let scenario_path = PathBuf::from(scenario);
        let scenario_root = if scenario_path.is_absolute() {
            scenario_path
        } else {
            artifacts_dir.join(scenario_path)
        };
        if scenario_root.join("result.json").is_file() {
            return Ok(scenario_root);
        }
        if let Some(run_id) = run_id.map(str::trim).filter(|value| !value.is_empty()) {
            let run_root = scenario_root.join(run_id);
            if run_root.join("result.json").is_file() {
                return Ok(run_root);
            }
            return Err(format!(
                "requested scenario run result.json was not found: {}",
                run_root.join("result.json").to_string_lossy()
            ));
        }
        return latest_child_run_root(&scenario_root)
            .or_else(|_| latest_descendant_run_root(&scenario_root));
    }
    latest_descendant_run_root(artifacts_dir)
}

fn latest_child_run_root(scenario_root: &Path) -> Result<PathBuf, String> {
    let mut candidates = fs::read_dir(scenario_root)
        .map_err(|error| {
            format!(
                "failed to read scenario artifacts directory {}: {error}",
                scenario_root.to_string_lossy()
            )
        })?
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|path| path.join("result.json").is_file())
        .collect::<Vec<_>>();
    candidates.sort();
    candidates.pop().ok_or_else(|| {
        format!(
            "no scenario result.json artifacts found under {}",
            scenario_root.to_string_lossy()
        )
    })
}

fn latest_descendant_run_root(artifacts_dir: &Path) -> Result<PathBuf, String> {
    let mut candidates = Vec::new();
    for scenario_entry in fs::read_dir(artifacts_dir)
        .map_err(|error| {
            format!(
                "failed to read artifacts directory {}: {error}",
                artifacts_dir.to_string_lossy()
            )
        })?
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|path| path.is_dir())
    {
        if scenario_entry.join("result.json").is_file() {
            candidates.push(scenario_entry);
            continue;
        }
        if let Ok(run_root) = latest_child_run_root(&scenario_entry) {
            candidates.push(run_root);
        }
    }
    candidates.sort();
    candidates.pop().ok_or_else(|| {
        format!(
            "no scenario result.json artifacts found under {}",
            artifacts_dir.to_string_lossy()
        )
    })
}

fn normalize_targets(value: &Value) -> Vec<TargetEvidence> {
    match value {
        Value::Array(items) => items
            .iter()
            .enumerate()
            .map(|(index, item)| normalize_target(item, index))
            .collect(),
        Value::Object(object) => {
            if let Some(items) = object.get("targets").and_then(Value::as_array) {
                return items
                    .iter()
                    .enumerate()
                    .map(|(index, item)| normalize_target(item, index))
                    .collect();
            }
            if looks_like_single_target(object) {
                return vec![normalize_target(value, 0)];
            }
            object
                .iter()
                .map(|(target_id, item)| {
                    let mut target = normalize_target(item, 0);
                    if target.target_id.starts_with("target[") {
                        target.target_id = target_id.clone();
                    }
                    target
                })
                .collect()
        }
        _ => Vec::new(),
    }
}

fn looks_like_single_target(object: &Map<String, Value>) -> bool {
    object.contains_key("target")
        || object.contains_key("target_id")
        || object.contains_key("qa_id")
        || object.contains_key("target_rect")
        || object.contains_key("rect")
        || object.contains_key("bounds")
        || object.contains_key("visual_evidence")
}

fn normalize_target(value: &Value, index: usize) -> TargetEvidence {
    let fallback_id = format!("target[{index}]");
    let Some(object) = value.as_object() else {
        return TargetEvidence {
            target_id: fallback_id,
            rect: parse_rect(value),
            visible_pixel_count: None,
        };
    };
    let evidence = object.get("visual_evidence").and_then(Value::as_object);
    let target_id = string_field(object, "target")
        .or_else(|| string_field(object, "target_id"))
        .or_else(|| string_field(object, "qa_id"))
        .or_else(|| evidence.and_then(|evidence| string_field(evidence, "target")))
        .unwrap_or(fallback_id);
    let rect = object
        .get("target_rect")
        .or_else(|| object.get("rect"))
        .or_else(|| object.get("bounds"))
        .or_else(|| evidence.and_then(|evidence| evidence.get("target_rect")))
        .or_else(|| evidence.and_then(|evidence| evidence.get("rect")))
        .and_then(parse_rect);
    let visible_pixel_count = object
        .get("visible_pixel_count")
        .or_else(|| object.get("useful_pixel_count"))
        .or_else(|| evidence.and_then(|evidence| evidence.get("visible_pixel_count")))
        .or_else(|| evidence.and_then(|evidence| evidence.get("useful_pixel_count")))
        .and_then(Value::as_u64);
    TargetEvidence {
        target_id,
        rect,
        visible_pixel_count,
    }
}

fn parse_rect(value: &Value) -> Option<Rect> {
    if let Some(items) = value.as_array() {
        if items.len() < 4 {
            return None;
        }
        return Some(Rect {
            x: items.first()?.as_f64()?,
            y: items.get(1)?.as_f64()?,
            width: items.get(2)?.as_f64()?,
            height: items.get(3)?.as_f64()?,
        });
    }
    let object = value.as_object()?;
    Some(Rect {
        x: object.get("x").and_then(Value::as_f64)?,
        y: object.get("y").and_then(Value::as_f64)?,
        width: object
            .get("width")
            .or_else(|| object.get("w"))
            .and_then(Value::as_f64)?,
        height: object
            .get("height")
            .or_else(|| object.get("h"))
            .and_then(Value::as_f64)?,
    })
}

fn valid_rect(rect: &Rect) -> bool {
    rect.x.is_finite()
        && rect.y.is_finite()
        && rect.width.is_finite()
        && rect.height.is_finite()
        && rect.width > 0.0
        && rect.height > 0.0
}

fn clamp_rect(rect: Rect, image_width: u32, image_height: u32) -> Option<[u32; 4]> {
    let left = rect.x.floor().max(0.0) as i64;
    let top = rect.y.floor().max(0.0) as i64;
    let right = (rect.x + rect.width).ceil().min(f64::from(image_width)) as i64;
    let bottom = (rect.y + rect.height).ceil().min(f64::from(image_height)) as i64;
    if right <= left || bottom <= top {
        return None;
    }
    Some([
        left as u32,
        top as u32,
        (right - left) as u32,
        (bottom - top) as u32,
    ])
}

fn measure_target(
    image: &RgbaImage,
    screenshot: &str,
    target_id: &str,
    rect: Rect,
    clamped: [u32; 4],
    visible_pixel_count: Option<u64>,
    options: &VisualAuditOptions,
) -> VisualAuditTargetMetrics {
    let target_color = median_color(rect_pixels(image, clamped));
    let background_color = median_color(background_pixels(image, clamped));
    let target_luminance = target_color.map(relative_luminance);
    let background_luminance = background_color.map(relative_luminance);
    let contrast_ratio = target_luminance
        .zip(background_luminance)
        .map(|(target, background)| contrast_ratio(target, background));
    let color_distance = target_color
        .zip(background_color)
        .map(|(target, background)| color_distance(target, background));
    let edge_contrast = estimate_edge_contrast(image, clamped);
    let pixel_area = u64::from(clamped[2]) * u64::from(clamped[3]);
    let small_target_warning = pixel_area < options.min_pixel_area
        || clamped[2] < options.min_width
        || clamped[3] < options.min_height;
    let low_contrast_warning = contrast_ratio
        .map(|value| value < options.min_contrast_ratio)
        .unwrap_or(false)
        || color_distance
            .map(|value| value < options.min_color_distance)
            .unwrap_or(false)
        || edge_contrast
            .map(|value| value < options.min_edge_contrast)
            .unwrap_or(false);

    VisualAuditTargetMetrics {
        target_id: target_id.to_string(),
        screenshot_path: screenshot.to_string(),
        inside_viewport: true,
        pixel_area,
        bounding_box_width: clamped[2],
        bounding_box_height: clamped[3],
        visible_pixel_count,
        estimated_local_background_color: background_color,
        estimated_target_color_or_median: target_color,
        relative_luminance_target: target_luminance,
        relative_luminance_background: background_luminance,
        contrast_ratio_estimate: contrast_ratio,
        local_color_distance: color_distance,
        edge_contrast_estimate: edge_contrast,
        small_target_warning,
        low_contrast_warning,
        offscreen_warning: false,
        occlusion_or_missing_pixels_warning: visible_pixel_count.map(|count| count == 0),
        target_rect: rect_array(rect),
        clamped_rect: Some(clamped),
    }
}

fn rect_pixels(image: &RgbaImage, rect: [u32; 4]) -> Vec<[u8; 4]> {
    let mut pixels = Vec::with_capacity((rect[2] * rect[3]) as usize);
    for y in rect[1]..rect[1] + rect[3] {
        for x in rect[0]..rect[0] + rect[2] {
            let pixel = image.get_pixel(x, y).0;
            if pixel[3] > 0 {
                pixels.push(pixel);
            }
        }
    }
    pixels
}

fn background_pixels(image: &RgbaImage, rect: [u32; 4]) -> Vec<[u8; 4]> {
    let (width, height) = image.dimensions();
    let margin = 4_u32;
    let left = rect[0].saturating_sub(margin);
    let top = rect[1].saturating_sub(margin);
    let right = min(width, rect[0] + rect[2] + margin);
    let bottom = min(height, rect[1] + rect[3] + margin);
    let target_right = rect[0] + rect[2];
    let target_bottom = rect[1] + rect[3];
    let mut pixels = Vec::new();
    for y in top..bottom {
        for x in left..right {
            if x >= rect[0] && x < target_right && y >= rect[1] && y < target_bottom {
                continue;
            }
            let pixel = image.get_pixel(x, y).0;
            if pixel[3] > 0 {
                pixels.push(pixel);
            }
        }
    }
    pixels
}

fn median_color(mut pixels: Vec<[u8; 4]>) -> Option<[u8; 4]> {
    if pixels.is_empty() {
        return None;
    }
    let middle = pixels.len() / 2;
    let mut color = [0_u8; 4];
    for channel in 0..4 {
        pixels.select_nth_unstable_by_key(middle, |pixel| pixel[channel]);
        color[channel] = pixels[middle][channel];
    }
    Some(color)
}

fn relative_luminance(color: [u8; 4]) -> f64 {
    fn component(value: u8) -> f64 {
        let value = f64::from(value) / 255.0;
        if value <= 0.03928 {
            value / 12.92
        } else {
            ((value + 0.055) / 1.055).powf(2.4)
        }
    }
    0.2126 * component(color[0]) + 0.7152 * component(color[1]) + 0.0722 * component(color[2])
}

fn contrast_ratio(a: f64, b: f64) -> f64 {
    let lighter = a.max(b);
    let darker = a.min(b);
    (lighter + 0.05) / (darker + 0.05)
}

fn color_distance(a: [u8; 4], b: [u8; 4]) -> f64 {
    let dr = f64::from(a[0]) - f64::from(b[0]);
    let dg = f64::from(a[1]) - f64::from(b[1]);
    let db = f64::from(a[2]) - f64::from(b[2]);
    (dr * dr + dg * dg + db * db).sqrt()
}

fn estimate_edge_contrast(image: &RgbaImage, rect: [u32; 4]) -> Option<f64> {
    let (width, height) = image.dimensions();
    let left = rect[0];
    let top = rect[1];
    let right = rect[0] + rect[2] - 1;
    let bottom = rect[1] + rect[3] - 1;
    let mut samples = Vec::new();

    for x in left..=right {
        if top > 0 {
            samples.push(edge_sample(image, x, top, x, top - 1));
        }
        if bottom + 1 < height {
            samples.push(edge_sample(image, x, bottom, x, bottom + 1));
        }
    }
    for y in top..=bottom {
        if left > 0 {
            samples.push(edge_sample(image, left, y, left - 1, y));
        }
        if right + 1 < width {
            samples.push(edge_sample(image, right, y, right + 1, y));
        }
    }

    let mut count = 0_u64;
    let sum = samples
        .into_iter()
        .flatten()
        .inspect(|_| count += 1)
        .sum::<f64>();
    (count > 0).then_some(sum / count as f64)
}

fn edge_sample(image: &RgbaImage, ax: u32, ay: u32, bx: u32, by: u32) -> Option<f64> {
    let a = image.get_pixel(ax, ay).0;
    let b = image.get_pixel(bx, by).0;
    if a[3] == 0 || b[3] == 0 {
        return None;
    }
    Some((relative_luminance(a) - relative_luminance(b)).abs())
}

fn push_metric_findings(
    metrics: &VisualAuditTargetMetrics,
    options: &VisualAuditOptions,
    findings: &mut Vec<VisualAuditFinding>,
) {
    if metrics.offscreen_warning {
        findings.push(metric_finding(
            RULE_TARGET_OFFSCREEN,
            metrics,
            json!({"inside_viewport": metrics.inside_viewport, "target_rect": metrics.target_rect}),
            "target bounds are outside the screenshot viewport",
            "confirm the target is expected to be visible or capture a frame where it is onscreen",
            options,
        ));
    }
    if metrics.small_target_warning {
        findings.push(metric_finding(
            RULE_TARGET_TOO_SMALL,
            metrics,
            json!({
                "pixel_area": metrics.pixel_area,
                "bounding_box_width": metrics.bounding_box_width,
                "bounding_box_height": metrics.bounding_box_height,
            }),
            "target bounds are smaller than the configured readability threshold",
            "increase target size or lower the audit threshold for intentional small targets",
            options,
        ));
    }
    if metrics
        .contrast_ratio_estimate
        .map(|value| value < options.min_contrast_ratio)
        .unwrap_or(false)
    {
        findings.push(metric_finding(
            RULE_LOW_LUMINANCE_CONTRAST,
            metrics,
            json!({
                "contrast_ratio_estimate": metrics.contrast_ratio_estimate,
                "relative_luminance_target": metrics.relative_luminance_target,
                "relative_luminance_background": metrics.relative_luminance_background,
            }),
            "target luminance contrast against local background is low",
            "increase luminance contrast or provide explicit bounds for a more representative target region",
            options,
        ));
    }
    if metrics
        .local_color_distance
        .map(|value| value < options.min_color_distance)
        .unwrap_or(false)
    {
        findings.push(metric_finding(
            RULE_LOW_COLOR_DISTANCE,
            metrics,
            json!({"local_color_distance": metrics.local_color_distance}),
            "target color is close to its local background color",
            "increase color separation between the target and nearby background",
            options,
        ));
    }
    if metrics
        .edge_contrast_estimate
        .map(|value| value < options.min_edge_contrast)
        .unwrap_or(false)
    {
        findings.push(metric_finding(
            RULE_LOW_EDGE_CONTRAST,
            metrics,
            json!({"edge_contrast_estimate": metrics.edge_contrast_estimate}),
            "target edge contrast against neighboring pixels is low",
            "add clearer edge separation or lower the threshold for intentionally soft edges",
            options,
        ));
    }
    if metrics.occlusion_or_missing_pixels_warning == Some(true) {
        findings.push(metric_finding(
            RULE_TARGET_MISSING,
            metrics,
            json!({"visible_pixel_count": metrics.visible_pixel_count}),
            "target bounds were available but visible pixel evidence reports no target pixels",
            "confirm the target is rendered, unoccluded, and visually different from the background",
            options,
        ));
    }
}

fn metric_finding(
    rule_id: &str,
    metrics: &VisualAuditTargetMetrics,
    actual: Value,
    reason: &str,
    suggested_fix: &str,
    options: &VisualAuditOptions,
) -> VisualAuditFinding {
    finding(
        rule_id,
        &metrics.screenshot_path,
        Some(&metrics.target_id),
        actual,
        reason,
        suggested_fix,
        options,
    )
}

fn finding(
    rule_id: &str,
    screenshot: &str,
    target: Option<&str>,
    actual: Value,
    reason: &str,
    suggested_fix: &str,
    options: &VisualAuditOptions,
) -> VisualAuditFinding {
    let mut details = Map::from_iter([
        ("rule_id".to_string(), json!(rule_id)),
        ("reason".to_string(), json!(reason)),
        ("suggested_fix".to_string(), json!(suggested_fix)),
    ]);
    if let Some(target) = target {
        details.insert("target_id".to_string(), json!(target));
    }
    FailureDetail {
        kind: rule_id.to_string(),
        path: Some(screenshot.to_string()),
        message: Some(reason.to_string()),
        actual: Some(actual),
        target: target.map(ToOwned::to_owned),
        severity: Some(if options.strict_warnings {
            "error".to_string()
        } else {
            "warning".to_string()
        }),
        artifacts: Some(Map::from_iter([(
            "screenshot".to_string(),
            json!(screenshot),
        )])),
        details,
        ..FailureDetail::default()
    }
}

fn string_field(object: &Map<String, Value>, key: &str) -> Option<String> {
    object
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

fn rect_array(rect: Rect) -> [f64; 4] {
    [rect.x, rect.y, rect.width, rect.height]
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{ImageBuffer, Rgba};
    use serde_json::json;
    use std::fs;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT_ID: AtomicU64 = AtomicU64::new(0);

    fn temp_dir(name: &str) -> PathBuf {
        let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("godot-qa-{name}-{id}"));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn save_png(path: &Path, width: u32, height: u32, color_at: impl Fn(u32, u32) -> [u8; 4]) {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        let image = ImageBuffer::from_fn(width, height, |x, y| Rgba(color_at(x, y)));
        image.save(path).unwrap();
    }

    fn audit(path: &Path, targets: Value) -> VisualAuditReport {
        audit_screenshot(path, &targets, &VisualAuditOptions::default()).unwrap()
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

    #[test]
    fn visual_audit_readable_target_has_metrics_without_findings() {
        let root = temp_dir("visual-audit-readable");
        let path = root.join("readable.png");
        save_png(&path, 80, 80, |x, y| {
            if (20..52).contains(&x) && (20..52).contains(&y) {
                [245, 245, 245, 255]
            } else {
                [20, 24, 28, 255]
            }
        });

        let report = audit(
            &path,
            json!([{"target": "qa:readable", "target_rect": [20, 20, 32, 32], "visible_pixel_count": 1024}]),
        );

        assert!(report.findings.is_empty());
        let metrics = &report.targets[0];
        assert_eq!(metrics.target_id, "qa:readable");
        assert_eq!(metrics.pixel_area, 1024);
        assert_eq!(metrics.bounding_box_width, 32);
        assert_eq!(metrics.bounding_box_height, 32);
        assert_eq!(metrics.visible_pixel_count, Some(1024));
        assert!(metrics.contrast_ratio_estimate.unwrap() > 3.0);
        assert!(metrics.local_color_distance.unwrap() > 24.0);
        assert!(metrics.edge_contrast_estimate.unwrap() > 0.08);
    }

    #[test]
    fn visual_audit_reports_tiny_target() {
        let root = temp_dir("visual-audit-tiny");
        let path = root.join("tiny.png");
        save_png(&path, 80, 80, |x, y| {
            if (20..28).contains(&x) && (20..28).contains(&y) {
                [255, 255, 255, 255]
            } else {
                [10, 10, 10, 255]
            }
        });

        let report = audit(
            &path,
            json!([{"target": "qa:tiny", "rect": [20, 20, 8, 8]}]),
        );

        assert!(report.targets[0].small_target_warning);
        assert!(
            report
                .findings
                .iter()
                .any(|f| f.kind == RULE_TARGET_TOO_SMALL)
        );
    }

    #[test]
    fn visual_audit_reports_low_contrast_rules() {
        let root = temp_dir("visual-audit-low-contrast");
        let path = root.join("low.png");
        save_png(&path, 80, 80, |x, y| {
            if (20..52).contains(&x) && (20..52).contains(&y) {
                [104, 104, 104, 255]
            } else {
                [100, 100, 100, 255]
            }
        });

        let report = audit(
            &path,
            json!([{"target": "qa:low_contrast", "bounds": {"x": 20, "y": 20, "width": 32, "height": 32}}]),
        );

        assert!(report.targets[0].low_contrast_warning);
        assert!(
            report
                .findings
                .iter()
                .any(|f| f.kind == RULE_LOW_LUMINANCE_CONTRAST)
        );
        assert!(
            report
                .findings
                .iter()
                .any(|f| f.kind == RULE_LOW_COLOR_DISTANCE)
        );
        assert!(
            report
                .findings
                .iter()
                .any(|f| f.kind == RULE_LOW_EDGE_CONTRAST)
        );
    }

    #[test]
    fn visual_audit_reports_offscreen_target_after_clamping() {
        let root = temp_dir("visual-audit-offscreen");
        let path = root.join("offscreen.png");
        save_png(&path, 80, 80, |_x, _y| [20, 20, 20, 255]);

        let report = audit(
            &path,
            json!([{"target": "qa:offscreen", "target_rect": [100, 100, 20, 20]}]),
        );

        assert_eq!(report.targets[0].pixel_area, 0);
        assert!(report.targets[0].offscreen_warning);
        assert!(
            report
                .findings
                .iter()
                .any(|f| f.kind == RULE_TARGET_OFFSCREEN)
        );
    }

    #[test]
    fn visual_audit_reports_missing_or_malformed_bounds_without_inference() {
        let root = temp_dir("visual-audit-missing-bounds");
        let path = root.join("missing.png");
        save_png(&path, 80, 80, |x, y| {
            if (20..52).contains(&x) && (20..52).contains(&y) {
                [255, 0, 0, 255]
            } else {
                [0, 0, 0, 255]
            }
        });

        let report = audit(
            &path,
            json!([
                {"target": "qa:missing"},
                {"target": "qa:malformed", "target_rect": [20, 20, -1, 10]}
            ]),
        );

        assert!(report.targets.is_empty());
        assert_eq!(report.findings.len(), 2);
        assert!(
            report
                .findings
                .iter()
                .all(|f| f.kind == RULE_TARGET_BOUNDS_UNAVAILABLE)
        );
    }

    #[test]
    fn visual_audit_reports_missing_visible_pixels() {
        let root = temp_dir("visual-audit-missing-visible-pixels");
        let path = root.join("missing-visible.png");
        save_png(&path, 80, 80, |x, y| {
            if (20..52).contains(&x) && (20..52).contains(&y) {
                [250, 250, 250, 255]
            } else {
                [15, 15, 15, 255]
            }
        });

        let report = audit(
            &path,
            json!([{"target": "qa:hidden", "target_rect": [20, 20, 32, 32], "visible_pixel_count": 0}]),
        );

        assert_eq!(
            report.targets[0].occlusion_or_missing_pixels_warning,
            Some(true)
        );
        assert!(
            report
                .findings
                .iter()
                .any(|f| f.kind == RULE_TARGET_MISSING)
        );
    }

    #[test]
    fn visual_audit_strict_warnings_emit_error_severity() {
        let root = temp_dir("visual-audit-strict");
        let path = root.join("strict.png");
        save_png(&path, 80, 80, |_x, _y| [30, 30, 30, 255]);
        let options = VisualAuditOptions {
            strict_warnings: true,
            ..VisualAuditOptions::default()
        };

        let report = audit_screenshot(
            &path,
            &json!([{"target": "qa:tiny", "target_rect": [20, 20, 4, 4]}]),
            &options,
        )
        .unwrap();

        assert!(!report.findings.is_empty());
        assert!(
            report
                .findings
                .iter()
                .all(|finding| finding.severity.as_deref() == Some("error"))
        );
    }

    #[test]
    fn visual_audit_live_normalizes_runtime_evidence_into_target_bounds() {
        let root = temp_dir("visual-audit-live-normalize");
        let path = root.join("live.png");
        save_png(&path, 80, 80, |x, y| {
            if (20..52).contains(&x) && (20..52).contains(&y) {
                [245, 245, 245, 255]
            } else {
                [20, 24, 28, 255]
            }
        });
        let evidence = normalize_live_target_evidence(json!({
            "target_count": 1,
            "targets": [
                {
                    "locator": "qa:live_button",
                    "resolved": "path:/root/Button",
                    "visible": true,
                    "visual_evidence": {
                        "coordinate_space": "viewport_2d",
                        "rect": [20.0, 20.0, 32.0, 32.0],
                        "rect_source": "control_global_rect"
                    }
                }
            ]
        }));

        let report = audit_screenshot(&path, &evidence, &VisualAuditOptions::default()).unwrap();

        assert_eq!(report.targets[0].target_id, "qa:live_button");
        assert_eq!(report.targets[0].target_rect, [20.0, 20.0, 32.0, 32.0]);
        assert!(report.findings.is_empty());
        assert_eq!(
            evidence["targets"][0]["target_rect"],
            json!([20.0, 20.0, 32.0, 32.0])
        );
    }

    #[test]
    fn visual_audit_live_preserves_screenshot_session_failure_contract() {
        let root = temp_dir("visual-audit-live-session-missing");
        let config = test_config(&root);

        let result = audit_live_session(
            &config,
            "visual audit",
            &["qa:button".to_string()],
            &VisualAuditOptions::default(),
        )
        .unwrap_err();

        assert!(!result.ok);
        assert_eq!(result.command, "visual audit");
        assert_eq!(result.message, "live command failed");
        assert_eq!(result.failures[0].kind, "session_not_found");
        assert!(!godot_qa_runtime::session_json_path(&config).exists());
    }

    #[test]
    fn visual_audit_sequence_manifest_summarizes_frames_and_missing_targets() {
        let root = temp_dir("visual-audit-sequence");
        let sequence_dir = root.join("sequence");
        let frame0 = sequence_dir.join("frames/frame_0000.png");
        let frame1 = sequence_dir.join("frames/frame_0010.png");
        let frame2 = sequence_dir.join("frames/frame_0020.png");
        save_png(&frame0, 80, 80, |x, y| {
            if (20..52).contains(&x) && (20..52).contains(&y) {
                [245, 245, 245, 255]
            } else {
                [20, 24, 28, 255]
            }
        });
        save_png(&frame1, 80, 80, |x, y| {
            if (20..28).contains(&x) && (20..28).contains(&y) {
                [245, 245, 245, 255]
            } else {
                [20, 24, 28, 255]
            }
        });
        save_png(&frame2, 80, 80, |_x, _y| [20, 24, 28, 255]);
        let manifest_path = sequence_dir.join("manifest.json");
        fs::write(
            &manifest_path,
            serde_json::to_string_pretty(&json!({
                "frames": [
                    {
                        "path": "frames/frame_0000.png",
                        "targets": [{"target": "qa:button", "target_rect": [20, 20, 32, 32]}]
                    },
                    {
                        "path": "frames/frame_0010.png",
                        "targets": [{"target": "qa:button", "target_rect": [20, 20, 8, 8]}]
                    },
                    {
                        "path": "frames/frame_0020.png",
                        "targets": []
                    }
                ]
            }))
            .unwrap(),
        )
        .unwrap();

        let report = audit_sequence_manifest(
            &manifest_path,
            &["qa:button".to_string()],
            &VisualAuditOptions::default(),
        )
        .unwrap();

        assert_eq!(report.frames_checked, Some(3));
        assert_eq!(
            report
                .frames_with_target_missing
                .as_ref()
                .unwrap()
                .get("qa:button"),
            Some(&1)
        );
        assert_eq!(
            report
                .minimum_pixel_area_across_frames
                .as_ref()
                .unwrap()
                .get("qa:button"),
            Some(&64)
        );
        assert!(
            report
                .minimum_contrast_across_frames
                .as_ref()
                .unwrap()
                .get("qa:button")
                .is_some()
        );
        assert!(
            report
                .findings
                .iter()
                .any(|finding| finding.kind == RULE_SEQUENCE_TARGET_MISSING_FRAMES)
        );
        assert!(
            report
                .worst_frame_path_by_rule
                .as_ref()
                .unwrap()
                .get("qa:button")
                .unwrap()
                .contains_key(RULE_TARGET_TOO_SMALL)
        );
    }

    #[test]
    fn visual_audit_sequence_scenario_artifacts_reuse_saved_snapshot_evidence() {
        let root = temp_dir("visual-audit-sequence-scenario");
        let run_root = root.join(".qa/artifacts/menu/run-001");
        let screenshot = run_root.join("snapshots/current/screenshot.png");
        save_png(&screenshot, 80, 80, |x, y| {
            if (20..52).contains(&x) && (20..52).contains(&y) {
                [245, 245, 245, 255]
            } else {
                [20, 24, 28, 255]
            }
        });
        fs::write(
            run_root.join("result.json"),
            serde_json::to_string_pretty(&json!({
                "snapshot_artifacts": {
                    "current": {
                        "screenshot": "snapshots/current/screenshot.png",
                        "targets": [
                            {"target": "qa:button", "target_rect": [20, 20, 32, 32]}
                        ]
                    }
                }
            }))
            .unwrap(),
        )
        .unwrap();

        let report = audit_scenario_artifacts(
            root.join(".qa/artifacts"),
            Some("menu"),
            Some("run-001"),
            &["qa:button".to_string()],
            &VisualAuditOptions::default(),
        )
        .unwrap();

        assert_eq!(report.frames_checked, Some(1));
        assert_eq!(report.targets[0].target_id, "qa:button");
        assert!(report.findings.is_empty());
        assert_eq!(
            Path::new(&report.screenshot_path),
            run_root.join("result.json")
        );
    }
}
