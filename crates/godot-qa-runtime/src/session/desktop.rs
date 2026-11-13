use super::*;

pub fn session_status_result(config: &ProjectConfig) -> CommandResult {
    session_status_like_result(config, "session status")
}

pub fn session_list_result(config: &ProjectConfig) -> CommandResult {
    session_status_like_result(config, "session list")
}

pub fn session_liveness(record: &SessionRecord) -> SessionLiveness {
    match record.target {
        SessionTarget::Desktop => {
            if godot_qa_process::process_identity_matches(record.pid, record.process_identity) {
                SessionLiveness::Running
            } else {
                SessionLiveness::Stopped
            }
        }
        // Android liveness requires ADB. A missing/disconnected device is not equivalent
        // to a stopped host process, so retain metadata until the Android adapter answers.
        SessionTarget::Android => SessionLiveness::Unavailable,
    }
}

pub(super) fn classify_session_connect_failure(
    record: &SessionRecord,
    command: &str,
    metadata_path: &Path,
    mut failure: FailureDetail,
) -> FailureDetail {
    if failure.kind != "session_connect_failed" {
        return failure;
    }
    match record.target {
        SessionTarget::Desktop if session_liveness(record) == SessionLiveness::Stopped => {
            stale_session_failure(record, command, metadata_path)
        }
        SessionTarget::Android => {
            let Some(android) = &record.android else {
                return failure;
            };
            match godot_qa_process::android::query_pid(
                &android.adb_path,
                &android.device_serial,
                &android.package,
            ) {
                Ok(None) => stale_session_failure(record, command, metadata_path),
                Ok(Some(pid)) if pid != android.device_pid => {
                    stale_session_failure(record, command, metadata_path)
                }
                Ok(Some(_)) => failure,
                Err(query_failure) => {
                    failure.kind = "android_device_unavailable".to_string();
                    failure.message = Some(
                        "Android device liveness could not be queried; session metadata was retained"
                            .to_string(),
                    );
                    failure.expected = Some(json!("reachable authorized Android device"));
                    failure.actual = query_failure.actual;
                    failure
                        .details
                        .insert("metadata_retained".to_string(), json!(true));
                    failure
                        .details
                        .insert("device".to_string(), json!(android.device_serial));
                    failure
                }
            }
        }
        _ => failure,
    }
}

pub(super) fn session_status_like_result(config: &ProjectConfig, command: &str) -> CommandResult {
    let loaded = match load_session_record_with_source(config, command) {
        Ok(loaded) => loaded,
        Err(failure) => return unavailable_result(command, failure),
    };
    let record = loaded.record.clone();
    let mut result =
        run_live_protocol_command_with_loaded(command, SESSION_STATUS, Map::new(), loaded, None);
    if result.ok {
        let mut data = Map::from_iter([
            ("session_id".to_string(), json!(record.session_id)),
            ("pid".to_string(), json!(record.pid)),
            ("host".to_string(), json!(record.host)),
            ("port".to_string(), json!(record.port)),
            ("scene".to_string(), json!(record.scene)),
            (
                "viewport".to_string(),
                if record.target == SessionTarget::Android {
                    result
                        .data
                        .get("viewport")
                        .cloned()
                        .unwrap_or_else(|| json!(record.viewport))
                } else {
                    json!(record.viewport)
                },
            ),
            ("artifact_dir".to_string(), json!(record.artifact_dir)),
            ("started_at".to_string(), json!(record.started_at)),
            ("headless".to_string(), json!(record.headless)),
            (
                "display_server".to_string(),
                result
                    .data
                    .get("display_server")
                    .cloned()
                    .unwrap_or(Value::Null),
            ),
            ("runtime".to_string(), Value::Object(result.data.clone())),
        ]);
        if let Some(android) = &record.android {
            data.insert("target".to_string(), json!(record.target.as_str()));
            data.insert("device".to_string(), json!(android.device_serial));
            data.insert("package".to_string(), json!(android.package));
            data.insert("preset".to_string(), json!(android.preset));
            data.insert("device_pid".to_string(), json!(android.device_pid));
            data.insert("device_port".to_string(), json!(android.device_port));
            data.insert(
                "forwarded_host_port".to_string(),
                json!(android.forwarded_host_port),
            );
            data.insert(
                "forwarded_host_endpoint".to_string(),
                json!(format!("127.0.0.1:{}", android.forwarded_host_port)),
            );
            data.insert("deployment_mode".to_string(), json!("debug_apk"));
            data.insert(
                "artifacts".to_string(),
                json!({
                    "root": record.artifact_dir,
                    "deployment_log": android.deployment_log_path,
                    "logcat": android.logcat_log_path,
                }),
            );
        }
        if let Some(frame_pacing) = &record.frame_pacing {
            data.insert("frame_pacing".to_string(), frame_pacing.to_json());
        }
        if let Some(host_record) = load_debug_host_record(config) {
            data.insert(
                "debug_host".to_string(),
                json!({
                    "pid": host_record.pid,
                    "control_port": host_record.control_port,
                    "dbg_port": host_record.dbg_port,
                    "running": pid_is_running(host_record.pid),
                }),
            );
            data.insert("hot_reload".to_string(), json!(true));
        } else {
            data.insert("hot_reload".to_string(), json!(false));
        }
        result.data = data;
    }
    result
}

