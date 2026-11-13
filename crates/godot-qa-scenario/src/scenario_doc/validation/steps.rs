use super::super::{
    ScenarioError, is_non_empty_string, is_safe_artifact_segment, python_type_name,
    validation_failure,
};
use super::assertions::validate_supported_assertion_payloads;
use super::state::{
    validate_drive_policy_step, validate_record_state_step, validate_wait_until_step,
};
use super::{reject_unknown_keys, validate_optional_bool_field, validate_to_point_object};
use serde_json::{Map, Value, json};
use std::path::Path;

/// Every step operation the scenario runner understands. A step map may only
/// carry one of these keys (plus the shared timeout siblings). Unknown keys are
/// rejected during validation instead of being silently ignored/normalized.
const KNOWN_STEP_KEYS: &[&str] = &[
    "wait_frames",
    "wait_seconds",
    "wait_until",
    "snapshot",
    "tap_action",
    "hold_action",
    "release_action",
    "input_key",
    "input_key_hold",
    "input_key_release",
    "input_joy_button",
    "input_joy_motion",
    "input_joy_state",
    "drive_policy",
    "click",
    "node_inspect",
    "record_state",
    "teleport_node",
    "fixture_teleport",
    "spawn_scene",
    "set_property",
    "set_state",
    "joy_connect",
    "joy_disconnect",
    "call_method",
    "fixture_call",
    "audio_capture",
    "audio_export",
    "physics_overlay",
    "assert",
    "no_errors",
];

const AUDIO_CAPTURE_MAX_DURATION_SECONDS: f64 = 60.0;

