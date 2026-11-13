use super::super::{ScenarioError, is_non_empty_string, python_type_name, validation_failure};
use super::state::{
    validate_assertion_temporal_sampling, validate_never_forbidden, validate_never_observation,
    validate_observe_until_assertion, validate_state_matches_assertion,
    validate_temporal_state_observation, validate_temporal_window,
};
use super::{
    optional_finite_non_negative_number, reject_unknown_keys, require_finite_non_negative_number,
    require_non_empty_string_field, require_numeric_field, require_positive_number_field,
    validate_optional_positive_integer, validate_to_point_object,
};
use serde_json::{Map, Value, json};
use std::path::Path;

const LAYOUT_AUDIT_MODES: [&str; 2] = ["default", "strict"];

/// Every assertion the scenario runner understands. An assertion map may only
/// carry one of these keys (plus the shared timeout siblings).
const KNOWN_ASSERTION_KEYS: &[&str] = &[
    "exists",
    "not_exists",
    "visible",
    "hidden",
    "enabled",
    "disabled",
    "focused",
    "inside_viewport",
    "text_equals",
    "text_contains",
    "no_overlap",
    "no_errors",
    "layout_audit",
    "screenshot_match",
    "focus_path",
    "max_delta_position",
    "follows",
    "within_limits",
    "never",
    "observe_until",
    "distance",
    "distance_between",
    "distance_changed",
    "visible_pixels",
    "saved_snapshot_pixels",
    "not_occluded",
    "renders_above",
    "renders_below",
    "surface_probe",
    "surface_matches",
    "state_matches",
    "trend",
    "frame_time",
];

/// Trend directions understood by `assert.temporal.trend` (kept in sync with
/// `TREND_DIRECTIONS` in qa_assertions_impl.gd).
const TREND_DIRECTIONS: &[&str] = &[
    "increasing",
    "decreasing",
    "non_increasing",
    "non_decreasing",
];

