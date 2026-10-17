use godot_qa_core::results::{CommandResult, FailureDetail};
use serde_json::{Map, Value, json};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

const COMMAND: &str = "physics collision-report";
const DATA_COMMAND: &str = "physics.collision_report";
const EPSILON: f64 = 0.000_001;
const DEFAULT_MAX_NORMAL_RETENTION: f64 = 0.85;
const DEFAULT_MAX_VELOCITY_REVERSALS: i64 = 6;
const DEFAULT_MAX_SAME_CONTACT_FRAMES: i64 = 180;
const DEFAULT_MAX_ANGULAR_SPIKE: f64 = 25.0;
const DEFAULT_MAX_OVERLAP_DEPTH: f64 = 32.0;
const DEFAULT_MAX_LOW_PROGRESS_CONTACT_FRAMES: i64 = 120;

#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct Thresholds {
    pub(crate) max_normal_retention: Option<f64>,
    pub(crate) max_velocity_reversals: Option<i64>,
    pub(crate) max_same_contact_frames: Option<i64>,
    pub(crate) max_angular_velocity_spike: Option<f64>,
    pub(crate) max_overlap_depth: Option<f64>,
    pub(crate) max_low_progress_contact_frames: Option<i64>,
}

#[derive(Debug, Clone, Copy, Default)]
struct Vector {
    x: f64,
    y: f64,
}

impl Vector {
    fn dot(self, other: Self) -> f64 {
        self.x * other.x + self.y * other.y
    }

    fn len(self) -> f64 {
        self.dot(self).sqrt()
    }

    fn normalized(self) -> Option<Self> {
        let len = self.len();
        (len > EPSILON).then_some(Self {
            x: self.x / len,
            y: self.y / len,
        })
    }
}

#[derive(Debug, Clone)]
struct Contact {
    contact_id: String,
    surface_type: String,
    normal: Option<Vector>,
    depth: Option<f64>,
    is_soft: bool,
}

#[derive(Debug, Clone)]
struct Sample {
    velocity: Option<Vector>,
    angular_velocity: Option<f64>,
    progress_scalar: Option<f64>,
    rescue_push_count: i64,
    contacts: Vec<Contact>,
    generic_overlap: bool,
    provider_enabled: bool,
    provider_available: bool,
    provider_invalid: bool,
}

#[derive(Debug, Default)]
struct Aggregates {
    frames_sampled: i64,
    contact_frame_count: i64,
    contact_count_by_surface_type: BTreeMap<String, i64>,
    unique_contact_ids: BTreeSet<String>,
    same_contact_max_consecutive_frames: i64,
    largest_overlap_depth: Option<f64>,
    max_speed_before_contact: Option<f64>,
    max_speed_after_contact: Option<f64>,
    max_speed_delta_after_contact: Option<f64>,
    normal_velocity_before: Option<f64>,
    normal_velocity_after: Option<f64>,
    normal_velocity_retention_ratio: Option<f64>,
    tangent_velocity_retention_ratio: Option<f64>,
    velocity_sign_reversal_count: i64,
    max_angular_velocity_before_contact: Option<f64>,
    max_angular_velocity_after_contact: Option<f64>,
    angular_velocity_spike_count: i64,
    soft_contact_duration_frames: i64,
    hard_contact_duration_frames: i64,
    rescue_push_count: i64,
    stuck_like_frame_count: i64,
    low_progress_while_contact_frames: i64,
    provider_enabled: bool,
    provider_available: bool,
    provider_invalid: bool,
    has_motion_data: bool,
    has_normal_data: bool,
    soft_contact_has_hard_response: bool,
}

