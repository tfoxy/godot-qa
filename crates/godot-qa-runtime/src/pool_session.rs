// Multi-concurrent live-session support.
//
// The interactive workflow (`session start/stop/status/...`) is a SINGLETON: one
// `.qa/session.json` + `.qa/session.token` per project. That singleton cannot host
// more than one session, so the warm bundle POOL cannot use it for K concurrent
// sessions. This module adds pooled sessions that are addressed by an explicit
// in-memory handle (`PooledSessionHandle`) instead of the singleton files.
//
// Each pooled session is fully independent: its own ephemeral port (already random
// per session, see `pick_port`), its own random token, and its own isolated
// `.qa/artifacts/live/<session-id>/` directory + `session.json`. Because the
// transport addresses a session purely by `record.host`/`record.port` and a token
// override (see `send_live_request`), K pooled sessions coexist without ever
// touching the singleton — so the existing single-session commands keep working
// unchanged while the pool drives K sessions concurrently.

/// A handle to one independently-addressable live session started for the pool.
/// Carries the full [`SessionRecord`] (host/port/pid/artifact dir) and the session
/// token in memory, so the singleton `.qa/session.json` / `.qa/session.token` is
/// never read or written for a pooled session.
#[derive(Debug, Clone)]
pub struct PooledSessionHandle {
    pub record: SessionRecord,
    pub token: String,
}

impl PooledSessionHandle {
    /// The session's fixed_fps (frame pacing), or the supplied default when the
    /// session did not record one.
    pub fn fixed_fps_or(&self, default: i64) -> i64 {
        self.record
            .frame_pacing
            .as_ref()
            .map(|pacing| pacing.fixed_fps)
            .unwrap_or(default)
    }
}

/// Attach the pooled session's identity to a start failure: which session, which
/// artifact directory, and where its engine log would be. A warm bundle records
/// these details verbatim, so one failed start inside a K-lane pool stays traceable
/// to a single directory instead of being an anonymous "session start failed".
fn pooled_start_context(
    mut failure: FailureDetail,
    record: &SessionRecord,
    logs_path: &Path,
) -> FailureDetail {
    failure
        .details
        .insert("session_id".to_string(), json!(record.session_id));
    failure
        .details
        .insert("artifact_dir".to_string(), json!(record.artifact_dir));
    failure.details.insert(
        "logs_path".to_string(),
        json!(logs_path.to_string_lossy().into_owned()),
    );
    failure
}

/// Start one independent live session for the pool, mirroring `session_start_result`
/// but WITHOUT any singleton `.qa/session.json` interaction: no already-running
/// check, no `clear_session_files`, no singleton persist. The token stays in memory
/// (also written to the per-session artifact dir for debuggability). Returns the
/// handle on success, or the startup `FailureDetail` on failure (the launched
/// process, if any, is stopped before returning so no orphan is left behind).
pub fn start_pooled_session(
    config: &ProjectConfig,
    scene: Option<&str>,
    viewport_raw: &str,
    headless: bool,
    audio: bool,
    project_args: &[String],
    frame_pacing_request: Option<Option<i64>>,
) -> Result<PooledSessionHandle, FailureDetail> {
    let command = "session start";
    if let Some(failure) = config.project_config_failure_for_command(command) {
        return Err(failure);
    }
    let Some(binary) = config.godot_binary.as_deref() else {
        return Err(FailureDetail {
            kind: "missing_godot".to_string(),
            command: Some(command.to_string()),
            message: Some("Godot binary not found in PATH or GODOT_BIN".to_string()),
            expected: Some(json!(
                "discoverable Godot binary via --godot-bin, GODOT_BIN, or PATH"
            )),
            actual: Some(json!("missing")),
            ..FailureDetail::default()
        });
    };
    let Some(viewport) = parse_viewport(viewport_raw) else {
        return Err(FailureDetail {
            kind: "invalid_viewport".to_string(),
            command: Some(command.to_string()),
            message: Some("viewport must use WIDTHxHEIGHT".to_string()),
            expected: Some(json!("positive WIDTHxHEIGHT viewport string")),
            actual: Some(json!(viewport_raw)),
            ..FailureDetail::default()
        });
    };
    let frame_pacing = resolve_live_frame_pacing(config, frame_pacing_request, command)?;
    let scene = resolve_session_start_scene(config, scene)?;
    let effective_project_args = merged_args(&config.godot_project_args, project_args);
    // link_latest = false: a pooled session is addressed by this in-memory handle,
    // never through `.qa/artifacts/live/latest`, so rotating that shared link would
    // only make K concurrent lanes race each other over a link nothing reads.
    let prepared = prepare_live_session_with_latest_link(
        config,
        &scene,
        viewport,
        headless,
        &effective_project_args,
        frame_pacing.clone(),
        false,
    )?;
    let locate = |failure: FailureDetail| {
        pooled_start_context(failure, &prepared.record, &prepared.logs_path)
    };
    let launch = godot_qa_process::launch_godot_session(
        binary,
        &config.godot_launch_prefix,
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
        None,
    )
    .map_err(locate)?;
    let mut persisted = SessionRecord {
        process_identity: godot_qa_process::process_identity(launch.pid),
        pid: launch.pid,
        ..prepared.record.clone()
    };
    // Keep the token in the per-session artifact dir, never the singleton, so
    // concurrent pooled sessions cannot collide on `.qa/session.token`.
    let session_json_path = Path::new(&persisted.artifact_dir).join("session.json");
    let session_token_path = Path::new(&persisted.artifact_dir).join("session.token");
    persisted.token_file = session_token_path.to_string_lossy().into_owned();
    match probe_live_session_startup(config, &persisted, &prepared.token) {
        Ok(response) => {
            if let Some(error) = response.get("error").and_then(Value::as_object) {
                let _ = stop_launched_session_process(config, &persisted);
                return Err(locate(FailureDetail {
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
                            .unwrap_or("unable to start pooled live session")
                            .to_string(),
                    ),
                    ..FailureDetail::default()
                }));
            }
            // Persist the per-session record + token into the isolated artifact dir
            // (NOT the singleton) so the session is self-describing / debuggable.
            let _ = write_json(&session_json_path, &persisted.to_json_value());
            let _ = fs::write(&session_token_path, &prepared.token);
            Ok(PooledSessionHandle {
                record: persisted,
                token: prepared.token,
            })
        }
        Err(failure) => {
            let _ = stop_launched_session_process(config, &persisted);
            Err(locate(failure))
        }
    }
}