/// Frame-timing bounds accepted by the `frame_time` assertion; at least one is
/// required.
const FRAME_TIME_BOUND_FIELDS: &[&str] = &["max_avg_ms", "max_p95_ms", "min_fps"];
const SURFACE_COORDINATE_SPACES: [&str; 3] = ["world_2d", "viewport_2d", "world_3d"];
const SURFACE_MAX_POINTS: usize = 32;
const SURFACE_MAX_RADIUS: f64 = 1024.0;
pub(super) fn validate_supported_assertion_payloads(
    assertions: &[Value],
    path: &Path,
) -> Result<(), ScenarioError> {
    for (index, assertion) in assertions.iter().enumerate() {
        let Some(assertion) = assertion.as_object() else {
            continue;
        };

        reject_unknown_keys(assertion, KNOWN_ASSERTION_KEYS, "assertion", index, path)?;

        if let Some(state_matches) = assertion.get("state_matches") {
            validate_state_matches_assertion(path, state_matches)?;
        }

        for name in [
            "exists",
            "not_exists",
            "visible",
            "hidden",
            "enabled",
            "disabled",
            "focused",
            "inside_viewport",
        ] {
            if let Some(value) = assertion.get(name) {
                if !is_non_empty_string(value) {
                    return Err(validation_failure(
                        path,
                        &format!("{name} assertion value must be a non-empty string"),
                        json!("non-empty string"),
                        value.clone(),
                    ));
                }
            }
        }

        for name in ["text_equals", "text_contains"] {
            if let Some(value) = assertion.get(name) {
                let Some(value) = value.as_object() else {
                    return Err(validation_failure(
                        path,
                        &format!("{name} assertion value must be an object"),
                        json!("object"),
                        json!(python_type_name(value)),
                    ));
                };
                if !value
                    .get("target")
                    .map(is_non_empty_string)
                    .unwrap_or(false)
                {
                    return Err(validation_failure(
                        path,
                        &format!("{name} assertion field 'target' must be a non-empty string"),
                        json!("non-empty string"),
                        value.get("target").cloned().unwrap_or(Value::Null),
                    ));
                }
                if !matches!(value.get("expected"), Some(Value::String(_))) {
                    return Err(validation_failure(
                        path,
                        &format!("{name} assertion field 'expected' must be a string"),
                        json!("string"),
                        value.get("expected").cloned().unwrap_or(Value::Null),
                    ));
                }
            }
        }

        if let Some(value) = assertion.get("no_overlap") {
            let Some(value) = value.as_object() else {
                return Err(validation_failure(
                    path,
                    "no_overlap assertion value must be an object",
                    json!("object"),
                    json!(python_type_name(value)),
                ));
            };
            for field_name in ["a", "b"] {
                if !value
                    .get(field_name)
                    .map(is_non_empty_string)
                    .unwrap_or(false)
                {
                    return Err(validation_failure(
                        path,
                        &format!(
                            "no_overlap assertion field '{field_name}' must be a non-empty string"
                        ),
                        json!("non-empty string"),
                        value.get(field_name).cloned().unwrap_or(Value::Null),
                    ));
                }
            }
        }

        if let Some(value) = assertion.get("no_errors") {
            if !value.is_boolean() {
                return Err(validation_failure(
                    path,
                    "no_errors assertion value must be a boolean",
                    json!("boolean"),
                    value.clone(),
                ));
            }
        }

        if let Some(value) = assertion.get("layout_audit") {
            let Some(mode) = value.as_str() else {
                return Err(validation_failure(
                    path,
                    "layout_audit assertion value must be a non-empty string",
                    json!("non-empty string"),
                    value.clone(),
                ));
            };
            let trimmed = mode.trim();
            if trimmed.is_empty() {
                return Err(validation_failure(
                    path,
                    "layout_audit assertion value must be a non-empty string",
                    json!("non-empty string"),
                    value.clone(),
                ));
            }
            if !LAYOUT_AUDIT_MODES.contains(&trimmed) {
                return Err(validation_failure(
                    path,
                    "layout_audit assertion value must be a supported mode",
                    json!(LAYOUT_AUDIT_MODES),
                    value.clone(),
                ));
            }
        }

        if let Some(value) = assertion.get("screenshot_match") {
            let Some(value) = value.as_object() else {
                return Err(validation_failure(
                    path,
                    "screenshot_match assertion value must be an object",
                    json!("object"),
                    json!(python_type_name(value)),
                ));
            };
            if !value
                .get("snapshot")
                .map(is_non_empty_string)
                .unwrap_or(false)
            {
                return Err(validation_failure(
                    path,
                    "screenshot_match assertion field 'snapshot' must be a non-empty string",
                    json!("non-empty string"),
                    value.get("snapshot").cloned().unwrap_or(Value::Null),
                ));
            }
            if !value
                .get("baseline")
                .map(is_non_empty_string)
                .unwrap_or(false)
            {
                return Err(validation_failure(
                    path,
                    "screenshot_match assertion field 'baseline' must be a non-empty string",
                    json!("non-empty string"),
                    value.get("baseline").cloned().unwrap_or(Value::Null),
                ));
            }
            if let Some(threshold) = value.get("threshold") {
                let Some(number) = threshold.as_f64() else {
                    return Err(validation_failure(
                        path,
                        "screenshot_match assertion field 'threshold' must be between 0.0 and 1.0",
                        json!("number between 0.0 and 1.0"),
                        threshold.clone(),
                    ));
                };
                if !(0.0..=1.0).contains(&number) {
                    return Err(validation_failure(
                        path,
                        "screenshot_match assertion field 'threshold' must be between 0.0 and 1.0",
                        json!("number between 0.0 and 1.0"),
                        threshold.clone(),
                    ));
                }
            }
        }

        if let Some(value) = assertion.get("max_delta_position") {
            let Some(value) = value.as_object() else {
                return Err(validation_failure(
                    path,
                    "max_delta_position assertion value must be an object",
                    json!("object"),
                    json!(python_type_name(value)),
                ));
            };
            require_non_empty_string_field(path, value, "max_delta_position", "target")?;
            validate_assertion_temporal_sampling(path, value, "max_delta_position")?;
            if value.contains_key("window_frames") || value.contains_key("window_seconds") {
                validate_temporal_window(path, value, "max_delta_position")?;
            }
            validate_optional_positive_integer(
                path,
                value,
                "max_delta_position",
                "sample_every_frames",
            )?;
            if !value.contains_key("max") && !value.contains_key("max_delta") {
                return Err(validation_failure(
                    path,
                    "max_delta_position assertion requires 'max' or compatibility field 'max_delta'",
                    json!("numeric max or max_delta"),
                    Value::Null,
                ));
            }
            if value.contains_key("max") {
                require_numeric_field(path, value, "max_delta_position", "max")?;
            }
            if value.contains_key("max_delta") {
                require_numeric_field(path, value, "max_delta_position", "max_delta")?;
            }
        }

        if let Some(value) = assertion.get("follows") {
            let Some(value) = value.as_object() else {
                return Err(validation_failure(
                    path,
                    "follows assertion value must be an object",
                    json!("object"),
                    json!(python_type_name(value)),
                ));
            };
            require_non_empty_string_field(path, value, "follows", "leader")?;
            require_non_empty_string_field(path, value, "follows", "follower")?;
            validate_assertion_temporal_sampling(path, value, "follows")?;
            validate_temporal_window(path, value, "follows")?;
            validate_optional_positive_integer(path, value, "follows", "sample_every_frames")?;
            require_numeric_field(path, value, "follows", "tolerance")?;
        }

        if let Some(value) = assertion.get("within_limits") {
            let Some(value) = value.as_object() else {
                return Err(validation_failure(
                    path,
                    "within_limits assertion value must be an object",
                    json!("object"),
                    json!(python_type_name(value)),
                ));
            };
            let observes_state = validate_temporal_state_observation(path, value, "within_limits")?;
            if !observes_state {
                require_non_empty_string_field(path, value, "within_limits", "target")?;
            }
            validate_assertion_temporal_sampling(path, value, "within_limits")?;
            validate_temporal_window(path, value, "within_limits")?;
            validate_optional_positive_integer(
                path,
                value,
                "within_limits",
                "sample_every_frames",
            )?;
            require_numeric_field(path, value, "within_limits", "min")?;
            require_numeric_field(path, value, "within_limits", "max")?;
        }

        if let Some(value) = assertion.get("trend") {
            let Some(value) = value.as_object() else {
                return Err(validation_failure(
                    path,
                    "trend assertion value must be an object",
                    json!("object"),
                    json!(python_type_name(value)),
                ));
            };
            let direction = value
                .get("trend")
                .and_then(Value::as_str)
                .map(str::trim)
                .unwrap_or_default();
            if !TREND_DIRECTIONS.contains(&direction) {
                return Err(validation_failure(
                    path,
                    "trend assertion field 'trend' must be a supported direction",
                    json!(TREND_DIRECTIONS),
                    value.get("trend").cloned().unwrap_or(Value::Null),
                ));
            }
            let observes_state = validate_temporal_state_observation(path, value, "trend")?;
            if !observes_state {
                require_non_empty_string_field(path, value, "trend", "target")?;
            }
            validate_assertion_temporal_sampling(path, value, "trend")?;
            validate_temporal_window(path, value, "trend")?;
            validate_optional_positive_integer(path, value, "trend", "sample_every_frames")?;
            if value.contains_key("min_delta") {
                require_finite_non_negative_number(path, value, "trend", "min_delta")?;
            }
            if value.contains_key("tolerance") {
                require_finite_non_negative_number(path, value, "trend", "tolerance")?;
            }
        }

        if let Some(value) = assertion.get("frame_time") {
            let Some(value) = value.as_object() else {
                return Err(validation_failure(
                    path,
                    "frame_time assertion value must be an object",
                    json!("object"),
                    json!(python_type_name(value)),
                ));
            };
            if value.contains_key("temporal_sampling") {
                return Err(validation_failure(
                    path,
                    "frame_time assertion does not support 'temporal_sampling' because it measures wall-clock frame times",
                    json!("no temporal_sampling field"),
                    value
                        .get("temporal_sampling")
                        .cloned()
                        .unwrap_or(Value::Null),
                ));
            }
            validate_temporal_window(path, value, "frame_time")?;
            validate_optional_positive_integer(path, value, "frame_time", "sample_every_frames")?;
            let mut bound_count = 0;
            for field_name in FRAME_TIME_BOUND_FIELDS {
                if value.contains_key(*field_name) {
                    require_positive_number_field(path, value, "frame_time", field_name)?;
                    bound_count += 1;
                }
            }
            if bound_count == 0 {
                return Err(validation_failure(
                    path,
                    "frame_time assertion requires at least one bound: 'max_avg_ms', 'max_p95_ms', or 'min_fps'",
                    json!(FRAME_TIME_BOUND_FIELDS),
                    Value::Object(value.clone()),
                ));
            }
        }

        if let Some(value) = assertion.get("distance") {
            validate_distance_assertion_payload(path, value, "distance", "min", "max")?;
        }

        if let Some(value) = assertion.get("distance_between") {
            validate_distance_assertion_payload(
                path,
                value,
                "distance_between",
                "min_distance",
                "max_distance",
            )?;
        }

        if let Some(value) = assertion.get("distance_changed") {
            let Some(value) = value.as_object() else {
                return Err(validation_failure(
                    path,
                    "distance_changed assertion value must be an object",
                    json!("object"),
                    json!(python_type_name(value)),
                ));
            };
            require_non_empty_string_field(path, value, "distance_changed", "a")?;
            require_non_empty_string_field(path, value, "distance_changed", "b")?;
            validate_assertion_temporal_sampling(path, value, "distance_changed")?;
            validate_temporal_window(path, value, "distance_changed")?;
            validate_optional_positive_integer(
                path,
                value,
                "distance_changed",
                "sample_every_frames",
            )?;
            require_finite_non_negative_number(path, value, "distance_changed", "min")?;
        }

        if let Some(value) = assertion.get("visible_pixels") {
            let Some(value) = value.as_object() else {
                return Err(validation_failure(
                    path,
                    "visible_pixels assertion value must be an object",
                    json!("object"),
                    json!(python_type_name(value)),
                ));
            };
            require_non_empty_string_field(path, value, "visible_pixels", "target")?;
            require_non_empty_string_field(path, value, "visible_pixels", "snapshot")?;
            require_visual_threshold(path, value, "visible_pixels")?;
            validate_visual_threshold_fields(path, value, "visible_pixels")?;
        }

        if let Some(value) = assertion.get("saved_snapshot_pixels") {
            let Some(value) = value.as_object() else {
                return Err(validation_failure(
                    path,
                    "saved_snapshot_pixels assertion value must be an object",
                    json!("object"),
                    json!(python_type_name(value)),
                ));
            };
            require_non_empty_string_field(path, value, "saved_snapshot_pixels", "target")?;
            require_non_empty_string_field(path, value, "saved_snapshot_pixels", "snapshot")?;
            require_visual_threshold(path, value, "saved_snapshot_pixels")?;
            validate_visual_threshold_fields(path, value, "saved_snapshot_pixels")?;
        }

        if let Some(value) = assertion.get("not_occluded") {
            let Some(value) = value.as_object() else {
                return Err(validation_failure(
                    path,
                    "not_occluded assertion value must be an object",
                    json!("object"),
                    json!(python_type_name(value)),
                ));
            };
            require_non_empty_string_field(path, value, "not_occluded", "target")?;
            require_non_empty_string_field(path, value, "not_occluded", "snapshot")?;
            if value.contains_key("occluder") {
                require_non_empty_string_field(path, value, "not_occluded", "occluder")?;
            }
            validate_visual_threshold_fields(path, value, "not_occluded")?;
        }

        for assertion_name in ["renders_above", "renders_below"] {
            if let Some(value) = assertion.get(assertion_name) {
                let Some(value) = value.as_object() else {
                    return Err(validation_failure(
                        path,
                        &format!("{assertion_name} assertion value must be an object"),
                        json!("object"),
                        json!(python_type_name(value)),
                    ));
                };
                if !value.contains_key("target") && !value.contains_key("a") {
                    require_non_empty_string_field(path, value, assertion_name, "target")?;
                }
                if !value.contains_key("other") && !value.contains_key("b") {
                    require_non_empty_string_field(path, value, assertion_name, "other")?;
                }
                if value.contains_key("target") {
                    require_non_empty_string_field(path, value, assertion_name, "target")?;
                }
                if value.contains_key("other") {
                    require_non_empty_string_field(path, value, assertion_name, "other")?;
                }
                if value.contains_key("a") {
                    require_non_empty_string_field(path, value, assertion_name, "a")?;
                }
                if value.contains_key("b") {
                    require_non_empty_string_field(path, value, assertion_name, "b")?;
                }
                if value.contains_key("snapshot") {
                    require_non_empty_string_field(path, value, assertion_name, "snapshot")?;
                }
            }
        }

        if let Some(value) = assertion.get("never") {
            let Some(value) = value.as_object() else {
                return Err(validation_failure(
                    path,
                    "never assertion value must be an object",
                    json!("object"),
                    json!(python_type_name(value)),
                ));
            };
            let observes_state = validate_temporal_state_observation(path, value, "never")?;
            if observes_state {
                for field_name in ["property", "text", "token"] {
                    if value.contains_key(field_name) {
                        return Err(validation_failure(
                            path,
                            &format!(
                                "never assertion cannot combine 'source' with observation field '{field_name}'"
                            ),
                            json!("either a state 'source' or one of 'property'/'text'/'token'"),
                            Value::Object(value.clone()),
                        ));
                    }
                }
            } else {
                require_non_empty_string_field(path, value, "never", "target")?;
                validate_never_observation(path, value)?;
            }
            validate_assertion_temporal_sampling(path, value, "never")?;
            validate_temporal_window(path, value, "never")?;
            validate_optional_positive_integer(path, value, "never", "sample_every_frames")?;
            validate_never_forbidden(path, value)?;
        }
        if let Some(value) = assertion.get("observe_until") {
            validate_observe_until_assertion(path, value)?;
        }
        if let Some(value) = assertion.get("surface_probe") {
            let Some(value) = value.as_object() else {
                return Err(validation_failure(
                    path,
                    "surface_probe assertion value must be an object",
                    json!("object"),
                    json!(python_type_name(value)),
                ));
            };
            if value.contains_key("probe_id") {
                if value.contains_key("provider") {
                    require_non_empty_string_field(path, value, "surface_probe", "provider")?;
                } else {
                    require_non_empty_string_field(path, value, "surface_probe", "provider_id")?;
                }
                require_non_empty_string_field(path, value, "surface_probe", "probe_id")?;
                if let Some(expected_surface) = value.get("expected_surface")
                    && !matches!(expected_surface, Value::String(_))
                {
                    return Err(validation_failure(
                        path,
                        "surface_probe assertion field 'expected_surface' must be a string",
                        json!("string"),
                        expected_surface.clone(),
                    ));
                }
                if let Some(expected_walkable) = value.get("expected_walkable")
                    && !expected_walkable.is_boolean()
                {
                    return Err(validation_failure(
                        path,
                        "surface_probe assertion field 'expected_walkable' must be a boolean",
                        json!("boolean"),
                        expected_walkable.clone(),
                    ));
                }
                continue;
            }
            require_non_empty_string_field(path, value, "surface_probe", "target")?;
            if value.contains_key("provider") {
                require_non_empty_string_field(path, value, "surface_probe", "provider")?;
            } else {
                require_non_empty_string_field(path, value, "surface_probe", "provider_id")?;
            }
            if let Some(space) = value.get("coordinate_space").and_then(Value::as_str) {
                if !SURFACE_COORDINATE_SPACES.contains(&space.trim()) {
                    return Err(validation_failure(
                        path,
                        "surface_probe assertion field 'coordinate_space' must be supported",
                        json!(SURFACE_COORDINATE_SPACES),
                        value
                            .get("coordinate_space")
                            .cloned()
                            .unwrap_or(Value::Null),
                    ));
                }
            }
            if let Some(radius) = value.get("radius") {
                let Some(radius) = radius.as_f64() else {
                    return Err(validation_failure(
                        path,
                        "surface_probe assertion field 'radius' must be bounded",
                        json!(format!("number in range [0.0, {SURFACE_MAX_RADIUS}]")),
                        radius.clone(),
                    ));
                };
                if !radius.is_finite() || !(0.0..=SURFACE_MAX_RADIUS).contains(&radius) {
                    return Err(validation_failure(
                        path,
                        "surface_probe assertion field 'radius' must be bounded",
                        json!(format!("number in range [0.0, {SURFACE_MAX_RADIUS}]")),
                        json!(radius),
                    ));
                }
            }
            let Some(points) = value.get("points").and_then(Value::as_array) else {
                return Err(validation_failure(
                    path,
                    "surface_probe assertion field 'points' must be an array",
                    json!("array"),
                    value.get("points").cloned().unwrap_or(Value::Null),
                ));
            };
            if points.is_empty() || points.len() > SURFACE_MAX_POINTS {
                return Err(validation_failure(
                    path,
                    "surface_probe assertion field 'points' must be bounded",
                    json!(format!("1..={SURFACE_MAX_POINTS} points")),
                    json!(points.len()),
                ));
            }
            let surfaces = value.get("surfaces").and_then(Value::as_array);
            for point in points {
                let Some(point) = point.as_object() else {
                    return Err(validation_failure(
                        path,
                        "surface_probe point must be an object",
                        json!("object"),
                        point.clone(),
                    ));
                };
                for field_name in ["x", "y"] {
                    if !point.get(field_name).map(Value::is_number).unwrap_or(false) {
                        return Err(validation_failure(
                            path,
                            &format!("surface_probe point field '{field_name}' must be numeric"),
                            json!("number"),
                            point.get(field_name).cloned().unwrap_or(Value::Null),
                        ));
                    }
                }
                if let Some(z) = point.get("z")
                    && !z.is_number()
                {
                    return Err(validation_failure(
                        path,
                        "surface_probe point field 'z' must be numeric",
                        json!("number"),
                        z.clone(),
                    ));
                }
                if let Some(expected) = point.get("expected").and_then(Value::as_str)
                    && let Some(surfaces) = surfaces
                {
                    let labels: Vec<&str> = surfaces
                        .iter()
                        .filter_map(Value::as_str)
                        .map(str::trim)
                        .collect();
                    if !labels.contains(&expected.trim()) {
                        return Err(validation_failure(
                            path,
                            "surface_probe point expected label must exist in surfaces",
                            json!(labels),
                            json!(expected),
                        ));
                    }
                }
            }
        }
        if let Some(value) = assertion.get("surface_matches") {
            let Some(value) = value.as_object() else {
                return Err(validation_failure(
                    path,
                    "surface_matches assertion value must be an object",
                    json!("object"),
                    json!(python_type_name(value)),
                ));
            };
            require_non_empty_string_field(path, value, "surface_matches", "target")?;
            if !value.contains_key("surface") && !value.contains_key("expected") {
                return Err(validation_failure(
                    path,
                    "surface_matches assertion requires 'surface' or compatibility field 'expected'",
                    json!("surface or expected"),
                    Value::Null,
                ));
            }
            if value.contains_key("surface") {
                require_non_empty_string_field(path, value, "surface_matches", "surface")?;
            }
            if value.contains_key("expected") {
                require_non_empty_string_field(path, value, "surface_matches", "expected")?;
            }
        }
    }

    Ok(())
}