#[allow(clippy::too_many_arguments)]
pub fn session_start_result(
    config: &ProjectConfig,
    scene: Option<&str>,
    viewport_raw: &str,
    headless: bool,
    audio: bool,
    project_args: &[String],
    frame_pacing_request: Option<Option<i64>>,
    hot_reload: bool,
) -> CommandResult {
    let command = "session start";
    if let Some(failure) = config.project_config_failure_for_command(command) {
        return CommandResult {
            ok: false,
            command: command.to_string(),
            message: "unable to start live session".to_string(),
            data: Map::new(),
            failures: vec![failure],
            exit_code: 1,
        };
    }
    let Some(binary) = config.godot_binary.as_deref() else {
        return CommandResult {
            ok: false,
            command: command.to_string(),
            message: "unable to start live session".to_string(),
            data: Map::new(),
            failures: vec![FailureDetail {
                kind: "missing_godot".to_string(),
                command: Some(command.to_string()),
                message: Some("Godot binary not found in PATH or GODOT_BIN".to_string()),
                expected: Some(json!(
                    "discoverable Godot binary via --godot-bin, GODOT_BIN, or PATH"
                )),
                actual: Some(json!("missing")),
                ..FailureDetail::default()
            }],
            exit_code: 1,
        };
    };
    let Some(viewport) = parse_viewport(viewport_raw) else {
        return CommandResult {
            ok: false,
            command: command.to_string(),
            message: "unable to start live session".to_string(),
            data: Map::new(),
            failures: vec![FailureDetail {
                kind: "invalid_viewport".to_string(),
                command: Some(command.to_string()),
                message: Some("viewport must use WIDTHxHEIGHT".to_string()),
                expected: Some(json!("positive WIDTHxHEIGHT viewport string")),
                actual: Some(json!(viewport_raw)),
                ..FailureDetail::default()
            }],
            exit_code: 1,
        };
    };
    let frame_pacing = match resolve_live_frame_pacing(config, frame_pacing_request, command) {
        Ok(frame_pacing) => frame_pacing,
        Err(failure) => {
            return CommandResult {
                ok: false,
                command: command.to_string(),
                message: "unable to start live session".to_string(),
                data: Map::new(),
                failures: vec![failure],
                exit_code: 1,
            };
        }
    };
    let scene = match resolve_session_start_scene(config, scene) {
        Ok(scene) => scene,
        Err(failure) => {
            return CommandResult {
                ok: false,
                command: command.to_string(),
                message: "unable to start live session".to_string(),
                data: Map::new(),
                failures: vec![failure],
                exit_code: 1,
            };
        }
    };
    let effective_project_args = merged_args(&config.godot_project_args, project_args);
    match load_session_record(config, command) {
        Ok(existing) => {
            if session_liveness(&existing) != SessionLiveness::Stopped {
                return CommandResult {
                    ok: false,
                    command: command.to_string(),
                    message: "unable to start live session".to_string(),
                    data: Map::new(),
                    failures: vec![FailureDetail {
                        kind: "session_already_running".to_string(),
                        command: Some(command.to_string()),
                        message: Some("a live session is already running".to_string()),
                        expected: Some(json!("no active .qa/session.json")),
                        actual: Some(existing.to_json_value()),
                        ..FailureDetail::default()
                    }],
                    exit_code: 1,
                };
            }
            clear_session_files(config);
        }
        Err(failure) if failure.kind == "session_not_found" => {}
        Err(failure) => {
            return CommandResult {
                ok: false,
                command: command.to_string(),
                message: "unable to start live session".to_string(),
                data: Map::new(),
                failures: vec![failure],
                exit_code: 1,
            };
        }
    }
    let prepared = match prepare_live_session(
        config,
        &scene,
        viewport,
        headless,
        &effective_project_args,
        frame_pacing.clone(),
    ) {
        Ok(prepared) => prepared,
        Err(failure) => {
            return CommandResult {
                ok: false,
                command: command.to_string(),
                message: "unable to start live session".to_string(),
                data: Map::new(),
                failures: vec![failure],
                exit_code: 1,
            };
        }
    };
    // When hot reload is requested, bring up the debug-host sidecar first so it is
    // listening before Godot dials in via `--remote-debug`.
    let debug_host = if hot_reload {
        match spawn_debug_host(config, &prepared.token, &scene) {
            Ok(record) => Some(record),
            Err(failure) => {
                clear_session_files(config);
                return CommandResult {
                    ok: false,
                    command: command.to_string(),
                    message: "unable to start live session".to_string(),
                    data: Map::new(),
                    failures: vec![failure],
                    exit_code: 1,
                };
            }
        }
    } else {
        None
    };
    let remote_debug_url = debug_host
        .as_ref()
        .map(|record| format!("tcp://127.0.0.1:{}", record.dbg_port));
    let launch = match godot_qa_process::launch_godot_session(
        binary,
        &config.godot_launch_prefix,
        config.godot_joypad_isolation,
        &config.godot_windowed_launch_prefix,
        &config.project_root,
        &prepared.request_path,
        &prepared.logs_path,
        prepared.record.port,
        &prepared.token,
        headless,
        audio,
        Some(&prepared.record.viewport),
        frame_pacing.as_ref().map(|pacing| pacing.fixed_fps),
        &config.godot_args,
        &config.godot_project_args,
        project_args,
        remote_debug_url.as_deref(),
    ) {
        Ok(launch) => launch,
        Err(failure) => {
            clear_session_files(config);
            return CommandResult {
                ok: false,
                command: command.to_string(),
                message: "unable to start live session".to_string(),
                data: Map::new(),
                failures: vec![failure],
                exit_code: 1,
            };
        }
    };
    let persisted = SessionRecord {
        process_identity: godot_qa_process::process_identity(launch.pid),
        pid: launch.pid,
        ..prepared.record.clone()
    };
    match probe_live_session_startup(config, &persisted, &prepared.token) {
        Ok(response) => {
            if let Some(error) = response.get("error").and_then(Value::as_object) {
                let cleanup_failure = stop_launched_session_process(config, &persisted);
                clear_session_files(config);
                let mut failures = vec![FailureDetail {
                    kind: error
                        .get("type")
                        .and_then(Value::as_str)
                        .unwrap_or("runtime_error")
                        .to_string(),
                    command: Some(command.to_string()),
                    message: Some(
                        error
                            .get("message")
                            .and_then(Value::as_str)
                            .unwrap_or("unable to start live session")
                            .to_string(),
                    ),
                    ..FailureDetail::default()
                }];
                if let Some(cleanup_failure) = cleanup_failure {
                    failures.push(cleanup_failure);
                }
                return CommandResult {
                    ok: false,
                    command: command.to_string(),
                    message: error
                        .get("message")
                        .and_then(Value::as_str)
                        .unwrap_or("unable to start live session")
                        .to_string(),
                    data: Map::new(),
                    failures,
                    exit_code: 1,
                };
            }
            if let Err(failure) = persist_session(config, &persisted, &prepared.token) {
                let cleanup_failure = stop_launched_session_process(config, &persisted);
                clear_session_files(config);
                let mut failures = vec![failure];
                if let Some(cleanup_failure) = cleanup_failure {
                    failures.push(cleanup_failure);
                }
                return CommandResult {
                    ok: false,
                    command: command.to_string(),
                    message: "unable to start live session".to_string(),
                    data: Map::new(),
                    failures,
                    exit_code: 1,
                };
            }
            let mut data = Map::from_iter([
                ("session_id".to_string(), json!(persisted.session_id)),
                ("pid".to_string(), json!(persisted.pid)),
                ("host".to_string(), json!(persisted.host)),
                ("port".to_string(), json!(persisted.port)),
                ("scene".to_string(), json!(persisted.scene)),
                ("viewport".to_string(), json!(persisted.viewport)),
                ("artifact_dir".to_string(), json!(persisted.artifact_dir)),
                ("started_at".to_string(), json!(persisted.started_at)),
                ("headless".to_string(), json!(persisted.headless)),
                (
                    "audio_dir".to_string(),
                    json!(Path::new(&persisted.artifact_dir).join("audio")),
                ),
                ("godot_args".to_string(), json!(config.godot_args)),
                ("project_args".to_string(), json!(persisted.project_args)),
                (
                    "launch_display".to_string(),
                    display_launch_metadata_json(&launch.display),
                ),
                (
                    "display_server".to_string(),
                    response
                        .get("data")
                        .and_then(Value::as_object)
                        .and_then(|data| data.get("display_server"))
                        .or_else(|| response.get("display_server"))
                        .cloned()
                        .unwrap_or(Value::Null),
                ),
                (
                    "runtime".to_string(),
                    response.get("data").cloned().unwrap_or(Value::Null),
                ),
            ]);
            if let Some(frame_pacing) = &persisted.frame_pacing {
                data.insert("frame_pacing".to_string(), frame_pacing.to_json());
            }
            data.insert("hot_reload".to_string(), json!(hot_reload));
            if let Some(record) = &debug_host {
                data.insert(
                    "debug_host".to_string(),
                    json!({
                        "pid": record.pid,
                        "dbg_port": record.dbg_port,
                        "control_port": record.control_port,
                    }),
                );
            }
            CommandResult {
                ok: true,
                command: command.to_string(),
                message: "live session started".to_string(),
                data,
                failures: Vec::new(),
                exit_code: 0,
            }
        }
        Err(failure) => {
            let cleanup_failure = stop_launched_session_process(config, &persisted);
            clear_session_files(config);
            let mut failures = vec![failure];
            if let Some(cleanup_failure) = cleanup_failure {
                failures.push(cleanup_failure);
            }
            CommandResult {
                ok: false,
                command: command.to_string(),
                message: "unable to start live session".to_string(),
                data: Map::new(),
                failures,
                exit_code: 1,
            }
        }
    }
}

