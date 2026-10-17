// Extracted from lib.rs; included by lib.rs to preserve the original crate namespace.

fn merged_args(default_args: &[String], specific_args: &[String]) -> Vec<String> {
    let mut args = Vec::with_capacity(default_args.len() + specific_args.len());
    args.extend(default_args.iter().cloned());
    args.extend(specific_args.iter().cloned());
    args
}

pub fn persist_session(
    config: &ProjectConfig,
    record: &SessionRecord,
    token: &str,
) -> Result<(), FailureDetail> {
    if let Some(parent) = session_json_path(config).parent() {
        fs::create_dir_all(parent).map_err(|error| artifact_write_failure(parent, &error))?;
    }
    let session_json = record.to_json_value();
    write_json(&session_json_path(config), &session_json)?;
    write_json(
        &Path::new(&record.artifact_dir).join("session.json"),
        &session_json,
    )?;
    fs::write(session_token_path(config), token)
        .map_err(|error| artifact_write_failure(&session_token_path(config), &error))
}

pub fn send_live_request(
    config: &ProjectConfig,
    record: &SessionRecord,
    protocol_command: &str,
    params: Map<String, Value>,
    token_override: Option<&str>,
) -> Result<Value, FailureDetail> {
    let metadata_path = session_metadata_path(config, record);
    send_live_request_with_metadata_path(
        record,
        &metadata_path,
        protocol_command,
        params,
        token_override,
    )
}

fn send_live_request_with_metadata_path(
    record: &SessionRecord,
    metadata_path: &Path,
    protocol_command: &str,
    mut params: Map<String, Value>,
    token_override: Option<&str>,
) -> Result<Value, FailureDetail> {
    if let Some(android) = &record.android {
        let mapper = android_artifacts::AndroidArtifactMapper::new(
            &record.artifact_dir,
            &android.device_artifact_root,
        )?;
        let mut value = Value::Object(params);
        mapper.map_outgoing_params(&mut value)?;
        params = value.as_object().cloned().unwrap_or_default();
    }
    let timeout = live_request_timeout(protocol_command, &params);
    let mut response = send_live_request_with_timeout(
        record,
        metadata_path,
        protocol_command,
        params,
        token_override,
        timeout,
    )?;
    if let Some(android) = &record.android {
        let mapper = android_artifacts::AndroidArtifactMapper::new(
            &record.artifact_dir,
            &android.device_artifact_root,
        )?;
        mapper.materialize_response(
            &mut response,
            &mut |device_path: &str, temporary: &Path| {
                let bytes = godot_qa_process::android::read_private_artifact(
                    &android.adb_path,
                    &android.device_serial,
                    &android.package,
                    &record.session_id,
                    device_path,
                )?;
                fs::write(temporary, bytes)
                    .map_err(|error| artifact_write_failure(temporary, &error))
            },
        )?;
    }
    Ok(response)
}

