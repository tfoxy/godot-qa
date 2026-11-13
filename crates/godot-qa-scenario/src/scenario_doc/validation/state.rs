use super::super::{
    ScenarioError, is_non_empty_string, is_safe_artifact_segment, python_type_name,
    validation_failure,
};
use super::{require_non_empty_string_field, validate_optional_positive_integer};
use serde_json::{Map, Value, json};
use std::path::Path;

/// State-source vocabulary shared by `state_matches` and `wait_until`.
const STATE_MATCH_SOURCES: &[&str] = &[
    "provider",
    "method",
    "qa_method",
    "metadata",
    "meta",
    "inspect",
    "auto",
];

/// Comparison operators understood by the runtime `state_matches` evaluator
/// (kept in sync with `_state_match_operators()` in qa_assertions_impl.gd).
const STATE_MATCH_OPERATORS: &[&str] = &[
    "equals",
    "contains",
    "present",
    "absent",
    "approx_equals",
    "approximately_equals",
    "greater_than",
    "less_than",
    "within",
];

/// Operators that require an `expected` value.
const OPERATORS_REQUIRING_EXPECTED: &[&str] = &[
    "equals",
    "contains",
    "approx_equals",
    "approximately_equals",
    "greater_than",
    "less_than",
    "within",
];

pub(super) fn validate_temporal_window(
    path: &Path,
    value: &Map<String, Value>,
    assertion_name: &str,
) -> Result<(), ScenarioError> {
    let has_frames = value.contains_key("window_frames");
    let has_seconds = value.contains_key("window_seconds");
    if has_frames == has_seconds {
        return Err(validation_failure(
            path,
            &format!(
                "{assertion_name} assertion must declare exactly one of 'window_frames' or 'window_seconds'"
            ),
            json!("exactly one window form"),
            Value::Object(value.clone()),
        ));
    }
    let field = if has_frames {
        "window_frames"
    } else {
        "window_seconds"
    };
    let Some(window) = value.get(field).and_then(Value::as_f64) else {
        return Err(validation_failure(
            path,
            &format!("{assertion_name} assertion field '{field}' must be a positive number"),
            json!("positive number"),
            value.get(field).cloned().unwrap_or(Value::Null),
        ));
    };
    if window <= 0.0 {
        return Err(validation_failure(
            path,
            &format!("{assertion_name} assertion field '{field}' must be a positive number"),
            json!("positive number"),
            value.get(field).cloned().unwrap_or(Value::Null),
        ));
    }
    Ok(())
}

/// Validate the optional state-expression observation shared by temporal
/// assertions (`within_limits`, `never`, `trend`). Absent `source` means the
/// assertion observes the resolved target node directly (legacy behavior) and
/// this returns `Ok(false)`. With `source` present the payload reuses the
/// `state_matches`/`wait_until` state-reading vocabulary: `provider` (registered
/// state provider) or `method` (`target` + read-only QA method), plus an
/// optional dot-separated `path` into the returned state.
pub(super) fn validate_temporal_state_observation(
    path: &Path,
    value: &Map<String, Value>,
    assertion_name: &str,
) -> Result<bool, ScenarioError> {
    if !value.contains_key("source") {
        return Ok(false);
    }
    match value.get("source").and_then(Value::as_str).map(str::trim) {
        Some("provider") => {
            if value.contains_key("provider") || !value.contains_key("provider_id") {
                require_non_empty_string_field(path, value, assertion_name, "provider")?;
            } else {
                require_non_empty_string_field(path, value, assertion_name, "provider_id")?;
            }
        }
        Some("method") | Some("qa_method") => {
            require_non_empty_string_field(path, value, assertion_name, "target")?;
            require_non_empty_string_field(path, value, assertion_name, "method")?;
        }
        _ => {
            return Err(validation_failure(
                path,
                &format!(
                    "{assertion_name} assertion field 'source' must be 'provider' or 'method'"
                ),
                json!(["provider", "method"]),
                value.get("source").cloned().unwrap_or(Value::Null),
            ));
        }
    }
    if let Some(path_value) = value.get("path") {
        let Some(path_value) = path_value.as_str() else {
            return Err(validation_failure(
                path,
                &format!(
                    "{assertion_name} assertion field 'path' must use non-empty dot-separated state paths"
                ),
                json!("non-empty dot-separated state path"),
                path_value.clone(),
            ));
        };
        validate_state_path(path, path_value, assertion_name, "path")?;
    }
    Ok(true)
}