pub fn session_stop_result(config: &ProjectConfig) -> CommandResult {
    let command = "session stop";
    let record = match load_session_record(config, command) {
        Ok(record) => record,
        Err(failure) => return unavailable_result(command, failure),
    };
    let result = run_live_protocol_command(config, command, SESSION_STOP, Map::new());
    if record.target == SessionTarget::Android {
        let Some(android) = &record.android else {
            return unavailable_result(
                command,
                session_failure(
                    "session_file_invalid",
                    command,
                    &session_json_path(config),
                    "Android session metadata is missing",
                    json!("session.android metadata"),
                    json!("missing"),
                ),
            );
        };
        let mut cleanup_failures = Vec::new();
        let still_running = match godot_qa_process::android::query_pid(
            &android.adb_path,
            &android.device_serial,
            &android.package,
        ) {
            Ok(pid) => pid.is_some(),
            Err(failure) => {
                cleanup_failures.push(failure);
                true
            }
        };
        if still_running
            && let Err(failure) = godot_qa_process::android::force_stop(
                &android.adb_path,
                &android.device_serial,
                &android.package,
            )
        {
            cleanup_failures.push(failure);
        }
        if let Some(logcat_pid) = android.logcat_pid
            && let Err(failure) = godot_qa_process::android::stop_logcat_with_identity(
                logcat_pid,
                android.logcat_process_identity,
            )
        {
            cleanup_failures.push(failure);
        }
        if let Err(failure) = godot_qa_process::android::remove_exact_forward(
            &android.adb_path,
            &android.device_serial,
            android.forwarded_host_port,
            android.device_port,
        ) {
            cleanup_failures.push(failure);
        }
        if let Err(failure) = godot_qa_process::android::remove_private_session(
            &android.adb_path,
            &android.device_serial,
            &android.package,
            &record.session_id,
        ) {
            cleanup_failures.push(failure);
        }
        if let Err(failure) = godot_qa_process::android::remove_private_bootstrap(
            &android.adb_path,
            &android.device_serial,
            &android.package,
        ) {
            cleanup_failures.push(failure);
        }
        if cleanup_failures.is_empty() {
            clear_session_files(config);
            return CommandResult {
                ok: true,
                command: command.to_string(),
                message: if result.ok {
                    "Android live session stopped".to_string()
                } else {
                    "Android live session stopped after ADB fallback".to_string()
                },
                data: Map::from_iter([
                    ("target".to_string(), json!("android")),
                    ("session_id".to_string(), json!(record.session_id)),
                    ("device".to_string(), json!(android.device_serial)),
                    ("package".to_string(), json!(android.package)),
                    ("bridge_stop_ok".to_string(), json!(result.ok)),
                    ("forward_removed".to_string(), json!(true)),
                    ("private_session_removed".to_string(), json!(true)),
                ]),
                failures: Vec::new(),
                exit_code: 0,
            };
        }
        return CommandResult {
            ok: false,
            command: command.to_string(),
            message:
                "Android live session cleanup was incomplete; recoverable metadata was retained"
                    .to_string(),
            data: Map::from_iter([
                ("target".to_string(), json!("android")),
                ("session_id".to_string(), json!(record.session_id)),
                ("device".to_string(), json!(android.device_serial)),
                ("package".to_string(), json!(android.package)),
                ("metadata_retained".to_string(), json!(true)),
            ]),
            failures: cleanup_failures,
            exit_code: 1,
        };
    }
    if result.ok {
        if !godot_qa_process::wait_for_godot_process_exit(
            record.pid,
            PROCESS_STOP_WAIT_TIMEOUT_SECONDS,
        ) {
            match godot_qa_process::stop_godot_process_with_identity(
                record.pid,
                PROCESS_STOP_WAIT_TIMEOUT_SECONDS,
                record.process_identity,
            ) {
                Ok(fallback) => {
                    clear_session_files(config);
                    return CommandResult {
                        ok: true,
                        command: command.to_string(),
                        message: "live session stopped after process fallback".to_string(),
                        data: Map::from_iter([
                            ("fallback".to_string(), json!(fallback)),
                            ("session_id".to_string(), json!(record.session_id)),
                        ]),
                        failures: Vec::new(),
                        exit_code: 0,
                    };
                }
                Err(failure) => {
                    return CommandResult {
                        ok: false,
                        command: command.to_string(),
                        message: "live session did not stop cleanly".to_string(),
                        data: Map::new(),
                        failures: vec![failure],
                        exit_code: 1,
                    };
                }
            }
        }
        clear_session_files(config);
        return result;
    }
    let failure = result.failures.first().cloned();
    if let Some(failure) = failure {
        if matches!(
            failure.kind.as_str(),
            "session_connect_failed" | "session_request_failed" | "stale_session"
        ) && session_liveness(&record) == SessionLiveness::Stopped
        {
            clear_session_files(config);
            return CommandResult {
                ok: true,
                command: command.to_string(),
                message: "cleaned stale live session metadata".to_string(),
                data: Map::from_iter([
                    ("stale".to_string(), json!(true)),
                    ("session_id".to_string(), json!(record.session_id)),
                ]),
                failures: Vec::new(),
                exit_code: 0,
            };
        }
        if matches!(
            failure.kind.as_str(),
            "session_connect_failed"
                | "session_request_failed"
                | "session_bridge_busy"
                | "resource_unavailable"
                | "resource_busy"
                | "bridge_busy"
        ) && godot_qa_process::process_identity_matches(record.pid, record.process_identity)
        {
            match godot_qa_process::stop_godot_process_with_identity(
                record.pid,
                PROCESS_STOP_WAIT_TIMEOUT_SECONDS,
                record.process_identity,
            ) {
                Ok(fallback) => {
                    clear_session_files(config);
                    return CommandResult {
                        ok: true,
                        command: command.to_string(),
                        message: "terminated live session after bridge request failed".to_string(),
                        data: Map::from_iter([
                            ("fallback".to_string(), json!(fallback)),
                            ("session_id".to_string(), json!(record.session_id)),
                        ]),
                        failures: Vec::new(),
                        exit_code: 0,
                    };
                }
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
            }
        }
    }
    result
}