fn live_request_timeout(protocol_command: &str, params: &Map<String, Value>) -> Duration {
    if protocol_command == TRACE_RECORD {
        let frames = params
            .get("frames")
            .and_then(Value::as_i64)
            .unwrap_or(0)
            .max(0) as f64;
        let wait_seconds = (frames / LIVE_TRACE_RECORD_TIMEOUT_FPS).ceil()
            + LIVE_TRACE_RECORD_TIMEOUT_GRACE_SECONDS;
        return Duration::from_secs_f64(wait_seconds.clamp(
            LIVE_SESSION_REQUEST_TIMEOUT_SECONDS,
            LIVE_SESSION_WAIT_TIMEOUT_MAX_SECONDS,
        ));
    }
    if protocol_command == EVENT_TRACE || protocol_command == TELEMETRY_RECORD {
        let frames = params
            .get("frames")
            .and_then(Value::as_i64)
            .unwrap_or(0)
            .max(0) as f64;
        let wait_seconds =
            (frames / LIVE_EVENT_TRACE_TIMEOUT_FPS).ceil() + LIVE_EVENT_TRACE_TIMEOUT_GRACE_SECONDS;
        return Duration::from_secs_f64(wait_seconds.clamp(
            LIVE_SESSION_REQUEST_TIMEOUT_SECONDS,
            LIVE_SESSION_WAIT_TIMEOUT_MAX_SECONDS,
        ));
    }
    if protocol_command == SESSION_WAIT_FRAMES {
        let frames = params
            .get("frames")
            .and_then(Value::as_i64)
            .unwrap_or(0)
            .max(0) as f64;
        let wait_seconds = (frames / LIVE_SESSION_WAIT_TIMEOUT_FPS).ceil()
            + LIVE_SESSION_WAIT_TIMEOUT_GRACE_SECONDS;
        return Duration::from_secs_f64(wait_seconds.clamp(
            LIVE_SESSION_REQUEST_TIMEOUT_SECONDS,
            LIVE_SESSION_WAIT_TIMEOUT_MAX_SECONDS,
        ));
    }
    if protocol_command == INPUT_TEXT {
        // _type_text spends 2 rendered frames per codepoint (press + release),
        // so derive the wall clock from the text length like a wait_frames call.
        let frames = params
            .get("text")
            .and_then(Value::as_str)
            .map(|text| text.chars().count() * 2)
            .unwrap_or(0) as f64;
        let wait_seconds = (frames / LIVE_SESSION_WAIT_TIMEOUT_FPS).ceil()
            + LIVE_SESSION_WAIT_TIMEOUT_GRACE_SECONDS;
        return Duration::from_secs_f64(wait_seconds.clamp(
            LIVE_SESSION_REQUEST_TIMEOUT_SECONDS,
            LIVE_SESSION_WAIT_TIMEOUT_MAX_SECONDS,
        ));
    }
    if protocol_command == SESSION_PROFILE {
        let wait_seconds = if let Some(seconds) = params.get("seconds").and_then(Value::as_f64) {
            seconds.max(0.0) + LIVE_PROFILE_TIMEOUT_GRACE_SECONDS
        } else {
            let frames = params
                .get("frames")
                .and_then(Value::as_i64)
                .unwrap_or(60)
                .max(0) as f64;
            (frames / LIVE_PROFILE_TIMEOUT_FPS).ceil() + LIVE_PROFILE_TIMEOUT_GRACE_SECONDS
        };
        return Duration::from_secs_f64(wait_seconds.clamp(
            LIVE_SESSION_REQUEST_TIMEOUT_SECONDS,
            LIVE_SESSION_WAIT_TIMEOUT_MAX_SECONDS,
        ));
    }
    if protocol_command == SCENARIO_RUN {
        // A warm scenario can run many wait_frames + asserts in one round-trip.
        // The warm runner passes timeout_seconds derived from the scenario (the
        // same budget the cold path uses); fall back to a generous default.
        // When timeout_frames is present the engine enforces the real
        // (load-independent) deadline itself, so the wall clock here is only a
        // hang backstop: declared timeout with slack, higher cap, and never
        // tighter than the declared wall.
        if params.get("timeout_frames").is_some() {
            let declared = params
                .get("timeout_seconds")
                .and_then(Value::as_f64)
                .unwrap_or(LIVE_SCENARIO_RUN_DEFAULT_TIMEOUT_SECONDS)
                .max(0.0);
            let backstop = (declared * LIVE_SCENARIO_RUN_FRAME_BUDGET_SLACK)
                .min(LIVE_SCENARIO_RUN_FRAME_BUDGET_TIMEOUT_MAX_SECONDS)
                .max(declared)
                + LIVE_SCENARIO_RUN_GRACE_SECONDS;
            return Duration::from_secs_f64(backstop.max(LIVE_SESSION_REQUEST_TIMEOUT_SECONDS));
        }
        let seconds = params
            .get("timeout_seconds")
            .and_then(Value::as_f64)
            .unwrap_or(LIVE_SCENARIO_RUN_DEFAULT_TIMEOUT_SECONDS)
            .max(0.0)
            + LIVE_SCENARIO_RUN_GRACE_SECONDS;
        return Duration::from_secs_f64(seconds.clamp(
            LIVE_SESSION_REQUEST_TIMEOUT_SECONDS,
            LIVE_SCENARIO_RUN_TIMEOUT_MAX_SECONDS,
        ));
    }
    if protocol_command == AUDIO_CAPTURE || protocol_command == AUDIO_RENDER {
        let seconds = params
            .get("duration_seconds")
            .and_then(Value::as_f64)
            .unwrap_or(0.0)
            .max(0.0);
        return Duration::from_secs_f64(
            (seconds + LIVE_AUDIO_REQUEST_GRACE_SECONDS).max(LIVE_SESSION_REQUEST_TIMEOUT_SECONDS),
        );
    }
    Duration::from_secs_f64(LIVE_SESSION_REQUEST_TIMEOUT_SECONDS)
}

