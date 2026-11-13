use super::artifacts::{failure_artifacts, value_as_object_map};
use super::{
    BatchEvidence, CHECK_PROFILE_MAX_FRAME_SAMPLES, CHECK_PROFILE_TOP_SCRIPT_METHODS,
    CheckTestArtifacts, CheckTestRunOptions, ManifestTestEntry,
};
use godot_qa_core::profile;
use godot_qa_core::results::FailureDetail;
use serde_json::{Map, Value, json};
use std::fs;

pub(super) fn load_runtime_test_result(
    test_entry: &ManifestTestEntry,
    artifacts: &CheckTestArtifacts,
    batch_evidence: Option<BatchEvidence<'_>>,
) -> Result<Value, FailureDetail> {
    let raw = fs::read_to_string(&artifacts.result_path).map_err(|_| FailureDetail {
        kind: "check_result_invalid".to_string(),
        command: Some("check".to_string()),
        path: Some(artifacts.result_path.to_string_lossy().into_owned()),
        target: Some(test_entry.id.clone()),
        message: Some("check result.json was not written".to_string()),
        expected: Some(json!("JSON result written by qa_runner")),
        actual: Some(json!("missing")),
        artifacts: failure_artifacts(artifacts, batch_evidence),
        ..FailureDetail::default()
    })?;
    let value: Value = serde_json::from_str(&raw).map_err(|error| FailureDetail {
        kind: "check_result_invalid".to_string(),
        command: Some("check".to_string()),
        path: Some(artifacts.result_path.to_string_lossy().into_owned()),
        target: Some(test_entry.id.clone()),
        message: Some("check result.json was not valid JSON".to_string()),
        expected: Some(json!("valid JSON object")),
        actual: Some(json!(error.to_string())),
        artifacts: failure_artifacts(artifacts, batch_evidence),
        ..FailureDetail::default()
    })?;
    if !value.is_object() {
        return Err(FailureDetail {
            kind: "check_result_invalid".to_string(),
            command: Some("check".to_string()),
            path: Some(artifacts.result_path.to_string_lossy().into_owned()),
            target: Some(test_entry.id.clone()),
            message: Some("check result.json root must be an object".to_string()),
            expected: Some(json!("JSON object")),
            actual: Some(json!(python_type_name(&value))),
            artifacts: failure_artifacts(artifacts, batch_evidence),
            ..FailureDetail::default()
        });
    }
    Ok(value)
}

#[cfg(test)]
pub(super) fn validate_runtime_test_result(
    test_entry: &ManifestTestEntry,
    runtime_result: Value,
    artifacts: &CheckTestArtifacts,
    process_result: &godot_qa_process::GodotRunResult,
) -> (Value, Vec<FailureDetail>) {
    validate_runtime_test_result_with_evidence(
        test_entry,
        runtime_result,
        artifacts,
        process_result,
        None,
        CheckTestRunOptions::default(),
    )
}

