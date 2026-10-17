use godot_qa_core::config::ProjectConfig;
use godot_qa_core::results::{CommandResult, FailureDetail};
use godot_qa_runtime as runtime;
use image::{Rgba, RgbaImage};
use serde_json::{Map, Value, json};
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::fs;
use std::path::{Path, PathBuf};

const COMMAND: &str = "surface.audit";
const COMMAND_TEXT: &str = "surface audit";
const KNOWN_SURFACES: &[&str] = &[
    "walkable", "blocked", "soft", "solid", "floor", "ground", "road", "water", "empty",
];
const SYMMETRY_WARNING_THRESHOLD: f64 = 0.25;

#[derive(Debug, Clone)]
pub(crate) struct AuditRequest {
    params: Map<String, Value>,
}

#[derive(Debug, Clone)]
struct SurfaceSample {
    x: f64,
    y: f64,
    surface: String,
    solid: bool,
    soft: bool,
    valid: bool,
    raw: Value,
}

#[derive(Debug, Clone)]
pub(crate) struct AuditMetrics {
    sample_count: usize,
    walkable_count: usize,
    blocked_count: usize,
    soft_count: usize,
    unknown_count: usize,
    spawn_clearance_min_distance: Option<f64>,
    spawn_clearance_pass: Option<bool>,
    minimum_corridor_width: Option<f64>,
    corridor_width_pass: Option<bool>,
    hard_block_cluster_count: usize,
    soft_block_cluster_count: usize,
    dead_end_pocket_count: usize,
    largest_dead_end_area: f64,
    route_clearance_fail_points: Vec<Value>,
    symmetry_delta_score: Option<f64>,
    left_right_passability_delta: Option<f64>,
    provider_missing_or_invalid: bool,
    unknown_labels: Vec<String>,
}

pub(crate) fn surface_audit_result(
    config: &ProjectConfig,
    params: Map<String, Value>,
) -> CommandResult {
    let request = AuditRequest { params };
    let mut probe_result = runtime::run_live_protocol_command(
        config,
        COMMAND_TEXT,
        runtime::SURFACE_AUDIT_PROBE,
        request.params.clone(),
    );
    if !probe_result.ok {
        return retag_failure_result(probe_result, "surface audit probe failed");
    }

    let artifact_path = surface_artifact_path(config, &request.params);
    let heatmap_path = heatmap_artifact_path(&request.params);
    let metrics = compute_metrics(&request.params, &probe_result.data);
    let artifact_context = artifact_context(&artifact_path, heatmap_path.as_deref());
    let warnings = warning_rules(&request.params, &metrics, &artifact_context);
    let report = surface_audit_report(&request, &probe_result.data, &metrics, &warnings);
    if let Err(error) = write_json(&artifact_path, &report) {
        return failed(
            "surface audit artifact write failed",
            artifact_write_failure(&artifact_path, &error),
            probe_result.data,
        );
    }
    if let Some(heatmap_path) = heatmap_path.as_deref()
        && let Err(error) = render_heatmap_png(heatmap_path, &request.params, &probe_result.data)
    {
        return failed(
            "surface audit heatmap write failed",
            artifact_write_failure(heatmap_path, &error),
            probe_result.data,
        );
    }

    let mut data = Map::from_iter([
        ("command".to_string(), json!(COMMAND)),
        ("metrics".to_string(), metrics.to_json()),
        ("warnings".to_string(), Value::Array(warnings.clone())),
        ("artifacts".to_string(), Value::Object(artifact_context)),
    ]);
    if let Some(provider) = probe_result.data.get("provider").cloned() {
        data.insert("provider".to_string(), provider);
    }
    if let Some(provider_method) = probe_result.data.get("provider_method").cloned() {
        data.insert("provider_method".to_string(), provider_method);
    }
    if let Some(failure) = promoted_warning_failure(&request.params, &warnings) {
        return failed("surface audit validation failed", failure, data);
    }

    probe_result.command = COMMAND.to_string();
    probe_result.message = "surface audit completed".to_string();
    probe_result.data = data;
    probe_result
}