/// Run one live protocol command against a specific pooled session (addressed by
/// its handle + in-memory token), reusing the shared retry loop. Never touches the
/// singleton session files.
pub fn pooled_session_command(
    handle: &PooledSessionHandle,
    command: &str,
    protocol_command: &str,
    params: Map<String, Value>,
) -> CommandResult {
    let loaded = LoadedSessionRecord {
        record: handle.record.clone(),
        source_path: PathBuf::from(&handle.record.request_path),
    };
    run_live_protocol_command_with_loaded(
        command,
        protocol_command,
        params,
        loaded,
        Some(&handle.token),
    )
}

/// Stop one pooled session: ask the bridge to stop, then make sure the process is
/// gone (never leaving an orphan). Only touches this session's own process/record —
/// no singleton files, no debug host.
pub fn stop_pooled_session(config: &ProjectConfig, handle: &PooledSessionHandle) -> CommandResult {
    let command = "session stop";
    let _ = pooled_session_command(handle, command, SESSION_STOP, Map::new());
    let record = &handle.record;
    if godot_qa_process::wait_for_godot_process_exit(record.pid, PROCESS_STOP_WAIT_TIMEOUT_SECONDS) {
        return CommandResult {
            ok: true,
            command: command.to_string(),
            message: "pooled live session stopped".to_string(),
            data: Map::from_iter([("session_id".to_string(), json!(record.session_id))]),
            failures: Vec::new(),
            exit_code: 0,
        };
    }
    match godot_qa_process::stop_godot_process_with_identity(
        record.pid,
        PROCESS_STOP_WAIT_TIMEOUT_SECONDS,
        record.process_identity,
    ) {
        Ok(fallback) => CommandResult {
            ok: true,
            command: command.to_string(),
            message: "pooled live session stopped after process fallback".to_string(),
            data: Map::from_iter([
                ("fallback".to_string(), json!(fallback)),
                ("session_id".to_string(), json!(record.session_id)),
            ]),
            failures: Vec::new(),
            exit_code: 0,
        },
        Err(failure) => {
            let _ = config;
            CommandResult {
                ok: false,
                command: command.to_string(),
                message: "pooled live session did not stop cleanly".to_string(),
                data: Map::new(),
                failures: vec![failure],
                exit_code: 1,
            }
        }
    }
}

/// Inputs for one ephemeral Android scenario-mode session. The deploy log/logcat paths
/// are caller-chosen so the session lives entirely inside its own artifact dir; nothing
/// touches the singleton `.qa/session.json`.
pub struct AndroidScenarioSessionRequest<'a> {
    pub preset: &'a str,
    pub device: Option<&'a str>,
    pub package: Option<&'a str>,
    pub project_args: &'a [String],
    pub no_apk_cache: bool,
    pub deployment_log: PathBuf,
    pub logcat_log: PathBuf,
}