pub(super) fn validate_supported_step_payloads(
    steps: &[Value],
    path: &Path,
) -> Result<(), ScenarioError> {
    // `record_state` names key the top-level `state_records` evidence map, so a
    // duplicate name across the scenario's record_state steps would silently
    // overwrite earlier evidence. Reject collisions during validation.
    let mut record_state_names: Vec<String> = Vec::new();
    for (index, step) in steps.iter().enumerate() {
        let Some(step) = step.as_object() else {
            continue;
        };

        reject_unknown_keys(step, KNOWN_STEP_KEYS, "step", index, path)?;

        if let Some(wait_until) = step.get("wait_until") {
            validate_wait_until_step(path, wait_until)?;
        }

        if let Some(wait_frames) = step.get("wait_frames") {
            if wait_frames.as_i64().is_none()
                || !wait_frames.is_i64()
                || wait_frames.as_i64().unwrap() < 0
            {
                return Err(validation_failure(
                    path,
                    "wait_frames step value must be a non-negative integer",
                    json!("non-negative integer"),
                    wait_frames.clone(),
                ));
            }
        }

        if let Some(tap_action) = step.get("tap_action") {
            if !is_non_empty_string(tap_action) {
                return Err(validation_failure(
                    path,
                    "tap_action step value must be a non-empty string",
                    json!("non-empty string"),
                    tap_action.clone(),
                ));
            }
        }

        if let Some(hold_action) = step.get("hold_action") {
            let Some(hold_action) = hold_action.as_object() else {
                return Err(validation_failure(
                    path,
                    "hold_action step value must be an object",
                    json!("object"),
                    json!(python_type_name(hold_action)),
                ));
            };
            if !hold_action
                .get("action")
                .map(is_non_empty_string)
                .unwrap_or(false)
            {
                return Err(validation_failure(
                    path,
                    "hold_action step field 'action' must be a non-empty string",
                    json!("non-empty string"),
                    hold_action.get("action").cloned().unwrap_or(Value::Null),
                ));
            }
            let frames = hold_action.get("frames").and_then(Value::as_i64);
            if frames.is_none()
                || !hold_action.get("frames").unwrap_or(&Value::Null).is_i64()
                || frames.unwrap() < 0
            {
                return Err(validation_failure(
                    path,
                    "hold_action step field 'frames' must be a non-negative integer",
                    json!("non-negative integer"),
                    hold_action.get("frames").cloned().unwrap_or(Value::Null),
                ));
            }
        }

        if let Some(release_action) = step.get("release_action") {
            if !is_non_empty_string(release_action) {
                return Err(validation_failure(
                    path,
                    "release_action step value must be a non-empty string",
                    json!("non-empty string"),
                    release_action.clone(),
                ));
            }
        }

        for kind in ["input_key", "input_key_hold", "input_key_release"] {
            if let Some(value) = step.get(kind) {
                validate_input_key_step(value, kind, path)?;
            }
        }
        if let Some(input_joy_button) = step.get("input_joy_button") {
            validate_input_joy_button_step(input_joy_button, path)?;
        }

        if let Some(input_joy_motion) = step.get("input_joy_motion") {
            validate_input_joy_motion_step(input_joy_motion, path)?;
        }

        if let Some(input_joy_state) = step.get("input_joy_state") {
            validate_input_joy_state_step(input_joy_state, path)?;
        }

        if let Some(joy_connect) = step.get("joy_connect") {
            validate_joy_connection_step(joy_connect, path, "joy_connect", true)?;
        }

        if let Some(joy_disconnect) = step.get("joy_disconnect") {
            validate_joy_connection_step(joy_disconnect, path, "joy_disconnect", false)?;
        }

        if let Some(drive_policy) = step.get("drive_policy") {
            validate_drive_policy_step(path, drive_policy)?;
        }

        if let Some(click) = step.get("click") {
            if !is_non_empty_string(click) {
                return Err(validation_failure(
                    path,
                    "click step value must be a non-empty string",
                    json!("non-empty string"),
                    click.clone(),
                ));
            }
        }

        if let Some(node_inspect) = step.get("node_inspect") {
            if !is_non_empty_string(node_inspect) {
                return Err(validation_failure(
                    path,
                    "node_inspect step value must be a non-empty string",
                    json!("non-empty target locator"),
                    node_inspect.clone(),
                ));
            }
        }

        if let Some(record_state) = step.get("record_state") {
            validate_record_state_step(path, record_state, &mut record_state_names)?;
        }

        if let Some(teleport_node) = step.get("teleport_node") {
            let Some(teleport_node) = teleport_node.as_object() else {
                return Err(validation_failure(
                    path,
                    "teleport_node step value must be an object",
                    json!("object"),
                    json!(python_type_name(teleport_node)),
                ));
            };
            if !teleport_node
                .get("target")
                .map(is_non_empty_string)
                .unwrap_or(false)
            {
                return Err(validation_failure(
                    path,
                    "teleport_node step field 'target' must be a non-empty string",
                    json!("non-empty string"),
                    teleport_node.get("target").cloned().unwrap_or(Value::Null),
                ));
            }
            validate_optional_position_or_locator(path, "teleport_node", teleport_node, true)?;
            validate_optional_bool_field(
                path,
                "teleport_node",
                teleport_node,
                "preserve_velocity",
            )?;
        }

        if let Some(fixture_teleport) = step.get("fixture_teleport") {
            let Some(fixture_teleport) = fixture_teleport.as_object() else {
                return Err(validation_failure(
                    path,
                    "fixture_teleport step value must be an object",
                    json!("object"),
                    json!(python_type_name(fixture_teleport)),
                ));
            };
            if !fixture_teleport
                .get("target")
                .map(is_non_empty_string)
                .unwrap_or(false)
            {
                return Err(validation_failure(
                    path,
                    "fixture_teleport step field 'target' must be a non-empty string",
                    json!("non-empty string"),
                    fixture_teleport
                        .get("target")
                        .cloned()
                        .unwrap_or(Value::Null),
                ));
            }
            validate_optional_position_or_locator(
                path,
                "fixture_teleport",
                fixture_teleport,
                true,
            )?;
            validate_optional_bool_field(
                path,
                "fixture_teleport",
                fixture_teleport,
                "preserve_velocity",
            )?;
        }

        if let Some(spawn_scene) = step.get("spawn_scene") {
            let Some(spawn_scene) = spawn_scene.as_object() else {
                return Err(validation_failure(
                    path,
                    "spawn_scene step value must be an object",
                    json!("object"),
                    json!(python_type_name(spawn_scene)),
                ));
            };
            if !spawn_scene
                .get("scene")
                .map(is_non_empty_string)
                .unwrap_or(false)
            {
                return Err(validation_failure(
                    path,
                    "spawn_scene step field 'scene' must be a non-empty string",
                    json!("non-empty string"),
                    spawn_scene.get("scene").cloned().unwrap_or(Value::Null),
                ));
            }
            for field_name in ["parent", "name", "qa_id", "at"] {
                if let Some(value) = spawn_scene.get(field_name) {
                    if !is_non_empty_string(value) {
                        return Err(validation_failure(
                            path,
                            &format!(
                                "spawn_scene step field '{field_name}' must be a non-empty string"
                            ),
                            json!("non-empty string"),
                            value.clone(),
                        ));
                    }
                }
            }
            validate_optional_position_or_locator(path, "spawn_scene", spawn_scene, false)?;
        }

        if let Some(set_property) = step.get("set_property") {
            let Some(set_property) = set_property.as_object() else {
                return Err(validation_failure(
                    path,
                    "set_property step value must be an object",
                    json!("object"),
                    json!(python_type_name(set_property)),
                ));
            };
            for field_name in ["target", "property"] {
                if !set_property
                    .get(field_name)
                    .map(is_non_empty_string)
                    .unwrap_or(false)
                {
                    return Err(validation_failure(
                        path,
                        &format!(
                            "set_property step field '{field_name}' must be a non-empty string"
                        ),
                        json!("non-empty string"),
                        set_property.get(field_name).cloned().unwrap_or(Value::Null),
                    ));
                }
            }
            if !set_property.contains_key("value") {
                return Err(validation_failure(
                    path,
                    "set_property step field 'value' is required",
                    json!("any JSON value"),
                    Value::Null,
                ));
            }
        }

        if let Some(call_method) = step.get("call_method") {
            let Some(call_method) = call_method.as_object() else {
                return Err(validation_failure(
                    path,
                    "call_method step value must be an object",
                    json!("object"),
                    json!(python_type_name(call_method)),
                ));
            };
            for field_name in ["target", "method"] {
                if !call_method
                    .get(field_name)
                    .map(is_non_empty_string)
                    .unwrap_or(false)
                {
                    return Err(validation_failure(
                        path,
                        &format!(
                            "call_method step field '{field_name}' must be a non-empty string"
                        ),
                        json!("non-empty string"),
                        call_method.get(field_name).cloned().unwrap_or(Value::Null),
                    ));
                }
            }
            if let Some(method) = call_method.get("method").and_then(Value::as_str) {
                if !method.trim().starts_with("qa_") {
                    return Err(validation_failure(
                        path,
                        "call_method step field 'method' must name a qa_ fixture method",
                        json!("method name beginning with qa_"),
                        json!(method),
                    ));
                }
            }
            if let Some(args) = call_method.get("args") {
                if !args.is_array() {
                    return Err(validation_failure(
                        path,
                        "call_method step field 'args' must be a list when present",
                        json!("list"),
                        json!(python_type_name(args)),
                    ));
                }
            }
        }

        if let Some(fixture_call) = step.get("fixture_call") {
            let Some(fixture_call) = fixture_call.as_object() else {
                return Err(validation_failure(
                    path,
                    "fixture_call step value must be an object",
                    json!("object"),
                    json!(python_type_name(fixture_call)),
                ));
            };
            for field_name in ["driver", "action"] {
                if !fixture_call
                    .get(field_name)
                    .map(is_non_empty_string)
                    .unwrap_or(false)
                {
                    return Err(validation_failure(
                        path,
                        &format!(
                            "fixture_call step field '{field_name}' must be a non-empty string"
                        ),
                        json!("non-empty string"),
                        fixture_call.get(field_name).cloned().unwrap_or(Value::Null),
                    ));
                }
            }
            if let Some(params) = fixture_call.get("params")
                && !params.is_object()
            {
                return Err(validation_failure(
                    path,
                    "fixture_call step field 'params' must be an object when present",
                    json!("object"),
                    json!(python_type_name(params)),
                ));
            }
        }

        if let Some(audio_capture) = step.get("audio_capture") {
            validate_audio_capture_step(path, audio_capture)?;
        }

        if let Some(audio_export) = step.get("audio_export") {
            validate_audio_export_step(path, audio_export)?;
        }

        if let Some(assertions) = step.get("assert") {
            let Some(assertions) = assertions.as_array() else {
                return Err(validation_failure(
                    path,
                    "assert step value must be a list",
                    json!("list"),
                    json!(python_type_name(assertions)),
                ));
            };
            validate_supported_assertion_payloads(assertions, path)?;
        }
    }
    Ok(())
}