pub(super) fn validate_temporal_sampling(
    determinism: &Map<String, Value>,
    path: &Path,
) -> Result<(), ScenarioError> {
    let Some(temporal_sampling) = determinism.get("temporal_sampling") else {
        return Ok(());
    };
    validate_temporal_sampling_value(path, "determinism.temporal_sampling", temporal_sampling)
}

pub(super) fn validate_assertion_temporal_sampling(
    path: &Path,
    value: &Map<String, Value>,
    assertion_name: &str,
) -> Result<(), ScenarioError> {
    let Some(temporal_sampling) = value.get("temporal_sampling") else {
        return Ok(());
    };
    validate_temporal_sampling_value(
        path,
        &format!("{assertion_name}.temporal_sampling"),
        temporal_sampling,
    )
}

fn validate_temporal_sampling_value(
    path: &Path,
    field_path: &str,
    temporal_sampling: &Value,
) -> Result<(), ScenarioError> {
    let Some(temporal_sampling) = temporal_sampling.as_object() else {
        return Err(validation_failure(
            path,
            &format!("{field_path} must be an object"),
            json!("object"),
            json!(python_type_name(temporal_sampling)),
        ));
    };
    let Some(mode) = temporal_sampling.get("mode").and_then(Value::as_str) else {
        return Err(validation_failure(
            path,
            &format!("{field_path}.mode must be a supported mode"),
            json!(["fixed_delta", "max_frame_duration"]),
            temporal_sampling
                .get("mode")
                .cloned()
                .unwrap_or(Value::Null),
        ));
    };
    let expected_field = match mode.trim() {
        "fixed_delta" => "fixed_delta_seconds",
        "max_frame_duration" => "max_frame_duration_seconds",
        _ => {
            return Err(validation_failure(
                path,
                &format!("{field_path}.mode must be a supported mode"),
                json!(["fixed_delta", "max_frame_duration"]),
                json!(mode),
            ));
        }
    };
    let duration_fields = ["fixed_delta_seconds", "max_frame_duration_seconds"]
        .into_iter()
        .filter(|field| temporal_sampling.contains_key(*field))
        .collect::<Vec<_>>();
    if duration_fields.len() != 1 || duration_fields[0] != expected_field {
        return Err(validation_failure(
            path,
            &format!("{field_path} must declare exactly one duration field matching mode"),
            json!({
                "fixed_delta": "fixed_delta_seconds",
                "max_frame_duration": "max_frame_duration_seconds",
            }),
            Value::Object(temporal_sampling.clone()),
        ));
    }
    let Some(duration) = temporal_sampling
        .get(expected_field)
        .and_then(Value::as_f64)
        .filter(|value| value.is_finite() && *value > 0.0)
    else {
        return Err(validation_failure(
            path,
            &format!("{field_path}.{expected_field} must be a positive number"),
            json!("positive number"),
            temporal_sampling
                .get(expected_field)
                .cloned()
                .unwrap_or(Value::Null),
        ));
    };
    if duration <= 0.0 {
        return Err(validation_failure(
            path,
            &format!("{field_path}.{expected_field} must be a positive number"),
            json!("positive number"),
            temporal_sampling
                .get(expected_field)
                .cloned()
                .unwrap_or(Value::Null),
        ));
    }
    Ok(())
}