pub(crate) fn collision_report_from_trace_result(
    trace_path: &Path,
    target: Option<&str>,
    thresholds: Thresholds,
    strict_warnings: bool,
) -> CommandResult {
    let trace = match load_trace(trace_path) {
        Ok(trace) => trace,
        Err(failure) => return failure_result("collision report failed", failure, Map::new()),
    };
    let report_path = report_path_for_trace(trace_path);
    let target = target
        .map(str::trim)
        .filter(|target| !target.is_empty())
        .map(str::to_string)
        .or_else(|| infer_target(&trace));
    let Some(target) = target else {
        let failure = FailureDetail {
            kind: "collision_report_target_missing".to_string(),
            command: Some(COMMAND.to_string()),
            path: Some(trace_path.to_string_lossy().into_owned()),
            message: Some("trace did not contain a target sample".to_string()),
            expected: Some(json!("trace samples with at least one target")),
            actual: Some(json!(trace.get("samples"))),
            ..FailureDetail::default()
        };
        return failure_result("collision report failed", failure, Map::new());
    };

    let samples = normalize_samples(&trace, &target);
    let aggregates = aggregate_samples(&samples);
    let warnings = warning_rules(&aggregates, thresholds);
    let metrics = metrics_json(&aggregates);
    let artifacts = Map::from_iter([
        (
            "report".to_string(),
            json!(absolute_path_string(&report_path)),
        ),
        ("trace".to_string(), json!(absolute_path_string(trace_path))),
    ]);
    let summary = json!({
        "contact_frame_count": aggregates.contact_frame_count,
        "velocity_sign_reversal_count": aggregates.velocity_sign_reversal_count,
        "same_contact_max_consecutive_frames": aggregates.same_contact_max_consecutive_frames,
        "warnings": warnings,
    });
    let data = Map::from_iter([
        ("command".to_string(), json!(DATA_COMMAND)),
        ("target".to_string(), json!(target)),
        (
            "frames_sampled".to_string(),
            json!(aggregates.frames_sampled),
        ),
        ("artifacts".to_string(), Value::Object(artifacts.clone())),
        ("summary".to_string(), summary.clone()),
        ("metrics".to_string(), metrics.clone()),
    ]);
    let report = Value::Object(data.clone());
    if let Err(failure) = write_report(&report_path, &report) {
        return failure_result("collision report failed", failure, data);
    }

    let warning_values = warnings.as_array().cloned().unwrap_or_default();
    if strict_warnings && !warning_values.is_empty() {
        let failure = FailureDetail {
            kind: "collision_report_strict_warning".to_string(),
            command: Some(COMMAND.to_string()),
            message: Some(
                "collision report produced warnings while --strict-warnings was enabled"
                    .to_string(),
            ),
            expected: Some(json!("no collision report warnings")),
            actual: Some(Value::Array(warning_values)),
            severity: Some("error".to_string()),
            artifacts: Some(artifacts),
            ..FailureDetail::default()
        };
        return failure_result("collision report strict warnings failed", failure, data);
    }

    CommandResult {
        ok: true,
        command: COMMAND.to_string(),
        message: "collision report completed".to_string(),
        data,
        failures: Vec::new(),
        exit_code: 0,
    }
}

fn load_trace(path: &Path) -> Result<Value, FailureDetail> {
    let raw = fs::read_to_string(path).map_err(|error| FailureDetail {
        kind: "trace_read_failed".to_string(),
        command: Some(COMMAND.to_string()),
        path: Some(path.to_string_lossy().into_owned()),
        message: Some("trace artifact could not be read".to_string()),
        expected: Some(json!("readable trace JSON file")),
        actual: Some(json!(error.to_string())),
        ..FailureDetail::default()
    })?;
    let value: Value = serde_json::from_str(&raw).map_err(|error| FailureDetail {
        kind: "trace_parse_failed".to_string(),
        command: Some(COMMAND.to_string()),
        path: Some(path.to_string_lossy().into_owned()),
        message: Some("trace artifact was not valid JSON".to_string()),
        expected: Some(json!("trace JSON object")),
        actual: Some(json!(error.to_string())),
        ..FailureDetail::default()
    })?;
    if !value.is_object() || value.get("samples").and_then(Value::as_array).is_none() {
        return Err(FailureDetail {
            kind: "trace_invalid".to_string(),
            command: Some(COMMAND.to_string()),
            path: Some(path.to_string_lossy().into_owned()),
            message: Some("trace artifact must contain a samples array".to_string()),
            expected: Some(json!("trace.record object with samples[]")),
            actual: Some(value),
            ..FailureDetail::default()
        });
    }
    Ok(value)
}

