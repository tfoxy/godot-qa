// Extracted from lib.rs; included by lib.rs to preserve the original crate namespace.

pub fn run_live_protocol_command(
    config: &ProjectConfig,
    command: &str,
    protocol_command: &str,
    params: Map<String, Value>,
) -> CommandResult {
    let loaded = match load_session_record_with_source(config, command) {
        Ok(loaded) => loaded,
        Err(failure) => {
            let message = if command == "session status" {
                "live session unavailable"
            } else {
                "live command failed"
            };
            return CommandResult {
                ok: false,
                command: command.to_string(),
                message: message.to_string(),
                data: Map::new(),
                failures: vec![failure],
                exit_code: 1,
            };
        }
    };
    run_live_protocol_command_with_loaded(command, protocol_command, params, loaded, None)
}

fn run_live_protocol_command_with_loaded(
    command: &str,
    protocol_command: &str,
    params: Map<String, Value>,
    loaded: LoadedSessionRecord,
    token_override: Option<&str>,
) -> CommandResult {
    let mut retry_failures = Vec::new();
    let max_attempts = LIVE_COMMAND_RETRY_BACKOFF_MILLIS.len() + 1;
    for attempt in 1..=max_attempts {
        let mut result = run_live_protocol_command_once_with_loaded(
            command,
            protocol_command,
            params.clone(),
            loaded.clone(),
            token_override,
        );
        if result.ok || !live_command_result_is_retryable(&result) || attempt == max_attempts {
            if !retry_failures.is_empty() {
                result.data.insert(
                    "retry".to_string(),
                    live_command_retry_metadata(attempt, max_attempts, &retry_failures, result.ok),
                );
            }
            return result;
        }
        let backoff_ms = live_command_retry_backoff_ms(&result, attempt);
        retry_failures.push(live_command_retry_failure_summary(
            &result, attempt, backoff_ms,
        ));
        thread::sleep(Duration::from_millis(backoff_ms));
    }
    unreachable!("live command retry loop always returns on final attempt")
}

fn run_live_protocol_command_once_with_loaded(
    command: &str,
    protocol_command: &str,
    params: Map<String, Value>,
    loaded: LoadedSessionRecord,
    token_override: Option<&str>,
) -> CommandResult {
    let record = loaded.record;
    let response = match send_live_request_with_metadata_path(
        &record,
        &loaded.source_path,
        protocol_command,
        params,
        token_override,
    ) {
        Ok(response) => response,
        Err(failure) => {
            let failure =
                classify_session_connect_failure(&record, command, &loaded.source_path, failure);
            let message = if command == "session status" {
                "live session unavailable"
            } else {
                "live command failed"
            };
            return CommandResult {
                ok: false,
                command: command.to_string(),
                message: message.to_string(),
                data: Map::new(),
                failures: vec![failure],
                exit_code: 1,
            };
        }
    };
    if let Err(error) = validate_response(command, &response) {
        return CommandResult {
            ok: false,
            command: command.to_string(),
            message: "live command failed".to_string(),
            data: Map::new(),
            failures: vec![error.failure],
            exit_code: 1,
        };
    }
    let object = response.as_object().unwrap_or(&Map::new()).clone();
    if object.get("ok").and_then(Value::as_bool).unwrap_or(false) {
        return CommandResult {
            ok: true,
            command: command.to_string(),
            message: "live command completed".to_string(),
            data: object
                .get("data")
                .and_then(Value::as_object)
                .cloned()
                .unwrap_or_default(),
            failures: Vec::new(),
            exit_code: 0,
        };
    }
    let error_data = object
        .get("error")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    CommandResult {
        ok: false,
        command: command.to_string(),
        message: error_data
            .get("message")
            .and_then(Value::as_str)
            .unwrap_or("live command failed")
            .to_string(),
        data: Map::new(),
        failures: vec![failure_detail_from_live_error(command, &error_data)],
        exit_code: 1,
    }
}

fn live_command_result_is_retryable(result: &CommandResult) -> bool {
    result
        .failures
        .iter()
        .any(live_command_failure_is_retryable)
}

fn live_command_failure_is_retryable(failure: &FailureDetail) -> bool {
    match failure.kind.as_str() {
        "session_bridge_busy" | "resource_unavailable" | "resource_busy" | "bridge_busy" => true,
        "session_connect_failed" => {
            failure_text_contains(failure, "Resource temporarily unavailable")
                || failure_text_contains(failure, "os error 11")
        }
        _ => false,
    }
}