fn validate_optional_position_or_locator(
    path: &Path,
    step_name: &str,
    step: &Map<String, Value>,
    require_one: bool,
) -> Result<(), ScenarioError> {
    let locator_field = if step_name == "spawn_scene" {
        "at"
    } else {
        "to"
    };
    let has_locator = step.contains_key(locator_field);
    let has_position = step.contains_key("position");
    let has_to_point = step.contains_key("to_point");
    if require_one && !has_locator && !has_position && !has_to_point {
        return Err(validation_failure(
            path,
            &format!("{step_name} step requires '{locator_field}', 'position', or 'to_point'"),
            json!(format!(
                "non-empty '{locator_field}' locator, position object, or to_point provider"
            )),
            Value::Object(step.clone()),
        ));
    }
    let destination_count = [has_locator, has_position, has_to_point]
        .into_iter()
        .filter(|present| *present)
        .count();
    if destination_count > 1 {
        return Err(validation_failure(
            path,
            &format!("{step_name} step cannot combine destination fields"),
            json!(format!(
                "only one of '{locator_field}', 'position', or 'to_point'"
            )),
            Value::Object(step.clone()),
        ));
    }
    if let Some(locator) = step.get(locator_field) {
        if !is_non_empty_string(locator) {
            return Err(validation_failure(
                path,
                &format!("{step_name} step field '{locator_field}' must be a non-empty string"),
                json!("non-empty string"),
                locator.clone(),
            ));
        }
    }
    if let Some(position) = step.get("position") {
        validate_position_object(path, step_name, position)?;
    }
    if let Some(to_point) = step.get("to_point") {
        validate_to_point_object(path, step_name, to_point)?;
    }
    if let Some(offset) = step.get("offset") {
        validate_position_object(path, step_name, offset)?;
    }
    Ok(())
}