fn infer_target(trace: &Value) -> Option<String> {
    trace
        .get("targets")
        .and_then(Value::as_array)
        .and_then(|targets| targets.first())
        .and_then(|target| target.get("target").or_else(|| target.get("path")))
        .and_then(Value::as_str)
        .map(str::to_string)
        .or_else(|| {
            trace
                .get("samples")
                .and_then(Value::as_array)
                .and_then(|samples| samples.first())
                .and_then(|sample| sample.get("targets"))
                .and_then(Value::as_array)
                .and_then(|targets| targets.first())
                .and_then(|target| target.get("target").or_else(|| target.get("path")))
                .and_then(Value::as_str)
                .map(str::to_string)
        })
}

fn normalize_samples(trace: &Value, target: &str) -> Vec<Sample> {
    trace
        .get("samples")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|sample| normalize_sample(sample, target))
        .collect()
}

fn normalize_sample(sample: &Value, target: &str) -> Option<Sample> {
    let target_sample = target_sample(sample, target)?;
    let project_states = project_states(target_sample);
    let mut contacts = Vec::new();
    let mut provider_enabled = false;
    let mut provider_available = false;
    let mut provider_invalid = false;
    let mut provider_velocity = None;
    let mut angular_velocity = number_at(target_sample, "angular_velocity");
    let mut progress_scalar = number_at(target_sample, "progress_scalar");
    let mut rescue_push_count = integer_at(target_sample, "rescue_push_count").unwrap_or(0);

    for state in project_states {
        provider_enabled |= state
            .get("enabled")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        provider_available |= state
            .get("available")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        provider_invalid |= state.get("error").is_some();
        extract_state_value_payloads(state, &mut |payload, ok| {
            provider_invalid |= !ok;
            if ok {
                contacts.extend(extract_contacts(payload));
                provider_velocity = provider_velocity.or_else(|| vector_at(payload, "velocity"));
                angular_velocity =
                    angular_velocity.or_else(|| number_at(payload, "angular_velocity"));
                progress_scalar = progress_scalar.or_else(|| number_at(payload, "progress_scalar"));
                rescue_push_count += integer_at(payload, "rescue_push_count").unwrap_or(0);
            }
        });
    }
    contacts.extend(extract_contacts(target_sample));

    let generic_overlap = target_sample
        .get("physics")
        .and_then(|physics| physics.get("target_shape_overlap_count"))
        .and_then(Value::as_i64)
        .unwrap_or(0)
        > 0;
    Some(Sample {
        velocity: vector_at(target_sample, "velocity").or(provider_velocity),
        angular_velocity,
        progress_scalar,
        rescue_push_count,
        contacts,
        generic_overlap,
        provider_enabled,
        provider_available,
        provider_invalid,
    })
}

fn target_sample<'a>(sample: &'a Value, target: &str) -> Option<&'a Value> {
    let targets = sample.get("targets")?.as_array()?;
    targets
        .iter()
        .find(|entry| {
            ["target", "path", "resolved", "qa_id"]
                .iter()
                .filter_map(|key| entry.get(*key).and_then(Value::as_str))
                .any(|value| value == target)
        })
        .or_else(|| targets.first())
}

fn project_states(target_sample: &Value) -> Vec<&Value> {
    let mut states = Vec::new();
    if let Some(state) = target_sample.get("project_state") {
        states.push(state);
    }
    if let Some(state) = target_sample
        .get("physics")
        .and_then(|physics| physics.get("project_state"))
    {
        states.push(state);
    }
    states
}

fn extract_state_value_payloads<F>(state: &Value, visitor: &mut F)
where
    F: FnMut(&Value, bool),
{
    if state.get("contacts").is_some()
        || state.get("velocity").is_some()
        || state.get("angular_velocity").is_some()
    {
        visitor(state, true);
    }
    let providers = state
        .get("providers")
        .and_then(Value::as_array)
        .into_iter()
        .flatten();
    for provider in providers {
        let methods = provider
            .get("methods")
            .and_then(Value::as_array)
            .into_iter()
            .flatten();
        for method in methods {
            let ok = method.get("ok").and_then(Value::as_bool).unwrap_or(false);
            if let Some(value) = method.get("value") {
                visitor(value, ok && value.is_object());
            } else if !ok {
                visitor(method, false);
            }
        }
    }
}

