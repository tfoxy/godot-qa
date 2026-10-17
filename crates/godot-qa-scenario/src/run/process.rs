// Extracted from run.rs; included by mod.rs to preserve the original module namespace.

fn scenario_godot_args(scenario: &Value) -> Vec<String> {
    scenario
        .get("godot_args")
        .and_then(Value::as_array)
        .map(|args| {
            args.iter()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default()
}

pub fn scenario_project_args(scenario: &Value) -> Vec<String> {
    scenario
        .get("project_args")
        .and_then(Value::as_array)
        .map(|args| {
            args.iter()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default()
}

fn merged_launch_args(default_args: &[String], specific_args: &[String]) -> Vec<String> {
    let mut args = Vec::with_capacity(default_args.len() + specific_args.len());
    args.extend(default_args.iter().cloned());
    args.extend(specific_args.iter().cloned());
    args
}

fn scenario_with_effective_launch_args(config: &ProjectConfig, scenario: &Value) -> Value {
    let godot_args = merged_launch_args(&config.godot_args, &scenario_godot_args(scenario));
    let project_args =
        merged_launch_args(&config.godot_project_args, &scenario_project_args(scenario));
    scenario_with_launch_args(scenario, &godot_args, &project_args)
}

fn scenario_with_launch_args(
    scenario: &Value,
    godot_args: &[String],
    project_args: &[String],
) -> Value {
    let mut scenario = scenario.clone();
    if let Some(object) = scenario.as_object_mut() {
        object.insert("godot_args".to_string(), json!(godot_args));
        object.insert("project_args".to_string(), json!(project_args));
    }
    scenario
}

pub(crate) const SCENARIO_TIMEOUT_FIELDS: [&str; 4] = [
    "timeout_seconds",
    "timeout",
    "run_timeout_seconds",
    "run_timeout",
];
const SCENARIO_TIMEOUT_SECONDS_FIELDS: [&str; 2] = ["timeout_seconds", "run_timeout_seconds"];
const SCENARIO_TIMEOUT_STRING_FIELDS: [&str; 2] = ["timeout", "run_timeout"];

pub(crate) fn declared_scenario_timeout_fields(scenario: &Value) -> Vec<&'static str> {
    SCENARIO_TIMEOUT_FIELDS
        .iter()
        .copied()
        .filter(|field| scenario.get(field).is_some())
        .collect()
}

pub(crate) fn parse_timeout_seconds_value(value: &Value) -> Option<f64> {
    let seconds = value.as_f64()?;
    (seconds.is_finite() && seconds > 0.0).then_some(seconds)
}

pub(crate) fn parse_timeout_duration_string(value: &Value) -> Option<f64> {
    let raw = value.as_str().map(str::trim)?;
    let (number, multiplier) = if let Some(number) = raw.strip_suffix("ms") {
        (number, 0.001)
    } else if let Some(number) = raw.strip_suffix('s') {
        (number, 1.0)
    } else if let Some(number) = raw.strip_suffix('m') {
        (number, 60.0)
    } else {
        return None;
    };
    let amount = number.trim().parse::<f64>().ok()?;
    let seconds = amount * multiplier;
    (seconds.is_finite() && seconds > 0.0).then_some(seconds)
}

fn scenario_timeout_duration(
    command: &str,
    scenario_path: &Path,
    scenario: &Value,
) -> Result<Option<Duration>, FailureDetail> {
    let declared = declared_scenario_timeout_fields(scenario);
    if declared.len() > 1 {
        return Err(scenario_timeout_validation_failure(
            command,
            scenario_path,
            json!(declared),
            "scenario timeout must use exactly one of timeout_seconds, timeout, run_timeout_seconds, or run_timeout",
        ));
    }
    for field in SCENARIO_TIMEOUT_SECONDS_FIELDS {
        if let Some(value) = scenario.get(field) {
            return parse_timeout_seconds_value(value)
                .map(|seconds| Some(Duration::from_secs_f64(seconds)))
                .ok_or_else(|| {
                    invalid_scenario_timeout_failure(command, scenario_path, value.clone())
                });
        }
    }
    for field in SCENARIO_TIMEOUT_STRING_FIELDS {
        if let Some(value) = scenario.get(field) {
            return parse_timeout_duration_string(value)
                .map(|seconds| Some(Duration::from_secs_f64(seconds)))
                .ok_or_else(|| {
                    invalid_scenario_timeout_failure(command, scenario_path, value.clone())
                });
        }
    }
    Ok(None)
}

/// Frame budget the engine enforces instead of trusting wall clocks: on hosts under load,
/// `--fixed-fps` frames run at CPU speed, so only a frame count is load-independent.
/// Present iff the scenario declares a run timeout AND frame pacing is active (no
/// `time_control`); undeclared/time_control scenarios keep pure wall-clock behavior.
pub(crate) fn scenario_frame_budget_json(scenario: &Value, project_root: &Path) -> Option<Value> {
    let timeout = scenario_timeout_duration("scenario run", Path::new(""), scenario)
        .ok()
        .flatten()?;
    let pacing = scenario_frame_pacing(scenario, project_root).ok().flatten()?;
    let timeout_seconds = timeout.as_secs_f64();
    let frames = (timeout_seconds * pacing.fixed_fps as f64).ceil().max(1.0) as u64;
    Some(json!({
        "frames": frames,
        "timeout_seconds": timeout_seconds,
        "fixed_fps": pacing.fixed_fps,
        "source": pacing.source,
    }))
}

fn invalid_scenario_timeout_failure(
    command: &str,
    scenario_path: &Path,
    actual: Value,
) -> FailureDetail {
    scenario_timeout_validation_failure(
        command,
        scenario_path,
        actual,
        "scenario timeout must be a positive finite duration",
    )
}

fn scenario_timeout_validation_failure(
    command: &str,
    scenario_path: &Path,
    actual: Value,
    message: &str,
) -> FailureDetail {
    FailureDetail {
        kind: "scenario_validation_failed".to_string(),
        command: Some(command.to_string()),
        path: Some(scenario_path.to_string_lossy().into_owned()),
        message: Some(message.to_string()),
        expected: Some(json!(
            "positive number of seconds or duration string with ms, s, or m suffix"
        )),
        actual: Some(actual),
        ..FailureDetail::default()
    }
}

fn duration_millis_u64(duration: Duration) -> u64 {
    u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
}

pub fn redact_project_args_for_metadata(project_args: &Value) -> Value {
    let Some(args) = project_args.as_array() else {
        return json!([]);
    };
    let mut redacted = Vec::with_capacity(args.len());
    let mut redact_next = false;
    for arg in args {
        let Some(arg) = arg.as_str() else {
            redacted.push(arg.clone());
            redact_next = false;
            continue;
        };
        if redact_next {
            redacted.push(json!(redacted_arg_value(arg)));
            redact_next = false;
            continue;
        }
        if let Some((key, value)) = arg.split_once('=') {
            if is_secret_bearing_key(key) {
                redacted.push(json!(format!("{key}={}", redacted_arg_value(value))));
                continue;
            }
        }
        if is_secret_bearing_key(arg) {
            redacted.push(json!(arg));
            redact_next = true;
            continue;
        }
        redacted.push(json!(arg));
    }
    Value::Array(redacted)
}

fn redact_command_args_for_metadata(args: &[String]) -> Vec<String> {
    let redacted = redact_project_args_for_metadata(&json!(args));
    redacted
        .as_array()
        .map(|values| {
            values
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

fn is_secret_bearing_key(key: &str) -> bool {
    let normalized = key
        .trim_start_matches('-')
        .chars()
        .filter(|ch| *ch != '-' && *ch != '_')
        .collect::<String>()
        .to_ascii_lowercase();
    matches!(
        normalized.as_str(),
        "apikey"
            | "accesstoken"
            | "authtoken"
            | "clientsecret"
            | "credential"
            | "credentials"
            | "jwt"
            | "password"
            | "passwd"
            | "secret"
            | "session"
            | "sessionid"
            | "token"
    ) || normalized.ends_with("token")
        || normalized.ends_with("secret")
        || normalized.ends_with("password")
}

fn redacted_arg_value(value: &str) -> String {
    format!("<redacted:{}>", value.chars().count())
}


fn progress_evidence_summary_json(progress_path: &str) -> Value {
    let record_count = fs::read_to_string(progress_path)
        .map(|contents| {
            contents
                .lines()
                .filter(|line| !line.trim().is_empty())
                .count()
        })
        .unwrap_or(0);
    json!({
        "path": progress_path,
        "record_count": record_count,
        "exists": Path::new(progress_path).is_file(),
    })
}

fn slow_scenario_summary_json(
    scenario: &Value,
    progress_path: &str,
    artifact_path: &Path,
) -> Value {
    let threshold_ms = scenario_slow_summary_threshold_ms(scenario);
    let mut slow_operations = fs::read_to_string(progress_path)
        .map(|contents| {
            contents
                .lines()
                .filter_map(|line| serde_json::from_str::<Value>(line).ok())
                .filter(|event| event.get("event").and_then(Value::as_str) == Some("finish"))
                .filter_map(|event| {
                    let elapsed_ms = event.get("elapsed_ms").and_then(Value::as_f64)?;
                    if elapsed_ms < threshold_ms as f64 {
                        return None;
                    }
                    Some(json!({
                        "operation": event.get("operation").cloned().unwrap_or(Value::Null),
                        "index": event.get("index").cloned().unwrap_or(Value::Null),
                        "kind": event.get("kind").cloned().unwrap_or(Value::Null),
                        "command": event.get("command").cloned().unwrap_or(Value::Null),
                        "target": event.get("target").cloned().unwrap_or(Value::Null),
                        "ok": event.get("ok").cloned().unwrap_or(Value::Null),
                        "elapsed_ms": elapsed_ms,
                        "threshold_ms": threshold_ms,
                        "artifacts": event.get("artifacts").cloned().unwrap_or_else(|| json!({"progress": progress_path})),
                    }))
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    slow_operations.sort_by(|left, right| {
        right
            .get("elapsed_ms")
            .and_then(Value::as_f64)
            .unwrap_or(0.0)
            .partial_cmp(
                &left
                    .get("elapsed_ms")
                    .and_then(Value::as_f64)
                    .unwrap_or(0.0),
            )
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    slow_operations.truncate(SLOW_SUMMARY_LIMIT);
    json!({
        "threshold_ms": threshold_ms,
        "limit": SLOW_SUMMARY_LIMIT,
        "slow_count": slow_operations.len(),
        "slow_operations": slow_operations,
        "artifacts": {
            "slow_summary": artifact_path,
            "progress": progress_path,
        }
    })
}

fn scenario_slow_summary_threshold_ms(scenario: &Value) -> u64 {
    scenario
        .get("slow_summary_threshold_ms")
        .and_then(Value::as_u64)
        .or_else(|| {
            scenario
                .get("slow_summary")
                .and_then(Value::as_object)
                .and_then(|summary| summary.get("threshold_ms"))
                .and_then(Value::as_u64)
        })
        .unwrap_or(DEFAULT_SLOW_SUMMARY_THRESHOLD_MS)
}

fn scenario_run_timeout_failure(
    command: &str,
    result_path: &Path,
    artifacts: &Map<String, Value>,
    process_result: &GodotRunResult,
    configured_timeout: Option<Duration>,
) -> FailureDetail {
    let progress_path = artifacts
        .get("progress")
        .or_else(|| artifacts.get("progress_path"))
        .and_then(Value::as_str);
    let active = progress_path.and_then(latest_active_progress_event);
    let mut actual = Map::from_iter([
        ("exit_code".to_string(), json!(process_result.returncode)),
        ("timed_out".to_string(), json!(true)),
        (
            "elapsed_ms".to_string(),
            json!(
                process_result
                    .timeout_ms
                    .or_else(|| configured_timeout.map(duration_millis_u64))
            ),
        ),
        (
            "timeout_ms".to_string(),
            json!(
                process_result
                    .timeout_ms
                    .or_else(|| configured_timeout.map(duration_millis_u64))
            ),
        ),
        ("process".to_string(), process_summary_json(process_result)),
    ]);
    if let Some(path) = progress_path {
        actual.insert("progress_path".to_string(), json!(path));
    }
    if let Some(cleanup) = process_result.cleanup.as_ref() {
        actual.insert("cleanup".to_string(), process_cleanup_json(cleanup));
    }
    if let Some(active) = active {
        if active.get("operation").and_then(Value::as_str) == Some("assertion") {
            actual.insert("active_assertion".to_string(), active.clone());
        } else {
            actual.insert("active_step".to_string(), active.clone());
        }
        actual.insert("latest_progress".to_string(), active);
    }

    let mut artifact_map = artifacts.clone();
    if let Some(path) = progress_path {
        artifact_map.insert("progress".to_string(), json!(path));
    }
    classify_failure_detail(
        failure_with_artifacts(
            FailureDetail {
                kind: "scenario_run_timeout".to_string(),
                command: Some(command.to_string()),
                path: Some(result_path.to_string_lossy().into_owned()),
                message: Some("Godot scenario exceeded the configured timeout".to_string()),
                expected: Some(json!("Godot scenario exits before the configured timeout")),
                actual: Some(Value::Object(actual)),
                ..FailureDetail::default()
            },
            &artifact_map,
        ),
        None,
    )
}

fn latest_active_progress_event(progress_path: &str) -> Option<Value> {
    let contents = fs::read_to_string(progress_path).ok()?;
    let mut active: Option<Value> = None;
    for line in contents.lines().filter(|line| !line.trim().is_empty()) {
        let Ok(mut event) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        sanitize_progress_event(&mut event);
        match event.get("event").and_then(Value::as_str) {
            Some("start") => active = Some(event),
            Some("finish") => active = None,
            _ => active = Some(event),
        }
    }
    active
}

fn sanitize_progress_event(event: &mut Value) {
    if let Some(object) = event.as_object_mut() {
        object.remove("session_token");
        object.remove("token");
        if let Some(project_args) = object.get_mut("project_args") {
            *project_args = redact_project_args_for_metadata(project_args);
        }
        if let Some(args) = object.get_mut("args") {
            *args = redact_project_args_for_metadata(args);
        }
    }
}


fn known_wrapper_exit_codes(scenario: &Value) -> Vec<i32> {
    scenario
        .get("process_outcome_policy")
        .and_then(Value::as_object)
        .and_then(|policy| policy.get("known_wrapper_exit_codes"))
        .and_then(Value::as_array)
        .map(|codes| {
            codes
                .iter()
                .filter_map(|value| value.as_i64())
                .filter(|code| *code > 0)
                .filter_map(|code| i32::try_from(code).ok())
                .collect()
        })
        .unwrap_or_default()
}

fn process_summary_json(process_result: &GodotRunResult) -> Value {
    let command = redact_command_args_for_metadata(&process_result.command);
    let command_line = command.join(" ");
    json!({
        "command": command,
        "command_line": command_line,
        "exit_code": process_result.returncode,
        "status": if process_result.returncode == 0 { "passed" } else { "failed" },
        "pid": process_result.pid,
        "process_group": process_result.process_group,
        "cleanup": process_result.cleanup.as_ref().map(process_cleanup_json),
        "infrastructure_failure_kind": process_result.infrastructure_failure_kind,
        "timed_out": process_result.timed_out,
        "timeout_ms": process_result.timeout_ms,
        "output_truncation": process_result.output_truncation,
        "failure_category": process_failure_category(process_result),
        "retryable": process_result.infrastructure_failure_kind.is_some(),
        "retry_guidance": process_result
            .infrastructure_failure_kind
            .as_ref()
            .and(process_result.display.startup_failure_guidance.as_ref()),
    })
}

fn process_cleanup_json(cleanup: &godot_qa_process::ProcessCleanupResult) -> Value {
    json!({
        "status": cleanup.status,
        "failure_kind": cleanup.failure_kind,
        "failure_category": cleanup.failure_kind.as_ref().map(|_| "process"),
        "retryable": false,
        "retry_guidance": cleanup.failure_kind.as_ref().map(|_| "Resolve stale owned Godot child process cleanup before retrying the scenario."),
        "pid": cleanup.pid,
        "process_group": cleanup.process_group,
        "owned": cleanup.owned,
        "message": cleanup.message,
    })
}

fn process_summary_with_warning_json(
    process_result: &GodotRunResult,
    result_path: &Path,
    artifacts: &Map<String, Value>,
) -> Value {
    let mut process = process_summary_json(process_result);
    let command = redact_command_args_for_metadata(&process_result.command);
    let command_line = command.join(" ");
    process["warnings"] = json!([{
        "type": "godot_run_failed",
        "severity": "warning",
        "message": "Godot exited with a known wrapper-only non-zero status after a clean runtime result",
        "expected": 0,
        "actual": process_result.returncode,
        "command": command,
        "command_line": command_line,
        "path": result_path.to_string_lossy().into_owned(),
        "artifacts": artifacts,
    }]);
    process
}

fn runtime_result_summary_json(
    result_path: &Path,
    status: &str,
    ok: Option<bool>,
    failure_count: usize,
) -> Value {
    json!({
        "path": result_path.to_string_lossy().into_owned(),
        "status": status,
        "ok": ok,
        "failure_count": failure_count,
    })
}

/// CLI-owned `data.logs` summary.
///
/// `ignored_count` is how many `ERROR:`/`WARNING:` lines an ignore list silenced
/// during the scan that produced `failures`, and `ignore_patterns_applied` is how
/// many project `logs.ignore` patterns were in force for the run. Both are `0` on
/// the paths that report log health without scanning (timeout, missing or invalid
/// runtime result), where `error_count`/`warning_count` are already `0`.
fn log_status_summary_json(
    logs_path: &Path,
    failures: &[FailureDetail],
    ignored_count: usize,
    ignore_patterns_applied: usize,
) -> Value {
    let error_count = failures
        .iter()
        .filter(|failure| failure.kind == "godot_log_error")
        .count();
    let warning_count = failures
        .iter()
        .filter(|failure| failure.kind == "godot_log_warning")
        .count();
    let status = if error_count > 0 {
        "error"
    } else if warning_count > 0 {
        "warning"
    } else if logs_path.is_file() {
        "clean"
    } else {
        "missing"
    };
    json!({
        "path": logs_path.to_string_lossy().into_owned(),
        "status": status,
        "error_count": error_count,
        "warning_count": warning_count,
        "ignored_count": ignored_count,
        "ignore_patterns_applied": ignore_patterns_applied,
    })
}

fn outcome_summary_json(
    status: &str,
    reason: Option<&str>,
    failures: Option<&[FailureDetail]>,
) -> Value {
    let mut outcome = json!({
        "status": status,
        "reason": reason,
    });
    if let Some(failures) = failures {
        if let Some(classification) = outcome_classification_json(failures) {
            if let Some(outcome) = outcome.as_object_mut() {
                for (key, value) in classification {
                    outcome.insert(key, value);
                }
            }
        }
    }
    outcome
}

fn outcome_warning_summary_json() -> Value {
    json!({
        "status": "pass_with_warning",
        "reason": "godot_run_failed",
        "message": "scenario run passed with warning",
        "ok": true,
        "exit_code": 0,
    })
}

fn outcome_reason_for(ok: bool, failures: &[FailureDetail]) -> Option<&str> {
    if ok {
        return None;
    }
    failures
        .iter()
        .find(|failure| failure.kind == "godot_log_error")
        .or_else(|| {
            failures
                .iter()
                .find(|failure| failure.kind == "godot_run_failed")
        })
        .or_else(|| failures.first())
        .map(|failure| failure.kind.as_str())
}

