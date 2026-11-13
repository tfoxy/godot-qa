mod assertions;
mod state;
mod steps;

use self::assertions::validate_supported_assertion_payloads;
use self::state::validate_temporal_sampling;
use self::steps::validate_supported_step_payloads;
use super::{
    ScenarioError, is_non_empty_string, is_safe_artifact_segment, python_type_name,
    validation_failure,
};
use crate::frame_pacing::{
    time_control_fixed_fps_conflict_failure, validate_fixed_fps_for_scenario,
};
use crate::time_control::validate_time_control_for_scenario;
use serde_json::{Map, Value, json};
use std::path::Path;

const REQUIRED_FIELDS: [&str; 3] = ["name", "scene", "steps"];
/// Operation-local metadata that may accompany the single step/assertion key.
const STEP_ASSERTION_SIBLING_KEYS: &[&str] = &["timeout", "timeout_seconds"];

pub fn validate_scenario_document(data: &Value, path: &Path) -> Result<(), ScenarioError> {
    let Some(document) = data.as_object() else {
        return Err(validation_failure(
            path,
            "scenario document root must be an object",
            json!("object"),
            json!(python_type_name(data)),
        ));
    };

    let missing_fields: Vec<Value> = REQUIRED_FIELDS
        .iter()
        .filter(|field| !document.contains_key(**field))
        .map(|field| json!(field))
        .collect();
    if !missing_fields.is_empty() {
        return Err(validation_failure(
            path,
            "scenario document is missing required fields",
            json!(REQUIRED_FIELDS),
            json!({ "missing_fields": missing_fields }),
        ));
    }

    let name = require_non_empty_string(
        document,
        "name",
        "scenario field 'name' must be a non-empty string",
        path,
    )?;
    let _scene = require_non_empty_string(
        document,
        "scene",
        "scenario field 'scene' must be a non-empty string",
        path,
    )?;
    let steps = document
        .get("steps")
        .and_then(Value::as_array)
        .ok_or_else(|| {
            validation_failure(
                path,
                "scenario field 'steps' must be a list",
                json!("list"),
                json!(python_type_name(
                    document.get("steps").unwrap_or(&Value::Null)
                )),
            )
        })?;

    if !is_safe_artifact_segment(name.trim()) {
        return Err(validation_failure(
            path,
            "scenario field 'name' must be a safe artifact path segment",
            json!("letters, numbers, dot, underscore, or hyphen"),
            document.get("name").cloned().unwrap_or(Value::Null),
        ));
    }

    for step in steps {
        let Some(step_object) = step.as_object() else {
            continue;
        };
        if let Some(snapshot_name) = step_object.get("snapshot") {
            let Some(snapshot_name) = snapshot_name.as_str() else {
                return Err(validation_failure(
                    path,
                    "snapshot step name must be a safe artifact path segment",
                    json!("letters, numbers, dot, underscore, or hyphen"),
                    snapshot_name.clone(),
                ));
            };
            if !is_safe_artifact_segment(snapshot_name.trim()) {
                return Err(validation_failure(
                    path,
                    "snapshot step name must be a safe artifact path segment",
                    json!("letters, numbers, dot, underscore, or hyphen"),
                    json!(snapshot_name),
                ));
            }
        }
    }

    for field_name in ["viewport", "determinism", "snapshots"] {
        if let Some(value) = document.get(field_name) {
            if !value.is_object() {
                return Err(validation_failure(
                    path,
                    &format!("scenario field '{field_name}' must be an object"),
                    json!("object"),
                    json!(python_type_name(value)),
                ));
            }
        }
    }
    if let Some(time_control) = document.get("time_control") {
        validate_time_control_for_scenario(time_control, path)
            .map_err(|failure| ScenarioError { failure })?;
    }
    validate_scenario_timeout(document, path)?;
    if let Some(determinism) = document.get("determinism").and_then(Value::as_object) {
        if document.get("time_control").is_some() && determinism.get("fixed_fps").is_some() {
            return Err(ScenarioError {
                failure: time_control_fixed_fps_conflict_failure(
                    "scenario_validation_failed",
                    Some(path),
                    None,
                ),
            });
        }
        if let Some(fixed_fps) = determinism.get("fixed_fps") {
            validate_fixed_fps_for_scenario(fixed_fps, path)
                .map_err(|failure| ScenarioError { failure })?;
        }
        validate_temporal_sampling(determinism, path)?;
    }
    if let Some(snapshots) = document.get("snapshots").and_then(Value::as_object) {
        if let Some(value) = snapshots.get("fail_on_low_information") {
            if !value.is_boolean() {
                return Err(validation_failure(
                    path,
                    "snapshots.fail_on_low_information must be a boolean",
                    json!("boolean"),
                    json!(python_type_name(value)),
                ));
            }
        }
        validate_snapshot_filters(snapshots, path)?;
    }

    if let Some(value) = document.get("assert") {
        if !value.is_array() {
            return Err(validation_failure(
                path,
                "scenario field 'assert' must be a list",
                json!("list"),
                json!(python_type_name(value)),
            ));
        }
    }

    if let Some(viewports) = document.get("viewports") {
        validate_viewports(viewports, path)?;
    }
    if let Some(policy) = document.get("process_outcome_policy") {
        validate_process_outcome_policy(policy, path)?;
    }
    if let Some(execution) = document.get("execution") {
        validate_execution(execution, path)?;
    }
    validate_argument_contract(document, path)?;

    validate_supported_step_payloads(steps, path)?;
    if let Some(assertions) = document.get("assert").and_then(Value::as_array) {
        validate_supported_assertion_payloads(assertions, path)?;
    }
    validate_frame_time_frame_pacing_conflict(document, steps, path)?;
    validate_screenshot_match_viewport_matrix_conflict(document, path)?;

    Ok(())
}