pub(super) fn validate_observe_until_assertion(
    path: &Path,
    value: &Value,
) -> Result<(), ScenarioError> {
    let Some(value) = value.as_object() else {
        return Err(validation_failure(
            path,
            "observe_until assertion value must be an object",
            json!("object"),
            json!(python_type_name(value)),
        ));
    };
    require_non_empty_string_field(path, value, "observe_until", "target")?;
    validate_assertion_temporal_sampling(path, value, "observe_until")?;
    validate_temporal_window(path, value, "observe_until")?;
    validate_optional_positive_integer(path, value, "observe_until", "sample_every_frames")?;

    let has_property = value.contains_key("property")
        || value.contains_key("operator")
        || value.contains_key("expected");
    let has_visible = value.contains_key("visible");
    let has_state_matches = value.contains_key("source")
        || value.contains_key("method")
        || value.contains_key("matches");
    let family_count = [has_property, has_visible, has_state_matches]
        .into_iter()
        .filter(|present| *present)
        .count();
    if family_count != 1 {
        return Err(validation_failure(
            path,
            "observe_until assertion must declare exactly one condition family",
            json!([
                "property/operator/expected",
                "visible",
                "source/method/matches"
            ]),
            Value::Object(value.clone()),
        ));
    }

    if has_property {
        require_non_empty_string_field(path, value, "observe_until", "property")?;
        require_non_empty_string_field(path, value, "observe_until", "operator")?;
        if !value.contains_key("expected") {
            return Err(validation_failure(
                path,
                "observe_until assertion field 'expected' is required for property conditions",
                json!("expected value"),
                Value::Null,
            ));
        }
        validate_within_operator_tolerance(path, value, "observe_until assertion")?;
    } else if has_visible {
        if !value.get("visible").is_some_and(Value::is_boolean) {
            return Err(validation_failure(
                path,
                "observe_until assertion field 'visible' must be a boolean",
                json!("boolean"),
                value.get("visible").cloned().unwrap_or(Value::Null),
            ));
        }
    } else {
        require_non_empty_string_field(path, value, "observe_until", "source")?;
        require_non_empty_string_field(path, value, "observe_until", "method")?;
        let Some(matches) = value.get("matches").and_then(Value::as_object) else {
            return Err(validation_failure(
                path,
                "observe_until assertion field 'matches' must be an object",
                json!("object"),
                value.get("matches").cloned().unwrap_or(Value::Null),
            ));
        };
        if matches.is_empty() {
            return Err(validation_failure(
                path,
                "observe_until assertion field 'matches' must include at least one state path",
                json!("non-empty object"),
                Value::Object(matches.clone()),
            ));
        }
        for path_value in matches.keys() {
            validate_state_path(path, path_value, "observe_until", "matches")?;
        }
    }

    Ok(())
}

fn validate_state_path(
    path: &Path,
    path_value: &str,
    assertion_name: &str,
    field_name: &str,
) -> Result<(), ScenarioError> {
    let trimmed = path_value.trim();
    if trimmed.is_empty()
        || trimmed.split('.').any(|segment| segment.trim().is_empty())
        || trimmed != path_value
    {
        return Err(validation_failure(
            path,
            &format!(
                "{assertion_name} assertion field '{field_name}' must use non-empty dot-separated state paths"
            ),
            json!("non-empty dot-separated state path"),
            json!(path_value),
        ));
    }
    Ok(())
}

/// The `within` operator asserts `|actual - expected| <= tolerance` with an
/// absolute tolerance (distinct from `approx_equals`' `epsilon`). Wherever a
/// state check declares `operator: within`, `tolerance` must be a positive
/// number.
fn validate_within_operator_tolerance(
    path: &Path,
    value: &Map<String, Value>,
    context_name: &str,
) -> Result<(), ScenarioError> {
    let operator = value
        .get("operator")
        .or_else(|| value.get("op"))
        .and_then(Value::as_str)
        .map(str::trim)
        .unwrap_or_default();
    if operator != "within" {
        return Ok(());
    }
    let raw = value.get("tolerance").cloned().unwrap_or(Value::Null);
    let valid = raw
        .as_f64()
        .map(|number| number.is_finite() && number > 0.0)
        .unwrap_or(false);
    if !valid {
        return Err(validation_failure(
            path,
            &format!(
                "{context_name} field 'tolerance' must be a positive number for operator 'within'"
            ),
            json!("positive number"),
            raw,
        ));
    }
    Ok(())
}