fn failure_text_contains(failure: &FailureDetail, needle: &str) -> bool {
    failure
        .message
        .as_deref()
        .is_some_and(|message| message.contains(needle))
        || failure
            .actual
            .as_ref()
            .and_then(Value::as_str)
            .is_some_and(|actual| actual.contains(needle))
}

fn live_command_retry_backoff_ms(result: &CommandResult, attempt: usize) -> u64 {
    result
        .failures
        .iter()
        .filter_map(live_command_busy_backoff_ms)
        .max()
        .unwrap_or_else(|| {
            LIVE_COMMAND_RETRY_BACKOFF_MILLIS
                .get(attempt.saturating_sub(1))
                .copied()
                .unwrap_or(50)
        })
}

fn live_command_busy_backoff_ms(failure: &FailureDetail) -> Option<u64> {
    match failure.kind.as_str() {
        "session_bridge_busy" | "resource_unavailable" | "resource_busy" | "bridge_busy" => {}
        "session_connect_failed"
            if failure_text_contains(failure, "Resource temporarily unavailable")
                || failure_text_contains(failure, "os error 11") => {}
        _ => return None,
    }
    let in_progress = failure
        .details
        .get("in_progress")
        .and_then(Value::as_object)?;
    let command = in_progress.get("command").and_then(Value::as_str)?;
    let elapsed_ms = in_progress
        .get("elapsed_msec")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let estimated_ms = match command {
        SESSION_WAIT_FRAMES | TRACE_RECORD | EVENT_TRACE | TELEMETRY_RECORD => {
            let frames = in_progress
                .get("frames")
                .and_then(Value::as_i64)
                .unwrap_or(0)
                .max(0) as f64;
            ((frames / LIVE_COMMAND_BUSY_FPS).ceil() * 1000.0) as u64
                + LIVE_COMMAND_BUSY_GRACE_MILLIS
        }
        SESSION_PROFILE => {
            let frames = in_progress
                .get("frames")
                .and_then(Value::as_i64)
                .unwrap_or(60)
                .max(0) as f64;
            (((frames / LIVE_PROFILE_TIMEOUT_FPS).ceil() + LIVE_PROFILE_TIMEOUT_GRACE_SECONDS)
                * 1000.0) as u64
        }
        AUDIO_CAPTURE => {
            let seconds = in_progress
                .get("duration_seconds")
                .or_else(|| in_progress.get("seconds"))
                .and_then(Value::as_f64)
                .unwrap_or(0.0)
                .max(0.0);
            (seconds * 1000.0).ceil() as u64 + LIVE_COMMAND_BUSY_GRACE_MILLIS
        }
        _ => return None,
    };
    Some(
        estimated_ms
            .saturating_sub(elapsed_ms)
            .clamp(25, LIVE_COMMAND_BUSY_MAX_BACKOFF_MILLIS),
    )
}

fn live_command_retry_failure_summary(
    result: &CommandResult,
    attempt: usize,
    backoff_ms: u64,
) -> Value {
    let failures: Vec<Value> = result
        .failures
        .iter()
        .map(|failure| {
            json!({
                "kind": &failure.kind,
                "message": &failure.message,
                "actual": &failure.actual,
                "details": &failure.details,
            })
        })
        .collect();
    json!({
        "attempt": attempt,
        "backoff_ms": backoff_ms,
        "message": result.message,
        "failures": failures,
    })
}

fn live_command_retry_metadata(
    attempts: usize,
    max_attempts: usize,
    retry_failures: &[Value],
    ok: bool,
) -> Value {
    json!({
        "attempts": attempts,
        "max_attempts": max_attempts,
        "backoff_milliseconds": LIVE_COMMAND_RETRY_BACKOFF_MILLIS,
        "retryable_failure_kinds": LIVE_COMMAND_RETRYABLE_FAILURE_KINDS,
        "observed_retryable_failures": retry_failures,
        "final_status": if ok { "success" } else { "failure" },
        "exhausted": !ok && attempts == max_attempts,
    })
}

