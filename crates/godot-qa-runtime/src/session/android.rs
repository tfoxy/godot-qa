use super::*;

pub(super) fn android_option_unsupported(option: &str, actual: Value) -> CommandResult {
    CommandResult {
        ok: false,
        command: "session start".to_string(),
        message: "unable to start Android live session".to_string(),
        data: Map::new(),
        failures: vec![FailureDetail {
            kind: "android_option_unsupported".to_string(),
            command: Some("session start".to_string()),
            message: Some(format!("Android live sessions do not support `{option}`")),
            expected: Some(json!("option omitted for --target android")),
            actual: Some(actual),
            ..FailureDetail::default()
        }],
        exit_code: 1,
    }
}

pub fn session_start_target_result(
    config: &ProjectConfig,
    request: SessionStartRequest<'_>,
) -> CommandResult {
    if request.target == SessionTarget::Desktop {
        return session_start_result(
            config,
            request.scene,
            request.viewport.unwrap_or("1920x1080"),
            request.headless,
            request.audio,
            request.project_args,
            request.frame_pacing_request,
            request.hot_reload,
        );
    }

    if request.headless {
        return android_option_unsupported("--headless", json!(true));
    }
    if let Some(viewport) = request.viewport {
        return android_option_unsupported("--viewport", json!(viewport));
    }
    if request.hot_reload {
        return android_option_unsupported("--hot-reload", json!(true));
    }
    if request.fastest {
        return android_option_unsupported("--fastest", json!(true));
    }
    if request.frame_pacing_request.is_some() {
        return android_option_unsupported("--fixed-fps", json!(request.frame_pacing_request));
    }
    if request.no_audio {
        return android_option_unsupported("--no-audio", json!(true));
    }
    let Some(preset) = request.preset.filter(|value| !value.trim().is_empty()) else {
        return CommandResult {
            ok: false,
            command: "session start".to_string(),
            message: "unable to start Android live session".to_string(),
            data: Map::new(),
            failures: vec![FailureDetail {
                kind: "android_preset_required".to_string(),
                command: Some("session start".to_string()),
                message: Some("Android live sessions require `--preset`".to_string()),
                expected: Some(json!("non-empty Android export preset name")),
                actual: Some(json!(request.preset)),
                ..FailureDetail::default()
            }],
            exit_code: 1,
        };
    };

    session_start_android_result(config, &request, preset)
}

pub(super) fn android_start_failure(
    failure: FailureDetail,
    data: Map<String, Value>,
) -> CommandResult {
    CommandResult {
        ok: false,
        command: "session start".to_string(),
        message: "unable to start Android live session".to_string(),
        data,
        failures: vec![failure],
        exit_code: 1,
    }
}

pub(super) fn android_start_cleanup(
    record: &SessionRecord,
    forwarded: bool,
    launched: bool,
) -> Vec<FailureDetail> {
    let Some(android) = &record.android else {
        return Vec::new();
    };
    let mut failures = Vec::new();
    if let Some(logcat_pid) = android.logcat_pid
        && let Err(failure) = godot_qa_process::android::stop_logcat_with_identity(
            logcat_pid,
            android.logcat_process_identity,
        )
    {
        failures.push(failure);
    }
    if launched
        && let Err(failure) = godot_qa_process::android::force_stop(
            &android.adb_path,
            &android.device_serial,
            &android.package,
        )
    {
        failures.push(failure);
    }
    if forwarded
        && let Err(failure) = godot_qa_process::android::remove_exact_forward(
            &android.adb_path,
            &android.device_serial,
            android.forwarded_host_port,
            android.device_port,
        )
    {
        failures.push(failure);
    }
    if let Err(failure) = godot_qa_process::android::remove_private_session(
        &android.adb_path,
        &android.device_serial,
        &android.package,
        &record.session_id,
    ) {
        failures.push(failure);
    }
    if let Err(failure) = godot_qa_process::android::remove_private_bootstrap(
        &android.adb_path,
        &android.device_serial,
        &android.package,
    ) {
        failures.push(failure);
    }
    failures
}

pub(super) fn android_start_failure_after_cleanup(
    config: &ProjectConfig,
    failure: FailureDetail,
    mut data: Map<String, Value>,
    record: &SessionRecord,
    token: &str,
    forwarded: bool,
    launched: bool,
) -> CommandResult {
    let mut result = android_start_failure(failure, data.clone());
    let cleanup_failures = android_start_cleanup(record, forwarded, launched);
    if cleanup_failures.is_empty() {
        clear_session_files(config);
        return result;
    }

    result.failures.extend(cleanup_failures);
    match persist_session(config, record, token) {
        Ok(()) => {
            data.insert("metadata_retained".to_string(), json!(true));
            data.insert("session_id".to_string(), json!(record.session_id));
            result.data = data;
        }
        Err(persist_failure) => {
            data.insert("metadata_retained".to_string(), json!(false));
            result.data = data;
            result.failures.push(persist_failure);
        }
    }
    result
}