/// Minimal structural validation for `state_matches` assertions. Historically
/// this assertion had no Rust-side validation, so a `matches` mis-shape sailed
/// through until it silently failed at runtime. We keep this lenient (consumers
/// use a wide field vocabulary and empty `path` with `where`/`value_path`) but
/// reject clearly malformed `source`/`matches` shapes early.
pub(super) fn validate_state_matches_assertion(
    path: &Path,
    value: &Value,
) -> Result<(), ScenarioError> {
    let Some(value) = value.as_object() else {
        return Err(validation_failure(
            path,
            "state_matches assertion value must be an object",
            json!("object"),
            json!(python_type_name(value)),
        ));
    };
    if let Some(source) = value.get("source").and_then(Value::as_str) {
        let trimmed = source.trim();
        if !trimmed.is_empty() && !STATE_MATCH_SOURCES.contains(&trimmed) {
            return Err(validation_failure(
                path,
                "state_matches assertion field 'source' must be a supported source",
                json!(STATE_MATCH_SOURCES),
                json!(source),
            ));
        }
    }
    validate_within_operator_tolerance(path, value, "state_matches assertion")?;
    match value.get("matches") {
        None | Some(Value::Object(_)) => {}
        Some(Value::Array(entries)) => {
            for entry in entries {
                let Some(entry) = entry.as_object() else {
                    return Err(validation_failure(
                        path,
                        "state_matches assertion 'matches' entries must be objects",
                        json!("object"),
                        entry.clone(),
                    ));
                };
                validate_within_operator_tolerance(path, entry, "state_matches assertion check")?;
            }
        }
        Some(other) => {
            return Err(validation_failure(
                path,
                "state_matches assertion field 'matches' must be a list or object",
                json!("list or object"),
                json!(python_type_name(other)),
            ));
        }
    }
    Ok(())
}

/// Validate a `record_state` step: a read-only observation that captures a
/// state VALUE as run evidence under a scenario-unique `name`. It reuses the
/// exact `{source, provider | target+method, path}` state-source vocabulary that
/// `wait_until`/`state_matches` read from (same `require_non_empty_string_field`
/// and `validate_state_path` doc-side validators), but records the value instead
/// of asserting on it — so it carries no operator/expected fields. `name`
/// follows the same safe-artifact-segment charset as `snapshot` names because it
/// keys the `state_records` evidence map, and must be unique across the
/// scenario's record_state steps so no earlier record is silently overwritten.
pub(super) fn validate_record_state_step(
    path: &Path,
    value: &Value,
    seen_names: &mut Vec<String>,
) -> Result<(), ScenarioError> {
    const RECORD_STATE_FIELDS: &[&str] = &[
        "name",
        "source",
        "provider",
        "provider_id",
        "method",
        "target",
        "path",
    ];

    let Some(value) = value.as_object() else {
        return Err(validation_failure(
            path,
            "record_state step value must be an object",
            json!("object"),
            json!(python_type_name(value)),
        ));
    };

    for key in value.keys() {
        if !RECORD_STATE_FIELDS.contains(&key.as_str()) {
            return Err(validation_failure(
                path,
                &format!("record_state step uses unsupported field '{key}'"),
                json!(RECORD_STATE_FIELDS),
                json!({ "unknown_key": key }),
            ));
        }
    }

    let Some(name) = value.get("name").and_then(Value::as_str) else {
        return Err(validation_failure(
            path,
            "record_state step field 'name' must be a non-empty string",
            json!("non-empty string"),
            value.get("name").cloned().unwrap_or(Value::Null),
        ));
    };
    let trimmed_name = name.trim();
    if !is_safe_artifact_segment(trimmed_name) {
        return Err(validation_failure(
            path,
            "record_state step field 'name' must be a safe artifact path segment",
            json!("letters, numbers, dot, underscore, or hyphen"),
            json!(name),
        ));
    }
    if seen_names.iter().any(|seen| seen == trimmed_name) {
        return Err(validation_failure(
            path,
            &format!(
                "record_state step name '{trimmed_name}' is already used by an earlier record_state step"
            ),
            json!("unique record_state name"),
            json!(trimmed_name),
        ));
    }
    seen_names.push(trimmed_name.to_string());

    match value.get("source").and_then(Value::as_str).map(str::trim) {
        Some("provider") => {
            require_non_empty_string_field(path, value, "record_state", "provider")?;
        }
        Some("method") | Some("qa_method") => {
            require_non_empty_string_field(path, value, "record_state", "target")?;
            require_non_empty_string_field(path, value, "record_state", "method")?;
        }
        _ => {
            return Err(validation_failure(
                path,
                "record_state step field 'source' must be 'provider' or 'method'",
                json!(["provider", "method"]),
                value.get("source").cloned().unwrap_or(Value::Null),
            ));
        }
    }

    if let Some(path_value) = value.get("path") {
        let path_str = path_value.as_str().unwrap_or_default();
        validate_state_path(path, path_str, "record_state", "path")?;
    }

    Ok(())
}