/// `screenshot_match` compares a rendered snapshot against a single host-viewport
/// baseline, so it has no meaning across a multi-entry `viewports` matrix (each
/// cell renders at a different resolution with no per-cell baseline contract).
/// Reject the combination statically — the compile-time mirror of the runtime
/// `screenshot_match_viewports_unsupported` guard — so authors learn at lint time
/// instead of at run time. A single `viewports` entry (or none) is fine: it just
/// sets the window resolution and `screenshot_match` runs the normal single path.
fn validate_screenshot_match_viewport_matrix_conflict(
    document: &Map<String, Value>,
    path: &Path,
) -> Result<(), ScenarioError> {
    let viewports = document
        .get("viewports")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or(&[]);
    if viewports.len() <= 1 {
        return Ok(());
    }
    let has_screenshot_match = document
        .get("assert")
        .and_then(Value::as_array)
        .is_some_and(|assertions| {
            assertions
                .iter()
                .any(|assertion| assertion.get("screenshot_match").is_some())
        });
    if !has_screenshot_match {
        return Ok(());
    }
    Err(validation_failure(
        path,
        "screenshot_match does not support multi-viewport scenarios; use a single viewport or drop the 'viewports' matrix",
        json!("single viewport scenario without a multi-entry 'viewports' matrix"),
        json!(viewports),
    ))
}