pub fn physics_inspect_result(config: &ProjectConfig, params: Map<String, Value>) -> CommandResult {
    let command = "physics inspect";
    let mut result = run_live_protocol_command(config, command, PHYSICS_INSPECT, params);
    if !result.ok {
        return result;
    }
    let mut data = std::mem::take(&mut result.data);
    let capture_id = unique_session_id();
    let record = match load_session_record(config, command) {
        Ok(record) => record,
        Err(failure) => {
            return CommandResult {
                ok: false,
                command: command.to_string(),
                message: "physics inspection failed".to_string(),
                data,
                failures: vec![failure],
                exit_code: 1,
            };
        }
    };
    let artifact_path = PathBuf::from(&record.artifact_dir)
        .join("physics")
        .join(capture_id)
        .join("physics.json");
    if let Err(mut failure) = write_json(&artifact_path, &Value::Object(data.clone())) {
        failure.command = Some(command.to_string());
        return CommandResult {
            ok: false,
            command: command.to_string(),
            message: "physics inspection failed".to_string(),
            data,
            failures: vec![failure],
            exit_code: 1,
        };
    }
    let mut artifacts = data
        .get("artifacts")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    artifacts.insert("physics".to_string(), json!(artifact_path));
    data.insert("artifacts".to_string(), Value::Object(artifacts));

    CommandResult {
        ok: true,
        command: command.to_string(),
        message: "physics inspection completed".to_string(),
        data,
        failures: Vec::new(),
        exit_code: 0,
    }
}


fn failure_detail_from_live_error(command: &str, error_data: &Map<String, Value>) -> FailureDetail {
    let data = error_data
        .get("data")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    let mut detail = runtime_failure_extra_fields(&data);
    FailureDetail {
        kind: error_data
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or("runtime_error")
            .to_string(),
        command: Some(command.to_string()),
        message: error_data
            .get("message")
            .and_then(Value::as_str)
            .map(str::to_string),
        expected: data.get("expected").cloned(),
        actual: data.get("actual").cloned(),
        target: data
            .get("target")
            .and_then(Value::as_str)
            .map(str::to_string),
        severity: data
            .get("severity")
            .and_then(Value::as_str)
            .map(str::to_string),
        path: data.get("path").and_then(Value::as_str).map(str::to_string),
        artifacts: data.get("artifacts").and_then(Value::as_object).cloned(),
        viewport: take_i64_vec(&mut detail, "viewport"),
        a: data.get("a").and_then(Value::as_str).map(str::to_string),
        b: data.get("b").and_then(Value::as_str).map(str::to_string),
        rect: take_f64_vec(&mut detail, "rect"),
        rect_a: take_f64_vec(&mut detail, "rect_a"),
        rect_b: take_f64_vec(&mut detail, "rect_b"),
        details: detail,
    }
}


fn is_timeout_actual(value: &Value) -> bool {
    value
        .as_str()
        .map(|actual| actual == "TimeoutError")
        .unwrap_or(false)
}

fn runtime_failure_extra_fields(value: &Map<String, Value>) -> Map<String, Value> {
    value
        .iter()
        .filter_map(|(key, value)| match key.as_str() {
            "type" | "message" | "command" | "path" | "expected" | "actual" | "target"
            | "severity" | "artifacts" => None,
            "a" | "b" if value.is_string() => None,
            _ => Some((key.clone(), value.clone())),
        })
        .collect()
}

fn take_i64_vec(detail: &mut Map<String, Value>, key: &str) -> Option<Vec<i64>> {
    let values = detail.get(key)?.as_array()?;
    let parsed: Option<Vec<i64>> = values.iter().map(Value::as_i64).collect();
    if parsed.is_some() {
        detail.remove(key);
    }
    parsed
}

fn take_f64_vec(detail: &mut Map<String, Value>, key: &str) -> Option<Vec<f64>> {
    let values = detail.get(key)?.as_array()?;
    let parsed: Option<Vec<f64>> = values.iter().map(Value::as_f64).collect();
    if parsed.is_some() {
        detail.remove(key);
    }
    parsed
}

pub fn run_live_screenshot_protocol_command(
    config: &ProjectConfig,
    command: &str,
    protocol_command: &str,
    params: Map<String, Value>,
) -> CommandResult {
    let record = match load_session_record(config, command) {
        Ok(record) => record,
        Err(failure) => {
            return CommandResult {
                ok: false,
                command: command.to_string(),
                message: "live command failed".to_string(),
                data: Map::new(),
                failures: vec![failure],
                exit_code: 1,
            };
        }
    };
    if record.headless {
        return CommandResult {
            ok: false,
            command: command.to_string(),
            message: "live command failed".to_string(),
            data: Map::new(),
            failures: vec![headless_screenshot_unsupported(command, &record)],
            exit_code: 1,
        };
    }
    run_live_protocol_command(config, command, protocol_command, params)
}


fn unavailable_result(command: &str, failure: FailureDetail) -> CommandResult {
    CommandResult {
        ok: false,
        command: command.to_string(),
        message: "live session unavailable".to_string(),
        data: Map::new(),
        failures: vec![failure],
        exit_code: 1,
    }
}