fn validate_audio_capture_step(path: &Path, audio_capture: &Value) -> Result<(), ScenarioError> {
    let Some(audio_capture) = audio_capture.as_object() else {
        return Err(validation_failure(
            path,
            "audio_capture step value must be an object",
            json!("object"),
            json!(python_type_name(audio_capture)),
        ));
    };
    validate_audio_artifact_name(path, audio_capture, "audio_capture")?;
    let duration = audio_capture
        .get("duration_seconds")
        .and_then(Value::as_f64)
        .unwrap_or(f64::NAN);
    if !duration.is_finite() || duration <= 0.0 || duration > AUDIO_CAPTURE_MAX_DURATION_SECONDS {
        return Err(validation_failure(
            path,
            "audio_capture step field 'duration_seconds' must be a bounded positive number",
            json!(format!(
                "number in range (0.0, {AUDIO_CAPTURE_MAX_DURATION_SECONDS}]"
            )),
            audio_capture
                .get("duration_seconds")
                .cloned()
                .unwrap_or(Value::Null),
        ));
    }
    if let Some(bus) = audio_capture.get("bus") {
        if !is_non_empty_string(bus) {
            return Err(validation_failure(
                path,
                "audio_capture step field 'bus' must be a non-empty string when present",
                json!("non-empty string"),
                bus.clone(),
            ));
        }
    }
    validate_optional_bool_field(path, "audio_capture", audio_capture, "analyze")?;
    Ok(())
}