pub(crate) fn compute_metrics(
    params: &Map<String, Value>,
    response: &Map<String, Value>,
) -> AuditMetrics {
    let raw_samples = response
        .get("samples")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let samples = raw_samples.iter().map(normalize_sample).collect::<Vec<_>>();
    let provider_missing_or_invalid = response.get("command")
        != Some(&json!(runtime::SURFACE_AUDIT_PROBE))
        || raw_samples.is_empty()
        || samples.iter().any(|sample| !sample.valid);
    let valid_samples = samples
        .iter()
        .filter(|sample| sample.valid)
        .cloned()
        .collect::<Vec<_>>();
    let sample_count = valid_samples.len();
    let blocked_count = valid_samples.iter().filter(|sample| sample.solid).count();
    let soft_count = valid_samples
        .iter()
        .filter(|sample| sample.soft && !sample.solid)
        .count();
    let walkable_count = valid_samples
        .iter()
        .filter(|sample| !sample.solid && !sample.soft)
        .count();
    let unknown_labels = unknown_surface_labels(&valid_samples);
    let unknown_count = valid_samples
        .iter()
        .filter(|sample| is_unknown_surface_label(&sample.surface))
        .count();
    let sample_step = number_param(params, "sample_step").unwrap_or(1.0).max(1.0);
    let actor_radius = number_param(params, "actor_radius").unwrap_or(0.0).max(0.0);
    let spawn_clearance_min_distance =
        spawn_clearance_min_distance(params, response, &valid_samples, actor_radius);
    let spawn_clearance_pass = spawn_clearance_min_distance.map(|distance| {
        let threshold = number_param(params, "min_spawn_clearance").unwrap_or(actor_radius);
        distance >= threshold
    });
    let minimum_corridor_width = minimum_corridor_width(&valid_samples, sample_step, actor_radius);
    let corridor_width_pass = minimum_corridor_width.map(|width| {
        number_param(params, "min_corridor_width").is_none_or(|threshold| width >= threshold)
    });
    let hard_block_cluster_count = cluster_count(
        valid_samples.iter().filter(|sample| sample.solid),
        sample_step,
    );
    let soft_block_cluster_count = cluster_count(
        valid_samples
            .iter()
            .filter(|sample| sample.soft && !sample.solid),
        sample_step,
    );
    let (dead_end_pocket_count, largest_dead_end_area) =
        dead_end_pockets(&valid_samples, sample_step);
    let route_clearance_fail_points = route_clearance_fail_points(params, &valid_samples);
    let (symmetry_delta_score, left_right_passability_delta) =
        symmetry_metrics(params, &valid_samples);

    AuditMetrics {
        sample_count,
        walkable_count,
        blocked_count,
        soft_count,
        unknown_count,
        spawn_clearance_min_distance,
        spawn_clearance_pass,
        minimum_corridor_width,
        corridor_width_pass,
        hard_block_cluster_count,
        soft_block_cluster_count,
        dead_end_pocket_count,
        largest_dead_end_area,
        route_clearance_fail_points,
        symmetry_delta_score,
        left_right_passability_delta,
        provider_missing_or_invalid,
        unknown_labels,
    }
}

pub(crate) fn warning_rules(
    params: &Map<String, Value>,
    metrics: &AuditMetrics,
    artifacts: &Map<String, Value>,
) -> Vec<Value> {
    let mut warnings = Vec::new();
    if metrics.provider_missing_or_invalid {
        warnings.push(warning(
            "surface_audit.provider_invalid",
            "surface audit provider response was missing or malformed",
            json!("surface.audit_probe data with valid samples"),
            json!({"sample_count": metrics.sample_count}),
            params,
            artifacts,
        ));
    }
    if metrics.sample_count < 2 {
        warnings.push(warning(
            "surface_audit.not_enough_data",
            "surface audit did not receive enough samples for geometry metrics",
            json!("at least two valid samples"),
            json!({"sample_count": metrics.sample_count}),
            params,
            artifacts,
        ));
    }
    if metrics.unknown_count > 0 {
        warnings.push(warning(
            "surface_audit.unknown_surface_label",
            "surface audit observed project-defined surface labels",
            json!(KNOWN_SURFACES),
            json!({"unknown_count": metrics.unknown_count, "labels": metrics.unknown_labels}),
            params,
            artifacts,
        ));
    }
    if params.contains_key("min_spawn_clearance") && metrics.spawn_clearance_pass == Some(false) {
        let threshold = params
            .get("min_spawn_clearance")
            .cloned()
            .unwrap_or(Value::Null);
        warnings.push(warning(
            "surface_audit.spawn_clearance_failed",
            "spawn clearance is below the configured threshold",
            threshold.clone(),
            json!({
                "threshold": threshold,
                "actual_clearance": metrics.spawn_clearance_min_distance,
            }),
            params,
            artifacts,
        ));
    }
    if params.contains_key("min_corridor_width") && metrics.corridor_width_pass == Some(false) {
        warnings.push(warning(
            "surface_audit.corridor_width_failed",
            "corridor width is below the configured threshold",
            params
                .get("min_corridor_width")
                .cloned()
                .unwrap_or(Value::Null),
            json!(metrics.minimum_corridor_width),
            params,
            artifacts,
        ));
    }
    if metrics.dead_end_pocket_count > 0 {
        warnings.push(warning(
            "surface_audit.dead_end_pocket_detected",
            "surface audit found enclosed passable pockets",
            json!("no enclosed passable pockets"),
            json!({"dead_end_pocket_count": metrics.dead_end_pocket_count, "largest_dead_end_area": metrics.largest_dead_end_area}),
            params,
            artifacts,
        ));
    }
    if !metrics.route_clearance_fail_points.is_empty() {
        warnings.push(warning(
            "surface_audit.hard_block_cluster_blocks_route",
            "route samples include hard blockers",
            json!("no hard blockers on route samples"),
            json!(metrics.route_clearance_fail_points),
            params,
            artifacts,
        ));
    }
    if params.get("mode").and_then(Value::as_str) == Some("route") && metrics.soft_count > 0 {
        warnings.push(warning(
            "surface_audit.soft_blocker_turns_corridor_risky",
            "route samples include soft blockers",
            json!("no soft blockers on route samples"),
            json!({"soft_count": metrics.soft_count}),
            params,
            artifacts,
        ));
    }
    if metrics
        .symmetry_delta_score
        .is_some_and(|score| score > SYMMETRY_WARNING_THRESHOLD)
    {
        warnings.push(warning(
            "surface_audit.symmetry_delta_high",
            "mirrored regions have different passability",
            json!({"max_delta": SYMMETRY_WARNING_THRESHOLD}),
            json!({
                "symmetry_delta_score": metrics.symmetry_delta_score,
                "left_right_passability_delta": metrics.left_right_passability_delta,
            }),
            params,
            artifacts,
        ));
    }
    warnings
}