fn extract_contacts(payload: &Value) -> Vec<Contact> {
    payload
        .get("contacts")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|contact| {
            let object = contact.as_object()?;
            let contact_id = object
                .get("contact_id")
                .or_else(|| object.get("other_id"))
                .or_else(|| object.get("node_path"))
                .and_then(Value::as_str)
                .unwrap_or("unknown")
                .to_string();
            let surface_type = object
                .get("surface_type")
                .and_then(Value::as_str)
                .unwrap_or("unknown")
                .to_string();
            Some(Contact {
                contact_id,
                surface_type,
                normal: vector_at(contact, "normal"),
                depth: number_at(contact, "depth"),
                is_soft: object
                    .get("is_soft")
                    .and_then(Value::as_bool)
                    .unwrap_or(false)
                    || object
                        .get("surface_type")
                        .and_then(Value::as_str)
                        .is_some_and(|surface| matches!(surface, "soft" | "trigger")),
            })
        })
        .collect()
}

fn aggregate_samples(samples: &[Sample]) -> Aggregates {
    let mut agg = Aggregates {
        frames_sampled: samples.len() as i64,
        ..Aggregates::default()
    };
    let mut previous: Option<&Sample> = None;
    let mut consecutive: BTreeMap<String, i64> = BTreeMap::new();
    let mut previous_rescue_count = 0;
    let mut low_progress_run = 0;

    for sample in samples {
        let has_contact = !sample.contacts.is_empty() || sample.generic_overlap;
        agg.provider_enabled |= sample.provider_enabled;
        agg.provider_available |= sample.provider_available;
        agg.provider_invalid |= sample.provider_invalid;
        agg.has_motion_data |= sample.velocity.is_some();
        if has_contact {
            agg.contact_frame_count += 1;
        }
        for contact in &sample.contacts {
            *agg.contact_count_by_surface_type
                .entry(contact.surface_type.clone())
                .or_insert(0) += 1;
            agg.unique_contact_ids.insert(contact.contact_id.clone());
            if let Some(depth) = contact.depth.filter(|depth| depth.is_finite()) {
                agg.largest_overlap_depth =
                    Some(agg.largest_overlap_depth.unwrap_or(0.0).max(depth));
            }
            if contact.is_soft {
                agg.soft_contact_duration_frames += 1;
            } else {
                agg.hard_contact_duration_frames += 1;
            }
        }

        let ids = sample
            .contacts
            .iter()
            .map(|contact| contact.contact_id.clone())
            .collect::<BTreeSet<_>>();
        let mut next_consecutive = BTreeMap::new();
        for id in ids {
            let count = consecutive.get(&id).copied().unwrap_or(0) + 1;
            agg.same_contact_max_consecutive_frames =
                agg.same_contact_max_consecutive_frames.max(count);
            next_consecutive.insert(id, count);
        }
        consecutive = next_consecutive;

        if let Some(prev) = previous {
            update_transition_metrics(&mut agg, prev, sample, has_contact);
            if has_contact && stuck_like(prev, sample) {
                agg.stuck_like_frame_count += 1;
            }
            if has_contact && low_progress(prev, sample) {
                low_progress_run += 1;
            } else {
                low_progress_run = 0;
            }
            agg.low_progress_while_contact_frames =
                agg.low_progress_while_contact_frames.max(low_progress_run);
        }
        agg.rescue_push_count += (sample.rescue_push_count - previous_rescue_count).max(0);
        previous_rescue_count = sample.rescue_push_count;
        previous = Some(sample);
    }
    agg
}