fn session_profile_invalid_result(
    command: &str,
    message: &str,
    expected: Value,
    actual: Value,
    retry: Option<Value>,
) -> CommandResult {
    let mut data = Map::new();
    if let Some(retry) = retry {
        data.insert("retry".to_string(), retry);
    }
    CommandResult {
        ok: false,
        command: command.to_string(),
        message: "live session profile failed".to_string(),
        data,
        failures: vec![FailureDetail {
            kind: "profile_result_invalid".to_string(),
            command: Some(command.to_string()),
            message: Some(message.to_string()),
            expected: Some(expected),
            actual: Some(actual),
            ..FailureDetail::default()
        }],
        exit_code: 1,
    }
}

fn session_profile_artifact_failed(
    command: &str,
    artifact_path: &Path,
    error: std::io::Error,
    retry: Option<Value>,
) -> CommandResult {
    let mut data = Map::new();
    if let Some(retry) = retry {
        data.insert("retry".to_string(), retry);
    }
    CommandResult {
        ok: false,
        command: command.to_string(),
        message: "live session profile failed".to_string(),
        data,
        failures: vec![FailureDetail {
            kind: "artifact_write_failed".to_string(),
            command: Some(command.to_string()),
            path: Some(artifact_path.to_string_lossy().into_owned()),
            message: Some(error.to_string()),
            expected: Some(json!(
                "profile.json written under the live session profile root"
            )),
            actual: Some(json!("OSError")),
            ..FailureDetail::default()
        }],
        exit_code: 1,
    }
}

fn display_launch_metadata_json(display: &godot_qa_process::DisplayLaunchMetadata) -> Value {
    json!({
        "headless": display.headless,
        "selected_mode": display.selected_mode,
        "platform": display.platform,
        "linuxbsd_target": display.linuxbsd_target,
        "x11_display_available": display.x11_display_available,
        "wayland_display_available": display.wayland_display_available,
        "selected_display_driver": display.selected_display_driver,
        "project_prefers_wayland": display.project_prefers_wayland,
        "requested_display_driver": display.requested_display_driver,
        "virtual_display_used": display.virtual_display_used,
        "virtual_display_command": display.virtual_display_command,
        "fallback_reason": display.fallback_reason,
        "launch_prefix_used": display.launch_prefix_used,
        "display_backend_available": display.display_backend_available,
        "startup_attempted": display.startup_attempted,
        "virtual_display_exit_code": display.virtual_display_exit_code,
        "startup_failure_kind": display.startup_failure_kind,
        "startup_failure_retryable": display.startup_failure_retryable,
        "startup_failure_guidance": display.startup_failure_guidance,
    })
}

pub fn headless_screenshot_unsupported(command: &str, record: &SessionRecord) -> FailureDetail {
    FailureDetail {
        kind: "headless_screenshot_unsupported".to_string(),
        command: Some(command.to_string()),
        message: Some("screenshot-producing commands require a windowed Godot session".to_string()),
        expected: Some(json!("windowed live session started without --headless")),
        actual: Some(json!({
            "session_id": record.session_id,
            "headless": record.headless,
        })),
        severity: Some("error".to_string()),
        ..FailureDetail::default()
    }
}

fn stale_session_failure(
    record: &SessionRecord,
    command: &str,
    metadata_path: &Path,
) -> FailureDetail {
    if record.target == SessionTarget::Android {
        let android = record.android.as_ref();
        return FailureDetail {
            kind: "stale_session".to_string(),
            command: Some(command.to_string()),
            path: Some(metadata_path.to_string_lossy().into_owned()),
            message: Some(
                "saved live session metadata points to a stopped Android application".to_string(),
            ),
            expected: Some(json!(
                "running Android application referenced by saved session metadata"
            )),
            actual: Some(json!({
                "target": "android",
                "device": android.map(|value| value.device_serial.as_str()),
                "package": android.map(|value| value.package.as_str()),
                "device_pid": android.map(|value| value.device_pid),
                "host": record.host,
                "port": record.port,
            })),
            ..FailureDetail::default()
        };
    }
    FailureDetail {
        kind: "stale_session".to_string(),
        command: Some(command.to_string()),
        path: Some(metadata_path.to_string_lossy().into_owned()),
        message: Some("saved live session metadata points to a dead process".to_string()),
        expected: Some(json!(
            "running live session referenced by saved session metadata"
        )),
        actual: Some(json!({
            "pid": record.pid,
            "host": record.host,
            "port": record.port,
        })),
        ..FailureDetail::default()
    }
}

fn pid_is_running(pid: i64) -> bool {
    godot_qa_process::pid_is_running(pid)
}