pub(super) fn append_android_deployment_output(
    path: &Path,
    stage: &str,
    output: &godot_qa_process::android::AndroidCommandOutput,
) {
    let command = godot_qa_process::format_command(&output.command);
    let _ = append_android_deployment_note(
        path,
        &format!(
            "{stage}: status={}\ncommand: {command}\nstdout:\n{}\nstderr:\n{}",
            output.status, output.stdout, output.stderr
        ),
    );
}

pub(super) fn append_android_deployment_note(path: &Path, note: &str) -> std::io::Result<()> {
    let mut file = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)?;
    writeln!(file, "\n[godot-qa android]\n{note}")
}

pub(super) fn append_android_deployment_failure(
    path: &Path,
    stage: &str,
    failure: &FailureDetail,
    session_token: &str,
) {
    let mut note = format!(
        "{stage}: failed\nkind: {}\ncommand: {}\nmessage: {}\nexpected: {}\nactual: {}",
        failure.kind,
        failure.command.as_deref().unwrap_or(""),
        failure.message.as_deref().unwrap_or(""),
        failure
            .expected
            .as_ref()
            .map(Value::to_string)
            .unwrap_or_default(),
        failure
            .actual
            .as_ref()
            .map(Value::to_string)
            .unwrap_or_default(),
    );
    if !session_token.is_empty() {
        note = note.replace(session_token, "[REDACTED]");
    }
    let _ = append_android_deployment_note(path, &note);
}

/// Debug-APK cache outcome for a session start, surfaced in the result JSON and deployment log.
#[derive(Debug)]
pub(super) struct ApkCacheOutcome {
    pub(super) status: &'static str,
    pub(super) key: Option<String>,
    pub(super) reason: Option<String>,
}

impl ApkCacheOutcome {
    pub(super) fn to_json(&self) -> Value {
        let mut data = Map::from_iter([
            ("status".to_string(), json!(self.status)),
            ("key".to_string(), json!(self.key)),
        ]);
        if let Some(reason) = &self.reason {
            data.insert("reason".to_string(), json!(reason));
        }
        Value::Object(data)
    }
}

/// Directories excluded from the cache manifest beyond the built-in hidden/`.gdignore` rules:
/// the artifacts dir is derived data, so exclude it when it resolves inside the project root (a
/// dot-prefixed default like `.qa/artifacts` is already skipped by the hidden-name rule).
pub(super) fn android_manifest_excluded_dirs(config: &ProjectConfig) -> Vec<PathBuf> {
    if config.artifacts_dir.starts_with(&config.project_root) {
        vec![config.artifacts_dir.clone()]
    } else {
        Vec::new()
    }
}

pub(super) fn export_debug_apk_into(
    config: &ProjectConfig,
    godot: &str,
    preset: &godot_qa_process::android::AndroidExportPreset,
    apk_path: &Path,
    logs_path: &Path,
) -> Result<(), FailureDetail> {
    godot_qa_process::android::export_debug_apk(
        godot,
        &config.project_root,
        &preset.name,
        apk_path,
        logs_path,
    )
    .map(|_| ())
}

