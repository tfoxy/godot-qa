use godot_qa_core::results::FailureDetail;
use serde_json::{Map, Value, json};
use std::path::Path;

pub const MAX_TIME_SCALE: f64 = 64.0;
pub const MIN_PHYSICS_TICKS_PER_SECOND: i64 = 1;
pub const MIN_MAX_PHYSICS_STEPS_PER_FRAME: i64 = 1;

#[derive(Debug, Clone, PartialEq)]
pub struct TimeControlSettings {
    pub time_scale: f64,
    pub physics_ticks_per_second: Option<i64>,
    pub max_physics_steps_per_frame: Option<i64>,
}

impl TimeControlSettings {
    pub fn from_value(
        value: &Value,
        failure_kind: &str,
        path: Option<&Path>,
    ) -> Result<Self, FailureDetail> {
        let Some(settings) = value.as_object() else {
            return Err(time_control_failure(
                failure_kind,
                path,
                "time_control must be an object",
                json!("object"),
                json!(json_type_name(value)),
            ));
        };

        let Some(time_scale_value) = settings.get("time_scale") else {
            return Err(time_control_failure(
                failure_kind,
                path,
                "time_control.time_scale is required",
                json!(format!("positive finite number <= {MAX_TIME_SCALE}")),
                Value::Null,
            ));
        };
        let Some(time_scale) = time_scale_value.as_f64() else {
            return Err(time_control_failure(
                failure_kind,
                path,
                "time_control.time_scale must be a positive finite number",
                json!(format!("positive finite number <= {MAX_TIME_SCALE}")),
                json!(json_type_name(time_scale_value)),
            ));
        };
        if !time_scale.is_finite() || time_scale <= 0.0 || time_scale > MAX_TIME_SCALE {
            return Err(time_control_failure(
                failure_kind,
                path,
                &format!(
                    "time_control.time_scale must be greater than 0.0 and <= {MAX_TIME_SCALE}"
                ),
                json!(format!("positive finite number <= {MAX_TIME_SCALE}")),
                time_scale_value.clone(),
            ));
        }

        let physics_ticks_per_second = optional_integer_at_least(
            settings,
            "physics_ticks_per_second",
            MIN_PHYSICS_TICKS_PER_SECOND,
            failure_kind,
            path,
        )?;
        let max_physics_steps_per_frame = optional_integer_at_least(
            settings,
            "max_physics_steps_per_frame",
            MIN_MAX_PHYSICS_STEPS_PER_FRAME,
            failure_kind,
            path,
        )?;

        Ok(Self {
            time_scale,
            physics_ticks_per_second,
            max_physics_steps_per_frame,
        })
    }

    pub fn to_normalized_value(&self) -> Value {
        let mut normalized = Map::new();
        normalized.insert("time_scale".to_string(), json!(self.time_scale));
        if let Some(value) = self.physics_ticks_per_second {
            normalized.insert("physics_ticks_per_second".to_string(), json!(value));
        }
        if let Some(value) = self.max_physics_steps_per_frame {
            normalized.insert("max_physics_steps_per_frame".to_string(), json!(value));
        }
        Value::Object(normalized)
    }
}

pub fn validate_time_control_for_scenario(
    value: &Value,
    path: &Path,
) -> Result<TimeControlSettings, FailureDetail> {
    TimeControlSettings::from_value(value, "scenario_validation_failed", Some(path))
}

pub fn validate_time_control_for_manifest(
    value: &Value,
) -> Result<TimeControlSettings, FailureDetail> {
    TimeControlSettings::from_value(value, "test_manifest_invalid", None)
}

fn optional_integer_at_least(
    settings: &Map<String, Value>,
    field: &str,
    minimum: i64,
    failure_kind: &str,
    path: Option<&Path>,
) -> Result<Option<i64>, FailureDetail> {
    let Some(value) = settings.get(field) else {
        return Ok(None);
    };
    let Some(integer) = value.as_i64() else {
        return Err(time_control_failure(
            failure_kind,
            path,
            &format!("time_control.{field} must be an integer >= {minimum}"),
            json!(format!("integer >= {minimum}")),
            json!(json_type_name(value)),
        ));
    };
    if integer < minimum {
        return Err(time_control_failure(
            failure_kind,
            path,
            &format!("time_control.{field} must be an integer >= {minimum}"),
            json!(format!("integer >= {minimum}")),
            value.clone(),
        ));
    }
    Ok(Some(integer))
}

fn time_control_failure(
    failure_kind: &str,
    path: Option<&Path>,
    message: &str,
    expected: Value,
    actual: Value,
) -> FailureDetail {
    FailureDetail {
        kind: failure_kind.to_string(),
        path: path.map(|path| path.to_string_lossy().into_owned()),
        message: Some(message.to_string()),
        expected: Some(expected),
        actual: Some(actual),
        ..FailureDetail::default()
    }
}

fn json_type_name(value: &Value) -> &'static str {
    match value {
        Value::Null => "NoneType",
        Value::Bool(_) => "bool",
        Value::Number(_) => "number",
        Value::String(_) => "str",
        Value::Array(_) => "list",
        Value::Object(_) => "dict",
    }
}