fn send_live_request_with_timeout(
    record: &SessionRecord,
    metadata_path: &Path,
    protocol_command: &str,
    params: Map<String, Value>,
    token_override: Option<&str>,
    timeout: Duration,
) -> Result<Value, FailureDetail> {
    let resolved_token = match token_override {
        Some(token) => token.to_string(),
        None => read_session_token(record, protocol_command)?,
    };
    let request_id = live_request_id();
    let request = build_request(
        protocol_command,
        params,
        Some(&request_id),
        Some(&resolved_token),
    );
    let payload = serde_json::to_vec(&request).unwrap_or_else(|_| b"null".to_vec());
    let mut payload = payload;
    payload.push(b'\n');
    let address = format!("{}:{}", record.host, record.port);
    let socket_address = address
        .to_socket_addrs()
        .map_err(|error| {
            session_failure(
                "session_connect_failed",
                protocol_command,
                metadata_path,
                &error.to_string(),
                json!(format!(
                    "reachable live session at {}:{}",
                    record.host, record.port
                )),
                json!("AddrParseError"),
            )
        })?
        .next()
        .ok_or_else(|| {
            session_failure(
                "session_connect_failed",
                protocol_command,
                metadata_path,
                "no socket address resolved for live session host",
                json!(format!(
                    "reachable live session at {}:{}",
                    record.host, record.port
                )),
                json!("AddrParseError"),
            )
        })?;
    let mut stream = match TcpStream::connect_timeout(&socket_address, timeout) {
        Ok(stream) => stream,
        Err(error)
            if error.kind() == std::io::ErrorKind::PermissionDenied
                && file_live_transport_allowed(record) =>
        {
            return send_file_live_request(
                record,
                metadata_path,
                protocol_command,
                &request_id,
                &request,
                &error.to_string(),
                timeout,
            );
        }
        Err(error) => {
            return Err(session_failure(
                "session_connect_failed",
                protocol_command,
                metadata_path,
                &error.to_string(),
                json!(format!(
                    "reachable live session at {}:{}",
                    record.host, record.port
                )),
                json!(io_error_name(&error)),
            ));
        }
    };
    let _ = stream.set_read_timeout(Some(timeout));
    let _ = stream.set_write_timeout(Some(timeout));
    if let Err(error) = stream.write_all(&payload) {
        if error.kind() == std::io::ErrorKind::PermissionDenied
            && file_live_transport_allowed(record)
        {
            return send_file_live_request(
                record,
                metadata_path,
                protocol_command,
                &request_id,
                &request,
                &error.to_string(),
                timeout,
            );
        }
        return Err(session_failure(
            "session_connect_failed",
            protocol_command,
            metadata_path,
            &error.to_string(),
            json!(format!(
                "reachable live session at {}:{}",
                record.host, record.port
            )),
            json!(io_error_name(&error)),
        ));
    }
    let mut response_bytes = Vec::new();
    let mut chunk = [0u8; 65536];
    loop {
        match stream.read(&mut chunk) {
            Ok(0) => break,
            Ok(read) => {
                response_bytes.extend_from_slice(&chunk[..read]);
                if response_bytes.ends_with(b"\n") {
                    break;
                }
            }
            Err(error) => {
                return Err(session_failure(
                    "session_connect_failed",
                    protocol_command,
                    metadata_path,
                    &error.to_string(),
                    json!(format!(
                        "reachable live session at {}:{}",
                        record.host, record.port
                    )),
                    json!(io_error_name(&error)),
                ));
            }
        }
    }
    serde_json::from_slice::<Value>(response_bytes.trim_ascii()).map_err(|error| FailureDetail {
        kind: "protocol_error".to_string(),
        command: Some(protocol_command.to_string()),
        message: Some(error.to_string()),
        expected: Some(json!("valid JSON line response")),
        actual: Some(json!("invalid_json")),
        ..FailureDetail::default()
    })
}

fn file_live_transport_allowed(record: &SessionRecord) -> bool {
    // Android never polls host files and its authenticated request token must not be
    // written into a host RPC artifact as a fallback.
    record.target == SessionTarget::Desktop
}