impl AuditMetrics {
    fn to_json(&self) -> Value {
        json!({
            "sample_count": self.sample_count,
            "walkable_count": self.walkable_count,
            "blocked_count": self.blocked_count,
            "soft_count": self.soft_count,
            "unknown_count": self.unknown_count,
            "spawn_clearance_min_distance": self.spawn_clearance_min_distance,
            "spawn_clearance_pass": self.spawn_clearance_pass,
            "minimum_corridor_width": self.minimum_corridor_width,
            "corridor_width_pass": self.corridor_width_pass,
            "hard_block_cluster_count": self.hard_block_cluster_count,
            "soft_block_cluster_count": self.soft_block_cluster_count,
            "dead_end_pocket_count": self.dead_end_pocket_count,
            "largest_dead_end_area": self.largest_dead_end_area,
            "route_clearance_fail_points": self.route_clearance_fail_points,
            "symmetry_delta_score": self.symmetry_delta_score,
            "left_right_passability_delta": self.left_right_passability_delta,
            "provider_missing_or_invalid": self.provider_missing_or_invalid,
        })
    }
}

fn normalize_sample(value: &Value) -> SurfaceSample {
    let Some(object) = value.as_object() else {
        return invalid_sample(value);
    };
    let coordinate_source = object
        .get("point")
        .and_then(Value::as_object)
        .unwrap_or(object);
    let Some(x) = coordinate_source.get("x").and_then(Value::as_f64) else {
        return invalid_sample(value);
    };
    let Some(y) = coordinate_source.get("y").and_then(Value::as_f64) else {
        return invalid_sample(value);
    };
    if !x.is_finite() || !y.is_finite() {
        return invalid_sample(value);
    }
    let surface = object
        .get("surface")
        .or_else(|| object.get("label"))
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase();
    let solid = object
        .get("solid")
        .and_then(Value::as_bool)
        .unwrap_or_else(|| matches!(surface.as_str(), "blocked" | "solid"));
    let soft = object.get("soft").and_then(Value::as_bool).unwrap_or(false);
    SurfaceSample {
        x,
        y,
        surface,
        solid,
        soft,
        valid: true,
        raw: value.clone(),
    }
}

fn invalid_sample(value: &Value) -> SurfaceSample {
    SurfaceSample {
        x: 0.0,
        y: 0.0,
        surface: String::new(),
        solid: false,
        soft: false,
        valid: false,
        raw: value.clone(),
    }
}

fn is_unknown_surface_label(label: &str) -> bool {
    !label.is_empty() && !KNOWN_SURFACES.contains(&label)
}

fn unknown_surface_labels(samples: &[SurfaceSample]) -> Vec<String> {
    samples
        .iter()
        .filter_map(|sample| {
            if is_unknown_surface_label(&sample.surface) {
                Some(sample.surface.clone())
            } else {
                None
            }
        })
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

fn number_param(params: &Map<String, Value>, key: &str) -> Option<f64> {
    params
        .get(key)
        .and_then(Value::as_f64)
        .filter(|value| value.is_finite())
}

fn spawn_clearance_min_distance(
    params: &Map<String, Value>,
    response: &Map<String, Value>,
    samples: &[SurfaceSample],
    actor_radius: f64,
) -> Option<f64> {
    if params.get("mode").and_then(Value::as_str) != Some("region") {
        return None;
    }
    let (cx, cy) = center_point(params, response).unwrap_or_else(|| sample_centroid(samples));
    samples
        .iter()
        .filter(|sample| sample.solid)
        .map(|sample| distance(cx, cy, sample.x, sample.y))
        .min_by(f64::total_cmp)
        .map(|distance| (distance - actor_radius).max(0.0))
        .or_else(|| Some(f64::INFINITY))
}

fn center_point(params: &Map<String, Value>, response: &Map<String, Value>) -> Option<(f64, f64)> {
    point_from_value(params.get("center_point"))
        .or_else(|| point_from_value(response.get("center")))
        .or_else(|| {
            response
                .get("anchors")
                .and_then(Value::as_object)
                .and_then(|anchors| anchors.get("center"))
                .and_then(|value| point_from_value(Some(value)))
        })
}

fn point_from_value(value: Option<&Value>) -> Option<(f64, f64)> {
    let object = value?.as_object()?;
    let source = object
        .get("point")
        .and_then(Value::as_object)
        .unwrap_or(object);
    let x = source.get("x").and_then(Value::as_f64)?;
    let y = source.get("y").and_then(Value::as_f64)?;
    (x.is_finite() && y.is_finite()).then_some((x, y))
}

fn sample_centroid(samples: &[SurfaceSample]) -> (f64, f64) {
    if samples.is_empty() {
        return (0.0, 0.0);
    }
    let (x, y) = samples
        .iter()
        .fold((0.0, 0.0), |(x, y), sample| (x + sample.x, y + sample.y));
    (x / samples.len() as f64, y / samples.len() as f64)
}

fn minimum_corridor_width(
    samples: &[SurfaceSample],
    sample_step: f64,
    actor_radius: f64,
) -> Option<f64> {
    let passable = samples
        .iter()
        .filter(|sample| !sample.solid)
        .collect::<Vec<_>>();
    if passable.is_empty() {
        return Some(0.0);
    }
    let hard = samples
        .iter()
        .filter(|sample| sample.solid)
        .collect::<Vec<_>>();
    if hard.is_empty() {
        return None;
    }
    passable
        .iter()
        .filter_map(|sample| {
            hard.iter()
                .map(|blocked| distance(sample.x, sample.y, blocked.x, blocked.y))
                .min_by(f64::total_cmp)
        })
        .map(|clearance| ((clearance - actor_radius).max(0.0) * 2.0).max(sample_step))
        .min_by(f64::total_cmp)
}

fn cluster_count<'a>(samples: impl Iterator<Item = &'a SurfaceSample>, sample_step: f64) -> usize {
    connected_components(samples.cloned().collect(), sample_step).len()
}