/// Materialize the session debug APK at `apk_path`, reusing a cached export when the project
/// and toolchain are unchanged. A cache hit copies the cached APK into the deployment path so
/// downstream install/`retained_apk`/cleanup logic is untouched. Every cache failure degrades
/// to a fresh export; only an export failure is fatal and is propagated to the caller unchanged.
pub(super) fn android_obtain_session_apk(
    config: &ProjectConfig,
    godot: &str,
    preset: &godot_qa_process::android::AndroidExportPreset,
    no_apk_cache: bool,
    apk_path: &Path,
    deployment_log: &Path,
) -> Result<ApkCacheOutcome, FailureDetail> {
    use godot_qa_process::apk_cache;

    // A stable key needs the Godot version; a probe failure bypasses the cache entirely.
    let version = match godot_qa_process::probe_godot_version(godot, &config.godot_launch_prefix) {
        Ok(version) => version,
        Err(_) => {
            export_debug_apk_into(config, godot, preset, apk_path, deployment_log)?;
            let _ = append_android_deployment_note(
                deployment_log,
                "apk-cache: bypassed (godot_version_probe_failed)",
            );
            return Ok(ApkCacheOutcome {
                status: "bypassed",
                key: None,
                reason: Some("godot_version_probe_failed".to_string()),
            });
        }
    };

    let excluded_dirs = android_manifest_excluded_dirs(config);
    let key_inputs = apk_cache::ApkCacheKeyInputs {
        godot_binary: godot,
        godot_version: &version,
        preset_name: &preset.name,
        package: &preset.package,
        project_root: &config.project_root,
        extra_excluded_dirs: &excluded_dirs,
    };
    let key = match apk_cache::compute_cache_key(&key_inputs) {
        Ok(key) => key,
        Err(_) => {
            export_debug_apk_into(config, godot, preset, apk_path, deployment_log)?;
            let _ = append_android_deployment_note(
                deployment_log,
                "apk-cache: bypassed (project_manifest_failed)",
            );
            return Ok(ApkCacheOutcome {
                status: "bypassed",
                key: None,
                reason: Some("project_manifest_failed".to_string()),
            });
        }
    };

    let cache_root = apk_cache::cache_root(&config.project_root);

    // Read side: skipped docker-style under --no-apk-cache; the fresh export still refreshes it.
    let mut deferred_notes: Vec<String> = Vec::new();
    if !no_apk_cache && let Some(entry) = apk_cache::lookup(&cache_root, &key) {
        match fs::copy(&entry.apk_path, apk_path) {
            Ok(_) => {
                apk_cache::touch(&entry);
                let _ = append_android_deployment_note(
                    deployment_log,
                    &format!(
                        "apk-cache: hit {key} ({} manifest files)",
                        entry.meta.manifest_files
                    ),
                );
                return Ok(ApkCacheOutcome {
                    status: "hit",
                    key: Some(key),
                    reason: None,
                });
            }
            Err(error) => {
                // A read hit we could not materialize: fall through to a fresh export.
                deferred_notes.push(format!("apk-cache: read copy failed (non-fatal): {error}"));
            }
        }
    }

    // Write side: export exactly as before (failure propagates), then log deferred notes and
    // refresh the cache entry. Notes are appended after export because export rewrites the log.
    export_debug_apk_into(config, godot, preset, apk_path, deployment_log)?;
    for note in &deferred_notes {
        let _ = append_android_deployment_note(deployment_log, note);
    }
    if no_apk_cache {
        let _ =
            append_android_deployment_note(deployment_log, "apk-cache: bypassed (--no-apk-cache)");
    }

    let manifest_files = apk_cache::manifest_file_count(&key_inputs).unwrap_or(0);
    let apk_bytes = fs::metadata(apk_path).map(|meta| meta.len()).unwrap_or(0);
    let now_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|delta| delta.as_millis() as u64)
        .unwrap_or(0);
    let meta = apk_cache::ApkCacheMeta {
        version: 1,
        key: key.clone(),
        created_at_ms: now_ms,
        last_used_at_ms: now_ms,
        godot_binary: godot.to_string(),
        godot_version: version,
        preset: preset.name.clone(),
        package: preset.package.clone(),
        manifest_files,
        apk_bytes,
        tool_version: env!("CARGO_PKG_VERSION").to_string(),
    };
    match apk_cache::store(&cache_root, &key, apk_path, meta) {
        Ok(_) => {
            apk_cache::prune(&cache_root, apk_cache::APK_CACHE_KEEP);
            let _ =
                append_android_deployment_note(deployment_log, &format!("apk-cache: stored {key}"));
        }
        Err(error) => {
            let _ = append_android_deployment_note(
                deployment_log,
                &format!("apk-cache: write failed (non-fatal): {error}"),
            );
        }
    }

    let (status, reason) = if no_apk_cache {
        ("bypassed", Some("--no-apk-cache".to_string()))
    } else {
        ("miss", None)
    };
    Ok(ApkCacheOutcome {
        status,
        key: Some(key),
        reason,
    })
}

/// Borrowed inputs for one Android debug deployment. The deploy paths are
/// caller-chosen so an ephemeral scenario session can point them at its own
/// artifact dir instead of the singleton session's.
pub(super) struct AndroidDeployRequest<'a> {
    pub(super) config: &'a ProjectConfig,
    pub(super) godot: &'a str,
    pub(super) adb: &'a str,
    pub(super) device: &'a godot_qa_process::android::AndroidDevice,
    pub(super) preset: &'a godot_qa_process::android::AndroidExportPreset,
    pub(super) prepared: &'a PreparedSession,
    pub(super) bootstrap_scene: &'a str,
    pub(super) bootstrap_mode: Option<&'a str>,
    pub(super) no_apk_cache: bool,
    pub(super) effective_project_args: &'a [String],
    pub(super) deployment_log: &'a Path,
    pub(super) logcat_log: &'a Path,
    pub(super) apk_path: &'a Path,
}