fn send_file_live_request(
    record: &SessionRecord,
    metadata_path: &Path,
    protocol_command: &str,
    request_id: &str,
    request: &Value,
    fallback_reason: &str,
    timeout: Duration,
) -> Result<Value, FailureDetail> {
    let rpc_root = Path::new(&record.artifact_dir).join("rpc");
    let requests_dir = rpc_root.join("requests");
    let responses_dir = rpc_root.join("responses");
    fs::create_dir_all(&requests_dir)
        .and_then(|_| fs::create_dir_all(&responses_dir))
        .map_err(|error| {
            session_failure(
                "session_connect_failed",
                protocol_command,
                metadata_path,
                &error.to_string(),
                json!("writable file-backed live session transport directories"),
                json!("OSError"),
            )
        })?;
    let request_path = requests_dir.join(format!("{request_id}.json"));
    let request_tmp_path = requests_dir.join(format!("{request_id}.tmp"));
    let response_path = responses_dir.join(format!("{request_id}.json"));
    let content = serde_json::to_string(request).unwrap_or_else(|_| "{}".to_string());
    fs::write(&request_tmp_path, content)
        .and_then(|_| fs::rename(&request_tmp_path, &request_path))
        .map_err(|error| {
            let _ = fs::remove_file(&request_tmp_path);
            session_failure(
                "session_connect_failed",
                protocol_command,
                metadata_path,
                &error.to_string(),
                json!("file-backed live request written under session artifacts"),
                json!("OSError"),
            )
        })?;
    let deadline = std::time::Instant::now() + timeout;
    while std::time::Instant::now() < deadline {
        if response_path.exists() {
            let contents = fs::read_to_string(&response_path).map_err(|error| {
                session_failure(
                    "session_connect_failed",
                    protocol_command,
                    metadata_path,
                    &error.to_string(),
                    json!("readable file-backed live response"),
                    json!("OSError"),
                )
            })?;
            let _ = fs::remove_file(&request_path);
            let _ = fs::remove_file(&response_path);
            return serde_json::from_str::<Value>(&contents).map_err(|error| FailureDetail {
                kind: "protocol_error".to_string(),
                command: Some(protocol_command.to_string()),
                message: Some(error.to_string()),
                expected: Some(json!("valid JSON file response")),
                actual: Some(json!("invalid_json")),
                ..FailureDetail::default()
            });
        }
        thread::sleep(Duration::from_millis(50));
    }
    let _ = fs::remove_file(&request_path);
    Err(session_failure(
        "session_connect_failed",
        protocol_command,
        metadata_path,
        &format!("file-backed live session response timed out after TCP failed: {fallback_reason}"),
        json!("file-backed live response under session artifacts"),
        json!("TimeoutError"),
    ))
}

/// Sleep schedule between startup probe attempts: start fine-grained so a
/// just-booted session is detected within ~10ms instead of a flat 100ms poll
/// (which wasted up to ~100ms of post-ready latency per session boot), backing
/// off exponentially to the old 100ms pace for slow boots.
fn startup_probe_backoff(previous: Duration) -> Duration {
    (previous * 2).min(Duration::from_millis(100))
}

pub fn probe_live_session_startup(
    config: &ProjectConfig,
    record: &SessionRecord,
    session_token: &str,
) -> Result<Value, FailureDetail> {
    let deadline =
        std::time::Instant::now() + Duration::from_secs_f64(LIVE_SESSION_STARTUP_TIMEOUT_SECONDS);
    let mut last_error = None;
    let mut process_exited = false;
    let mut backoff = Duration::from_millis(5);
    while std::time::Instant::now() < deadline {
        if session_liveness(record) == SessionLiveness::Stopped {
            process_exited = true;
            break;
        }
        match send_live_request(
            config,
            record,
            SESSION_STATUS,
            Map::new(),
            Some(session_token),
        ) {
            Ok(response) => {
                if let Err(error) = validate_response("session start", &response) {
                    return Err(error.failure);
                }
                return Ok(response);
            }
            Err(error) => {
                last_error = Some(error);
                backoff = startup_probe_backoff(backoff);
                thread::sleep(backoff);
            }
        }
    }
    if process_exited {
        return Err(last_error.unwrap_or_else(|| {
            session_failure(
                "session_connect_failed",
                "session start",
                &session_json_path(config),
                "live session process exited before the bridge became reachable",
                json!(format!(
                    "reachable live session at {}:{}",
                    record.host, record.port
                )),
                json!("process_exited"),
            )
        }));
    }
    // The process is still alive but never answered within the deadline. The most
    // common cause is the requested port being stolen/unavailable (the CLI picks it
    // with a bind-and-drop, so a stranger can grab it before Godot listens). Give an
    // actionable message that names the port and suggests a retry instead of leaking
    // the raw connect error.
    Err(session_failure(
        "session_connect_failed",
        "session start",
        &session_json_path(config),
        &format!(
            "live session bridge at {}:{} did not respond within {LIVE_SESSION_STARTUP_TIMEOUT_SECONDS:.0}s; the port may have been taken by another process — retry `session start`",
            record.host, record.port
        ),
        json!(format!(
            "reachable live session at {}:{}",
            record.host, record.port
        )),
        json!("bridge_unreachable"),
    ))
}