fn validate_audio_export_step(path: &Path, audio_export: &Value) -> Result<(), ScenarioError> {
    let Some(audio_export) = audio_export.as_object() else {
        return Err(validation_failure(
            path,
            "audio_export step value must be an object",
            json!("object"),
            json!(python_type_name(audio_export)),
        ));
    };
    validate_audio_artifact_name(path, audio_export, "audio_export")?;
    for field_name in ["target", "method"] {
        if !audio_export
            .get(field_name)
            .map(is_non_empty_string)
            .unwrap_or(false)
        {
            return Err(validation_failure(
                path,
                &format!("audio_export step field '{field_name}' must be a non-empty string"),
                json!("non-empty string"),
                audio_export.get(field_name).cloned().unwrap_or(Value::Null),
            ));
        }
    }
    if let Some(method) = audio_export.get("method").and_then(Value::as_str) {
        if !method.trim().starts_with("qa_") {
            return Err(validation_failure(
                path,
                "audio_export step field 'method' must name a qa_ fixture method",
                json!("method name beginning with qa_"),
                json!(method),
            ));
        }
    }
    if let Some(args) = audio_export.get("args") {
        if !args.is_array() {
            return Err(validation_failure(
                path,
                "audio_export step field 'args' must be a list when present",
                json!("list"),
                json!(python_type_name(args)),
            ));
        }
    }
    validate_optional_bool_field(path, "audio_export", audio_export, "analyze")?;
    Ok(())
}

fn validate_audio_artifact_name(
    path: &Path,
    step: &Map<String, Value>,
    step_name: &str,
) -> Result<(), ScenarioError> {
    let Some(name) = step.get("name").and_then(Value::as_str) else {
        return Err(validation_failure(
            path,
            &format!("{step_name} step field 'name' must be a safe artifact name"),
            json!("safe artifact name"),
            step.get("name").cloned().unwrap_or(Value::Null),
        ));
    };
    if !is_safe_artifact_segment(name.trim()) {
        return Err(validation_failure(
            path,
            &format!("{step_name} step field 'name' must be a safe artifact name"),
            json!("safe artifact name"),
            json!(name),
        ));
    }
    Ok(())
}

fn validate_position_object(
    path: &Path,
    step_name: &str,
    position: &Value,
) -> Result<(), ScenarioError> {
    let Some(position) = position.as_object() else {
        return Err(validation_failure(
            path,
            &format!("{step_name} step field 'position' must be an object"),
            json!("object with numeric x and y"),
            json!(python_type_name(position)),
        ));
    };
    for field_name in ["x", "y"] {
        if !position
            .get(field_name)
            .map(Value::is_number)
            .unwrap_or(false)
        {
            return Err(validation_failure(
                path,
                &format!("{step_name} step position field '{field_name}' must be a number"),
                json!("number"),
                position.get(field_name).cloned().unwrap_or(Value::Null),
            ));
        }
    }
    if let Some(z) = position.get("z") {
        if !z.is_number() {
            return Err(validation_failure(
                path,
                &format!("{step_name} step position field 'z' must be a number when present"),
                json!("number"),
                z.clone(),
            ));
        }
    }
    Ok(())
}

fn validate_input_key_step(value: &Value, kind: &str, path: &Path) -> Result<(), ScenarioError> {
    if is_non_empty_string(value) {
        return Ok(());
    }
    let Some(object) = value.as_object() else {
        return Err(validation_failure(
            path,
            &format!("{kind} step value must be a non-empty string or object"),
            json!("non-empty string or object"),
            value.clone(),
        ));
    };
    let allowed: &[&str] = if kind == "input_key_release" {
        &["key"]
    } else {
        &["key", "frames"]
    };
    for field in object.keys() {
        if !allowed.contains(&field.as_str()) {
            return Err(validation_failure(
                path,
                &format!("{kind} step uses unsupported field '{field}'"),
                json!(allowed),
                json!({"unknown_key": field}),
            ));
        }
    }
    if !object.get("key").map(is_non_empty_string).unwrap_or(false) {
        return Err(validation_failure(
            path,
            &format!("{kind} step field 'key' must be a non-empty string"),
            json!("non-empty string"),
            object.get("key").cloned().unwrap_or(Value::Null),
        ));
    }
    if kind != "input_key_release" {
        validate_optional_non_negative_int_field(path, kind, object, "frames")?;
    }
    Ok(())
}