fn update_transition_metrics(
    agg: &mut Aggregates,
    previous: &Sample,
    current: &Sample,
    current_has_contact: bool,
) {
    let previous_speed = previous.velocity.map(Vector::len);
    let current_speed = current.velocity.map(Vector::len);
    if current_has_contact {
        if let Some(speed) = previous_speed {
            agg.max_speed_before_contact =
                Some(agg.max_speed_before_contact.unwrap_or(0.0).max(speed));
        }
        if let Some(speed) = current_speed {
            agg.max_speed_after_contact =
                Some(agg.max_speed_after_contact.unwrap_or(0.0).max(speed));
        }
        if let (Some(before), Some(after)) = (previous_speed, current_speed) {
            agg.max_speed_delta_after_contact = Some(
                agg.max_speed_delta_after_contact
                    .unwrap_or(0.0)
                    .max((after - before).abs()),
            );
        }
        if let Some(angular) = previous.angular_velocity.map(f64::abs) {
            agg.max_angular_velocity_before_contact = Some(
                agg.max_angular_velocity_before_contact
                    .unwrap_or(0.0)
                    .max(angular),
            );
        }
        if let Some(angular) = current.angular_velocity.map(f64::abs) {
            agg.max_angular_velocity_after_contact = Some(
                agg.max_angular_velocity_after_contact
                    .unwrap_or(0.0)
                    .max(angular),
            );
        }
    }
    if let (Some(before), Some(after)) = (previous.angular_velocity, current.angular_velocity) {
        if current_has_contact && (after - before).abs() > DEFAULT_MAX_ANGULAR_SPIKE {
            agg.angular_velocity_spike_count += 1;
        }
    }
    if let (Some(before), Some(after)) = (previous.velocity, current.velocity) {
        if current_has_contact && before.dot(after) < 0.0 {
            agg.velocity_sign_reversal_count += 1;
        }
        for contact in &current.contacts {
            let Some(normal) = contact.normal.and_then(Vector::normalized) else {
                continue;
            };
            agg.has_normal_data = true;
            let tangent = Vector {
                x: -normal.y,
                y: normal.x,
            };
            let normal_before = before.dot(normal).abs();
            let normal_after = after.dot(normal).abs();
            let tangent_before = before.dot(tangent).abs();
            let tangent_after = after.dot(tangent).abs();
            agg.normal_velocity_before =
                Some(agg.normal_velocity_before.unwrap_or(0.0).max(normal_before));
            agg.normal_velocity_after =
                Some(agg.normal_velocity_after.unwrap_or(0.0).max(normal_after));
            if normal_before > EPSILON {
                let ratio = normal_after / normal_before;
                agg.normal_velocity_retention_ratio = Some(
                    agg.normal_velocity_retention_ratio
                        .unwrap_or(0.0)
                        .max(ratio),
                );
                if contact.is_soft && ratio > DEFAULT_MAX_NORMAL_RETENTION {
                    agg.soft_contact_has_hard_response = true;
                }
            }
            if tangent_before > EPSILON {
                let ratio = tangent_after / tangent_before;
                agg.tangent_velocity_retention_ratio = Some(
                    agg.tangent_velocity_retention_ratio
                        .unwrap_or(0.0)
                        .max(ratio),
                );
            }
        }
    }
}

fn stuck_like(previous: &Sample, current: &Sample) -> bool {
    let speed_low = current.velocity.map(Vector::len).unwrap_or(f64::INFINITY) < 1.0;
    speed_low && low_progress(previous, current)
}

fn low_progress(previous: &Sample, current: &Sample) -> bool {
    match (previous.progress_scalar, current.progress_scalar) {
        (Some(before), Some(after)) => (after - before).abs() < 0.01,
        _ => current.velocity.map(Vector::len).unwrap_or(f64::INFINITY) < 1.0,
    }
}

fn metrics_json(agg: &Aggregates) -> Value {
    json!({
        "frames_sampled": agg.frames_sampled,
        "contact_frame_count": agg.contact_frame_count,
        "contact_count_by_surface_type": agg.contact_count_by_surface_type,
        "unique_contact_ids": agg.unique_contact_ids.len(),
        "same_contact_max_consecutive_frames": agg.same_contact_max_consecutive_frames,
        "largest_overlap_depth": metric_optional_number(agg.largest_overlap_depth, "no contact depth data"),
        "max_speed_before_contact": metric_optional_number(agg.max_speed_before_contact, "no velocity data before contact"),
        "max_speed_after_contact": metric_optional_number(agg.max_speed_after_contact, "no velocity data after contact"),
        "max_speed_delta_after_contact": metric_optional_number(agg.max_speed_delta_after_contact, "no velocity transition data"),
        "normal_velocity_before": metric_optional_number(agg.normal_velocity_before, "no contact normal data"),
        "normal_velocity_after": metric_optional_number(agg.normal_velocity_after, "no contact normal data"),
        "normal_velocity_retention_ratio": metric_optional_number(agg.normal_velocity_retention_ratio, "no contact normal data"),
        "tangent_velocity_retention_ratio": metric_optional_number(agg.tangent_velocity_retention_ratio, "no contact normal data"),
        "velocity_sign_reversal_count": agg.velocity_sign_reversal_count,
        "max_angular_velocity_before_contact": metric_optional_number(agg.max_angular_velocity_before_contact, "no angular velocity data before contact"),
        "max_angular_velocity_after_contact": metric_optional_number(agg.max_angular_velocity_after_contact, "no angular velocity data after contact"),
        "angular_velocity_spike_count": agg.angular_velocity_spike_count,
        "soft_contact_duration_frames": agg.soft_contact_duration_frames,
        "hard_contact_duration_frames": agg.hard_contact_duration_frames,
        "rescue_push_count": agg.rescue_push_count,
        "stuck_like_frame_count": agg.stuck_like_frame_count,
        "low_progress_while_contact_frames": agg.low_progress_while_contact_frames,
    })
}