/// `frame_time` asserts wall-clock frame durations, which are meaningless when
/// `determinism.fixed_fps` forces frame pacing: the run simulates a fixed frame
/// delta while processing frames as fast as it can. Reject the combination so
/// authors either drop `fixed_fps` or keep the perf gate in a separate
/// realtime-pacing perf scenario.
fn validate_frame_time_frame_pacing_conflict(
    document: &Map<String, Value>,
    steps: &[Value],
    path: &Path,
) -> Result<(), ScenarioError> {
    let Some(fixed_fps) = document
        .get("determinism")
        .and_then(Value::as_object)
        .and_then(|determinism| determinism.get("fixed_fps"))
    else {
        return Ok(());
    };
    let top_level = document
        .get("assert")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default();
    let step_local = steps
        .iter()
        .filter_map(|step| step.get("assert").and_then(Value::as_array))
        .flatten();
    let declares_frame_time = top_level
        .iter()
        .chain(step_local)
        .any(|assertion| assertion.get("frame_time").is_some());
    if declares_frame_time {
        return Err(validation_failure(
            path,
            "frame_time assertions cannot be combined with determinism.fixed_fps: forced frame pacing makes wall-clock frame times meaningless; drop fixed_fps or move the frame_time gate into a separate perf scenario",
            json!("no determinism.fixed_fps alongside frame_time assertions"),
            json!({ "determinism.fixed_fps": fixed_fps }),
        ));
    }
    Ok(())
}

fn validate_scenario_timeout(
    document: &Map<String, Value>,
    path: &Path,
) -> Result<(), ScenarioError> {
    let scenario = Value::Object(document.clone());
    let declared = crate::run::declared_scenario_timeout_fields(&scenario);
    if declared.len() > 1 {
        return Err(validation_failure(
            path,
            "scenario timeout must use exactly one of timeout_seconds, timeout, run_timeout_seconds, or run_timeout",
            json!("one timeout field"),
            json!(declared),
        ));
    }
    for field in ["timeout_seconds", "run_timeout_seconds"] {
        if let Some(value) = document.get(field) {
            if crate::run::parse_timeout_seconds_value(value).is_none() {
                return Err(invalid_scenario_timeout(path, value.clone()));
            }
        }
    }
    for field in ["timeout", "run_timeout"] {
        if let Some(value) = document.get(field) {
            if crate::run::parse_timeout_duration_string(value).is_none() {
                return Err(invalid_scenario_timeout(path, value.clone()));
            }
        }
    }
    Ok(())
}

fn invalid_scenario_timeout(path: &Path, actual: Value) -> ScenarioError {
    validation_failure(
        path,
        "scenario timeout must be a positive finite duration",
        json!("positive number of seconds or duration string with ms, s, or m suffix"),
        actual,
    )
}

// The optional `execution` block controls per-scenario process isolation.
// `execution.isolation` is "warm" (default, warm-eligible) or "cold" (always run
// in a fresh isolated process, opting out of the warm in-process runner).
fn validate_execution(value: &Value, path: &Path) -> Result<(), ScenarioError> {
    let Some(execution) = value.as_object() else {
        return Err(validation_failure(
            path,
            "scenario field 'execution' must be an object",
            json!("object"),
            json!(python_type_name(value)),
        ));
    };
    if let Some(isolation) = execution.get("isolation") {
        let valid = isolation
            .as_str()
            .map(|isolation| matches!(isolation, "warm" | "cold"))
            .unwrap_or(false);
        if !valid {
            return Err(validation_failure(
                path,
                "scenario field 'execution.isolation' must be \"warm\" or \"cold\"",
                json!("\"warm\" or \"cold\""),
                isolation.clone(),
            ));
        }
    }
    Ok(())
}

fn validate_process_outcome_policy(value: &Value, path: &Path) -> Result<(), ScenarioError> {
    let Some(policy) = value.as_object() else {
        return Err(validation_failure(
            path,
            "scenario field 'process_outcome_policy' must be an object",
            json!("object"),
            json!(python_type_name(value)),
        ));
    };
    let Some(exit_codes) = policy.get("known_wrapper_exit_codes") else {
        return Err(validation_failure(
            path,
            "process_outcome_policy.known_wrapper_exit_codes must be a non-empty list of positive integers",
            json!("non-empty list of positive integers"),
            Value::Null,
        ));
    };
    let Some(exit_codes) = exit_codes.as_array() else {
        return Err(validation_failure(
            path,
            "process_outcome_policy.known_wrapper_exit_codes must be a non-empty list of positive integers",
            json!("non-empty list of positive integers"),
            json!(python_type_name(exit_codes)),
        ));
    };
    if exit_codes.is_empty()
        || exit_codes
            .iter()
            .any(|value| !value.as_i64().is_some_and(|code| code > 0))
    {
        return Err(validation_failure(
            path,
            "process_outcome_policy.known_wrapper_exit_codes must be a non-empty list of positive integers",
            json!("non-empty list of positive integers"),
            Value::Array(exit_codes.clone()),
        ));
    }
    Ok(())
}