pub fn session_profile_result(
    config: &ProjectConfig,
    frames: Option<i64>,
    seconds: Option<f64>,
    max_frame_samples: i64,
    top: i64,
) -> CommandResult {
    let command = "session profile";
    let record = match load_session_record(config, command) {
        Ok(record) => record,
        Err(failure) => return unavailable_result(command, failure),
    };
    let mut params = Map::from_iter([
        (
            "max_frame_samples".to_string(),
            json!(max_frame_samples.max(0)),
        ),
        ("top".to_string(), json!(top.max(0))),
    ]);
    if let Some(frames) = frames {
        params.insert("frames".to_string(), json!(frames));
    }
    if let Some(seconds) = seconds {
        params.insert("seconds".to_string(), json!(seconds));
    }
    let live_result = run_live_profile_protocol_command(config, command, &record, params);
    if !live_result.ok {
        return live_result;
    }
    let mut runtime_profile_data = live_result.data.clone();
    let retry = runtime_profile_data.remove("retry");
    let runtime_profile = Value::Object(runtime_profile_data);
    let Some(profile_object) = runtime_profile.as_object() else {
        return session_profile_invalid_result(
            command,
            "session.profile response data must be an object",
            json!("JSON object"),
            json!(type_name(&runtime_profile)),
            retry,
        );
    };
    let frame_samples = match profile::optional_number_array(profile_object.get("frame_times_ms")) {
        Ok(samples) => samples,
        Err(actual) => {
            return session_profile_invalid_result(
                command,
                "profile.frame_times_ms must be an array of non-negative numbers",
                json!("JSON array of non-negative numbers"),
                actual,
                retry,
            );
        }
    };
    let monitor_samples =
        match profile::optional_object_array(profile_object.get("monitor_samples")) {
            Ok(samples) => samples,
            Err(actual) => {
                return session_profile_invalid_result(
                    command,
                    "profile.monitor_samples must be an array of objects",
                    json!("JSON array of JSON objects"),
                    actual,
                    retry,
                );
            }
        };
    let frame_sample_limit = max_frame_samples.max(0) as usize;
    let percentiles = profile::profile_percentiles(&frame_samples);
    let profile_dir = Path::new(&record.artifact_dir)
        .join("profiles")
        .join(unique_profile_id());
    let artifact_path = profile_dir.join("profile.json");
    if let Err(error) = fs::create_dir_all(&profile_dir) {
        return session_profile_artifact_failed(command, &artifact_path, error, retry);
    }
    let sample_window = profile_object
        .get("sample_window")
        .cloned()
        .unwrap_or_else(|| json!({ "frames": frames, "seconds": seconds }));
    let sample_limits = json!({
        "max_frame_samples": frame_sample_limit,
        "top": top.max(0),
    });
    let artifact = json!({
        "session_id": record.session_id.clone(),
        "scene": record.scene.clone(),
        "viewport": record.viewport.clone(),
        "duration_ms": profile_object.get("duration_ms").cloned().unwrap_or(Value::Null),
        "sample_window": sample_window,
        "sample_limits": sample_limits,
        "frame_times_ms": profile::bounded_frame_values(&frame_samples, frame_sample_limit),
        "frame_samples": profile_object.get("frame_samples").cloned().unwrap_or_else(|| json!([])),
        "frame_time_percentiles_ms": profile::profile_percentiles_json(&percentiles),
        "monitor_samples": monitor_samples,
        "monitor_counters": profile::monitor_counter_summaries(&monitor_samples),
        "unsupported_monitors": profile_object.get("unsupported_monitors").cloned().unwrap_or_else(|| json!([])),
        "node_process_counts": profile_object.get("node_process_counts").cloned().unwrap_or_else(|| json!({})),
        "resource_activity": profile_object.get("resource_activity").cloned().unwrap_or_else(|| json!({"status": "unavailable"})),
        "top_script_methods": profile::top_script_methods_or_empty(profile_object.get("top_script_methods")),
    });
    if let Err(mut failure) = write_json(&artifact_path, &artifact) {
        failure.command = Some(command.to_string());
        failure.path = Some(artifact_path.to_string_lossy().into_owned());
        failure.expected = Some(json!(
            "profile.json written under the live session profile root"
        ));
        let mut data = Map::new();
        if let Some(retry) = retry {
            data.insert("retry".to_string(), retry);
        }
        return CommandResult {
            ok: false,
            command: command.to_string(),
            message: "live session profile failed".to_string(),
            data,
            failures: vec![failure],
            exit_code: 1,
        };
    }
    let mut artifacts = Map::new();
    artifacts.insert("profile".to_string(), json!(artifact_path));
    artifacts.insert("root".to_string(), json!(profile_dir));
    let mut data = Map::from_iter([
        ("session_id".to_string(), json!(record.session_id)),
        ("pid".to_string(), json!(record.pid)),
        ("scene".to_string(), json!(record.scene)),
        ("viewport".to_string(), json!(record.viewport)),
        ("headless".to_string(), json!(record.headless)),
        ("artifact_dir".to_string(), json!(record.artifact_dir)),
        ("sample_window".to_string(), sample_window),
        ("artifacts".to_string(), Value::Object(artifacts)),
        (
            "profile".to_string(),
            json!({
                "frame_time_ms": profile::profile_percentiles_json(&percentiles),
                "frame_time_percentiles_ms": profile::profile_percentiles_json(&percentiles),
                "monitor_counters": profile::monitor_counter_summaries(&monitor_samples),
                "monitor_sample_count": monitor_samples.len(),
                "node_process_counts": artifact.get("node_process_counts").cloned().unwrap_or_else(|| json!({})),
                "resource_activity": artifact.get("resource_activity").cloned().unwrap_or_else(|| json!({"status": "unavailable"})),
                "top_script_methods": artifact.get("top_script_methods").cloned().unwrap_or_else(profile::empty_top_script_methods_json),
                "sample_limits": sample_limits,
            }),
        ),
    ]);
    if let Some(retry) = retry {
        data.insert("retry".to_string(), retry);
    }
    CommandResult {
        ok: true,
        command: command.to_string(),
        message: "live session profile completed".to_string(),
        data,
        failures: Vec::new(),
        exit_code: 0,
    }
}