fn metric_optional_number(value: Option<f64>, reason: &str) -> Value {
    value
        .filter(|value| value.is_finite())
        .map(|value| json!(value))
        .unwrap_or_else(|| json!({"status": "not_evaluated", "reason": reason}))
}

fn warning_rules(agg: &Aggregates, thresholds: Thresholds) -> Value {
    let mut warnings = Vec::new();
    if !agg.provider_enabled || (!agg.provider_available && agg.contact_frame_count > 0) {
        warnings.push(warning("collision_report.provider_missing"));
    }
    if agg.provider_invalid {
        warnings.push(warning("collision_report.provider_invalid"));
    }
    if !agg.has_motion_data {
        warnings.push(warning("collision_report.insufficient_motion_data"));
    }
    if let Some(ratio) = agg.normal_velocity_retention_ratio {
        let max = thresholds
            .max_normal_retention
            .unwrap_or(DEFAULT_MAX_NORMAL_RETENTION);
        if ratio > max {
            warnings.push(warning("collision_report.normal_velocity_retention_high"));
            warnings.push(warning("collision_report.pinball_like_rebound"));
        }
    }
    if agg.velocity_sign_reversal_count
        > thresholds
            .max_velocity_reversals
            .unwrap_or(DEFAULT_MAX_VELOCITY_REVERSALS)
    {
        warnings.push(warning("collision_report.velocity_reversal_high"));
        warnings.push(warning("collision_report.repeated_bounce_loop"));
    }
    if agg.same_contact_max_consecutive_frames
        > thresholds
            .max_same_contact_frames
            .unwrap_or(DEFAULT_MAX_SAME_CONTACT_FRAMES)
    {
        warnings.push(warning("collision_report.same_contact_pin_risk"));
    }
    if agg.angular_velocity_spike_count > 0
        || agg.max_angular_velocity_after_contact.unwrap_or(0.0)
            > thresholds
                .max_angular_velocity_spike
                .unwrap_or(DEFAULT_MAX_ANGULAR_SPIKE)
    {
        warnings.push(warning("collision_report.angular_impulse_spike"));
    }
    if agg.soft_contact_has_hard_response {
        warnings.push(warning("collision_report.soft_contact_behaves_hard"));
    }
    if agg.largest_overlap_depth.unwrap_or(0.0)
        > thresholds
            .max_overlap_depth
            .unwrap_or(DEFAULT_MAX_OVERLAP_DEPTH)
        || agg.low_progress_while_contact_frames
            > thresholds
                .max_low_progress_contact_frames
                .unwrap_or(DEFAULT_MAX_LOW_PROGRESS_CONTACT_FRAMES)
    {
        warnings.push(warning("collision_report.overlap_without_recovery"));
    }
    Value::Array(warnings)
}

fn warning(rule_id: &str) -> Value {
    json!({"rule_id": rule_id, "severity": "warning"})
}

fn vector_at(value: &Value, key: &str) -> Option<Vector> {
    let object = value.get(key)?.as_object()?;
    let x = object.get("x")?.as_f64()?;
    let y = object.get("y")?.as_f64()?;
    (x.is_finite() && y.is_finite()).then_some(Vector { x, y })
}

fn number_at(value: &Value, key: &str) -> Option<f64> {
    value.get(key)?.as_f64().filter(|value| value.is_finite())
}

fn integer_at(value: &Value, key: &str) -> Option<i64> {
    value.get(key)?.as_i64()
}

fn report_path_for_trace(trace_path: &Path) -> PathBuf {
    trace_path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join("collision_report.json")
}

