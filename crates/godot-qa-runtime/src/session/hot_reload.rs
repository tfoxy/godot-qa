use super::*;

pub(super) fn asset_trial_reload_lease(
    config: &ProjectConfig,
) -> Result<godot_qa_process::project_access::LaunchLease, CommandResult> {
    godot_qa_process::project_access::LaunchLease::acquire(
        &config.project_root,
        godot_qa_process::project_access::LaunchClass::SourceWriter,
    )
    .map_err(|failure| CommandResult {
        ok: false,
        command: "session reload".into(),
        message: "project is reserved by an asset trial".into(),
        data: Map::new(),
        failures: vec![failure],
        exit_code: 1,
    })
}

pub fn session_reload_result(config: &ProjectConfig) -> CommandResult {
    let _project_lease = match asset_trial_reload_lease(config) {
        Ok(lease) => lease,
        Err(result) => return result,
    };
    let result =
        run_live_protocol_command(config, "session reload", SESSION_RELOAD_SCENE, Map::new());
    if result.ok
        && debug_host_available(config)
        && let Some(scene) = result.data.get("scene").and_then(Value::as_str)
        && let Err(failure) = debug_host_send(
            config,
            "session reload",
            &godot_qa_debug::host::ControlRequest::confirm_full_reload(vec![scene.to_string()]),
        )
    {
        return failure;
    }
    result
}

/// Engine-faithful, state-preserving soft reload of specific scripts via the debug host.
pub fn session_reload_scripts_result(config: &ProjectConfig, scripts: &[String]) -> CommandResult {
    granular_reload_result(
        config,
        "session reload",
        godot_qa_debug::host::ControlRequest::reload_scripts(scripts.to_vec()),
    )
}

/// Live reload of specific resources (`scene:reload_cached_files`) via the debug host.
pub fn session_reload_resources_result(
    config: &ProjectConfig,
    resources: &[String],
) -> CommandResult {
    granular_reload_result(
        config,
        "session reload",
        godot_qa_debug::host::ControlRequest::reload_resources(resources.to_vec()),
    )
}

/// Resource reloads (`scene:reload_cached_files`) are fire-and-forget over the
/// remote-debugger protocol (no ack), and their handler is compiled only under
/// `DEBUG_ENABLED` — so on a release-template export the game silently ignores them.
/// We therefore never claim a resource reload was "applied", only that it was requested.
pub const RESOURCE_RELOAD_CAVEAT: &str = "resource reload is best-effort and unconfirmed; \
    it requires a debug or editor build (DEBUG_ENABLED) and is silently ignored by \
    release-template exports. Script reloads work in any build.";

/// Build the success `CommandResult` for a granular reload. Resource reloads report
/// `status:"requested"` (never a false "applied") plus the [`RESOURCE_RELOAD_CAVEAT`];
/// script reloads (handled in any build) report `status:"applied"`.
pub(super) fn granular_reload_success(
    command: &str,
    op: &str,
    applied: Vec<String>,
    connected: bool,
) -> CommandResult {
    let mut data = Map::from_iter([
        ("op".to_string(), json!(op)),
        ("connected".to_string(), json!(connected)),
    ]);
    let message = if op == "reload_resources" {
        // Fire-and-forget and DEBUG_ENABLED-gated: report "requested", not "applied".
        data.insert("status".to_string(), json!("requested"));
        data.insert("requested".to_string(), json!(applied));
        data.insert("caveat".to_string(), json!(RESOURCE_RELOAD_CAVEAT));
        "requested resource hot reload (unconfirmed)"
    } else {
        data.insert("status".to_string(), json!("applied"));
        data.insert("applied".to_string(), json!(applied));
        "requested hot reload"
    };
    CommandResult {
        ok: true,
        command: command.to_string(),
        message: message.to_string(),
        data,
        failures: Vec::new(),
        exit_code: 0,
    }
}

/// Structural live-edit is applied over the same DEBUG_ENABLED-gated `scene:live_*`
/// protocol, so like resource reload it is best-effort and unconfirmed on release builds.
pub const STRUCTURE_RELOAD_CAVEAT: &str = "structural live-edit is best-effort and \
    unconfirmed; it requires a debug or editor build (DEBUG_ENABLED) and is silently \
    ignored by release-template exports.";

/// Build a shared "granular hot reload is unavailable" failure `CommandResult`.
pub(super) fn granular_failure(
    command: &str,
    kind: &str,
    message: &str,
    expected: Value,
    actual: Value,
) -> CommandResult {
    CommandResult {
        ok: false,
        command: command.to_string(),
        message: "granular hot reload is unavailable".to_string(),
        data: Map::new(),
        failures: vec![FailureDetail {
            kind: kind.to_string(),
            command: Some(command.to_string()),
            message: Some(message.to_string()),
            expected: Some(expected),
            actual: Some(actual),
            ..FailureDetail::default()
        }],
        exit_code: 1,
    }
}