pub(super) fn validate_runtime_test_result_with_evidence(
    test_entry: &ManifestTestEntry,
    runtime_result: Value,
    artifacts: &CheckTestArtifacts,
    process_result: &godot_qa_process::GodotRunResult,
    batch_evidence: Option<BatchEvidence<'_>>,
    options: CheckTestRunOptions,
) -> (Value, Vec<FailureDetail>) {
    let mut hard_failures = Vec::new();
    let object = runtime_result.as_object().cloned().unwrap_or_default();
    for (field_name, expected) in [
        ("id", Value::String(test_entry.id.clone())),
        ("kind", Value::String(test_entry.kind.clone())),
        ("target", Value::String(test_entry.target.clone())),
    ] {
        let Some(value) = object.get(field_name) else {
            hard_failures.push(check_result_invalid_failure(
                artifacts,
                &test_entry.id,
                &format!("check result field '{field_name}' must be a string"),
                json!("string"),
                json!("NoneType"),
                batch_evidence,
            ));
            continue;
        };
        let Some(value_string) = value.as_str() else {
            hard_failures.push(check_result_invalid_failure(
                artifacts,
                &test_entry.id,
                &format!("check result field '{field_name}' must be a string"),
                json!("string"),
                json!(python_type_name(value)),
                batch_evidence,
            ));
            continue;
        };
        if value_string != expected.as_str().unwrap_or_default() {
            hard_failures.push(check_result_invalid_failure(
                artifacts,
                &test_entry.id,
                &format!("check result field '{field_name}' must match the requested test"),
                expected,
                json!(value_string),
                batch_evidence,
            ));
        }
    }
    let ok = match object.get("ok").and_then(Value::as_bool) {
        Some(ok) => ok,
        None => {
            hard_failures.push(check_result_invalid_failure(
                artifacts,
                &test_entry.id,
                "check result field 'ok' must be a boolean",
                json!("boolean"),
                json!(python_type_name(object.get("ok").unwrap_or(&Value::Null))),
                batch_evidence,
            ));
            false
        }
    };
    let duration_ms = match object.get("duration_ms").and_then(Value::as_i64) {
        Some(duration_ms) if duration_ms >= 0 => json!(duration_ms),
        _ => {
            hard_failures.push(check_result_invalid_failure(
                artifacts,
                &test_entry.id,
                "check result field 'duration_ms' must be a non-negative integer",
                json!("non-negative integer"),
                object.get("duration_ms").cloned().unwrap_or(Value::Null),
                batch_evidence,
            ));
            Value::Null
        }
    };
    let runtime_errors = object.get("runtime_errors").cloned().unwrap_or(Value::Null);
    if !runtime_errors.is_array()
        || runtime_errors
            .as_array()
            .is_some_and(|entries| entries.iter().any(|entry| !entry.is_object()))
    {
        hard_failures.push(check_result_invalid_failure(
            artifacts,
            &test_entry.id,
            "check result field 'runtime_errors' must be an array of objects",
            json!("JSON array of objects"),
            json!(python_type_name(&runtime_errors)),
            batch_evidence,
        ));
    }
    let failure_values = object.get("failures").cloned().unwrap_or(Value::Null);
    if !failure_values.is_array()
        || failure_values
            .as_array()
            .is_some_and(|entries| entries.iter().any(|entry| !entry.is_object()))
    {
        hard_failures.push(check_result_invalid_failure(
            artifacts,
            &test_entry.id,
            "check result field 'failures' must be an array of objects",
            json!("JSON array of objects"),
            json!(python_type_name(&failure_values)),
            batch_evidence,
        ));
    }
    let data = object.get("data").cloned();
    if data.as_ref().is_some_and(|value| !value.is_object()) {
        hard_failures.push(check_result_invalid_failure(
            artifacts,
            &test_entry.id,
            "check result field 'data' must be an object when present",
            json!("JSON object"),
            json!(python_type_name(data.as_ref().unwrap_or(&Value::Null))),
            batch_evidence,
        ));
    }
    let mut profile_payload = None;
    if options.profile_enabled {
        match validate_check_profile_payload(
            test_entry,
            artifacts,
            object.get("profile"),
            batch_evidence,
        ) {
            Ok(profile) => profile_payload = Some(profile),
            Err(failure) => hard_failures.push(failure),
        }
    }
    if !hard_failures.is_empty() {
        return (
            json!({
                "id": test_entry.id,
                "kind": test_entry.kind,
                "target": test_entry.target,
                "tags": test_entry.tags,
                "ok": false,
                "duration_ms": Value::Null,
                "runtime_errors": [],
                "artifacts": artifacts.to_json(),
                "failures": hard_failures.clone(),
            }),
            hard_failures,
        );
    }

    let mut failures = failure_values
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_object)
        .map(|failure| normalized_failure_from_value(&test_entry.id, artifacts, failure))
        .collect::<Vec<_>>();
    if process_result.returncode != 0 && failures.is_empty() {
        let mut failure_artifacts = value_as_object_map(artifacts.to_json()).unwrap_or_default();
        if process_result.timed_out {
            failure_artifacts.insert(
                "process".to_string(),
                json!({
                    "timed_out": process_result.timed_out,
                    "timeout_ms": process_result.timeout_ms,
                    "cleanup": process_result.cleanup.as_ref().map(|cleanup| json!({
                        "status": cleanup.status,
                        "failure_kind": cleanup.failure_kind,
                        "pid": cleanup.pid,
                        "process_group": cleanup.process_group,
                        "owned": cleanup.owned,
                        "message": cleanup.message,
                    })),
                    "infrastructure_failure_kind": process_result.infrastructure_failure_kind,
                }),
            );
        }
        failures.push(FailureDetail {
            kind: if process_result.timed_out {
                "owned_process_timeout".to_string()
            } else {
                "godot_run_failed".to_string()
            },
            command: Some(godot_qa_process::format_command(&process_result.command)),
            path: Some(artifacts.logs_path.to_string_lossy().into_owned()),
            target: Some(test_entry.id.clone()),
            expected: Some(json!("Godot check exits with status 0")),
            actual: Some(if process_result.timed_out {
                json!({
                    "timed_out": true,
                    "timeout_ms": process_result.timeout_ms,
                    "exit_code": process_result.returncode,
                })
            } else {
                json!(process_result.returncode)
            }),
            details: [("termination".to_string(), json!(process_result.termination))]
                .into_iter()
                .collect(),
            artifacts: Some(failure_artifacts),
            ..FailureDetail::default()
        });
    }

    let mut result = Map::from_iter([
        ("id".to_string(), json!(test_entry.id)),
        ("kind".to_string(), json!(test_entry.kind)),
        ("target".to_string(), json!(test_entry.target)),
        ("tags".to_string(), json!(test_entry.tags)),
        ("ok".to_string(), json!(ok && failures.is_empty())),
        ("duration_ms".to_string(), duration_ms),
        (
            "display_server".to_string(),
            object.get("display_server").cloned().unwrap_or(Value::Null),
        ),
        ("runtime_errors".to_string(), runtime_errors.clone()),
        ("artifacts".to_string(), artifacts.to_json()),
        (
            "failures".to_string(),
            serde_json::to_value(&failures).unwrap_or_else(|_| json!([])),
        ),
    ]);
    if let Some(data) = data {
        result.insert("data".to_string(), data);
    }
    if let Some(profile_payload) = profile_payload {
        result.insert("profile".to_string(), profile_payload);
    }
    if let Some(time_control) = object.get("time_control") {
        result.insert("time_control".to_string(), time_control.clone());
    }
    if let Some(frame_pacing) = &test_entry.frame_pacing {
        result.insert("frame_pacing".to_string(), frame_pacing.to_json());
    }
    (Value::Object(result), failures)
}