fn validate_input_joy_button_step(value: &Value, path: &Path) -> Result<(), ScenarioError> {
    if is_non_empty_string(value) {
        return Ok(());
    }
    let Some(object) = value.as_object() else {
        return Err(validation_failure(
            path,
            "input_joy_button step value must be a non-empty string or object",
            json!("non-empty string or object"),
            json!(python_type_name(value)),
        ));
    };
    if !object
        .get("button")
        .map(is_non_empty_string)
        .unwrap_or(false)
    {
        return Err(validation_failure(
            path,
            "input_joy_button step field 'button' must be a non-empty string",
            json!("non-empty string"),
            object.get("button").cloned().unwrap_or(Value::Null),
        ));
    }
    validate_optional_non_negative_int_field(path, "input_joy_button", object, "device")?;
    validate_optional_non_negative_int_field(path, "input_joy_button", object, "frames")?;
    Ok(())
}

fn validate_input_joy_motion_step(value: &Value, path: &Path) -> Result<(), ScenarioError> {
    let Some(object) = value.as_object() else {
        return Err(validation_failure(
            path,
            "input_joy_motion step value must be an object",
            json!("object"),
            json!(python_type_name(value)),
        ));
    };
    if !object.get("axis").map(is_non_empty_string).unwrap_or(false) {
        return Err(validation_failure(
            path,
            "input_joy_motion step field 'axis' must be a non-empty string",
            json!("non-empty string"),
            object.get("axis").cloned().unwrap_or(Value::Null),
        ));
    }
    let raw_value = object.get("value").unwrap_or(&Value::Null);
    let Some(axis_value) = raw_value.as_f64() else {
        return Err(validation_failure(
            path,
            "input_joy_motion step field 'value' must be a number between -1.0 and 1.0",
            json!("number between -1.0 and 1.0"),
            raw_value.clone(),
        ));
    };
    if !(-1.0..=1.0).contains(&axis_value) {
        return Err(validation_failure(
            path,
            "input_joy_motion step field 'value' must be between -1.0 and 1.0",
            json!("number between -1.0 and 1.0"),
            json!(axis_value),
        ));
    }
    validate_optional_non_negative_int_field(path, "input_joy_motion", object, "device")?;
    validate_optional_non_negative_int_field(path, "input_joy_motion", object, "frames")?;
    Ok(())
}

/// Validate an `input_joy_state` step: an open-loop batch that applies many
/// devices' joypad inputs in the same frame (concurrent button holds and axis
/// positions that persist across later steps). Every entry in `inputs` is
/// validated before anything is emitted at runtime. Each entry uses the same
/// `{device, motion|button}` shape as `drive_policy` results, but there is no
/// device allowlist: any non-negative device is accepted. `frames` (default `1`)
/// is the post-emit wait; `0` emits and continues in the same frame.
fn validate_input_joy_state_step(value: &Value, path: &Path) -> Result<(), ScenarioError> {
    const INPUT_JOY_STATE_FIELDS: &[&str] = &["inputs", "frames"];

    let Some(object) = value.as_object() else {
        return Err(validation_failure(
            path,
            "input_joy_state step value must be an object",
            json!("object"),
            json!(python_type_name(value)),
        ));
    };
    for key in object.keys() {
        if !INPUT_JOY_STATE_FIELDS.contains(&key.as_str()) {
            return Err(validation_failure(
                path,
                &format!("input_joy_state step uses unsupported field '{key}'"),
                json!(INPUT_JOY_STATE_FIELDS),
                json!({ "unknown_key": key }),
            ));
        }
    }
    let Some(inputs) = object.get("inputs").and_then(Value::as_array) else {
        return Err(validation_failure(
            path,
            "input_joy_state step field 'inputs' must be a non-empty list of {device, motion|button} objects",
            json!("non-empty list of {device, motion|button} objects"),
            object.get("inputs").cloned().unwrap_or(Value::Null),
        ));
    };
    if inputs.is_empty() {
        return Err(validation_failure(
            path,
            "input_joy_state step field 'inputs' must be a non-empty list of {device, motion|button} objects",
            json!("non-empty list of {device, motion|button} objects"),
            json!(inputs),
        ));
    }
    for (index, entry) in inputs.iter().enumerate() {
        validate_joy_input_entry(path, index, entry)?;
    }
    validate_optional_non_negative_int_field(path, "input_joy_state", object, "frames")?;
    Ok(())
}