/// Load the debug-host record, verify its pid is alive, and send a control request.
/// Returns the successful `ControlResponse`, or a failure `CommandResult` for the
/// not-enabled / dead-pid / rejected / unreachable cases shared by all granular reloads.
pub(super) fn debug_host_send(
    config: &ProjectConfig,
    command: &str,
    request: &godot_qa_debug::host::ControlRequest,
) -> Result<godot_qa_debug::host::ControlResponse, CommandResult> {
    let _project_lease = asset_trial_reload_lease(config)?;
    if let Ok(record) = load_session_record(config, command)
        && record.target == SessionTarget::Android
    {
        return Err(granular_failure(
            command,
            "android_hot_reload_unsupported",
            "Android live sessions can reload packaged scenes but cannot reload host files",
            json!("plain `session reload` without --script/--resource/--scene"),
            json!(request.op),
        ));
    }
    let Some(record) = load_debug_host_record(config) else {
        return Err(granular_failure(
            command,
            "hot_reload_not_enabled",
            "no debug host is running; start the session with `session start --hot-reload`",
            json!("an active debug host (.qa/debug_host.json)"),
            json!("missing"),
        ));
    };
    if !godot_qa_process::process_identity_matches(record.pid, record.process_identity) {
        return Err(granular_failure(
            command,
            "hot_reload_unavailable",
            "the debug host process is not running",
            json!("a live debug host process"),
            json!(format!("dead pid {}", record.pid)),
        ));
    }
    let token = load_session_record(config, command)
        .and_then(|session| read_session_token(&session, command))
        .map_err(|failure| CommandResult {
            ok: false,
            command: command.to_string(),
            message: "unable to authenticate hot reload request".to_string(),
            data: Map::new(),
            failures: vec![failure],
            exit_code: 1,
        })?;
    match godot_qa_debug::host::send_control_request(record.control_port, &token, request) {
        Ok(response) if response.ok => Ok(response),
        Ok(response) => Err(CommandResult {
            ok: false,
            command: command.to_string(),
            message: "hot reload request was rejected".to_string(),
            data: Map::new(),
            failures: vec![FailureDetail {
                kind: "hot_reload_rejected".to_string(),
                command: Some(command.to_string()),
                message: response
                    .error
                    .or_else(|| Some("debug host rejected the request".to_string())),
                expected: Some(json!("ok reload response")),
                actual: Some(json!({"connected": response.connected})),
                ..FailureDetail::default()
            }],
            exit_code: 1,
        }),
        Err(error) => Err(granular_failure(
            command,
            "hot_reload_unavailable",
            &error.to_string(),
            json!("reachable debug host control channel"),
            json!("connection error"),
        )),
    }
}

pub(super) fn granular_reload_result(
    config: &ProjectConfig,
    command: &str,
    request: godot_qa_debug::host::ControlRequest,
) -> CommandResult {
    match debug_host_send(config, command, &request) {
        Ok(response) => {
            granular_reload_success(command, &request.op, response.applied, response.connected)
        }
        Err(failure) => failure,
    }
}

/// Faithful `.tscn` structural live-edit of the given scenes via the debug host. Reports
/// which scenes applied live and which `fell_back` (the caller full-scene-reloads those).
pub fn session_reload_structure_result(config: &ProjectConfig, scenes: &[String]) -> CommandResult {
    let command = "session reload";
    let request = godot_qa_debug::host::ControlRequest::reload_structure(scenes.to_vec());
    match debug_host_send(config, command, &request) {
        Ok(response) => {
            let mut data = Map::from_iter([
                ("op".to_string(), json!("reload_structure")),
                ("connected".to_string(), json!(response.connected)),
                ("status".to_string(), json!("requested")),
                ("applied".to_string(), json!(response.applied)),
                ("caveat".to_string(), json!(STRUCTURE_RELOAD_CAVEAT)),
            ]);
            if let Some(ops) = response.ops_applied {
                data.insert("ops_applied".to_string(), json!(ops));
            }
            if !response.fell_back.is_empty() {
                data.insert("fell_back".to_string(), json!(response.fell_back));
            }
            CommandResult {
                ok: true,
                command: command.to_string(),
                message: "requested structural hot reload".to_string(),
                data,
                failures: Vec::new(),
                exit_code: 0,
            }
        }
        Err(failure) => failure,
    }
}

const DEBUG_HOST_READY_TIMEOUT_SECONDS: f64 = 8.0;

/// `.qa/debug_host.json`, a sibling of `session.json`. Written by the debug-host
/// sidecar itself once it has bound both its debugger and control listeners.
pub fn debug_host_json_path(config: &ProjectConfig) -> PathBuf {
    match session_json_path(config).parent() {
        Some(parent) => parent.join("debug_host.json"),
        None => PathBuf::from("debug_host.json"),
    }
}