pub(super) fn normalized_failure(
    test_id: &str,
    artifacts: &CheckTestArtifacts,
    failure: &FailureDetail,
) -> FailureDetail {
    let mut normalized = failure.clone();
    normalized.command = Some("check".to_string());
    normalized.target = Some(test_id.to_string());
    normalized.artifacts = value_as_object_map(artifacts.to_json());
    normalized
}

fn validate_check_profile_payload(
    test_entry: &ManifestTestEntry,
    artifacts: &CheckTestArtifacts,
    profile_value: Option<&Value>,
    batch_evidence: Option<BatchEvidence<'_>>,
) -> Result<Value, FailureDetail> {
    let Some(profile_value) = profile_value else {
        return Err(profile_result_invalid_failure(
            artifacts,
            &test_entry.id,
            "profile payload is required when check profiling is enabled",
            json!("result.json profile object"),
            json!("missing"),
            batch_evidence,
        ));
    };
    let Some(profile_object) = profile_value.as_object() else {
        return Err(profile_result_invalid_failure(
            artifacts,
            &test_entry.id,
            "profile payload must be an object",
            json!("result.json profile object"),
            json!(python_type_name(profile_value)),
            batch_evidence,
        ));
    };
    let frame_times_ms = profile::optional_number_array(profile_object.get("frame_times_ms"))
        .map_err(|actual| {
            profile_result_invalid_failure(
                artifacts,
                &test_entry.id,
                "profile.frame_times_ms must be an array of non-negative numbers",
                json!("array of non-negative numbers"),
                actual,
                batch_evidence,
            )
        })?;
    let monitor_samples = profile::optional_object_array(profile_object.get("monitor_samples"))
        .map_err(|actual| {
            profile_result_invalid_failure(
                artifacts,
                &test_entry.id,
                "profile.monitor_samples must be an array of objects",
                json!("array of objects"),
                actual,
                batch_evidence,
            )
        })?;
    let step_timings =
        profile::optional_object_array(profile_object.get("step_timings")).map_err(|actual| {
            profile_result_invalid_failure(
                artifacts,
                &test_entry.id,
                "profile.step_timings must be an array of objects",
                json!("array of objects"),
                actual,
                batch_evidence,
            )
        })?;
    Ok(json!({
        "frame_times_ms": frame_times_ms,
        "monitor_samples": monitor_samples,
        "step_timings": step_timings,
        "top_script_methods": profile::top_script_methods_or_empty(profile_object.get("top_script_methods")),
        "unsupported_monitors": profile_object.get("unsupported_monitors").cloned().unwrap_or_else(|| json!([])),
        "sample_limits": profile_object.get("sample_limits").cloned().unwrap_or_else(|| json!({
            "max_frame_samples": CHECK_PROFILE_MAX_FRAME_SAMPLES,
            "top_script_methods": CHECK_PROFILE_TOP_SCRIPT_METHODS,
        })),
    }))
}