/// Validate a `wait_until` step: a bounded state predicate that reuses the
/// `state_matches` source/operator vocabulary.
pub(super) fn validate_wait_until_step(path: &Path, value: &Value) -> Result<(), ScenarioError> {
    const WAIT_UNTIL_FIELDS: &[&str] = &[
        "source",
        "provider",
        "provider_id",
        "method",
        "target",
        "path",
        "operator",
        "expected",
        "epsilon",
        "tolerance",
        "where",
        "value_path",
        "timeout_frames",
        "poll_every_frames",
    ];

    let Some(value) = value.as_object() else {
        return Err(validation_failure(
            path,
            "wait_until step value must be an object",
            json!("object"),
            json!(python_type_name(value)),
        ));
    };

    for key in value.keys() {
        if !WAIT_UNTIL_FIELDS.contains(&key.as_str()) {
            return Err(validation_failure(
                path,
                &format!("wait_until step uses unsupported field '{key}'"),
                json!(WAIT_UNTIL_FIELDS),
                json!({ "unknown_key": key }),
            ));
        }
    }

    match value.get("source").and_then(Value::as_str).map(str::trim) {
        Some("provider") => {
            require_non_empty_string_field(path, value, "wait_until", "provider")?;
        }
        Some("method") | Some("qa_method") => {
            require_non_empty_string_field(path, value, "wait_until", "target")?;
            require_non_empty_string_field(path, value, "wait_until", "method")?;
        }
        _ => {
            return Err(validation_failure(
                path,
                "wait_until step field 'source' must be 'provider' or 'method'",
                json!(["provider", "method"]),
                value.get("source").cloned().unwrap_or(Value::Null),
            ));
        }
    }

    let path_value = value
        .get("path")
        .and_then(Value::as_str)
        .unwrap_or_default();
    validate_state_path(path, path_value, "wait_until", "path")?;

    let operator = value
        .get("operator")
        .and_then(Value::as_str)
        .map(str::trim)
        .unwrap_or_default();
    if !STATE_MATCH_OPERATORS.contains(&operator) {
        return Err(validation_failure(
            path,
            "wait_until step field 'operator' must be a supported comparison",
            json!(STATE_MATCH_OPERATORS),
            value.get("operator").cloned().unwrap_or(Value::Null),
        ));
    }
    if OPERATORS_REQUIRING_EXPECTED.contains(&operator) && !value.contains_key("expected") {
        return Err(validation_failure(
            path,
            &format!("wait_until step field 'expected' is required for operator '{operator}'"),
            json!("expected value"),
            Value::Null,
        ));
    }
    validate_within_operator_tolerance(path, value, "wait_until step")?;
    if let Some(epsilon) = value.get("epsilon")
        && epsilon
            .as_f64()
            .map(|number| !number.is_finite())
            .unwrap_or(true)
    {
        return Err(validation_failure(
            path,
            "wait_until step field 'epsilon' must be a finite number",
            json!("finite number"),
            epsilon.clone(),
        ));
    }

    let Some(timeout_frames) = value.get("timeout_frames") else {
        return Err(validation_failure(
            path,
            "wait_until step field 'timeout_frames' is required",
            json!("positive integer"),
            Value::Null,
        ));
    };
    if timeout_frames.as_i64().is_none()
        || !timeout_frames.is_i64()
        || timeout_frames.as_i64().unwrap() <= 0
    {
        return Err(validation_failure(
            path,
            "wait_until step field 'timeout_frames' must be a positive integer",
            json!("positive integer"),
            timeout_frames.clone(),
        ));
    }
    validate_optional_positive_integer(path, value, "wait_until", "poll_every_frames")?;
    Ok(())
}