fn validate_argument_contract(
    document: &Map<String, Value>,
    path: &Path,
) -> Result<(), ScenarioError> {
    if let Some(value) = document.get("launch_args") {
        return Err(validation_failure(
            path,
            "scenario field 'launch_args' is not supported; use 'godot_args' for Godot engine flags before '--' or 'project_args' for reserved project/game flags after '--'",
            json!("godot_args or project_args"),
            value.clone(),
        ));
    }
    for field_name in ["godot_args", "project_args"] {
        let Some(value) = document.get(field_name) else {
            continue;
        };
        let Some(args) = value.as_array() else {
            return Err(validation_failure(
                path,
                &format!("scenario field '{field_name}' must be a list of non-empty strings"),
                json!("list of non-empty strings"),
                json!(python_type_name(value)),
            ));
        };
        for (index, arg) in args.iter().enumerate() {
            let Some(arg_value) = arg.as_str() else {
                return Err(validation_failure(
                    path,
                    &format!("scenario field '{field_name}' must contain only strings"),
                    json!("string"),
                    json!({"index": index, "actual": python_type_name(arg)}),
                ));
            };
            if arg_value.is_empty() {
                return Err(validation_failure(
                    path,
                    &format!("scenario field '{field_name}' must contain only non-empty strings"),
                    json!("non-empty string"),
                    json!({"index": index, "actual": arg_value}),
                ));
            }
        }
    }
    Ok(())
}

fn require_non_empty_string<'a>(
    document: &'a Map<String, Value>,
    field: &str,
    message: &str,
    path: &Path,
) -> Result<&'a str, ScenarioError> {
    document
        .get(field)
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| {
            validation_failure(
                path,
                message,
                json!("non-empty string"),
                document.get(field).cloned().unwrap_or(Value::Null),
            )
        })
}

fn validate_viewports(viewports: &Value, path: &Path) -> Result<(), ScenarioError> {
    let Some(viewports) = viewports.as_array() else {
        return Err(validation_failure(
            path,
            "scenario field 'viewports' must be a list",
            json!("list"),
            json!(python_type_name(viewports)),
        ));
    };

    for viewport in viewports {
        let Some(parts) = viewport.as_array() else {
            return Err(validation_failure(
                path,
                "viewports entries must be [width, height] arrays",
                json!("two-integer array"),
                viewport.clone(),
            ));
        };
        if parts.len() != 2 {
            return Err(validation_failure(
                path,
                "viewports entries must be [width, height] arrays",
                json!("two-integer array"),
                viewport.clone(),
            ));
        }
        let (Some(width), Some(height)) = (parts[0].as_i64(), parts[1].as_i64()) else {
            return Err(validation_failure(
                path,
                "viewports width and height must be positive integers",
                json!("positive integers"),
                viewport.clone(),
            ));
        };
        if width <= 0 || height <= 0 {
            return Err(validation_failure(
                path,
                "viewports width and height must be positive integers",
                json!("positive integers"),
                viewport.clone(),
            ));
        }
    }
    Ok(())
}