/// Successful deployment: the fully populated session record (pid + logcat filled,
/// viewport reconciled from the bridge), the bridge probe response, and the debug-APK
/// cache outcome for the result payload.
#[derive(Debug)]
pub(super) struct AndroidDeployOutcome {
    pub(super) record: SessionRecord,
    pub(super) probe_data: Value,
    pub(super) apk_cache: ApkCacheOutcome,
}

/// Failed deployment. `record` is filled up to the failure point so the caller's
/// cleanup honors any provisioned state (`logcat_pid`, forward, launch). `needs_cleanup`
/// distinguishes a pre-bootstrap failure (nothing deployed yet — the caller returns a
/// plain failure, optionally retaining the exported APK for debugging) from a
/// post-bootstrap failure (the caller must run the scoped cleanup).
#[derive(Debug)]
pub(super) struct AndroidDeployError {
    pub(super) failure: FailureDetail,
    pub(super) data_extras: Map<String, Value>,
    pub(super) forwarded: bool,
    pub(super) launched: bool,
    pub(super) needs_cleanup: bool,
    pub(super) record: SessionRecord,
}

pub(super) fn deploy_apk_error(
    failure: FailureDetail,
    apk_cache: &ApkCacheOutcome,
    retained_apk: Option<&Path>,
    forwarded: bool,
    launched: bool,
    needs_cleanup: bool,
    record: SessionRecord,
) -> AndroidDeployError {
    let mut data_extras = Map::from_iter([("apk_cache".to_string(), apk_cache.to_json())]);
    if let Some(apk_path) = retained_apk {
        data_extras.insert("retained_apk".to_string(), json!(apk_path));
    }
    AndroidDeployError {
        failure,
        data_extras,
        forwarded,
        launched,
        needs_cleanup,
        record,
    }
}