fn write_report(path: &Path, report: &Value) -> Result<(), FailureDetail> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| artifact_write_failure(path, error))?;
    }
    fs::write(
        path,
        serde_json::to_string_pretty(report).unwrap_or_else(|_| "{}".to_string()),
    )
    .map_err(|error| artifact_write_failure(path, error))
}

fn artifact_write_failure(path: &Path, error: impl std::fmt::Display) -> FailureDetail {
    FailureDetail {
        kind: "artifact_write_failed".to_string(),
        command: Some(COMMAND.to_string()),
        path: Some(path.to_string_lossy().into_owned()),
        message: Some("collision report artifact could not be written".to_string()),
        expected: Some(json!("writable collision_report.json artifact")),
        actual: Some(json!(error.to_string())),
        ..FailureDetail::default()
    }
}

fn absolute_path_string(path: &Path) -> String {
    path.canonicalize()
        .unwrap_or_else(|_| path.to_path_buf())
        .to_string_lossy()
        .into_owned()
}

fn failure_result(
    message: &str,
    failure: FailureDetail,
    data: Map<String, Value>,
) -> CommandResult {
    CommandResult {
        ok: false,
        command: COMMAND.to_string(),
        message: message.to_string(),
        data,
        failures: vec![failure],
        exit_code: 1,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_dir(name: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!("godot-qa-collision-report-{name}-{unique}"));
        fs::create_dir_all(&path).unwrap();
        path
    }

    fn write_trace(name: &str, trace: Value) -> PathBuf {
        let dir = temp_dir(name);
        let path = dir.join("trace.json");
        fs::write(&path, serde_json::to_string_pretty(&trace).unwrap()).unwrap();
        path
    }

    fn sample(frame: i64, velocity: Value, contacts: Value) -> Value {
        json!({
            "elapsed_frames": frame,
            "targets": [{
                "target": "qa:actor",
                "velocity": velocity,
                "project_state": {
                    "enabled": true,
                    "available": true,
                    "providers": [{
                        "methods": [{
                            "ok": true,
                            "method": "get_qa_collision_state",
                            "value": {"contacts": contacts}
                        }]
                    }]
                }
            }]
        })
    }

    #[test]
    fn collision_report_from_trace_writes_metrics_and_artifact() {
        let trace = json!({
            "targets": [{"target": "qa:actor"}],
            "samples": [
                sample(0, json!({"x": -10.0, "y": 0.0}), json!([])),
                sample(1, json!({"x": 8.0, "y": 1.0}), json!([{
                    "contact_id": "wall",
                    "surface_type": "hard",
                    "normal": {"x": 1.0, "y": 0.0},
                    "depth": 4.0
                }]))
            ]
        });
        let trace_path = write_trace("basic", trace);
        let result = collision_report_from_trace_result(
            &trace_path,
            None,
            Thresholds {
                max_normal_retention: Some(0.35),
                ..Thresholds::default()
            },
            false,
        );
        assert!(result.ok);
        assert_eq!(result.data["command"], json!(DATA_COMMAND));
        assert_eq!(result.data["frames_sampled"], json!(2));
        assert_eq!(result.data["metrics"]["contact_frame_count"], json!(1));
        assert_eq!(result.data["metrics"]["unique_contact_ids"], json!(1));
        assert!(trace_path.with_file_name("collision_report.json").exists());
        let warnings = result.data["summary"]["warnings"].as_array().unwrap();
        assert!(
            warnings
                .iter()
                .any(|warning| warning["rule_id"] == "collision_report.pinball_like_rebound")
        );
    }

    #[test]
    fn collision_report_strict_warnings_convert_to_failure() {
        let trace = json!({
            "targets": [{"target": "qa:actor"}],
            "samples": [
                sample(0, json!({"x": -10.0, "y": 0.0}), json!([])),
                sample(1, json!({"x": 9.0, "y": 0.0}), json!([{
                    "contact_id": "wall",
                    "surface_type": "hard",
                    "normal": {"x": 1.0, "y": 0.0}
                }]))
            ]
        });
        let trace_path = write_trace("strict", trace);
        let result = collision_report_from_trace_result(
            &trace_path,
            None,
            Thresholds {
                max_normal_retention: Some(0.1),
                ..Thresholds::default()
            },
            true,
        );
        assert!(!result.ok);
        assert_eq!(result.failures[0].kind, "collision_report_strict_warning");
    }

    #[test]
    fn collision_report_provider_missing_and_not_evaluated_metrics() {
        let trace = json!({
            "targets": [{"target": "qa:actor"}],
            "samples": [{
                "elapsed_frames": 0,
                "targets": [{
                    "target": "qa:actor",
                    "velocity": {"x": 0.0, "y": 0.0},
                    "physics": {"target_shape_overlap_count": 1}
                }]
            }]
        });
        let trace_path = write_trace("missing-provider", trace);
        let result =
            collision_report_from_trace_result(&trace_path, None, Thresholds::default(), false);
        assert!(result.ok);
        assert_eq!(
            result.data["metrics"]["normal_velocity_retention_ratio"]["status"],
            json!("not_evaluated")
        );
        let warnings = result.data["summary"]["warnings"].as_array().unwrap();
        assert!(
            warnings
                .iter()
                .any(|warning| warning["rule_id"] == "collision_report.provider_missing")
        );
    }

    #[test]
    fn collision_report_provider_invalid_warning() {
        let trace = json!({
            "targets": [{"target": "qa:actor"}],
            "samples": [{
                "elapsed_frames": 0,
                "targets": [{
                    "target": "qa:actor",
                    "project_state": {
                        "enabled": true,
                        "available": false,
                        "providers": [{"methods": [{"ok": false, "status": "invalid_state_method"}]}]
                    }
                }]
            }]
        });
        let trace_path = write_trace("invalid-provider", trace);
        let result =
            collision_report_from_trace_result(&trace_path, None, Thresholds::default(), false);
        let warnings = result.data["summary"]["warnings"].as_array().unwrap();
        assert!(
            warnings
                .iter()
                .any(|warning| warning["rule_id"] == "collision_report.provider_invalid")
        );
    }

    #[test]
    fn collision_report_soft_contact_behaves_hard_only_when_classified_soft() {
        let trace = json!({
            "targets": [{"target": "qa:actor"}],
            "samples": [
                sample(0, json!({"x": -10.0, "y": 0.0}), json!([])),
                sample(1, json!({"x": 10.0, "y": 0.0}), json!([{
                    "contact_id": "field",
                    "surface_type": "soft",
                    "normal": {"x": 1.0, "y": 0.0},
                    "is_soft": true
                }]))
            ]
        });
        let trace_path = write_trace("soft", trace);
        let result =
            collision_report_from_trace_result(&trace_path, None, Thresholds::default(), false);
        let warnings = result.data["summary"]["warnings"].as_array().unwrap();
        assert!(
            warnings
                .iter()
                .any(|warning| warning["rule_id"] == "collision_report.soft_contact_behaves_hard")
        );
    }

    #[test]
    fn collision_report_repeated_contacts_and_stuck_like_frames_warn() {
        let contacts = json!([{"contact_id": "wall", "surface_type": "hard", "depth": 40.0}]);
        let trace = json!({
            "targets": [{"target": "qa:actor"}],
            "samples": [
                sample(0, json!({"x": 0.0, "y": 0.0}), contacts.clone()),
                sample(1, json!({"x": 0.0, "y": 0.0}), contacts.clone()),
                sample(2, json!({"x": 0.0, "y": 0.0}), contacts)
            ]
        });
        let trace_path = write_trace("stuck", trace);
        let result = collision_report_from_trace_result(
            &trace_path,
            None,
            Thresholds {
                max_same_contact_frames: Some(1),
                max_low_progress_contact_frames: Some(1),
                max_overlap_depth: Some(1.0),
                ..Thresholds::default()
            },
            false,
        );
        assert_eq!(
            result.data["metrics"]["same_contact_max_consecutive_frames"],
            json!(3)
        );
        assert_eq!(result.data["metrics"]["stuck_like_frame_count"], json!(2));
        let warnings = result.data["summary"]["warnings"].as_array().unwrap();
        assert!(
            warnings
                .iter()
                .any(|warning| warning["rule_id"] == "collision_report.same_contact_pin_risk")
        );
        assert!(
            warnings
                .iter()
                .any(|warning| warning["rule_id"] == "collision_report.overlap_without_recovery")
        );
    }
}