fn dead_end_pockets(samples: &[SurfaceSample], sample_step: f64) -> (usize, f64) {
    let passable = samples
        .iter()
        .filter(|sample| !sample.solid)
        .cloned()
        .collect::<Vec<_>>();
    if passable.is_empty() {
        return (0, 0.0);
    }
    let Some((min_x, max_x, min_y, max_y)) = sample_bounds(samples) else {
        return (0, 0.0);
    };
    let components = connected_components(passable, sample_step);
    let mut count = 0usize;
    let mut largest = 0.0f64;
    for component in components {
        let touches_boundary = component.iter().any(|sample| {
            nearly(sample.x, min_x, sample_step)
                || nearly(sample.x, max_x, sample_step)
                || nearly(sample.y, min_y, sample_step)
                || nearly(sample.y, max_y, sample_step)
        });
        if !touches_boundary {
            count += 1;
            largest = largest.max(component.len() as f64 * sample_step * sample_step);
        }
    }
    (count, largest)
}

fn route_clearance_fail_points(
    params: &Map<String, Value>,
    samples: &[SurfaceSample],
) -> Vec<Value> {
    if params.get("mode").and_then(Value::as_str) != Some("route") {
        return Vec::new();
    }
    samples
        .iter()
        .filter(|sample| sample.solid)
        .map(|sample| sample.raw.clone())
        .collect()
}

fn symmetry_metrics(
    params: &Map<String, Value>,
    samples: &[SurfaceSample],
) -> (Option<f64>, Option<f64>) {
    if params.get("mode").and_then(Value::as_str) != Some("mirror") {
        return (None, None);
    }
    if samples.len() < 2 {
        return (None, None);
    }
    let median_x = median_x(samples);
    let mut left = Vec::new();
    let mut right = Vec::new();
    for sample in samples {
        match sample_side(sample, median_x) {
            Some("left") => left.push(sample),
            Some("right") => right.push(sample),
            _ => {}
        }
    }
    if left.is_empty() || right.is_empty() {
        return (None, None);
    }
    let left_passability = passability_ratio(&left);
    let right_passability = passability_ratio(&right);
    let passability_delta = (left_passability - right_passability).abs();
    let label_delta = mirrored_label_delta(&left, &right);
    (
        Some(passability_delta.max(label_delta)),
        Some(passability_delta),
    )
}

fn sample_side(sample: &SurfaceSample, median_x: f64) -> Option<&'static str> {
    sample
        .raw
        .get("side")
        .or_else(|| {
            sample
                .raw
                .get("metadata")
                .and_then(|value| value.get("side"))
        })
        .and_then(Value::as_str)
        .and_then(|side| match side {
            "left" => Some("left"),
            "right" => Some("right"),
            _ => None,
        })
        .or_else(|| {
            if sample.x < median_x {
                Some("left")
            } else if sample.x > median_x {
                Some("right")
            } else {
                None
            }
        })
}

fn passability_ratio(samples: &[&SurfaceSample]) -> f64 {
    samples.iter().filter(|sample| !sample.solid).count() as f64 / samples.len() as f64
}

fn mirrored_label_delta(left: &[&SurfaceSample], right: &[&SurfaceSample]) -> f64 {
    let mut left_sorted = left.to_vec();
    let mut right_sorted = right.to_vec();
    left_sorted.sort_by(|a, b| a.y.total_cmp(&b.y).then(a.x.total_cmp(&b.x)));
    right_sorted.sort_by(|a, b| a.y.total_cmp(&b.y).then(a.x.total_cmp(&b.x)));
    let pair_count = left_sorted.len().min(right_sorted.len());
    if pair_count == 0 {
        return 0.0;
    }
    let mismatches = (0..pair_count)
        .filter(|index| left_sorted[*index].solid != right_sorted[*index].solid)
        .count();
    mismatches as f64 / pair_count as f64
}

fn connected_components(samples: Vec<SurfaceSample>, sample_step: f64) -> Vec<Vec<SurfaceSample>> {
    let mut by_key = BTreeMap::new();
    for (position, sample) in samples.iter().enumerate() {
        by_key.insert(grid_key(sample, sample_step), position);
    }
    let mut visited = vec![false; samples.len()];
    let mut components = Vec::new();
    for start in 0..samples.len() {
        if visited[start] {
            continue;
        }
        let mut component = Vec::new();
        let mut queue = VecDeque::from([start]);
        visited[start] = true;
        while let Some(position) = queue.pop_front() {
            let sample = samples[position].clone();
            let (gx, gy) = grid_key(&sample, sample_step);
            component.push(sample);
            for neighbor in [(gx - 1, gy), (gx + 1, gy), (gx, gy - 1), (gx, gy + 1)] {
                if let Some(neighbor_position) = by_key.get(&neighbor).copied()
                    && !visited[neighbor_position]
                {
                    visited[neighbor_position] = true;
                    queue.push_back(neighbor_position);
                }
            }
        }
        components.push(component);
    }
    components
}

fn grid_key(sample: &SurfaceSample, sample_step: f64) -> (i64, i64) {
    (
        (sample.x / sample_step).round() as i64,
        (sample.y / sample_step).round() as i64,
    )
}