/// Run the ordered Android debug deployment: export/cache the APK, wireless reconnect,
/// revalidate the device, install, provision the private bootstrap, forward the loopback
/// port, launch, poll for the device PID, attach the logcat sidecar, and probe the bridge
/// through the injected `probe`. Every stage appends to `deployment_log`. This owns no
/// singleton `.qa/session.json` state and never runs cleanup itself; failures carry enough
/// state (`forwarded`/`launched`/`needs_cleanup` + the partially populated record) for the
/// caller to clean up in a target-appropriate way.
pub(super) fn android_deploy_stages(
    request: &AndroidDeployRequest<'_>,
    probe: &dyn Fn(&SessionRecord, &str) -> Result<Value, FailureDetail>,
) -> Result<AndroidDeployOutcome, AndroidDeployError> {
    let command = "session start";
    let config = request.config;
    let godot = request.godot;
    let adb = request.adb;
    let device = request.device;
    let preset = request.preset;
    let prepared = request.prepared;
    let bootstrap_scene = request.bootstrap_scene;
    let bootstrap_mode = request.bootstrap_mode;
    let no_apk_cache = request.no_apk_cache;
    let effective_project_args = request.effective_project_args;
    let deployment_log = request.deployment_log;
    let logcat_log = request.logcat_log;
    let apk_path = request.apk_path;

    let device_artifact_root = format!(
        "{}/{}",
        godot_qa_process::android::ANDROID_ARTIFACT_USER_ROOT,
        prepared.record.session_id
    );
    let mut persisted = SessionRecord {
        process_identity: None,
        pid: 0,
        target: SessionTarget::Android,
        android: Some(AndroidSessionMetadata {
            device_serial: device.serial.clone(),
            package: preset.package.clone(),
            preset: preset.name.clone(),
            device_pid: 0,
            device_port: prepared.record.port,
            forwarded_host_port: prepared.record.port,
            adb_path: adb.to_string(),
            device_artifact_root,
            logcat_pid: None,
            logcat_process_identity: None,
            deployment_log_path: deployment_log.to_string_lossy().into_owned(),
            logcat_log_path: logcat_log.to_string_lossy().into_owned(),
        }),
        ..prepared.record.clone()
    };

    if let Some(parent) = apk_path.parent()
        && let Err(error) = fs::create_dir_all(parent)
    {
        return Err(AndroidDeployError {
            failure: artifact_write_failure(parent, &error),
            data_extras: Map::new(),
            forwarded: false,
            launched: false,
            needs_cleanup: false,
            record: persisted,
        });
    }
    let apk_cache = match android_obtain_session_apk(
        config,
        godot,
        preset,
        no_apk_cache,
        apk_path,
        deployment_log,
    ) {
        Ok(outcome) => outcome,
        Err(failure) => {
            let _ = fs::remove_file(apk_path);
            return Err(AndroidDeployError {
                failure,
                data_extras: Map::new(),
                forwarded: false,
                launched: false,
                needs_cleanup: false,
                record: persisted,
            });
        }
    };
    if godot_qa_process::android::is_wireless_adb_serial(&device.serial) {
        match godot_qa_process::android::reconnect_wireless_device(adb, &device.serial) {
            Ok(output) => {
                append_android_deployment_output(deployment_log, "wireless reconnect", &output);
            }
            Err(failure) => {
                append_android_deployment_failure(
                    deployment_log,
                    "wireless reconnect",
                    &failure,
                    &prepared.token,
                );
                return Err(deploy_apk_error(
                    failure,
                    &apk_cache,
                    Some(apk_path),
                    false,
                    false,
                    false,
                    persisted,
                ));
            }
        }
    }
    if let Err(failure) =
        godot_qa_process::android::select_device(adb, Some(device.serial.as_str()))
    {
        append_android_deployment_failure(
            deployment_log,
            "device revalidation",
            &failure,
            &prepared.token,
        );
        return Err(deploy_apk_error(
            failure,
            &apk_cache,
            Some(apk_path),
            false,
            false,
            false,
            persisted,
        ));
    }
    let install = match godot_qa_process::android::install_apk(adb, &device.serial, apk_path) {
        Ok(output) => output,
        Err(failure) => {
            append_android_deployment_failure(deployment_log, "install", &failure, &prepared.token);
            return Err(deploy_apk_error(
                failure,
                &apk_cache,
                Some(apk_path),
                false,
                false,
                false,
                persisted,
            ));
        }
    };
    append_android_deployment_output(deployment_log, "install", &install);
    let _ = fs::remove_file(apk_path);

    let bootstrap = match android_bootstrap_bytes(
        prepared,
        effective_project_args,
        bootstrap_scene,
        bootstrap_mode,
    ) {
        Ok(bootstrap) => bootstrap,
        Err(failure) => {
            return Err(deploy_apk_error(
                failure, &apk_cache, None, false, false, false, persisted,
            ));
        }
    };
    if let Err(failure) = godot_qa_process::android::write_private_bootstrap(
        adb,
        &device.serial,
        &preset.package,
        &bootstrap,
    ) {
        return Err(deploy_apk_error(
            failure, &apk_cache, None, false, false, true, persisted,
        ));
    }
    let _ = append_android_deployment_note(
        deployment_log,
        "bootstrap: provisioned through private run-as stdin",
    );
    let device_port = persisted
        .android
        .as_ref()
        .map(|android| android.device_port)
        .unwrap_or(persisted.port);
    let forward = match godot_qa_process::android::create_forward(
        adb,
        &device.serial,
        persisted.port,
        device_port,
    ) {
        Ok(output) => output,
        Err(failure) => {
            append_android_deployment_failure(deployment_log, "forward", &failure, &prepared.token);
            return Err(deploy_apk_error(
                failure, &apk_cache, None, true, false, true, persisted,
            ));
        }
    };
    append_android_deployment_output(deployment_log, "forward", &forward);
    let launch = match godot_qa_process::android::launch_app(adb, &device.serial, &preset.package) {
        Ok(result) => result,
        Err(failure) => {
            append_android_deployment_failure(deployment_log, "launch", &failure, &prepared.token);
            return Err(deploy_apk_error(
                failure, &apk_cache, None, true, true, true, persisted,
            ));
        }
    };
    append_android_deployment_output(deployment_log, "pre-launch force-stop", &launch.force_stop);
    append_android_deployment_output(deployment_log, "launch", &launch.launch);

    let deadline =
        std::time::Instant::now() + Duration::from_secs_f64(LIVE_SESSION_STARTUP_TIMEOUT_SECONDS);
    let device_pid = loop {
        match godot_qa_process::android::query_pid(adb, &device.serial, &preset.package) {
            Ok(Some(pid)) => break Ok(pid),
            Ok(None) if std::time::Instant::now() < deadline => {
                thread::sleep(Duration::from_millis(50));
            }
            Ok(None) => {
                break Err(FailureDetail {
                    kind: "android_pid_query_failed".to_string(),
                    command: Some(command.to_string()),
                    message: Some("Android application did not start before timeout".to_string()),
                    expected: Some(json!("running application process")),
                    actual: Some(json!("pidof returned no process")),
                    ..FailureDetail::default()
                });
            }
            Err(failure) => break Err(failure),
        }
    };
    let device_pid = match device_pid {
        Ok(pid) => pid,
        Err(failure) => {
            append_android_deployment_failure(
                deployment_log,
                "pid query",
                &failure,
                &prepared.token,
            );
            return Err(deploy_apk_error(
                failure, &apk_cache, None, true, true, true, persisted,
            ));
        }
    };
    if let Some(android) = persisted.android.as_mut() {
        android.device_pid = device_pid;
    }
    let _ = append_android_deployment_note(
        deployment_log,
        &format!("pid: device process {device_pid}"),
    );
    match godot_qa_process::android::start_logcat(adb, &device.serial, device_pid, logcat_log) {
        Ok(sidecar) => {
            let _ = append_android_deployment_note(
                deployment_log,
                &format!("logcat: host sidecar pid {}", sidecar.pid),
            );
            if let Some(android) = persisted.android.as_mut() {
                android.logcat_pid = Some(sidecar.pid);
                android.logcat_process_identity = sidecar.process_identity;
                android.logcat_log_path = sidecar.logs_path;
            }
        }
        Err(failure) => {
            append_android_deployment_failure(deployment_log, "logcat", &failure, &prepared.token);
            return Err(deploy_apk_error(
                failure, &apk_cache, None, true, true, true, persisted,
            ));
        }
    }

    let response = match probe(&persisted, &prepared.token) {
        Ok(response) => response,
        Err(failure) => {
            append_android_deployment_failure(
                deployment_log,
                "bridge probe",
                &failure,
                &prepared.token,
            );
            return Err(deploy_apk_error(
                failure, &apk_cache, None, true, true, true, persisted,
            ));
        }
    };
    if let Some(error) = response.get("error").and_then(Value::as_object) {
        let failure = FailureDetail {
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
                    .unwrap_or("Android runtime rejected session startup")
                    .to_string(),
            ),
            ..FailureDetail::default()
        };
        append_android_deployment_failure(
            deployment_log,
            "bridge response",
            &failure,
            &prepared.token,
        );
        return Err(deploy_apk_error(
            failure, &apk_cache, None, true, true, true, persisted,
        ));
    }
    if let Some(viewport) = response
        .get("data")
        .and_then(Value::as_object)
        .and_then(|data| data.get("viewport"))
        .and_then(Value::as_array)
        .map(|items| items.iter().filter_map(Value::as_i64).collect::<Vec<_>>())
        .filter(|viewport| viewport.len() == 2)
    {
        persisted.viewport = viewport;
    }
    Ok(AndroidDeployOutcome {
        record: persisted,
        probe_data: response,
        apk_cache,
    })
}