/// Deploy one ephemeral Android scenario session with resolved backend inputs. Prepares
/// a sceneless session (id/token/port/artifact dirs) WITHOUT persisting the singleton,
/// runs the shared deploy stages in scenario mode (`bootstrap_mode = Some("scenario")`,
/// empty scene), and on deploy error runs the exact-scoped cleanup honoring
/// `forwarded`/`launched`. On success the handle carries the record (`target: Android`,
/// `android: Some(..)`) and the in-memory token so [`pooled_session_command`] authenticates
/// and the Android live transport engages, exactly like a warm desktop pooled session.
fn deploy_android_scenario_session(
    config: &ProjectConfig,
    request: &AndroidScenarioSessionRequest<'_>,
    godot: &str,
    adb: &str,
    device: &godot_qa_process::android::AndroidDevice,
    preset: &godot_qa_process::android::AndroidExportPreset,
    probe: &dyn Fn(&SessionRecord, &str) -> Result<Value, FailureDetail>,
) -> Result<PooledSessionHandle, FailureDetail> {
    let effective_project_args = merged_args(&config.godot_project_args, request.project_args);
    // Sceneless boot: QaScenarioExecutor owns each scenario's scene lifecycle.
    // link_latest = false for the same reason as the desktop pooled start above: this
    // session is addressed by its in-memory handle, never through the shared
    // `.qa/artifacts/live/latest` link, so rotating that link would only let a pooled
    // Android lane clobber (and race) a link that belongs to the singleton workflow.
    let prepared = prepare_live_session_with_latest_link(
        config,
        "",
        vec![0, 0],
        false,
        &effective_project_args,
        None,
        false,
    )?;

    // The APK is deployed beside the caller-chosen deployment log; ensure both parents exist.
    if let Some(parent) = request.deployment_log.parent() {
        fs::create_dir_all(parent).map_err(|error| artifact_write_failure(parent, &error))?;
    }
    if let Some(parent) = request.logcat_log.parent() {
        fs::create_dir_all(parent).map_err(|error| artifact_write_failure(parent, &error))?;
    }
    let apk_path = request
        .deployment_log
        .parent()
        .map(|parent| parent.join("session-debug.apk"))
        .unwrap_or_else(|| PathBuf::from("session-debug.apk"));

    let deploy_request = AndroidDeployRequest {
        config,
        godot,
        adb,
        device,
        preset,
        prepared: &prepared,
        bootstrap_scene: "",
        bootstrap_mode: Some("scenario"),
        no_apk_cache: request.no_apk_cache,
        effective_project_args: &effective_project_args,
        deployment_log: &request.deployment_log,
        logcat_log: &request.logcat_log,
        apk_path: &apk_path,
    };
    let outcome = match android_deploy_stages(&deploy_request, probe) {
        Ok(outcome) => outcome,
        Err(error) => {
            if error.needs_cleanup {
                let _ = android_start_cleanup(&error.record, error.forwarded, error.launched);
            }
            return Err(error.failure);
        }
    };

    // Carry the token in memory (never the singleton) exactly like start_pooled_session,
    // and self-describe the session inside its isolated artifact dir for debuggability.
    let mut record = outcome.record;
    let session_json_path = Path::new(&record.artifact_dir).join("session.json");
    let session_token_path = Path::new(&record.artifact_dir).join("session.token");
    record.token_file = session_token_path.to_string_lossy().into_owned();
    let _ = write_json(&session_json_path, &record.to_json_value());
    let _ = fs::write(&session_token_path, &prepared.token);
    Ok(PooledSessionHandle {
        record,
        token: prepared.token,
    })
}

/// Start one ephemeral Android scenario-mode session: resolve adb/device/preset, then
/// deploy through [`deploy_android_scenario_session`] with the production bridge probe.
pub fn start_android_scenario_session(
    config: &ProjectConfig,
    request: &AndroidScenarioSessionRequest<'_>,
) -> Result<PooledSessionHandle, FailureDetail> {
    let command = "session start";
    if let Some(failure) = config.project_config_failure_for_command(command) {
        return Err(failure);
    }
    let Some(godot) = config.godot_binary.as_deref() else {
        return Err(FailureDetail {
            kind: "missing_godot".to_string(),
            command: Some(command.to_string()),
            message: Some("Godot binary not found in PATH or GODOT_BIN".to_string()),
            expected: Some(json!(
                "discoverable Godot binary via --godot-bin, GODOT_BIN, or PATH"
            )),
            actual: Some(json!("missing")),
            ..FailureDetail::default()
        });
    };
    let adb = godot_qa_process::android::resolve_adb_binary(None)?;
    let device = godot_qa_process::android::select_device(&adb, request.device)?;
    let preset = godot_qa_process::android::load_export_preset(
        &config.project_root,
        request.preset,
        request.package,
    )?;
    deploy_android_scenario_session(
        config,
        request,
        godot,
        &adb,
        &device,
        &preset,
        &|record, token| probe_live_session_startup(config, record, token),
    )
}

/// Stop one ephemeral Android scenario session: a best-effort graceful bridge stop over
/// the pooled command path (authenticated by the in-memory token; the Android transport
/// engages via `record.android`), then the exact scoped ADB teardown — force-stop, logcat
/// stop, exact forward removal, private session dir removal, and bootstrap removal —
/// collecting any cleanup failures.
pub fn stop_android_scenario_session(
    config: &ProjectConfig,
    handle: &PooledSessionHandle,
) -> Vec<FailureDetail> {
    let _ = config;
    let _ = pooled_session_command(handle, "session stop", SESSION_STOP, Map::new());
    android_start_cleanup(&handle.record, true, true)
}