fn sample_bounds(samples: &[SurfaceSample]) -> Option<(f64, f64, f64, f64)> {
    let first = samples.first()?;
    let mut min_x = first.x;
    let mut max_x = first.x;
    let mut min_y = first.y;
    let mut max_y = first.y;
    for sample in samples.iter().skip(1) {
        min_x = min_x.min(sample.x);
        max_x = max_x.max(sample.x);
        min_y = min_y.min(sample.y);
        max_y = max_y.max(sample.y);
    }
    Some((min_x, max_x, min_y, max_y))
}

fn nearly(a: f64, b: f64, tolerance: f64) -> bool {
    (a - b).abs() <= tolerance * 0.5
}

fn median_x(samples: &[SurfaceSample]) -> f64 {
    let mut values = samples.iter().map(|sample| sample.x).collect::<Vec<_>>();
    values.sort_by(f64::total_cmp);
    values[values.len() / 2]
}

fn distance(ax: f64, ay: f64, bx: f64, by: f64) -> f64 {
    ((ax - bx).powi(2) + (ay - by).powi(2)).sqrt()
}

fn surface_audit_report(
    request: &AuditRequest,
    response: &Map<String, Value>,
    metrics: &AuditMetrics,
    warnings: &[Value],
) -> Value {
    json!({
        "command": COMMAND,
        "request": request.params,
        "provider_response": response,
        "metrics": metrics.to_json(),
        "warnings": warnings,
    })
}

fn surface_artifact_path(config: &ProjectConfig, params: &Map<String, Value>) -> PathBuf {
    params
        .get("out")
        .and_then(Value::as_str)
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            runtime::load_session_record(config, COMMAND_TEXT)
                .map(|record| PathBuf::from(record.artifact_dir))
                .unwrap_or_else(|_| config.artifacts_dir.clone())
                .join("surface")
                .join(unique_suffix())
                .join("surface_audit.json")
        })
}

fn heatmap_artifact_path(params: &Map<String, Value>) -> Option<PathBuf> {
    params
        .get("heatmap_out")
        .and_then(Value::as_str)
        .map(PathBuf::from)
}

fn artifact_context(path: &Path, heatmap_path: Option<&Path>) -> Map<String, Value> {
    let mut artifacts = Map::from_iter([("surface_audit".to_string(), json!(absolute_path(path)))]);
    if let Some(heatmap_path) = heatmap_path {
        artifacts.insert("heatmap".to_string(), json!(absolute_path(heatmap_path)));
    }
    artifacts
}

fn warning(
    id: &str,
    message: &str,
    expected: Value,
    actual: Value,
    params: &Map<String, Value>,
    artifacts: &Map<String, Value>,
) -> Value {
    json!({
        "id": id,
        "severity": "warning",
        "message": message,
        "command": COMMAND_TEXT,
        "target": params.get("provider").cloned().unwrap_or_else(|| json!("")),
        "expected": expected,
        "actual": actual,
        "artifacts": artifacts,
    })
}