pub(super) fn session_start_android_result(
    config: &ProjectConfig,
    request: &SessionStartRequest<'_>,
    preset_name: &str,
) -> CommandResult {
    let command = "session start";
    if let Some(failure) = config.project_config_failure_for_command(command) {
        return android_start_failure(failure, Map::new());
    }
    let Some(godot) = config.godot_binary.as_deref() else {
        return android_start_failure(
            FailureDetail {
                kind: "missing_godot".to_string(),
                command: Some(command.to_string()),
                message: Some("Godot binary not found in PATH or GODOT_BIN".to_string()),
                expected: Some(json!(
                    "discoverable Godot binary via --godot-bin, GODOT_BIN, or PATH"
                )),
                actual: Some(json!("missing")),
                ..FailureDetail::default()
            },
            Map::new(),
        );
    };
    let scene = match resolve_session_start_scene(config, request.scene) {
        Ok(scene) => scene,
        Err(failure) => return android_start_failure(failure, Map::new()),
    };
    if !is_valid_android_live_scene(&scene) {
        return android_start_failure(
            FailureDetail {
                kind: "android_scene_invalid".to_string(),
                command: Some(command.to_string()),
                message: Some("Android live scene must be a packaged Godot scene".to_string()),
                expected: Some(json!(
                    "non-empty res:// path ending in .tscn or .scn without traversal"
                )),
                actual: Some(json!(scene)),
                ..FailureDetail::default()
            },
            Map::new(),
        );
    }
    let adb = match godot_qa_process::android::resolve_adb_binary(None) {
        Ok(adb) => adb,
        Err(failure) => return android_start_failure(failure, Map::new()),
    };
    let device = match godot_qa_process::android::select_device(&adb, request.device) {
        Ok(device) => device,
        Err(failure) => return android_start_failure(failure, Map::new()),
    };
    let preset = match godot_qa_process::android::load_export_preset(
        &config.project_root,
        preset_name,
        request.package,
    ) {
        Ok(preset) => preset,
        Err(failure) => return android_start_failure(failure, Map::new()),
    };

    match load_session_record(config, command) {
        Ok(existing) if existing.target == SessionTarget::Android => {
            let Some(existing_android) = &existing.android else {
                return android_start_failure(
                    session_failure(
                        "session_file_invalid",
                        command,
                        &session_json_path(config),
                        "Android session metadata is missing",
                        json!("session.android metadata"),
                        json!("missing"),
                    ),
                    Map::new(),
                );
            };
            match godot_qa_process::android::query_pid(
                &existing_android.adb_path,
                &existing_android.device_serial,
                &existing_android.package,
            ) {
                Ok(Some(pid)) if pid == existing_android.device_pid => {
                    return android_start_failure(
                        FailureDetail {
                            kind: "session_already_running".to_string(),
                            command: Some(command.to_string()),
                            message: Some("a live Android session is already running".to_string()),
                            expected: Some(json!("no active .qa/session.json")),
                            actual: Some(existing.to_json_value()),
                            ..FailureDetail::default()
                        },
                        Map::new(),
                    );
                }
                Ok(Some(_)) | Ok(None) => {
                    let cleanup_failures = android_start_cleanup(&existing, true, false);
                    if !cleanup_failures.is_empty() {
                        return CommandResult {
                            ok: false,
                            command: command.to_string(),
                            message: "stale Android session cleanup was incomplete; recoverable metadata was retained".to_string(),
                            data: Map::from_iter([
                                ("target".to_string(), json!("android")),
                                ("session_id".to_string(), json!(existing.session_id)),
                                ("metadata_retained".to_string(), json!(true)),
                            ]),
                            failures: cleanup_failures,
                            exit_code: 1,
                        };
                    }
                    clear_session_files(config);
                }
                Err(mut failure) => {
                    failure.kind = "android_device_unavailable".to_string();
                    failure.message = Some(
                        "the existing Android session target is unavailable; metadata was retained"
                            .to_string(),
                    );
                    return android_start_failure(failure, Map::new());
                }
            }
        }
        Ok(existing) if session_liveness(&existing) == SessionLiveness::Running => {
            return android_start_failure(
                FailureDetail {
                    kind: "session_already_running".to_string(),
                    command: Some(command.to_string()),
                    message: Some("a desktop live session is already running".to_string()),
                    expected: Some(json!("no active .qa/session.json")),
                    actual: Some(existing.to_json_value()),
                    ..FailureDetail::default()
                },
                Map::new(),
            );
        }
        Ok(_) => clear_session_files(config),
        Err(failure) if failure.kind == "session_not_found" => {}
        Err(failure) => return android_start_failure(failure, Map::new()),
    }

    let effective_project_args = merged_args(&config.godot_project_args, request.project_args);
    let prepared = match prepare_live_session(
        config,
        &scene,
        vec![0, 0],
        false,
        &effective_project_args,
        None,
    ) {
        Ok(prepared) => prepared,
        Err(failure) => return android_start_failure(failure, Map::new()),
    };
    let artifact_root = PathBuf::from(&prepared.record.artifact_dir);
    let apk_path = artifact_root.join("deployment").join("session-debug.apk");
    let deployment_log = artifact_root.join("deployment").join("deployment.log");
    let logcat_log = artifact_root.join("logs").join("logcat.txt");
    // `None` scene means the packaged main scene; an explicit `--scene` overrides it.
    let bootstrap_scene = request.scene.map(|_| scene.as_str()).unwrap_or("");
    let base_data = || {
        Map::from_iter([
            ("target".to_string(), json!("android")),
            ("device".to_string(), json!(device.serial)),
            ("package".to_string(), json!(preset.package)),
            ("preset".to_string(), json!(preset.name)),
            ("artifact_dir".to_string(), json!(artifact_root)),
            ("deployment_log".to_string(), json!(deployment_log)),
            ("logcat".to_string(), json!(logcat_log)),
        ])
    };

    let deploy_request = AndroidDeployRequest {
        config,
        godot,
        adb: &adb,
        device: &device,
        preset: &preset,
        prepared: &prepared,
        bootstrap_scene,
        // Interactive live sessions boot in `live` mode (the payload omits `mode`).
        bootstrap_mode: None,
        no_apk_cache: request.no_apk_cache,
        effective_project_args: &effective_project_args,
        deployment_log: &deployment_log,
        logcat_log: &logcat_log,
        apk_path: &apk_path,
    };
    let outcome = match android_deploy_stages(&deploy_request, &|record, token| {
        probe_live_session_startup(config, record, token)
    }) {
        Ok(outcome) => outcome,
        Err(error) => {
            let mut data = base_data();
            for (key, value) in error.data_extras {
                data.insert(key, value);
            }
            if error.needs_cleanup {
                return android_start_failure_after_cleanup(
                    config,
                    error.failure,
                    data,
                    &error.record,
                    &prepared.token,
                    error.forwarded,
                    error.launched,
                );
            }
            return android_start_failure(error.failure, data);
        }
    };
    let persisted = outcome.record;
    let apk_cache_outcome = outcome.apk_cache;
    let response = outcome.probe_data;

    if let Err(failure) = persist_session(config, &persisted, &prepared.token) {
        append_android_deployment_failure(
            &deployment_log,
            "metadata persistence",
            &failure,
            &prepared.token,
        );
        let mut data = base_data();
        data.insert("apk_cache".to_string(), apk_cache_outcome.to_json());
        return android_start_failure_after_cleanup(
            config,
            failure,
            data,
            &persisted,
            &prepared.token,
            true,
            true,
        );
    }

    let android = persisted.android.as_ref().expect("Android metadata is set");
    CommandResult {
        ok: true,
        command: command.to_string(),
        message: "Android live session started".to_string(),
        data: Map::from_iter([
            ("target".to_string(), json!("android")),
            ("session_id".to_string(), json!(persisted.session_id)),
            ("device".to_string(), json!(android.device_serial)),
            ("package".to_string(), json!(android.package)),
            ("preset".to_string(), json!(android.preset)),
            ("device_pid".to_string(), json!(android.device_pid)),
            ("device_port".to_string(), json!(android.device_port)),
            (
                "forwarded_host_port".to_string(),
                json!(android.forwarded_host_port),
            ),
            (
                "forwarded_host_endpoint".to_string(),
                json!(format!("127.0.0.1:{}", android.forwarded_host_port)),
            ),
            ("host".to_string(), json!(persisted.host)),
            ("port".to_string(), json!(persisted.port)),
            ("scene".to_string(), json!(persisted.scene)),
            ("viewport".to_string(), json!(persisted.viewport)),
            ("deployment_mode".to_string(), json!("debug_apk")),
            ("apk_cache".to_string(), apk_cache_outcome.to_json()),
            ("artifact_dir".to_string(), json!(persisted.artifact_dir)),
            (
                "artifacts".to_string(),
                json!({
                    "root": persisted.artifact_dir,
                    "deployment_log": android.deployment_log_path,
                    "logcat": android.logcat_log_path,
                }),
            ),
            (
                "runtime".to_string(),
                response.get("data").cloned().unwrap_or(Value::Null),
            ),
        ]),
        failures: Vec::new(),
        exit_code: 0,
    }
}