#[cfg(test)]
mod transport_tests {
    use super::{
        INPUT_TEXT, PROTOCOL_VERSION, SCENARIO_RUN, SessionRecord, SessionTarget,
        file_live_transport_allowed, live_request_timeout, startup_probe_backoff,
    };
    use serde_json::{Map, json};
    use std::time::Duration;

    #[test]
    fn android_disables_token_bearing_file_transport_fallback() {
        let record = SessionRecord {
            process_identity: None,
            protocol_version: PROTOCOL_VERSION.to_string(),
            session_id: "android-test".to_string(),
            pid: 0,
            host: "127.0.0.1".to_string(),
            port: 32400,
            scene: "res://main.tscn".to_string(),
            viewport: vec![1080, 1920],
            artifact_dir: "/tmp/android-test".to_string(),
            request_path: "/tmp/android-test/request.json".to_string(),
            token_file: "/tmp/android-test/token".to_string(),
            started_at: "0Z".to_string(),
            headless: false,
            project_args: Vec::new(),
            frame_pacing: None,
            target: SessionTarget::Android,
            android: None,
        };
        assert!(!file_live_transport_allowed(&record));
    }

    #[test]
    fn input_text_timeout_scales_with_text_length() {
        // 10 chars = 20 frames: ceil(20/30) + 5 = 6s.
        let params: Map<String, serde_json::Value> =
            [("text".to_string(), json!("0123456789"))].into_iter().collect();
        assert_eq!(
            live_request_timeout(INPUT_TEXT, &params),
            Duration::from_secs_f64(6.0)
        );
        // Max-length 900-char text: ceil(1800/30) + 5 = 65s, under the 120s cap.
        let params: Map<String, serde_json::Value> =
            [("text".to_string(), json!("x".repeat(900)))].into_iter().collect();
        assert_eq!(
            live_request_timeout(INPUT_TEXT, &params),
            Duration::from_secs_f64(65.0)
        );
        // Missing text falls back to the 5s request floor.
        assert_eq!(
            live_request_timeout(INPUT_TEXT, &Map::new()),
            Duration::from_secs_f64(5.0)
        );
    }

    #[test]
    fn startup_probe_backoff_doubles_to_a_100ms_cap() {
        let mut backoff = Duration::from_millis(5);
        let mut schedule = Vec::new();
        for _ in 0..6 {
            backoff = startup_probe_backoff(backoff);
            schedule.push(backoff.as_millis());
        }
        assert_eq!(schedule, vec![10, 20, 40, 80, 100, 100]);
    }

    fn scenario_run_params(entries: &[(&str, serde_json::Value)]) -> Map<String, serde_json::Value> {
        entries
            .iter()
            .map(|(key, value)| (key.to_string(), value.clone()))
            .collect()
    }

    #[test]
    fn scenario_run_wall_clock_without_frame_budget_is_unchanged() {
        // Declared timeout + grace, clamped to the 600s cap.
        let params = scenario_run_params(&[("timeout_seconds", json!(120.0))]);
        assert_eq!(
            live_request_timeout(SCENARIO_RUN, &params),
            Duration::from_secs_f64(135.0)
        );
        let params = scenario_run_params(&[("timeout_seconds", json!(900.0))]);
        assert_eq!(
            live_request_timeout(SCENARIO_RUN, &params),
            Duration::from_secs_f64(600.0)
        );
        let params = scenario_run_params(&[]);
        assert_eq!(
            live_request_timeout(SCENARIO_RUN, &params),
            Duration::from_secs_f64(135.0)
        );
    }

    #[test]
    fn scenario_run_with_frame_budget_relaxes_wall_clock_to_a_backstop() {
        // 3x slack + grace under the 1800s cap.
        let params = scenario_run_params(&[
            ("timeout_seconds", json!(120.0)),
            ("timeout_frames", json!(7200)),
        ]);
        assert_eq!(
            live_request_timeout(SCENARIO_RUN, &params),
            Duration::from_secs_f64(120.0 * 3.0 + 15.0)
        );
        // 900s declared no longer truncates: capped at 1800 + grace.
        let params = scenario_run_params(&[
            ("timeout_seconds", json!(900.0)),
            ("timeout_frames", json!(54000)),
        ]);
        assert_eq!(
            live_request_timeout(SCENARIO_RUN, &params),
            Duration::from_secs_f64(1800.0 + 15.0)
        );
        // The backstop never undercuts a declared wall above the cap.
        let params = scenario_run_params(&[
            ("timeout_seconds", json!(2000.0)),
            ("timeout_frames", json!(120000)),
        ]);
        assert_eq!(
            live_request_timeout(SCENARIO_RUN, &params),
            Duration::from_secs_f64(2000.0 + 15.0)
        );
    }
}