pub fn load_debug_host_record(
    config: &ProjectConfig,
) -> Option<godot_qa_debug::host::DebugHostRecord> {
    let contents = fs::read_to_string(debug_host_json_path(config)).ok()?;
    serde_json::from_str(&contents).ok()
}

/// Whether a hot-reload debug host is running for this project (record present and pid alive).
pub fn debug_host_available(config: &ProjectConfig) -> bool {
    load_debug_host_record(config)
        .map(|record| {
            godot_qa_process::process_identity_matches(record.pid, record.process_identity)
        })
        .unwrap_or(false)
}

/// Spawn the persistent debug-host sidecar (this binary re-invoked as `__debug-host`)
/// and wait for it to bind its listeners. The sidecar picks ephemeral ports itself and
/// records them, which removes any pick/bind race with the parent.
pub(super) fn spawn_debug_host(
    config: &ProjectConfig,
    token: &str,
    scene: &str,
) -> Result<godot_qa_debug::host::DebugHostRecord, FailureDetail> {
    let _project_lease = godot_qa_process::project_access::LaunchLease::acquire(
        &config.project_root,
        godot_qa_process::project_access::LaunchClass::SourceWriter,
    )?;
    let record_path = debug_host_json_path(config);
    let _ = fs::remove_file(&record_path);
    if let Some(parent) = record_path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    // Publish the same per-session credential before the sidecar reads it at bind.
    // The existing private writer creates owner-only temporary files atomically.
    write_private_file(&session_token_path(config), token.as_bytes())
        .map_err(|error| artifact_write_failure(&session_token_path(config), &error))?;
    let exe = std::env::current_exe().map_err(|error| FailureDetail {
        kind: "debug_host_spawn_failed".to_string(),
        command: Some("session start".to_string()),
        message: Some(format!("could not resolve godot-qa executable: {error}")),
        expected: Some(json!("resolvable current executable")),
        actual: Some(json!("unavailable")),
        ..FailureDetail::default()
    })?;
    let mut command = debug_host_command(&exe, config, scene);
    command
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    #[cfg(unix)]
    unsafe {
        use std::os::unix::process::CommandExt;
        command.pre_exec(|| {
            if libc::setsid() == -1 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
    godot_qa_process::configure_background_command(&mut command);
    let child = command.spawn().map_err(|error| FailureDetail {
        kind: "debug_host_spawn_failed".to_string(),
        command: Some("session start".to_string()),
        message: Some(error.to_string()),
        expected: Some(json!("debug host process starts")),
        actual: Some(json!("OSError")),
        ..FailureDetail::default()
    })?;
    let child_pid = child.id() as i64;
    godot_qa_process::reap_background_child(child);
    let deadline = SystemTime::now() + Duration::from_secs_f64(DEBUG_HOST_READY_TIMEOUT_SECONDS);
    loop {
        if let Some(mut record) = load_debug_host_record(config)
            && record.bound
        {
            record.process_identity = godot_qa_process::process_identity(child_pid);
            write_json(&record_path, &serde_json::to_value(&record).unwrap())?;
            return Ok(record);
        }
        if !pid_is_running(child_pid) || SystemTime::now() >= deadline {
            break;
        }
        thread::sleep(Duration::from_millis(50));
    }
    let _ = godot_qa_process::stop_godot_process(child_pid, PROCESS_STOP_WAIT_TIMEOUT_SECONDS);
    let _ = fs::remove_file(&record_path);
    Err(FailureDetail {
        kind: "debug_host_start_failed".to_string(),
        command: Some("session start".to_string()),
        message: Some("debug host did not bind its listener before timeout".to_string()),
        expected: Some(json!("debug_host.json with bound=true")),
        actual: Some(json!("missing or unbound")),
        ..FailureDetail::default()
    })
}

pub(super) fn debug_host_command(
    exe: &Path,
    config: &ProjectConfig,
    scene: &str,
) -> std::process::Command {
    let record_path = debug_host_json_path(config);
    let log_path = record_path.with_file_name("debug_host.log");
    let mut command = std::process::Command::new(exe);
    command
        .arg("__debug-host")
        .arg("--record")
        .arg(&record_path)
        .arg("--log")
        .arg(&log_path)
        .arg("--project-root")
        .arg(&config.project_root);
    // Give the sidecar the launch scene so it can baseline-snapshot it for structural
    // (`.tscn`) live-edit diffs.
    command.arg("--scene").arg(scene);
    command
}

/// Stop the debug-host sidecar (if any) and remove its record. Safe to call when no
/// hot-reload session is running.
pub(super) fn stop_debug_host(config: &ProjectConfig) {
    if let Some(record) = load_debug_host_record(config)
        && godot_qa_process::process_identity_matches(record.pid, record.process_identity)
    {
        let _ = godot_qa_process::stop_godot_process_with_identity(
            record.pid,
            PROCESS_STOP_WAIT_TIMEOUT_SECONDS,
            record.process_identity,
        );
    }
    let _ = fs::remove_file(debug_host_json_path(config));
}