/// Validate a `joy_connect`/`joy_disconnect` step: a setup-class step that
/// simulates joypad hardware CONNECTION state (not player input). It calls the
/// public `Input.joy_connection_changed` entry point so a lobby enumerating
/// `Input.get_connected_joypads()` or listening for `joy_connection_changed`
/// actually sees the pads. `devices` is required: a non-empty list of UNIQUE
/// non-negative integers. `joy_connect` also accepts an optional non-empty string
/// `name` (default "QA Virtual Pad" runtime-side); `joy_disconnect` takes no
/// `name`. Unknown top-level fields are rejected.
fn validate_joy_connection_step(
    value: &Value,
    path: &Path,
    step_name: &str,
    allow_name: bool,
) -> Result<(), ScenarioError> {
    let allowed_fields: &[&str] = if allow_name {
        &["devices", "name"]
    } else {
        &["devices"]
    };

    let Some(object) = value.as_object() else {
        return Err(validation_failure(
            path,
            &format!("{step_name} step value must be an object"),
            json!("object"),
            json!(python_type_name(value)),
        ));
    };
    for key in object.keys() {
        if !allowed_fields.contains(&key.as_str()) {
            return Err(validation_failure(
                path,
                &format!("{step_name} step uses unsupported field '{key}'"),
                json!(allowed_fields),
                json!({ "unknown_key": key }),
            ));
        }
    }

    let Some(devices) = object.get("devices").and_then(Value::as_array) else {
        return Err(validation_failure(
            path,
            &format!(
                "{step_name} step field 'devices' must be a non-empty list of device integers"
            ),
            json!("non-empty list of unique non-negative integers"),
            object.get("devices").cloned().unwrap_or(Value::Null),
        ));
    };
    if devices.is_empty() {
        return Err(validation_failure(
            path,
            &format!(
                "{step_name} step field 'devices' must be a non-empty list of device integers"
            ),
            json!("non-empty list of unique non-negative integers"),
            json!(devices),
        ));
    }
    let mut seen: Vec<i64> = Vec::new();
    for device in devices {
        if device.as_i64().is_none() || !device.is_i64() || device.as_i64().unwrap() < 0 {
            return Err(validation_failure(
                path,
                &format!("{step_name} step 'devices' entries must be non-negative integers"),
                json!("non-negative integer"),
                device.clone(),
            ));
        }
        let device_index = device.as_i64().unwrap();
        if seen.contains(&device_index) {
            return Err(validation_failure(
                path,
                &format!("{step_name} step 'devices' entries must be unique"),
                json!("unique non-negative integers"),
                json!(device_index),
            ));
        }
        seen.push(device_index);
    }

    if allow_name {
        if let Some(name) = object.get("name") {
            if !is_non_empty_string(name) {
                return Err(validation_failure(
                    path,
                    &format!("{step_name} step field 'name' must be a non-empty string"),
                    json!("non-empty string"),
                    name.clone(),
                ));
            }
        }
    }

    Ok(())
}