/// Validate a `drive_policy` step: a bounded closed-loop input step that calls
/// a registered policy provider each tick and emits the joypad inputs it
/// returns. The optional `until` early-stop predicate reuses the
/// `wait_until`/`state_matches` source/operator vocabulary; its frame budget
/// comes from the step's own `max_frames`, so `timeout_frames` is not allowed.
pub(super) fn validate_drive_policy_step(path: &Path, value: &Value) -> Result<(), ScenarioError> {
    const DRIVE_POLICY_FIELDS: &[&str] = &[
        "policy",
        "params",
        "devices",
        "max_frames",
        "tick_every_frames",
        "until",
    ];

    let Some(value) = value.as_object() else {
        return Err(validation_failure(
            path,
            "drive_policy step value must be an object",
            json!("object"),
            json!(python_type_name(value)),
        ));
    };

    for key in value.keys() {
        if !DRIVE_POLICY_FIELDS.contains(&key.as_str()) {
            return Err(validation_failure(
                path,
                &format!("drive_policy step uses unsupported field '{key}'"),
                json!(DRIVE_POLICY_FIELDS),
                json!({ "unknown_key": key }),
            ));
        }
    }

    if !value
        .get("policy")
        .map(is_non_empty_string)
        .unwrap_or(false)
    {
        return Err(validation_failure(
            path,
            "drive_policy step field 'policy' must be a non-empty string",
            json!("non-empty string"),
            value.get("policy").cloned().unwrap_or(Value::Null),
        ));
    }

    if let Some(params) = value.get("params") {
        if !params.is_object() {
            return Err(validation_failure(
                path,
                "drive_policy step field 'params' must be an object when present",
                json!("object"),
                json!(python_type_name(params)),
            ));
        }
    }

    let Some(devices) = value.get("devices").and_then(Value::as_array) else {
        return Err(validation_failure(
            path,
            "drive_policy step field 'devices' must be a non-empty list of device integers",
            json!("non-empty list of non-negative integers"),
            value.get("devices").cloned().unwrap_or(Value::Null),
        ));
    };
    if devices.is_empty() {
        return Err(validation_failure(
            path,
            "drive_policy step field 'devices' must be a non-empty list of device integers",
            json!("non-empty list of non-negative integers"),
            json!(devices),
        ));
    }
    for device in devices {
        if device.as_i64().is_none() || !device.is_i64() || device.as_i64().unwrap() < 0 {
            return Err(validation_failure(
                path,
                "drive_policy step 'devices' entries must be non-negative integers",
                json!("non-negative integer"),
                device.clone(),
            ));
        }
    }

    let Some(max_frames) = value.get("max_frames") else {
        return Err(validation_failure(
            path,
            "drive_policy step field 'max_frames' is required",
            json!("positive integer"),
            Value::Null,
        ));
    };
    if max_frames.as_i64().is_none() || !max_frames.is_i64() || max_frames.as_i64().unwrap() <= 0 {
        return Err(validation_failure(
            path,
            "drive_policy step field 'max_frames' must be a positive integer",
            json!("positive integer"),
            max_frames.clone(),
        ));
    }
    validate_optional_positive_integer(path, value, "drive_policy", "tick_every_frames")?;

    if let Some(until) = value.get("until") {
        validate_drive_policy_until(path, until)?;
    }
    Ok(())
}