fn validate_distance_assertion_payload(
    path: &Path,
    value: &Value,
    assertion_name: &str,
    min_field: &str,
    max_field: &str,
) -> Result<(), ScenarioError> {
    let Some(value) = value.as_object() else {
        return Err(validation_failure(
            path,
            &format!("{assertion_name} assertion value must be an object"),
            json!("object"),
            json!(python_type_name(value)),
        ));
    };
    if value.contains_key("a_point") || value.contains_key("target_point") {
        let point_value = value
            .get("a_point")
            .or_else(|| value.get("target_point"))
            .unwrap_or(&Value::Null);
        validate_to_point_object(path, assertion_name, point_value)?;
    } else {
        require_non_empty_string_field(path, value, assertion_name, "a")?;
    }
    if value.contains_key("b_point") || value.contains_key("other_point") {
        let point_value = value
            .get("b_point")
            .or_else(|| value.get("other_point"))
            .unwrap_or(&Value::Null);
        validate_to_point_object(path, assertion_name, point_value)?;
    } else {
        require_non_empty_string_field(path, value, assertion_name, "b")?;
    }
    let min = optional_finite_non_negative_number(path, value, assertion_name, min_field)?;
    let max = optional_finite_non_negative_number(path, value, assertion_name, max_field)?;
    if min.is_none() && max.is_none() {
        let message = if assertion_name == "distance" {
            "distance assertion requires at least one of 'min' or 'max'".to_string()
        } else {
            format!("{assertion_name} assertion requires at least one distance bound")
        };
        return Err(validation_failure(
            path,
            &message,
            json!(format!("at least one of '{min_field}' or '{max_field}'")),
            json!(value),
        ));
    }
    Ok(())
}