/// Validate one `{device, motion|button}` joypad input entry from an
/// `input_joy_state` step. `device` must be a non-negative integer, and exactly
/// one of `motion` (`{axis, value}` with a non-empty axis and numeric value in
/// `-1.0..=1.0`) or `button` (`{name, pressed}` with a non-empty name and
/// boolean `pressed`) must be present. Axis/button names are validated for
/// non-emptiness here; the runtime resolves them against the supported vocabulary.
fn validate_joy_input_entry(path: &Path, index: usize, entry: &Value) -> Result<(), ScenarioError> {
    let Some(object) = entry.as_object() else {
        return Err(validation_failure(
            path,
            &format!("input_joy_state input {index} must be an object"),
            json!("{device, motion|button}"),
            json!(python_type_name(entry)),
        ));
    };
    let device = object.get("device").unwrap_or(&Value::Null);
    if device.as_i64().is_none() || !device.is_i64() || device.as_i64().unwrap() < 0 {
        return Err(validation_failure(
            path,
            &format!("input_joy_state input {index} field 'device' must be a non-negative integer"),
            json!("non-negative integer"),
            device.clone(),
        ));
    }
    let has_motion = object.contains_key("motion");
    let has_button = object.contains_key("button");
    if has_motion == has_button {
        return Err(validation_failure(
            path,
            &format!("input_joy_state input {index} must use exactly one of 'motion' or 'button'"),
            json!("exactly one of 'motion' or 'button'"),
            entry.clone(),
        ));
    }
    if has_motion {
        let motion = object.get("motion").unwrap_or(&Value::Null);
        let Some(motion_object) = motion.as_object() else {
            return Err(validation_failure(
                path,
                &format!("input_joy_state input {index} field 'motion' must be an object"),
                json!("{axis, value}"),
                json!(python_type_name(motion)),
            ));
        };
        if !motion_object
            .get("axis")
            .map(is_non_empty_string)
            .unwrap_or(false)
        {
            return Err(validation_failure(
                path,
                &format!("input_joy_state input {index} motion 'axis' must be a non-empty string"),
                json!("non-empty string"),
                motion_object.get("axis").cloned().unwrap_or(Value::Null),
            ));
        }
        let raw_value = motion_object.get("value").unwrap_or(&Value::Null);
        let Some(axis_value) = raw_value.as_f64() else {
            return Err(validation_failure(
                path,
                &format!(
                    "input_joy_state input {index} motion 'value' must be a number between -1.0 and 1.0"
                ),
                json!("number between -1.0 and 1.0"),
                raw_value.clone(),
            ));
        };
        if !(-1.0..=1.0).contains(&axis_value) {
            return Err(validation_failure(
                path,
                &format!(
                    "input_joy_state input {index} motion 'value' must be between -1.0 and 1.0"
                ),
                json!("number between -1.0 and 1.0"),
                json!(axis_value),
            ));
        }
        return Ok(());
    }
    let button = object.get("button").unwrap_or(&Value::Null);
    let Some(button_object) = button.as_object() else {
        return Err(validation_failure(
            path,
            &format!("input_joy_state input {index} field 'button' must be an object"),
            json!("{name, pressed}"),
            json!(python_type_name(button)),
        ));
    };
    if !button_object
        .get("name")
        .map(is_non_empty_string)
        .unwrap_or(false)
    {
        return Err(validation_failure(
            path,
            &format!("input_joy_state input {index} button 'name' must be a non-empty string"),
            json!("non-empty string"),
            button_object.get("name").cloned().unwrap_or(Value::Null),
        ));
    }
    match button_object.get("pressed") {
        Some(Value::Bool(_)) => {}
        other => {
            return Err(validation_failure(
                path,
                &format!("input_joy_state input {index} button 'pressed' must be a boolean"),
                json!("boolean"),
                other.cloned().unwrap_or(Value::Null),
            ));
        }
    }
    Ok(())
}

fn validate_optional_non_negative_int_field(
    path: &Path,
    step_name: &str,
    object: &Map<String, Value>,
    field: &str,
) -> Result<(), ScenarioError> {
    let Some(value) = object.get(field) else {
        return Ok(());
    };
    if value.as_i64().is_none() || !value.is_i64() || value.as_i64().unwrap() < 0 {
        return Err(validation_failure(
            path,
            &format!("{step_name} step field '{field}' must be a non-negative integer"),
            json!("non-negative integer"),
            value.clone(),
        ));
    }
    Ok(())
}