pub(super) fn parse_viewport(value: &str) -> Option<Vec<i64>> {
    let normalized = value.to_ascii_lowercase();
    let (width, height) = normalized.split_once('x')?;
    let width = width.parse::<i64>().ok()?;
    let height = height.parse::<i64>().ok()?;
    if width <= 0 || height <= 0 {
        return None;
    }
    Some(vec![width, height])
}

pub fn resolve_live_frame_pacing(
    config: &ProjectConfig,
    request: Option<Option<i64>>,
    command: &str,
) -> Result<Option<SessionFramePacing>, FailureDetail> {
    match request {
        None => Ok(None),
        Some(Some(fixed_fps)) => validate_live_fixed_fps(fixed_fps, command).map(|fixed_fps| {
            Some(SessionFramePacing {
                fixed_fps,
                source: "session".to_string(),
                project_setting: None,
            })
        }),
        Some(None) => Ok(Some(project_default_live_frame_pacing(
            &config.project_root,
        ))),
    }
}

pub(super) fn validate_live_fixed_fps(fixed_fps: i64, command: &str) -> Result<i64, FailureDetail> {
    if (1..=MAX_SAFE_FIXED_FPS).contains(&fixed_fps) {
        return Ok(fixed_fps);
    }
    Err(FailureDetail {
        kind: "invalid_params".to_string(),
        command: Some(command.to_string()),
        message: Some(
            "fixed_fps must be a positive integer within the supported range".to_string(),
        ),
        expected: Some(json!(format!("integer from 1 to {MAX_SAFE_FIXED_FPS}"))),
        actual: Some(json!(fixed_fps)),
        ..FailureDetail::default()
    })
}