fn validate_visual_threshold_fields(
    path: &Path,
    value: &Map<String, Value>,
    assertion_name: &str,
) -> Result<(), ScenarioError> {
    if let Some(min_ratio) = value.get("min_ratio") {
        let Some(number) = min_ratio.as_f64() else {
            return Err(validation_failure(
                path,
                &format!(
                    "{assertion_name} assertion field 'min_ratio' must be between 0.0 and 1.0"
                ),
                json!("number between 0.0 and 1.0"),
                min_ratio.clone(),
            ));
        };
        if !(0.0..=1.0).contains(&number) {
            return Err(validation_failure(
                path,
                &format!(
                    "{assertion_name} assertion field 'min_ratio' must be between 0.0 and 1.0"
                ),
                json!("number between 0.0 and 1.0"),
                min_ratio.clone(),
            ));
        }
    }

    if let Some(min_count) = value.get("min_count") {
        if min_count.as_i64().is_none() || !min_count.is_i64() || min_count.as_i64().unwrap() < 0 {
            return Err(validation_failure(
                path,
                &format!(
                    "{assertion_name} assertion field 'min_count' must be a non-negative integer"
                ),
                json!("non-negative integer"),
                min_count.clone(),
            ));
        }
    }

    Ok(())
}

fn require_visual_threshold(
    path: &Path,
    value: &Map<String, Value>,
    assertion_name: &str,
) -> Result<(), ScenarioError> {
    if value.contains_key("min_ratio") || value.contains_key("min_count") {
        return Ok(());
    }
    Err(validation_failure(
        path,
        &format!("{assertion_name} assertion requires at least one of 'min_ratio' or 'min_count'"),
        json!("at least one of 'min_ratio' or 'min_count'"),
        Value::Object(value.clone()),
    ))
}