fn promoted_warning_failure(
    params: &Map<String, Value>,
    warnings: &[Value],
) -> Option<FailureDetail> {
    let strict = params
        .get("strict_warnings")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let threshold_warning = warnings.iter().any(|warning| {
        matches!(
            warning.get("id").and_then(Value::as_str),
            Some("surface_audit.spawn_clearance_failed")
                | Some("surface_audit.corridor_width_failed")
                | Some("surface_audit.symmetry_delta_high")
        )
    });
    if !strict && !threshold_warning {
        return None;
    }
    Some(FailureDetail {
        kind: "surface_audit_warning_failed".to_string(),
        command: Some(COMMAND_TEXT.to_string()),
        message: Some("surface audit warnings were promoted to failures".to_string()),
        expected: Some(json!("no promoted surface audit warnings")),
        actual: Some(Value::Array(warnings.to_vec())),
        severity: Some("error".to_string()),
        ..FailureDetail::default()
    })
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

fn render_heatmap_png(
    path: &Path,
    params: &Map<String, Value>,
    response: &Map<String, Value>,
) -> Result<(), Box<dyn std::error::Error>> {
    let samples = response
        .get("samples")
        .and_then(Value::as_array)
        .map(|samples| samples.iter().map(normalize_sample).collect::<Vec<_>>())
        .unwrap_or_default();
    let valid_samples = samples
        .iter()
        .filter(|sample| sample.valid)
        .cloned()
        .collect::<Vec<_>>();
    if valid_samples.is_empty() {
        return Err("surface audit heatmap requires at least one valid sample".into());
    }
    let actor_radius = number_param(params, "actor_radius").unwrap_or(0.0).max(0.0);
    let anchors = heatmap_anchor_points(response);
    let Some((min_x, max_x, min_y, max_y)) = heatmap_bounds(&valid_samples, &anchors, actor_radius)
    else {
        return Err("surface audit heatmap bounds could not be calculated".into());
    };
    let padding = 16.0;
    let width = ((max_x - min_x) + padding * 2.0).ceil().clamp(64.0, 4096.0) as u32;
    let height = ((max_y - min_y) + padding * 2.0).ceil().clamp(64.0, 4096.0) as u32;
    let scale_x = if (max_x - min_x).abs() < f64::EPSILON {
        1.0
    } else {
        (width as f64 - padding * 2.0) / (max_x - min_x)
    };
    let scale_y = if (max_y - min_y).abs() < f64::EPSILON {
        1.0
    } else {
        (height as f64 - padding * 2.0) / (max_y - min_y)
    };
    let scale = scale_x.min(scale_y).max(1.0);
    let map_point = |x: f64, y: f64| -> (i32, i32) {
        (
            ((x - min_x) * scale + padding).round() as i32,
            ((y - min_y) * scale + padding).round() as i32,
        )
    };

    let mut image = RgbaImage::from_pixel(width, height, Rgba([22, 24, 28, 255]));
    let sample_radius = number_param(params, "sample_step")
        .map(|step| (step * scale * 0.35).round().clamp(2.0, 8.0) as i32)
        .unwrap_or(4);
    for sample in &valid_samples {
        let (x, y) = map_point(sample.x, sample.y);
        draw_disc(&mut image, (x, y), sample_radius, sample_color(sample));
    }
    if anchors.len() >= 2 {
        for pair in anchors.windows(2) {
            let start = map_point(pair[0].0, pair[0].1);
            let end = map_point(pair[1].0, pair[1].1);
            draw_line(&mut image, start, end, Rgba([245, 245, 245, 255]));
        }
    }
    for (x, y) in anchors {
        let center = map_point(x, y);
        if actor_radius > 0.0 {
            draw_circle_outline(
                &mut image,
                center,
                (actor_radius * scale).round().max(1.0) as i32,
                Rgba([255, 255, 255, 190]),
            );
        }
        draw_cross(&mut image, center, 6, Rgba([255, 255, 255, 255]));
    }
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    image.save(path)?;
    Ok(())
}

fn heatmap_bounds(
    samples: &[SurfaceSample],
    anchors: &[(f64, f64)],
    actor_radius: f64,
) -> Option<(f64, f64, f64, f64)> {
    let (mut min_x, mut max_x, mut min_y, mut max_y) = sample_bounds(samples)?;
    for (x, y) in anchors {
        min_x = min_x.min(*x - actor_radius);
        max_x = max_x.max(*x + actor_radius);
        min_y = min_y.min(*y - actor_radius);
        max_y = max_y.max(*y + actor_radius);
    }
    Some((min_x, max_x, min_y, max_y))
}

fn heatmap_anchor_points(response: &Map<String, Value>) -> Vec<(f64, f64)> {
    let Some(anchors) = response.get("anchors").and_then(Value::as_object) else {
        return Vec::new();
    };
    let mut entries = anchors.iter().collect::<Vec<_>>();
    entries.sort_by_key(|(key, _)| *key);
    entries
        .into_iter()
        .filter_map(|(_, value)| {
            value
                .get("position")
                .and_then(|position| point_from_value(Some(position)))
                .or_else(|| point_from_value(Some(value)))
        })
        .collect()
}

fn sample_color(sample: &SurfaceSample) -> Rgba<u8> {
    if is_unknown_surface_label(&sample.surface) {
        return Rgba([137, 87, 229, 255]);
    }
    if sample.solid {
        if sample.surface == "solid" {
            Rgba([92, 28, 28, 255])
        } else {
            Rgba([220, 50, 47, 255])
        }
    } else if sample.soft {
        Rgba([255, 176, 32, 255])
    } else if sample.surface == "water" {
        Rgba([56, 132, 255, 255])
    } else {
        Rgba([46, 179, 98, 255])
    }
}

fn draw_disc(image: &mut RgbaImage, center: (i32, i32), radius: i32, color: Rgba<u8>) {
    let radius_squared = radius * radius;
    for y in center.1 - radius..=center.1 + radius {
        for x in center.0 - radius..=center.0 + radius {
            let dx = x - center.0;
            let dy = y - center.1;
            if dx * dx + dy * dy <= radius_squared {
                put_pixel_checked(image, x, y, color);
            }
        }
    }
}

fn draw_cross(image: &mut RgbaImage, center: (i32, i32), radius: i32, color: Rgba<u8>) {
    for offset in -radius..=radius {
        put_pixel_checked(image, center.0 + offset, center.1, color);
        put_pixel_checked(image, center.0, center.1 + offset, color);
    }
}

fn draw_circle_outline(image: &mut RgbaImage, center: (i32, i32), radius: i32, color: Rgba<u8>) {
    if radius <= 0 {
        return;
    }
    let inner = (radius - 1).max(0) * (radius - 1).max(0);
    let outer = (radius + 1) * (radius + 1);
    for y in center.1 - radius - 1..=center.1 + radius + 1 {
        for x in center.0 - radius - 1..=center.0 + radius + 1 {
            let dx = x - center.0;
            let dy = y - center.1;
            let distance = dx * dx + dy * dy;
            if distance >= inner && distance <= outer {
                put_pixel_checked(image, x, y, color);
            }
        }
    }
}

fn draw_line(image: &mut RgbaImage, start: (i32, i32), end: (i32, i32), color: Rgba<u8>) {
    let mut x0 = start.0;
    let mut y0 = start.1;
    let x1 = end.0;
    let y1 = end.1;
    let dx = (x1 - x0).abs();
    let sx = if x0 < x1 { 1 } else { -1 };
    let dy = -(y1 - y0).abs();
    let sy = if y0 < y1 { 1 } else { -1 };
    let mut error = dx + dy;
    loop {
        draw_disc(image, (x0, y0), 1, color);
        if x0 == x1 && y0 == y1 {
            break;
        }
        let error2 = 2 * error;
        if error2 >= dy {
            error += dy;
            x0 += sx;
        }
        if error2 <= dx {
            error += dx;
            y0 += sy;
        }
    }
}

fn put_pixel_checked(image: &mut RgbaImage, x: i32, y: i32, color: Rgba<u8>) {
    if x >= 0 && y >= 0 && (x as u32) < image.width() && (y as u32) < image.height() {
        image.put_pixel(x as u32, y as u32, color);
    }
}

fn artifact_write_failure(path: &Path, error: &dyn std::fmt::Display) -> FailureDetail {
    FailureDetail {
        kind: "artifact_write_failed".to_string(),
        command: Some(COMMAND_TEXT.to_string()),
        path: Some(path.to_string_lossy().into_owned()),
        message: Some("surface audit artifact could not be written".to_string()),
        expected: Some(json!("writable surface audit artifact path")),
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

    #[test]
    fn surface_audit_metrics_clear_spawn_and_artifact_shape() {
        let params = region_params(Some(24.0), false);
        let response = response(vec![
            sample(-16.0, 0.0, "walkable", false, false),
            sample(0.0, 0.0, "walkable", false, false),
            sample(96.0, 0.0, "blocked", true, false),
        ]);
        let metrics = compute_metrics(&params, &response);
        assert_eq!(metrics.sample_count, 3);
        assert_eq!(metrics.walkable_count, 2);
        assert_eq!(metrics.blocked_count, 1);
        assert_eq!(metrics.soft_count, 0);
        assert_eq!(metrics.spawn_clearance_pass, Some(true));
        assert_eq!(metrics.hard_block_cluster_count, 1);
        let artifacts =
            Map::from_iter([("surface_audit".to_string(), json!("surface_audit.json"))]);
        let warnings = warning_rules(&params, &metrics, &artifacts);
        assert!(warnings.is_empty());
        let report = surface_audit_report(&AuditRequest { params }, &response, &metrics, &warnings);
        assert_eq!(report["command"], COMMAND);
        assert!(report["metrics"].get("sample_count").is_some());
        assert!(
            report["metrics"]
                .get("provider_missing_or_invalid")
                .is_some()
        );
    }

    #[test]
    fn surface_audit_metrics_blocked_spawn_promotes_threshold_warning() {
        let params = region_params(Some(72.0), false);
        let response = response(vec![
            sample(0.0, 0.0, "walkable", false, false),
            sample(32.0, 0.0, "blocked", true, false),
        ]);
        let metrics = compute_metrics(&params, &response);
        assert_eq!(metrics.spawn_clearance_pass, Some(false));
        let warnings = warning_rules(&params, &metrics, &artifact_map());
        assert!(warning_ids(&warnings).contains(&"surface_audit.spawn_clearance_failed"));
        assert!(promoted_warning_failure(&params, &warnings).is_some());
    }

    #[test]
    fn surface_audit_metrics_narrow_corridor_soft_and_hard_clusters() {
        let params = route_params(Some(64.0), false);
        let response = response(vec![
            sample(0.0, 0.0, "blocked", true, false),
            sample(16.0, 0.0, "blocked", true, false),
            sample(0.0, 16.0, "walkable", false, false),
            sample(16.0, 16.0, "soft", false, true),
            sample(0.0, 32.0, "blocked", true, false),
            sample(16.0, 32.0, "blocked", true, false),
        ]);
        let metrics = compute_metrics(&params, &response);
        assert_eq!(metrics.soft_count, 1);
        assert_eq!(metrics.hard_block_cluster_count, 2);
        assert_eq!(metrics.soft_block_cluster_count, 1);
        assert_eq!(metrics.corridor_width_pass, Some(false));
        assert!(!metrics.route_clearance_fail_points.is_empty());
        let warnings = warning_rules(&params, &metrics, &artifact_map());
        let ids = warning_ids(&warnings);
        assert!(ids.contains(&"surface_audit.corridor_width_failed"));
        assert!(ids.contains(&"surface_audit.hard_block_cluster_blocks_route"));
        assert!(ids.contains(&"surface_audit.soft_blocker_turns_corridor_risky"));
    }

    #[test]
    fn surface_audit_metrics_dead_end_pocket_and_custom_labels() {
        let params = region_params(None, false);
        let mut samples = Vec::new();
        for y in 0..=2 {
            for x in 0..=2 {
                let is_center = x == 1 && y == 1;
                samples.push(sample(
                    x as f64 * 16.0,
                    y as f64 * 16.0,
                    if is_center { "moss" } else { "blocked" },
                    !is_center,
                    false,
                ));
            }
        }
        let response = response(samples);
        let metrics = compute_metrics(&params, &response);
        assert_eq!(metrics.dead_end_pocket_count, 1);
        assert_eq!(metrics.unknown_count, 1);
        let warnings = warning_rules(&params, &metrics, &artifact_map());
        assert!(warning_ids(&warnings).contains(&"surface_audit.unknown_surface_label"));
        assert!(promoted_warning_failure(&params, &warnings).is_none());
    }

    #[test]
    fn surface_audit_metrics_mirrored_delta_and_strict_warning() {
        let params = mirror_params(true);
        let response = response(vec![
            sided_sample(-16.0, 0.0, "left", "walkable", false),
            sided_sample(-16.0, 16.0, "left", "walkable", false),
            sided_sample(16.0, 0.0, "right", "blocked", true),
            sided_sample(16.0, 16.0, "right", "walkable", false),
        ]);
        let metrics = compute_metrics(&params, &response);
        assert_eq!(metrics.left_right_passability_delta, Some(0.5));
        assert_eq!(metrics.symmetry_delta_score, Some(0.5));
        let warnings = warning_rules(&params, &metrics, &artifact_map());
        assert!(warning_ids(&warnings).contains(&"surface_audit.symmetry_delta_high"));
        assert!(promoted_warning_failure(&params, &warnings).is_some());
    }

    #[test]
    fn surface_audit_metrics_provider_invalid_payload() {
        let params = region_params(None, false);
        let response = Map::from_iter([
            ("command".to_string(), json!(runtime::SURFACE_AUDIT_PROBE)),
            ("samples".to_string(), json!([{"surface": "walkable"}])),
        ]);
        let metrics = compute_metrics(&params, &response);
        assert!(metrics.provider_missing_or_invalid);
        let warnings = warning_rules(&params, &metrics, &artifact_map());
        assert!(warning_ids(&warnings).contains(&"surface_audit.provider_invalid"));
    }

    #[test]
    fn surface_audit_heatmap_renders_png_and_artifact_metadata() {
        let root = std::env::temp_dir().join(format!("godot-qa-surface-{}", unique_suffix()));
        fs::create_dir_all(&root).expect("temp root");
        let heatmap = root.join("surface.png");
        let params = route_params(Some(64.0), false);
        let response = Map::from_iter([
            ("command".to_string(), json!(runtime::SURFACE_AUDIT_PROBE)),
            (
                "anchors".to_string(),
                json!({
                    "route_0": {"position": {"x": 0.0, "y": 0.0}},
                    "route_1": {"position": {"x": 32.0, "y": 32.0}},
                }),
            ),
            (
                "samples".to_string(),
                json!([
                    sample(0.0, 0.0, "walkable", false, false),
                    sample(16.0, 0.0, "soft", false, true),
                    sample(32.0, 0.0, "blocked", true, false),
                    sample(48.0, 0.0, "mystery", false, false),
                    sample(64.0, 0.0, "water", false, false),
                ]),
            ),
        ]);

        render_heatmap_png(&heatmap, &params, &response).expect("heatmap render");
        let rendered = image::open(&heatmap).expect("read heatmap").to_rgba8();
        assert!(rendered.width() >= 64);
        assert!(rendered.height() >= 64);
        for color in [
            [46, 179, 98, 255],
            [255, 176, 32, 255],
            [220, 50, 47, 255],
            [137, 87, 229, 255],
            [56, 132, 255, 255],
            [255, 255, 255, 255],
        ] {
            assert!(
                rendered.pixels().any(|pixel| pixel.0 == color),
                "missing representative color {color:?}"
            );
        }

        let report = root.join("surface_audit.json");
        let artifacts = artifact_context(&report, Some(&heatmap));
        assert!(
            artifacts["surface_audit"]
                .as_str()
                .unwrap()
                .ends_with("surface_audit.json")
        );
        assert!(
            artifacts["heatmap"]
                .as_str()
                .unwrap()
                .ends_with("surface.png")
        );
        let artifacts_without_heatmap = artifact_context(&report, None);
        assert!(artifacts_without_heatmap.get("heatmap").is_none());
    }

    fn region_params(min_spawn_clearance: Option<f64>, strict: bool) -> Map<String, Value> {
        let mut params = Map::from_iter([
            ("mode".to_string(), json!("region")),
            ("provider".to_string(), json!("qa:surface_provider")),
            ("sample_step".to_string(), json!(16.0)),
            ("actor_radius".to_string(), json!(16.0)),
            ("strict_warnings".to_string(), json!(strict)),
            ("center_point".to_string(), json!({"x": 0.0, "y": 0.0})),
        ]);
        if let Some(value) = min_spawn_clearance {
            params.insert("min_spawn_clearance".to_string(), json!(value));
        }
        params
    }

    fn route_params(min_corridor_width: Option<f64>, strict: bool) -> Map<String, Value> {
        let mut params = Map::from_iter([
            ("mode".to_string(), json!("route")),
            ("provider".to_string(), json!("qa:surface_provider")),
            ("sample_step".to_string(), json!(16.0)),
            ("actor_radius".to_string(), json!(8.0)),
            ("strict_warnings".to_string(), json!(strict)),
        ]);
        if let Some(value) = min_corridor_width {
            params.insert("min_corridor_width".to_string(), json!(value));
        }
        params
    }

    fn mirror_params(strict: bool) -> Map<String, Value> {
        Map::from_iter([
            ("mode".to_string(), json!("mirror")),
            ("provider".to_string(), json!("qa:surface_provider")),
            ("sample_step".to_string(), json!(16.0)),
            ("actor_radius".to_string(), json!(8.0)),
            ("strict_warnings".to_string(), json!(strict)),
        ])
    }

    fn response(samples: Vec<Value>) -> Map<String, Value> {
        Map::from_iter([
            ("command".to_string(), json!(runtime::SURFACE_AUDIT_PROBE)),
            ("samples".to_string(), Value::Array(samples)),
        ])
    }

    fn sample(x: f64, y: f64, surface: &str, solid: bool, soft: bool) -> Value {
        json!({
            "point": {"x": x, "y": y},
            "surface": surface,
            "solid": solid,
            "soft": soft,
        })
    }

    fn sided_sample(x: f64, y: f64, side: &str, surface: &str, solid: bool) -> Value {
        json!({
            "point": {"x": x, "y": y},
            "surface": surface,
            "solid": solid,
            "soft": false,
            "metadata": {"side": side},
        })
    }

    fn artifact_map() -> Map<String, Value> {
        Map::from_iter([("surface_audit".to_string(), json!("surface_audit.json"))])
    }

    fn warning_ids(warnings: &[Value]) -> BTreeSet<&str> {
        warnings
            .iter()
            .filter_map(|warning| warning.get("id").and_then(Value::as_str))
            .collect()
    }
}