pub(super) fn android_bootstrap_bytes(
    prepared: &PreparedSession,
    project_args: &[String],
    bootstrap_scene: &str,
    mode: Option<&str>,
) -> Result<Vec<u8>, FailureDetail> {
    let mut session = Map::from_iter([
        ("id".to_string(), json!(prepared.record.session_id)),
        ("token".to_string(), json!(prepared.token)),
        ("port".to_string(), json!(prepared.record.port)),
        ("scene".to_string(), json!(bootstrap_scene)),
        ("started_at".to_string(), json!(prepared.record.started_at)),
        ("project_args".to_string(), json!(project_args)),
    ]);
    // When absent the payload is byte-identical to the historical live-only bootstrap.
    if let Some(mode) = mode {
        session.insert("mode".to_string(), json!(mode));
    }
    serde_json::to_vec(&json!({
        "version": 1,
        "session": session,
    }))
    .map_err(|error| FailureDetail {
        kind: "android_bootstrap_invalid".to_string(),
        command: Some("session start".to_string()),
        message: Some(error.to_string()),
        expected: Some(json!("serializable in-memory Android bootstrap")),
        actual: Some(json!("serialization failed")),
        ..FailureDetail::default()
    })
}

pub(super) fn is_valid_android_live_scene(scene: &str) -> bool {
    !scene.is_empty()
        && scene.starts_with("res://")
        && !scene.contains("..")
        && !scene.contains('\\')
        && (scene.ends_with(".tscn") || scene.ends_with(".scn"))
}