fn profile_result_invalid_failure(
    artifacts: &CheckTestArtifacts,
    test_id: &str,
    message: &str,
    expected: Value,
    actual: Value,
    batch_evidence: Option<BatchEvidence<'_>>,
) -> FailureDetail {
    let mut artifact_map = failure_artifacts(artifacts, batch_evidence).unwrap_or_default();
    artifact_map.insert("profile".to_string(), json!(artifacts.result_path));
    FailureDetail {
        kind: "profile_result_invalid".to_string(),
        command: Some("check".to_string()),
        path: Some(artifacts.result_path.to_string_lossy().into_owned()),
        target: Some(test_id.to_string()),
        message: Some(message.to_string()),
        expected: Some(expected),
        actual: Some(actual),
        artifacts: Some(artifact_map),
        ..FailureDetail::default()
    }
}

fn normalized_failure_from_value(
    test_id: &str,
    artifacts: &CheckTestArtifacts,
    failure: &Map<String, Value>,
) -> FailureDetail {
    let mut merged_artifacts = value_as_object_map(artifacts.to_json()).unwrap_or_default();
    if let Some(extra_artifacts) = failure.get("artifacts").and_then(Value::as_object) {
        for (key, value) in extra_artifacts {
            merged_artifacts.insert(key.clone(), value.clone());
        }
    }
    FailureDetail {
        kind: failure
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or("runtime_error")
            .to_string(),
        command: Some("check".to_string()),
        path: failure
            .get("path")
            .and_then(Value::as_str)
            .map(str::to_string),
        target: failure
            .get("target")
            .and_then(Value::as_str)
            .map(str::to_string)
            .or_else(|| Some(test_id.to_string())),
        message: failure
            .get("message")
            .and_then(Value::as_str)
            .map(str::to_string),
        expected: failure.get("expected").cloned(),
        actual: failure.get("actual").cloned(),
        severity: failure
            .get("severity")
            .and_then(Value::as_str)
            .map(str::to_string),
        artifacts: Some(merged_artifacts),
        ..FailureDetail::default()
    }
}

fn check_result_invalid_failure(
    artifacts: &CheckTestArtifacts,
    test_id: &str,
    message: &str,
    expected: Value,
    actual: Value,
    batch_evidence: Option<BatchEvidence<'_>>,
) -> FailureDetail {
    FailureDetail {
        kind: "check_result_invalid".to_string(),
        command: Some("check".to_string()),
        path: Some(artifacts.result_path.to_string_lossy().into_owned()),
        target: Some(test_id.to_string()),
        message: Some(message.to_string()),
        expected: Some(expected),
        actual: Some(actual),
        artifacts: failure_artifacts(artifacts, batch_evidence),
        ..FailureDetail::default()
    }
}

pub(super) fn python_type_name(value: &Value) -> &'static str {
    match value {
        Value::Null => "NoneType",
        Value::Bool(_) => "bool",
        Value::Number(number) => {
            if number.is_i64() || number.is_u64() {
                "int"
            } else {
                "float"
            }
        }
        Value::String(_) => "str",
        Value::Array(_) => "list",
        Value::Object(_) => "dict",
    }
}