fn reject_unknown_keys(
    object: &Map<String, Value>,
    known: &[&str],
    kind_label: &str,
    index: usize,
    path: &Path,
) -> Result<(), ScenarioError> {
    for key in object.keys() {
        if known.contains(&key.as_str()) || STEP_ASSERTION_SIBLING_KEYS.contains(&key.as_str()) {
            continue;
        }
        return Err(validation_failure(
            path,
            &format!("{kind_label} {index} uses unsupported key '{key}'"),
            json!(known),
            json!({ "unknown_key": key, "index": index }),
        ));
    }
    Ok(())
}

fn validate_to_point_object(
    path: &Path,
    step_name: &str,
    to_point: &Value,
) -> Result<(), ScenarioError> {
    let Some(to_point) = to_point.as_object() else {
        return Err(validation_failure(
            path,
            &format!("{step_name} step field 'to_point' must be an object"),
            json!("object with provider, method, and id"),
            json!(python_type_name(to_point)),
        ));
    };
    for field_name in ["provider", "method", "id"] {
        if !to_point
            .get(field_name)
            .map(is_non_empty_string)
            .unwrap_or(false)
        {
            return Err(validation_failure(
                path,
                &format!(
                    "{step_name} step to_point field '{field_name}' must be a non-empty string"
                ),
                json!("non-empty string"),
                to_point.get(field_name).cloned().unwrap_or(Value::Null),
            ));
        }
    }
    if let Some(method) = to_point.get("method").and_then(Value::as_str) {
        if !method.trim().starts_with("qa_") {
            return Err(validation_failure(
                path,
                &format!("{step_name} step to_point field 'method' must name a qa_ probe method"),
                json!("method name beginning with qa_"),
                json!(method),
            ));
        }
    }
    Ok(())
}

fn validate_optional_bool_field(
    path: &Path,
    step_name: &str,
    step: &Map<String, Value>,
    field_name: &str,
) -> Result<(), ScenarioError> {
    if let Some(value) = step.get(field_name) {
        if !value.is_boolean() {
            return Err(validation_failure(
                path,
                &format!("{step_name} step field '{field_name}' must be a boolean when present"),
                json!("boolean"),
                value.clone(),
            ));
        }
    }
    Ok(())
}

fn validate_snapshot_filters(
    snapshots: &Map<String, Value>,
    path: &Path,
) -> Result<(), ScenarioError> {
    if let Some(value) = snapshots.get("target") {
        match value.as_str().map(str::trim) {
            Some(target) if !target.is_empty() => {}
            _ => {
                return Err(validation_failure(
                    path,
                    "snapshots.target must be a non-empty string",
                    json!("non-empty target locator"),
                    value.clone(),
                ));
            }
        }
    }
    if let Some(value) = snapshots.get("max_depth") {
        match value.as_i64() {
            Some(max_depth) if max_depth >= 0 => {}
            _ => {
                return Err(validation_failure(
                    path,
                    "snapshots.max_depth must be a non-negative integer",
                    json!("non-negative integer max_depth"),
                    value.clone(),
                ));
            }
        }
    }
    for field in ["qa_only", "include_metadata"] {
        if let Some(value) = snapshots.get(field) {
            if !value.is_boolean() {
                return Err(validation_failure(
                    path,
                    &format!("snapshots.{field} must be a boolean"),
                    json!("boolean"),
                    json!(python_type_name(value)),
                ));
            }
        }
    }
    if let Some(value) = snapshots.get("metadata_keys") {
        let Some(keys) = value.as_array() else {
            return Err(validation_failure(
                path,
                "snapshots.metadata_keys must be a list",
                json!("list of safe metadata keys"),
                json!(python_type_name(value)),
            ));
        };
        for key in keys {
            match key.as_str().map(str::trim) {
                Some(key) if is_safe_metadata_key(key) => {}
                _ => {
                    return Err(validation_failure(
                        path,
                        "snapshots.metadata_keys entries must be non-empty safe metadata keys",
                        json!("non-empty metadata key using [A-Za-z0-9._:-]"),
                        key.clone(),
                    ));
                }
            }
        }
    }
    Ok(())
}