/// Validate the `until` predicate of a `drive_policy` step: the state-reading
/// fields of `wait_until` without its own frame budget.
fn validate_drive_policy_until(path: &Path, value: &Value) -> Result<(), ScenarioError> {
    const DRIVE_POLICY_UNTIL_FIELDS: &[&str] = &[
        "source",
        "provider",
        "provider_id",
        "method",
        "target",
        "path",
        "operator",
        "expected",
        "epsilon",
        "tolerance",
        "where",
        "value_path",
    ];

    let Some(value) = value.as_object() else {
        return Err(validation_failure(
            path,
            "drive_policy step field 'until' must be an object",
            json!("object"),
            json!(python_type_name(value)),
        ));
    };

    for key in value.keys() {
        if !DRIVE_POLICY_UNTIL_FIELDS.contains(&key.as_str()) {
            return Err(validation_failure(
                path,
                &format!("drive_policy step 'until' uses unsupported field '{key}'"),
                json!(DRIVE_POLICY_UNTIL_FIELDS),
                json!({ "unknown_key": key }),
            ));
        }
    }

    match value.get("source").and_then(Value::as_str).map(str::trim) {
        Some("provider") => {
            require_non_empty_string_field(path, value, "drive_policy step 'until'", "provider")?;
        }
        Some("method") | Some("qa_method") => {
            require_non_empty_string_field(path, value, "drive_policy step 'until'", "target")?;
            require_non_empty_string_field(path, value, "drive_policy step 'until'", "method")?;
        }
        _ => {
            return Err(validation_failure(
                path,
                "drive_policy step 'until' field 'source' must be 'provider' or 'method'",
                json!(["provider", "method"]),
                value.get("source").cloned().unwrap_or(Value::Null),
            ));
        }
    }

    let path_value = value
        .get("path")
        .and_then(Value::as_str)
        .unwrap_or_default();
    validate_state_path(path, path_value, "drive_policy step 'until'", "path")?;

    let operator = value
        .get("operator")
        .and_then(Value::as_str)
        .map(str::trim)
        .unwrap_or_default();
    if !STATE_MATCH_OPERATORS.contains(&operator) {
        return Err(validation_failure(
            path,
            "drive_policy step 'until' field 'operator' must be a supported comparison",
            json!(STATE_MATCH_OPERATORS),
            value.get("operator").cloned().unwrap_or(Value::Null),
        ));
    }
    if OPERATORS_REQUIRING_EXPECTED.contains(&operator) && !value.contains_key("expected") {
        return Err(validation_failure(
            path,
            &format!(
                "drive_policy step 'until' field 'expected' is required for operator '{operator}'"
            ),
            json!("expected value"),
            Value::Null,
        ));
    }
    validate_within_operator_tolerance(path, value, "drive_policy step 'until'")?;
    if let Some(epsilon) = value.get("epsilon")
        && epsilon
            .as_f64()
            .map(|number| !number.is_finite())
            .unwrap_or(true)
    {
        return Err(validation_failure(
            path,
            "drive_policy step 'until' field 'epsilon' must be a finite number",
            json!("finite number"),
            epsilon.clone(),
        ));
    }
    Ok(())
}

pub(super) fn validate_never_observation(
    path: &Path,
    value: &Map<String, Value>,
) -> Result<(), ScenarioError> {
    let keys = ["property", "text", "token"];
    let present: Vec<&str> = keys
        .iter()
        .copied()
        .filter(|key| value.contains_key(*key))
        .collect();
    if present.len() != 1 {
        return Err(validation_failure(
            path,
            "never assertion must declare exactly one observation source: 'property', 'text', or 'token'",
            json!(["property", "text", "token"]),
            Value::Object(value.clone()),
        ));
    }
    require_non_empty_string_field(path, value, "never", present[0])
}

pub(super) fn validate_never_forbidden(
    path: &Path,
    value: &Map<String, Value>,
) -> Result<(), ScenarioError> {
    let Some(forbidden) = value.get("forbidden") else {
        return Err(validation_failure(
            path,
            "never assertion field 'forbidden' must be a string or list of non-empty strings",
            json!("string or list of non-empty strings"),
            Value::Null,
        ));
    };
    match forbidden {
        Value::String(raw) => {
            if raw.trim().is_empty() {
                return Err(validation_failure(
                    path,
                    "never assertion field 'forbidden' must be a string or list of non-empty strings",
                    json!("string or list of non-empty strings"),
                    forbidden.clone(),
                ));
            }
        }
        Value::Array(values) => {
            if values.is_empty()
                || values.iter().any(|value| {
                    value
                        .as_str()
                        .map(|raw| raw.trim().is_empty())
                        .unwrap_or(true)
                })
            {
                return Err(validation_failure(
                    path,
                    "never assertion field 'forbidden' must be a string or list of non-empty strings",
                    json!("string or list of non-empty strings"),
                    forbidden.clone(),
                ));
            }
        }
        _ => {
            return Err(validation_failure(
                path,
                "never assertion field 'forbidden' must be a string or list of non-empty strings",
                json!("string or list of non-empty strings"),
                forbidden.clone(),
            ));
        }
    }
    Ok(())
}
