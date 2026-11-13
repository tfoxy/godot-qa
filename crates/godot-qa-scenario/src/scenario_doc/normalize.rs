use super::{ScenarioError, load_scenario_document, validate_scenario_document};
use crate::time_control::validate_time_control_for_scenario;
use serde_json::{Map, Value, json};
use std::path::Path;

pub fn normalize_scenario_document(data: &Value) -> Value {
    let Some(document) = data.as_object() else {
        return json!({});
    };

    let steps = document
        .get("steps")
        .and_then(Value::as_array)
        .map(|steps| steps.iter().map(normalize_step).collect::<Vec<_>>())
        .unwrap_or_default();
    let assertions = document
        .get("assert")
        .and_then(Value::as_array)
        .map(|assertions| {
            assertions
                .iter()
                .map(normalize_assertion)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    let mut normalized = json!({
        "name": document.get("name").and_then(Value::as_str).unwrap_or_default().trim(),
        "scene": document.get("scene").and_then(Value::as_str).unwrap_or_default().trim(),
        "viewport": clone_object_or_default(document.get("viewport")),
        "determinism": clone_object_or_default(document.get("determinism")),
        "snapshots": normalize_snapshots(document.get("snapshots")),
        "steps": steps,
        "assert": assertions,
        "viewports": clone_array_or_default(document.get("viewports")),
    });
    if let Some(godot_args) = document.get("godot_args").and_then(Value::as_array) {
        normalized["godot_args"] = Value::Array(godot_args.clone());
    }
    if let Some(project_args) = document.get("project_args").and_then(Value::as_array) {
        normalized["project_args"] = Value::Array(project_args.clone());
    }
    for field in crate::run::SCENARIO_TIMEOUT_FIELDS {
        if let Some(timeout) = document.get(field) {
            normalized[field] = timeout.clone();
        }
    }
    if let Some(threshold) = document.get("slow_summary_threshold_ms") {
        normalized["slow_summary_threshold_ms"] = threshold.clone();
    }
    if let Some(slow_summary) = document.get("slow_summary").and_then(Value::as_object) {
        normalized["slow_summary"] = Value::Object(slow_summary.clone());
    }
    if let Some(policy) = document
        .get("process_outcome_policy")
        .and_then(Value::as_object)
    {
        normalized["process_outcome_policy"] = Value::Object(policy.clone());
    }
    if let Some(execution) = document.get("execution").and_then(Value::as_object) {
        normalized["execution"] = Value::Object(execution.clone());
    }
    if let Some(time_control) = document.get("time_control") {
        if let Ok(settings) = validate_time_control_for_scenario(time_control, Path::new("")) {
            normalized["time_control"] = settings.to_normalized_value();
        }
    }
    normalized
}

pub fn load_and_normalize_scenario_document(path: &Path) -> Result<Value, ScenarioError> {
    let document = load_scenario_document(path)?;
    validate_scenario_document(&document, path)?;
    Ok(normalize_scenario_document(&document))
}

fn normalize_step(step: &Value) -> Value {
    let Some(step) = step.as_object() else {
        return step.clone();
    };
    let mut normalized = step.clone();
    if let Some(snapshot) = normalized.get_mut("snapshot") {
        if let Some(snapshot_name) = snapshot.as_str() {
            *snapshot = json!(snapshot_name.trim());
        }
    }
    if let Some(node_inspect) = normalized.get_mut("node_inspect") {
        if let Some(target) = node_inspect.as_str() {
            *node_inspect = json!(target.trim());
        }
    }
    if let Some(record_state) = normalized.get_mut("record_state") {
        trim_step_fields(
            record_state,
            &[
                "name",
                "source",
                "provider",
                "provider_id",
                "method",
                "target",
                "path",
            ],
        );
    }
    if let Some(teleport_node) = normalized.get_mut("teleport_node") {
        trim_step_fields(teleport_node, &["target", "to"]);
        trim_to_point_fields(teleport_node);
    }
    if let Some(fixture_teleport) = normalized.get_mut("fixture_teleport") {
        trim_step_fields(fixture_teleport, &["target", "to"]);
        trim_to_point_fields(fixture_teleport);
    }
    if let Some(spawn_scene) = normalized.get_mut("spawn_scene") {
        trim_step_fields(spawn_scene, &["scene", "parent", "name", "qa_id", "at"]);
    }
    if let Some(set_property) = normalized.get_mut("set_property") {
        trim_step_fields(set_property, &["target", "property"]);
    }
    if let Some(call_method) = normalized.get_mut("call_method") {
        trim_step_fields(call_method, &["target", "method"]);
    }
    if let Some(fixture_call) = normalized.get_mut("fixture_call") {
        trim_step_fields(fixture_call, &["driver", "action"]);
    }
    for kind in ["input_key", "input_key_hold", "input_key_release"] {
        if let Some(value) = normalized.get_mut(kind) {
            if let Some(key) = value.as_str() {
                *value = json!(key.trim());
            } else {
                trim_step_fields(value, &["key"]);
            }
        }
    }
    if let Some(input_joy_button) = normalized.get_mut("input_joy_button") {
        if let Some(button) = input_joy_button.as_str() {
            *input_joy_button = json!(button.trim());
        } else {
            trim_step_fields(input_joy_button, &["button"]);
        }
    }
    if let Some(input_joy_motion) = normalized.get_mut("input_joy_motion") {
        trim_step_fields(input_joy_motion, &["axis"]);
    }
    if let Some(drive_policy) = normalized.get_mut("drive_policy") {
        trim_step_fields(drive_policy, &["policy"]);
    }
    if let Some(audio_capture) = normalized.get_mut("audio_capture") {
        trim_step_fields(audio_capture, &["name", "bus"]);
    }
    if let Some(audio_export) = normalized.get_mut("audio_export") {
        trim_step_fields(audio_export, &["name", "target", "method"]);
    }
    if let Some(assertions) = normalized.get_mut("assert").and_then(Value::as_array_mut) {
        for assertion in assertions {
            *assertion = normalize_assertion(assertion);
        }
    }
    Value::Object(normalized)
}

fn normalize_snapshots(value: Option<&Value>) -> Value {
    let mut snapshots = clone_object_or_default(value);
    if let Some(object) = snapshots.as_object_mut() {
        if let Some(target) = object.get_mut("target")
            && let Some(raw) = target.as_str()
        {
            *target = json!(raw.trim());
        }
        if let Some(metadata_keys) = object
            .get_mut("metadata_keys")
            .and_then(Value::as_array_mut)
        {
            for key in metadata_keys {
                if let Some(raw) = key.as_str() {
                    *key = json!(raw.trim());
                }
            }
        }
    }
    snapshots
}

fn trim_step_fields(step_value: &mut Value, fields: &[&str]) {
    let Some(step) = step_value.as_object_mut() else {
        return;
    };
    for field in fields {
        if let Some(value) = step.get_mut(*field) {
            if let Some(raw) = value.as_str() {
                *value = json!(raw.trim());
            }
        }
    }
}

fn trim_to_point_fields(step_value: &mut Value) {
    if let Some(to_point) = step_value
        .as_object_mut()
        .and_then(|step| step.get_mut("to_point"))
    {
        trim_step_fields(to_point, &["provider", "method", "id"]);
    }
}

fn trim_to_point_fields_named(value: &mut Value, fields: &[&str]) {
    let Some(object) = value.as_object_mut() else {
        return;
    };
    for field in fields {
        if let Some(point) = object.get_mut(*field) {
            trim_step_fields(point, &["provider", "method", "id"]);
        }
    }
}

fn normalize_assertion(assertion: &Value) -> Value {
    let Some(assertion) = assertion.as_object() else {
        return assertion.clone();
    };
    let mut normalized = assertion.clone();
    if let Some(layout_audit) = normalized.get_mut("layout_audit") {
        if let Some(mode) = layout_audit.as_str() {
            *layout_audit = json!(mode.trim());
        }
    }
    if let Some(screenshot_match) = normalized.get_mut("screenshot_match") {
        if let Some(screenshot_match) = screenshot_match.as_object_mut() {
            if let Some(snapshot) = screenshot_match.get_mut("snapshot") {
                if let Some(snapshot_name) = snapshot.as_str() {
                    *snapshot = json!(snapshot_name.trim());
                }
            }
            if let Some(baseline) = screenshot_match.get_mut("baseline") {
                if let Some(path) = baseline.as_str() {
                    *baseline = json!(path.trim());
                }
            }
        }
    }
    if let Some(text_equals) = normalized.get_mut("text_equals") {
        trim_assertion_fields(text_equals, &["target"]);
    }
    if let Some(text_contains) = normalized.get_mut("text_contains") {
        trim_assertion_fields(text_contains, &["target"]);
    }
    if let Some(max_delta_position) = normalized.get_mut("max_delta_position") {
        trim_assertion_fields(max_delta_position, &["target"]);
        if let Some(max_delta_position) = max_delta_position.as_object_mut() {
            if !max_delta_position.contains_key("max")
                && let Some(max_delta) = max_delta_position.get("max_delta").cloned()
            {
                max_delta_position.insert("max".to_string(), max_delta);
            }
            if !max_delta_position.contains_key("window_frames")
                && !max_delta_position.contains_key("window_seconds")
            {
                max_delta_position.insert("window_frames".to_string(), json!(1));
            }
        }
    }
    if let Some(follows) = normalized.get_mut("follows") {
        trim_assertion_fields(follows, &["leader", "follower"]);
    }
    if let Some(within_limits) = normalized.get_mut("within_limits") {
        trim_assertion_fields(
            within_limits,
            &[
                "target",
                "source",
                "provider",
                "provider_id",
                "method",
                "path",
            ],
        );
    }
    if let Some(trend) = normalized.get_mut("trend") {
        trim_assertion_fields(
            trend,
            &[
                "target",
                "trend",
                "property",
                "field",
                "source",
                "provider",
                "provider_id",
                "method",
                "path",
            ],
        );
    }
    if let Some(distance) = normalized.get_mut("distance") {
        trim_assertion_fields(distance, &["a", "b"]);
        trim_to_point_fields_named(
            distance,
            &["a_point", "b_point", "target_point", "other_point"],
        );
    }
    if let Some(distance_between) = normalized.remove("distance_between") {
        let mut distance = distance_between;
        trim_assertion_fields(&mut distance, &["a", "b"]);
        trim_to_point_fields_named(
            &mut distance,
            &["a_point", "b_point", "target_point", "other_point"],
        );
        if let Some(distance) = distance.as_object_mut() {
            if !distance.contains_key("min")
                && let Some(min_distance) = distance.get("min_distance").cloned()
            {
                distance.insert("min".to_string(), min_distance);
            }
            if !distance.contains_key("max")
                && let Some(max_distance) = distance.get("max_distance").cloned()
            {
                distance.insert("max".to_string(), max_distance);
            }
        }
        normalized.insert("distance".to_string(), distance);
    }
    if let Some(distance_changed) = normalized.get_mut("distance_changed") {
        trim_assertion_fields(distance_changed, &["a", "b"]);
    }
    if let Some(visible_pixels) = normalized.get_mut("visible_pixels") {
        trim_assertion_fields(visible_pixels, &["target", "snapshot"]);
    }
    if let Some(saved_snapshot_pixels) = normalized.get_mut("saved_snapshot_pixels") {
        trim_assertion_fields(saved_snapshot_pixels, &["target", "snapshot"]);
    }
    if let Some(not_occluded) = normalized.get_mut("not_occluded") {
        trim_assertion_fields(not_occluded, &["target", "occluder", "snapshot"]);
    }
    if let Some(renders_above) = normalized.get_mut("renders_above") {
        trim_assertion_fields(renders_above, &["target", "other", "snapshot", "a", "b"]);
        normalize_render_order_aliases(renders_above);
    }
    if let Some(renders_below) = normalized.get_mut("renders_below") {
        trim_assertion_fields(renders_below, &["target", "other", "snapshot", "a", "b"]);
        normalize_render_order_aliases(renders_below);
    }
    if let Some(never) = normalized.get_mut("never") {
        trim_assertion_fields(
            never,
            &[
                "target",
                "property",
                "text",
                "token",
                "source",
                "provider",
                "provider_id",
                "method",
                "path",
            ],
        );
        if let Some(never) = never.as_object_mut() {
            if let Some(forbidden) = never.get_mut("forbidden") {
                match forbidden {
                    Value::String(raw) => {
                        *forbidden = json!(raw.trim());
                    }
                    Value::Array(values) => {
                        for value in values {
                            if let Some(raw) = value.as_str() {
                                *value = json!(raw.trim());
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
    }
    if let Some(observe_until) = normalized.get_mut("observe_until") {
        trim_assertion_fields(observe_until, &["target", "method", "source", "property"]);
        if let Some(observe_until) = observe_until.as_object_mut()
            && let Some(matches) = observe_until
                .get_mut("matches")
                .and_then(Value::as_object_mut)
        {
            let trimmed_entries = matches
                .iter()
                .map(|(key, value)| (key.trim().to_string(), value.clone()))
                .collect::<Map<_, _>>();
            *matches = trimmed_entries;
        }
    }
    if let Some(surface_probe) = normalized.get_mut("surface_probe") {
        trim_assertion_fields(
            surface_probe,
            &[
                "target",
                "provider",
                "provider_id",
                "provider_method",
                "coordinate_space",
                "surface",
                "probe_id",
                "expected_surface",
            ],
        );
        if let Some(surface_probe) = surface_probe.as_object_mut()
            && let Some(points) = surface_probe
                .get_mut("points")
                .and_then(Value::as_array_mut)
        {
            for point in points {
                if let Some(point) = point.as_object_mut()
                    && let Some(expected) = point.get_mut("expected")
                    && let Some(raw) = expected.as_str()
                {
                    *expected = json!(raw.trim());
                }
            }
        }
    }
    if let Some(surface_matches) = normalized.get_mut("surface_matches") {
        trim_assertion_fields(surface_matches, &["target", "surface", "expected"]);
        if let Some(surface_matches) = surface_matches.as_object_mut()
            && !surface_matches.contains_key("surface")
            && let Some(expected) = surface_matches.get("expected").cloned()
        {
            surface_matches.insert("surface".to_string(), expected);
        }
    }
    Value::Object(normalized)
}

fn trim_assertion_fields(assertion_value: &mut Value, fields: &[&str]) {
    let Some(assertion) = assertion_value.as_object_mut() else {
        return;
    };
    for field in fields {
        if let Some(value) = assertion.get_mut(*field) {
            if let Some(raw) = value.as_str() {
                *value = json!(raw.trim());
            }
        }
    }
}

fn normalize_render_order_aliases(assertion_value: &mut Value) {
    let Some(assertion) = assertion_value.as_object_mut() else {
        return;
    };
    if !assertion.contains_key("target")
        && let Some(a) = assertion.get("a").cloned()
    {
        assertion.insert("target".to_string(), a);
    }
    if !assertion.contains_key("other")
        && let Some(b) = assertion.get("b").cloned()
    {
        assertion.insert("other".to_string(), b);
    }
}

fn clone_object_or_default(value: Option<&Value>) -> Value {
    match value {
        Some(Value::Object(map)) => Value::Object(map.clone()),
        _ => json!({}),
    }
}

fn clone_array_or_default(value: Option<&Value>) -> Value {
    match value {
        Some(Value::Array(items)) => Value::Array(items.clone()),
        _ => json!([]),
    }
}