fn is_safe_metadata_key(key: &str) -> bool {
    !key.is_empty()
        && key
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | '.' | ':'))
}

fn require_non_empty_string_field(
    path: &Path,
    value: &Map<String, Value>,
    assertion_name: &str,
    field_name: &str,
) -> Result<(), ScenarioError> {
    if !value
        .get(field_name)
        .map(is_non_empty_string)
        .unwrap_or(false)
    {
        return Err(validation_failure(
            path,
            &format!("{assertion_name} assertion field '{field_name}' must be a non-empty string"),
            json!("non-empty string"),
            value.get(field_name).cloned().unwrap_or(Value::Null),
        ));
    }
    Ok(())
}

fn require_numeric_field(
    path: &Path,
    value: &Map<String, Value>,
    assertion_name: &str,
    field_name: &str,
) -> Result<(), ScenarioError> {
    if !value.get(field_name).map(Value::is_number).unwrap_or(false) {
        return Err(validation_failure(
            path,
            &format!("{assertion_name} assertion field '{field_name}' must be a number"),
            json!("number"),
            value.get(field_name).cloned().unwrap_or(Value::Null),
        ));
    }
    Ok(())
}

fn require_finite_non_negative_number(
    path: &Path,
    value: &Map<String, Value>,
    assertion_name: &str,
    field_name: &str,
) -> Result<f64, ScenarioError> {
    let raw = value.get(field_name).cloned().unwrap_or(Value::Null);
    let Some(number) = raw.as_f64() else {
        return Err(validation_failure(
            path,
            &format!(
                "{assertion_name} assertion field '{field_name}' must be a finite non-negative number"
            ),
            json!("finite non-negative number"),
            raw,
        ));
    };
    if !number.is_finite() || number < 0.0 {
        return Err(validation_failure(
            path,
            &format!(
                "{assertion_name} assertion field '{field_name}' must be a finite non-negative number"
            ),
            json!("finite non-negative number"),
            value.get(field_name).cloned().unwrap_or(Value::Null),
        ));
    }
    Ok(number)
}

fn require_positive_number_field(
    path: &Path,
    value: &Map<String, Value>,
    assertion_name: &str,
    field_name: &str,
) -> Result<f64, ScenarioError> {
    let raw = value.get(field_name).cloned().unwrap_or(Value::Null);
    let Some(number) = raw.as_f64() else {
        return Err(validation_failure(
            path,
            &format!("{assertion_name} assertion field '{field_name}' must be a positive number"),
            json!("positive number"),
            raw,
        ));
    };
    if !number.is_finite() || number <= 0.0 {
        return Err(validation_failure(
            path,
            &format!("{assertion_name} assertion field '{field_name}' must be a positive number"),
            json!("positive number"),
            raw,
        ));
    }
    Ok(number)
}

fn optional_finite_non_negative_number(
    path: &Path,
    value: &Map<String, Value>,
    assertion_name: &str,
    field_name: &str,
) -> Result<Option<f64>, ScenarioError> {
    if !value.contains_key(field_name) {
        return Ok(None);
    }
    Ok(Some(require_finite_non_negative_number(
        path,
        value,
        assertion_name,
        field_name,
    )?))
}

fn validate_optional_positive_integer(
    path: &Path,
    value: &Map<String, Value>,
    assertion_name: &str,
    field_name: &str,
) -> Result<(), ScenarioError> {
    let Some(sample_every_frames) = value.get(field_name) else {
        return Ok(());
    };
    if sample_every_frames.as_i64().is_none()
        || !sample_every_frames.is_i64()
        || sample_every_frames.as_i64().unwrap() <= 0
    {
        return Err(validation_failure(
            path,
            &format!("{assertion_name} assertion field '{field_name}' must be a positive integer"),
            json!("positive integer"),
            sample_every_frames.clone(),
        ));
    }
    Ok(())
}