pub(super) fn project_default_live_frame_pacing(project_root: &Path) -> SessionFramePacing {
    let project_settings = parse_project_settings(&project_root.join("project.godot"));
    for key in [PROJECT_MAX_FPS_KEY, PROJECT_PHYSICS_TICKS_KEY] {
        if let Some(value) = project_settings
            .get(key)
            .copied()
            .filter(|value| *value > 0)
        {
            return SessionFramePacing {
                fixed_fps: value,
                source: "project_setting".to_string(),
                project_setting: Some(key.to_string()),
            };
        }
    }
    SessionFramePacing {
        fixed_fps: DEFAULT_FIXED_FPS,
        source: "default".to_string(),
        project_setting: None,
    }
}

pub(super) fn parse_project_settings(path: &Path) -> std::collections::HashMap<String, i64> {
    let Some(contents) = read_project_godot_cached(path) else {
        return std::collections::HashMap::new();
    };
    let mut section = String::new();
    let mut settings = std::collections::HashMap::new();
    for raw_line in contents.lines() {
        let line = raw_line.trim();
        if line.is_empty() || line.starts_with(';') {
            continue;
        }
        if line.starts_with('[') && line.ends_with(']') {
            section = line[1..line.len() - 1].to_string();
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let full_key = if section.is_empty() {
            key.trim().to_string()
        } else {
            format!("{}/{}", section, key.trim())
        };
        if full_key != PROJECT_MAX_FPS_KEY && full_key != PROJECT_PHYSICS_TICKS_KEY {
            continue;
        }
        let value = value
            .trim()
            .trim_matches('"')
            .parse::<i64>()
            .unwrap_or_default();
        settings.insert(full_key, value);
    }
    settings
}

pub(super) fn resolve_session_start_scene(
    config: &ProjectConfig,
    explicit_scene: Option<&str>,
) -> Result<String, FailureDetail> {
    if let Some(scene) = explicit_scene {
        let trimmed = scene.trim();
        // An EXPLICIT empty scene ("") is a sceneless boot request: the warm
        // bundle path passes it so the session comes up with NO scene and the
        // per-scenario QaScenarioExecutor owns the full scene lifecycle. Booting
        // the project main scene here would leave the game's menu/main scene live
        // ALONGSIDE the scenario scene — it would receive the same global input a
        // `drive_policy` step injects and spawn phantom matches, diverging warm
        // from cold. `None` (no --scene at all) still falls back to the project
        // main scene for a classic live session.
        if trimmed.is_empty() {
            return Ok(String::new());
        }
        return Ok(trimmed.to_string());
    }
    let project_path = config.project_root.join("project.godot");
    let contents = fs::read_to_string(&project_path).map_err(|error| FailureDetail {
        kind: "missing_main_scene".to_string(),
        command: Some("session start".to_string()),
        path: Some(project_path.to_string_lossy().into_owned()),
        message: Some(error.to_string()),
        expected: Some(json!(
            "run/main_scene in project.godot or explicit --scene <scene>"
        )),
        actual: Some(json!("project.godot unreadable")),
        ..FailureDetail::default()
    })?;
    parse_project_main_scene(&contents).ok_or_else(|| FailureDetail {
        kind: "missing_main_scene".to_string(),
        command: Some("session start".to_string()),
        path: Some(project_path.to_string_lossy().into_owned()),
        message: Some("project.godot does not define application run/main_scene".to_string()),
        expected: Some(json!(
            "run/main_scene in project.godot or explicit --scene <scene>"
        )),
        actual: Some(json!("missing")),
        ..FailureDetail::default()
    })
}

/// The session's launch scene (`res://…`) from `project.godot`, if any.
pub(super) fn parse_project_main_scene(contents: &str) -> Option<String> {
    let mut in_application = false;
    for raw_line in contents.lines() {
        let line = raw_line.trim();
        if line.is_empty() || line.starts_with(';') {
            continue;
        }
        if line.starts_with('[') && line.ends_with(']') {
            in_application = line == "[application]";
            continue;
        }
        if !in_application {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        if key.trim() != "run/main_scene" {
            continue;
        }
        let parsed = parse_project_setting_string(value.trim());
        if parsed.is_empty() {
            return None;
        }
        return Some(parsed);
    }
    None
}

pub(super) fn parse_project_setting_string(value: &str) -> String {
    if value.len() >= 2 && value.starts_with('"') && value.ends_with('"') {
        return value[1..value.len() - 1]
            .replace("\\\"", "\"")
            .replace("\\\\", "\\");
    }
    value.to_string()
}

pub(super) fn stop_launched_session_process(
    config: &ProjectConfig,
    record: &SessionRecord,
) -> Option<FailureDetail> {
    if record.target == SessionTarget::Android {
        return Some(FailureDetail {
            kind: "android_cleanup_unavailable".to_string(),
            command: Some("session start".to_string()),
            path: Some(session_json_path(config).to_string_lossy().into_owned()),
            message: Some(
                "Android session cleanup requires the Android process adapter".to_string(),
            ),
            expected: Some(json!("target-aware Android cleanup")),
            actual: Some(json!("unavailable")),
            ..FailureDetail::default()
        });
    }
    if godot_qa_process::wait_for_godot_process_exit(record.pid, PROCESS_STOP_WAIT_TIMEOUT_SECONDS)
    {
        return None;
    }
    godot_qa_process::stop_godot_process_with_identity(
        record.pid,
        PROCESS_STOP_WAIT_TIMEOUT_SECONDS,
        record.process_identity,
    )
    .err()
    .map(|mut failure| {
        if failure.command.is_none() {
            failure.command = Some("session start".to_string());
        }
        if failure.path.is_none() {
            failure.path = Some(session_json_path(config).to_string_lossy().into_owned());
        }
        failure
    })
}
