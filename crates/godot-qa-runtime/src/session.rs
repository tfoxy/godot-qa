// Extracted from lib.rs; included by lib.rs to preserve the original crate namespace.

pub fn load_session_record(
    config: &ProjectConfig,
    command: &str,
) -> Result<SessionRecord, FailureDetail> {
    load_session_record_with_source(config, command).map(|loaded| loaded.record)
}

#[derive(Debug, Clone)]
struct LoadedSessionRecord {
    record: SessionRecord,
    source_path: PathBuf,
}

fn load_session_record_with_source(
    config: &ProjectConfig,
    command: &str,
) -> Result<LoadedSessionRecord, FailureDetail> {
    let session_path = session_json_path(config);
    if session_path.exists() {
        return load_session_record_from_path(&session_path, command).map(|record| {
            LoadedSessionRecord {
                record,
                source_path: session_path,
            }
        });
    }

    if artifact_session_fallback_enabled(config) {
        if let Some(loaded) = load_artifact_session_record(config, command)? {
            return Ok(loaded);
        }
        return Err(session_failure(
            "session_not_found",
            command,
            &config.artifacts_dir,
            "live session file was not found",
            json!("existing .qa/session.json or live session metadata under --artifact-dir"),
            json!({
                "primary": session_path,
                "artifact_dir": config.artifacts_dir,
            }),
        ));
    }

    Err(session_failure(
        "session_not_found",
        command,
        &session_path,
        "live session file was not found",
        json!("existing .qa/session.json"),
        json!("missing"),
    ))
}

fn load_session_record_from_path(
    session_path: &Path,
    command: &str,
) -> Result<SessionRecord, FailureDetail> {
    let contents = fs::read_to_string(session_path).map_err(|error| {
        session_failure(
            "session_file_invalid",
            command,
            session_path,
            &error.to_string(),
            json!("valid JSON session file"),
            json!("OSError"),
        )
    })?;
    let payload: Value = serde_json::from_str(&contents).map_err(|error| {
        session_failure(
            "session_file_invalid",
            command,
            session_path,
            &error.to_string(),
            json!("valid JSON session file"),
            json!("JSONDecodeError"),
        )
    })?;
    serde_json::from_value::<SessionRecord>(payload).map_err(|error| {
        session_failure(
            "session_file_invalid",
            command,
            session_path,
            &error.to_string(),
            json!("complete session metadata"),
            json!("ValueError"),
        )
    })
}

fn artifact_session_fallback_enabled(config: &ProjectConfig) -> bool {
    config.artifacts_dir != config.project_root.join(".qa").join("artifacts")
        || session_token_path(config).exists()
}

fn load_artifact_session_record(
    config: &ProjectConfig,
    command: &str,
) -> Result<Option<LoadedSessionRecord>, FailureDetail> {
    let artifact_dir = &config.artifacts_dir;
    let direct_session = artifact_dir.join("session.json");
    if direct_session.exists() {
        return load_session_record_from_path(&direct_session, command).map(|record| {
            Some(LoadedSessionRecord {
                record,
                source_path: direct_session,
            })
        });
    }
    let direct_request = artifact_dir.join("request.json");
    if direct_request.exists() {
        return recover_legacy_session_record(config, artifact_dir, command).map(|record| {
            Some(LoadedSessionRecord {
                record,
                source_path: direct_request,
            })
        });
    }

    let newest_session = newest_valid_session_under(&artifact_dir.join("live"), command)?;
    if newest_session.is_some() {
        return Ok(newest_session);
    }

    Ok(newest_valid_legacy_session_under(
        config,
        &artifact_dir.join("live"),
        command,
    ))
}

fn newest_valid_session_under(
    live_dir: &Path,
    command: &str,
) -> Result<Option<LoadedSessionRecord>, FailureDetail> {
    if !live_dir.is_dir() {
        return Ok(None);
    }
    let mut newest: Option<(String, LoadedSessionRecord)> = None;
    for entry in fs::read_dir(live_dir).map_err(|error| {
        session_failure(
            "session_file_invalid",
            command,
            live_dir,
            &error.to_string(),
            json!("readable live artifact directory"),
            json!("OSError"),
        )
    })? {
        let Ok(entry) = entry else {
            continue;
        };
        let session_path = entry.path().join("session.json");
        if !session_path.exists() {
            continue;
        }
        let Ok(record) = load_session_record_from_path(&session_path, command) else {
            continue;
        };
        let sort_key = session_sort_key(&record, &session_path);
        let loaded = LoadedSessionRecord {
            record,
            source_path: session_path,
        };
        if newest
            .as_ref()
            .is_none_or(|(current_key, _)| sort_key > *current_key)
        {
            newest = Some((sort_key, loaded));
        }
    }
    Ok(newest.map(|(_, loaded)| loaded))
}

fn newest_valid_legacy_session_under(
    config: &ProjectConfig,
    live_dir: &Path,
    command: &str,
) -> Option<LoadedSessionRecord> {
    if !live_dir.is_dir() {
        return None;
    }
    let mut newest: Option<(String, LoadedSessionRecord)> = None;
    let entries = fs::read_dir(live_dir).ok()?;
    for entry in entries.flatten() {
        let artifact_root = entry.path();
        let request_path = artifact_root.join("request.json");
        if !request_path.exists() {
            continue;
        }
        let Ok(record) = recover_legacy_session_record(config, &artifact_root, command) else {
            continue;
        };
        let sort_key = session_sort_key(&record, &request_path);
        let loaded = LoadedSessionRecord {
            record,
            source_path: request_path,
        };
        if newest
            .as_ref()
            .is_none_or(|(current_key, _)| sort_key > *current_key)
        {
            newest = Some((sort_key, loaded));
        }
    }
    newest.map(|(_, loaded)| loaded)
}

fn session_sort_key(record: &SessionRecord, path: &Path) -> String {
    if !record.started_at.is_empty() {
        return format!("2:{}", record.started_at);
    }
    if !record.session_id.is_empty() {
        return format!("1:{}", record.session_id);
    }
    let modified = path
        .metadata()
        .and_then(|metadata| metadata.modified())
        .ok()
        .and_then(|modified| modified.duration_since(UNIX_EPOCH).ok())
        .map(|duration| duration.as_millis())
        .unwrap_or_default();
    format!("0:{modified:020}")
}

fn recover_legacy_session_record(
    config: &ProjectConfig,
    artifact_root: &Path,
    command: &str,
) -> Result<SessionRecord, FailureDetail> {
    let request_path = artifact_root.join("request.json");
    let request = read_json_file(
        &request_path,
        command,
        "valid live session request metadata",
    )?;
    let meta_path = artifact_root.join("meta.json");
    let meta = if meta_path.exists() {
        read_json_file(&meta_path, command, "valid live session artifact metadata")?
    } else {
        json!({})
    };
    let session = request.get("session").and_then(Value::as_object);
    let artifacts = request.get("artifacts").and_then(Value::as_object);
    let session_id = value_string(session.and_then(|data| data.get("id")))
        .or_else(|| value_string(meta.get("session_id")))
        .or_else(|| {
            artifact_root
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
        })
        .unwrap_or_else(|| "unknown".to_string());
    let port = value_u16(session.and_then(|data| data.get("port"))).ok_or_else(|| {
        session_failure(
            "session_file_invalid",
            command,
            &request_path,
            "legacy live session metadata did not include a valid port",
            json!("session.port between 1 and 65535"),
            session
                .and_then(|data| data.get("port"))
                .cloned()
                .unwrap_or(Value::Null),
        )
    })?;
    let artifact_dir = value_string(artifacts.and_then(|data| data.get("root")))
        .unwrap_or_else(|| artifact_root.to_string_lossy().into_owned());
    Ok(SessionRecord {
        process_identity: None,
        protocol_version: PROTOCOL_VERSION.to_string(),
        session_id,
        pid: 0,
        host: value_string(session.and_then(|data| data.get("host")))
            .unwrap_or_else(|| "127.0.0.1".to_string()),
        port,
        scene: value_string(session.and_then(|data| data.get("scene")))
            .or_else(|| value_string(meta.get("scene")))
            .unwrap_or_default(),
        viewport: value_i64_array(session.and_then(|data| data.get("viewport")))
            .or_else(|| value_i64_array(meta.get("viewport")))
            .unwrap_or_else(|| vec![0, 0]),
        artifact_dir,
        request_path: request_path.to_string_lossy().into_owned(),
        token_file: session_token_path(config).to_string_lossy().into_owned(),
        started_at: value_string(session.and_then(|data| data.get("started_at")))
            .or_else(|| value_string(meta.get("started_at")))
            .unwrap_or_default(),
        headless: value_bool(session.and_then(|data| data.get("headless")))
            .or_else(|| value_bool(meta.get("headless")))
            .unwrap_or(false),
        project_args: value_string_array(session.and_then(|data| data.get("project_args")))
            .or_else(|| value_string_array(meta.get("project_args")))
            .unwrap_or_default(),
        frame_pacing: session
            .and_then(|data| data.get("frame_pacing"))
            .filter(|value| !value.is_null())
            .and_then(|value| serde_json::from_value(value.clone()).ok()),
        target: SessionTarget::Desktop,
        android: None,
    })
}

fn read_json_file(path: &Path, command: &str, expected: &str) -> Result<Value, FailureDetail> {
    let contents = fs::read_to_string(path).map_err(|error| {
        session_failure(
            "session_file_invalid",
            command,
            path,
            &error.to_string(),
            json!(expected),
            json!("OSError"),
        )
    })?;
    serde_json::from_str(&contents).map_err(|error| {
        session_failure(
            "session_file_invalid",
            command,
            path,
            &error.to_string(),
            json!(expected),
            json!("JSONDecodeError"),
        )
    })
}

fn value_string(value: Option<&Value>) -> Option<String> {
    value.and_then(Value::as_str).map(str::to_string)
}

fn value_bool(value: Option<&Value>) -> Option<bool> {
    value.and_then(Value::as_bool)
}

fn value_u16(value: Option<&Value>) -> Option<u16> {
    value
        .and_then(Value::as_u64)
        .filter(|port| (1..=u16::MAX as u64).contains(port))
        .map(|port| port as u16)
}

fn value_i64_array(value: Option<&Value>) -> Option<Vec<i64>> {
    value?
        .as_array()
        .map(|items| items.iter().filter_map(Value::as_i64).collect())
}

fn value_string_array(value: Option<&Value>) -> Option<Vec<String>> {
    value?.as_array().map(|items| {
        items
            .iter()
            .filter_map(Value::as_str)
            .map(str::to_string)
            .collect()
    })
}

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

fn classify_session_connect_failure(
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

fn android_option_unsupported(option: &str, actual: Value) -> CommandResult {
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

fn android_start_failure(failure: FailureDetail, data: Map<String, Value>) -> CommandResult {
    CommandResult {
        ok: false,
        command: "session start".to_string(),
        message: "unable to start Android live session".to_string(),
        data,
        failures: vec![failure],
        exit_code: 1,
    }
}

fn android_start_cleanup(
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

fn android_start_failure_after_cleanup(
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

fn append_android_deployment_output(
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

fn append_android_deployment_note(path: &Path, note: &str) -> std::io::Result<()> {
    let mut file = fs::OpenOptions::new().create(true).append(true).open(path)?;
    writeln!(file, "\n[godot-qa android]\n{note}")
}

fn append_android_deployment_failure(
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
struct ApkCacheOutcome {
    status: &'static str,
    key: Option<String>,
    reason: Option<String>,
}

impl ApkCacheOutcome {
    fn to_json(&self) -> Value {
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
fn android_manifest_excluded_dirs(config: &ProjectConfig) -> Vec<PathBuf> {
    if config.artifacts_dir.starts_with(&config.project_root) {
        vec![config.artifacts_dir.clone()]
    } else {
        Vec::new()
    }
}

fn export_debug_apk_into(
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
fn android_obtain_session_apk(
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
    if !no_apk_cache
        && let Some(entry) = apk_cache::lookup(&cache_root, &key)
    {
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
        let _ = append_android_deployment_note(deployment_log, "apk-cache: bypassed (--no-apk-cache)");
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
            let _ = append_android_deployment_note(deployment_log, &format!("apk-cache: stored {key}"));
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
struct AndroidDeployRequest<'a> {
    config: &'a ProjectConfig,
    godot: &'a str,
    adb: &'a str,
    device: &'a godot_qa_process::android::AndroidDevice,
    preset: &'a godot_qa_process::android::AndroidExportPreset,
    prepared: &'a PreparedSession,
    bootstrap_scene: &'a str,
    bootstrap_mode: Option<&'a str>,
    no_apk_cache: bool,
    effective_project_args: &'a [String],
    deployment_log: &'a Path,
    logcat_log: &'a Path,
    apk_path: &'a Path,
}

/// Successful deployment: the fully populated session record (pid + logcat filled,
/// viewport reconciled from the bridge), the bridge probe response, and the debug-APK
/// cache outcome for the result payload.
#[derive(Debug)]
struct AndroidDeployOutcome {
    record: SessionRecord,
    probe_data: Value,
    apk_cache: ApkCacheOutcome,
}

/// Failed deployment. `record` is filled up to the failure point so the caller's
/// cleanup honors any provisioned state (`logcat_pid`, forward, launch). `needs_cleanup`
/// distinguishes a pre-bootstrap failure (nothing deployed yet — the caller returns a
/// plain failure, optionally retaining the exported APK for debugging) from a
/// post-bootstrap failure (the caller must run the scoped cleanup).
#[derive(Debug)]
struct AndroidDeployError {
    failure: FailureDetail,
    data_extras: Map<String, Value>,
    forwarded: bool,
    launched: bool,
    needs_cleanup: bool,
    record: SessionRecord,
}

fn deploy_apk_error(
    failure: FailureDetail,
    apk_cache: &ApkCacheOutcome,
    retained_apk: Option<&Path>,
    forwarded: bool,
    launched: bool,
    needs_cleanup: bool,
    record: SessionRecord,
) -> AndroidDeployError {
    let mut data_extras =
        Map::from_iter([("apk_cache".to_string(), apk_cache.to_json())]);
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
fn android_deploy_stages(
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
    let forward =
        match godot_qa_process::android::create_forward(adb, &device.serial, persisted.port, device_port)
        {
            Ok(output) => output,
            Err(failure) => {
                append_android_deployment_failure(
                    deployment_log,
                    "forward",
                    &failure,
                    &prepared.token,
                );
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

fn session_start_android_result(
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

fn session_status_like_result(config: &ProjectConfig, command: &str) -> CommandResult {
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
        match spawn_debug_host(config) {
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
            "session_connect_failed" | "stale_session"
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

pub fn session_reload_result(config: &ProjectConfig) -> CommandResult {
    run_live_protocol_command(config, "session reload", SESSION_RELOAD_SCENE, Map::new())
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
fn granular_reload_success(
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
fn granular_failure(
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
fn debug_host_send(
    config: &ProjectConfig,
    command: &str,
    request: &godot_qa_debug::host::ControlRequest,
) -> Result<godot_qa_debug::host::ControlResponse, CommandResult> {
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
    match godot_qa_debug::host::send_control_request(record.control_port, request) {
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

fn granular_reload_result(
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


pub fn session_json_path(config: &ProjectConfig) -> PathBuf {
    config.project_root.join(".qa").join("session.json")
}

fn session_metadata_path(config: &ProjectConfig, record: &SessionRecord) -> PathBuf {
    let project_session = session_json_path(config);
    if project_session.exists() {
        return project_session;
    }
    let artifact_session = Path::new(&record.artifact_dir).join("session.json");
    if artifact_session.exists() {
        return artifact_session;
    }
    if !record.request_path.is_empty() {
        return PathBuf::from(&record.request_path);
    }
    project_session
}

pub fn session_token_path(config: &ProjectConfig) -> PathBuf {
    config.project_root.join(".qa").join("session.token")
}


#[derive(Debug, Clone)]
struct PreparedSession {
    record: SessionRecord,
    request_path: PathBuf,
    logs_path: PathBuf,
    token: String,
}

fn android_bootstrap_bytes(
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

fn session_failure(
    kind: &str,
    command: &str,
    path: &Path,
    message: &str,
    expected: Value,
    actual: Value,
) -> FailureDetail {
    FailureDetail {
        kind: kind.to_string(),
        command: Some(command.to_string()),
        path: Some(path.to_string_lossy().into_owned()),
        message: Some(message.to_string()),
        expected: Some(expected),
        actual: Some(actual),
        ..FailureDetail::default()
    }
}

fn read_session_token(record: &SessionRecord, command: &str) -> Result<String, FailureDetail> {
    let token_path = PathBuf::from(&record.token_file);
    if !token_path.exists() {
        return Err(session_failure(
            "session_token_missing",
            command,
            &token_path,
            "live session token file was not found",
            json!("existing .qa/session.token"),
            json!("missing"),
        ));
    }
    fs::read_to_string(&token_path)
        .map(|contents| contents.trim().to_string())
        .map_err(|error| {
            session_failure(
                "session_token_missing",
                command,
                &token_path,
                &error.to_string(),
                json!("existing .qa/session.token"),
                json!("OSError"),
            )
        })
}

/// Tag a session-start failure with the session it belongs to. Once the session's
/// artifact directory exists, every later failure can name it — which is what turns
/// an anonymous "session start failed" inside a K-lane warm pool into a pointer at
/// the exact half-written `.qa/artifacts/live/<id>/` directory that is the evidence.
fn session_start_context(
    mut failure: FailureDetail,
    session_id: &str,
    artifact_dir: &Path,
) -> FailureDetail {
    failure
        .details
        .insert("session_id".to_string(), json!(session_id));
    failure.details.insert(
        "session_artifact_dir".to_string(),
        json!(artifact_dir.to_string_lossy().into_owned()),
    );
    failure
}

/// Prepare the SINGLETON-facing live session: identical to
/// [`prepare_live_session_with_latest_link`] with the `.qa/artifacts/live/latest`
/// convenience link refreshed, which is what the interactive `session start`
/// workflow wants.
fn prepare_live_session(
    config: &ProjectConfig,
    scene: &str,
    viewport: Vec<i64>,
    headless: bool,
    project_args: &[String],
    frame_pacing: Option<SessionFramePacing>,
) -> Result<PreparedSession, FailureDetail> {
    prepare_live_session_with_latest_link(
        config,
        scene,
        viewport,
        headless,
        project_args,
        frame_pacing,
        true,
    )
}

/// Prepare one live session's artifact tree, request, token, and port.
///
/// `link_latest` controls the `.qa/artifacts/live/latest` convenience symlink.
/// POOLED sessions pass `false`: nothing ever resolves a pooled session through
/// `latest` (they are addressed by an in-memory handle, never by the singleton
/// files), K concurrent lanes rotating one shared link would only fight over it,
/// and losing that race used to fail an otherwise healthy warm session start
/// outright — a session that had already written its meta/request and was one
/// step from booting Godot.
fn prepare_live_session_with_latest_link(
    config: &ProjectConfig,
    scene: &str,
    viewport: Vec<i64>,
    headless: bool,
    project_args: &[String],
    frame_pacing: Option<SessionFramePacing>,
    link_latest: bool,
) -> Result<PreparedSession, FailureDetail> {
    let session_id = unique_session_id();
    let started_at = unique_started_at();
    let artifact_root = config.artifacts_dir.join("live").join(&session_id);
    let snapshots_dir = artifact_root.join("snapshots");
    let screenshots_dir = artifact_root.join("screenshots");
    let audio_dir = artifact_root.join("audio");
    fs::create_dir_all(&snapshots_dir)
        .map_err(|error| artifact_write_failure(&artifact_root, &error))?;
    // The session's artifact directory now exists, so every failure from here on
    // can carry its identity.
    let locate =
        |failure: FailureDetail| session_start_context(failure, &session_id, &artifact_root);
    fs::create_dir_all(&screenshots_dir)
        .map_err(|error| locate(artifact_write_failure(&artifact_root, &error)))?;
    fs::create_dir_all(&audio_dir)
        .map_err(|error| locate(artifact_write_failure(&artifact_root, &error)))?;
    let request_path = artifact_root.join("request.json");
    let logs_path = artifact_root.join("logs.txt");
    let token_path = session_token_path(config);
    if let Some(parent) = token_path.parent() {
        fs::create_dir_all(parent).map_err(|error| locate(artifact_write_failure(parent, &error)))?;
    }
    // Random per-session token (32 lowercase hex chars) as promised by
    // docs/security-model.md; never derive it from the guessable session id.
    let token = crate::random::random_hex(16);
    let port = pick_port().map_err(locate)?;
    let record = SessionRecord {
        process_identity: None,
        protocol_version: PROTOCOL_VERSION.to_string(),
        session_id: session_id.clone(),
        pid: 0,
        host: "127.0.0.1".to_string(),
        port,
        scene: scene.to_string(),
        viewport: viewport.clone(),
        artifact_dir: artifact_root.to_string_lossy().into_owned(),
        request_path: request_path.to_string_lossy().into_owned(),
        token_file: token_path.to_string_lossy().into_owned(),
        started_at: started_at.clone(),
        headless,
        project_args: redact_project_args_for_metadata(project_args),
        frame_pacing: frame_pacing.clone(),
        target: SessionTarget::Desktop,
        android: None,
    };
    let redacted_project_args = redact_project_args_for_metadata(project_args);
    let frame_pacing_json = frame_pacing
        .as_ref()
        .map(SessionFramePacing::to_json)
        .unwrap_or(Value::Null);
    write_json(
        &artifact_root.join("meta.json"),
        &json!({
            "tool": "godot-qa",
            "session_id": session_id,
            "scene": scene,
            "viewport": viewport,
            "started_at": started_at,
            "headless": headless,
            "project_args": redacted_project_args,
            "frame_pacing": frame_pacing_json,
            "audio_dir": audio_dir,
        }),
    )
    .map_err(locate)?;
    write_json(
        &request_path,
        &json!({
            "session": {
                "id": record.session_id,
                "started_at": record.started_at,
                "host": record.host,
                "port": record.port,
                "scene": scene,
                "viewport": record.viewport,
                "headless": record.headless,
                "project_args": redacted_project_args,
                "frame_pacing": frame_pacing_json,
            },
            "artifacts": {
                "root": artifact_root,
                "meta": artifact_root.join("meta.json"),
                "request": request_path,
                "logs": logs_path,
                "snapshots_dir": snapshots_dir,
                "screenshots_dir": screenshots_dir,
                "audio_dir": audio_dir,
            },
        }),
    )
    .map_err(locate)?;
    if link_latest {
        update_live_latest_link(config, &artifact_root).map_err(locate)?;
    }
    Ok(PreparedSession {
        record,
        request_path,
        logs_path,
        token,
    })
}


fn parse_viewport(value: &str) -> Option<Vec<i64>> {
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

fn validate_live_fixed_fps(fixed_fps: i64, command: &str) -> Result<i64, FailureDetail> {
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

fn project_default_live_frame_pacing(project_root: &Path) -> SessionFramePacing {
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

fn parse_project_settings(path: &Path) -> std::collections::HashMap<String, i64> {
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

fn resolve_session_start_scene(
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

fn is_valid_android_live_scene(scene: &str) -> bool {
    !scene.is_empty()
        && scene.starts_with("res://")
        && !scene.contains("..")
        && !scene.contains('\\')
        && (scene.ends_with(".tscn") || scene.ends_with(".scn"))
}

/// The session's launch scene (`res://…`) from `project.godot`, if any.
fn project_main_scene(config: &ProjectConfig) -> Option<String> {
    let contents = read_project_godot_cached(&config.project_root.join("project.godot"))?;
    parse_project_main_scene(&contents)
}

fn parse_project_main_scene(contents: &str) -> Option<String> {
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

fn parse_project_setting_string(value: &str) -> String {
    if value.len() >= 2 && value.starts_with('"') && value.ends_with('"') {
        return value[1..value.len() - 1]
            .replace("\\\"", "\"")
            .replace("\\\\", "\\");
    }
    value.to_string()
}

pub fn clear_session_files(config: &ProjectConfig) {
    stop_debug_host(config);
    let _ = fs::remove_file(session_json_path(config));
    let _ = fs::remove_file(session_token_path(config));
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
        .map(|record| godot_qa_process::process_identity_matches(record.pid, record.process_identity))
        .unwrap_or(false)
}

/// Spawn the persistent debug-host sidecar (this binary re-invoked as `__debug-host`)
/// and wait for it to bind its listeners. The sidecar picks ephemeral ports itself and
/// records them, which removes any pick/bind race with the parent.
fn spawn_debug_host(
    config: &ProjectConfig,
) -> Result<godot_qa_debug::host::DebugHostRecord, FailureDetail> {
    let record_path = debug_host_json_path(config);
    let _ = fs::remove_file(&record_path);
    if let Some(parent) = record_path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    let log_path = record_path.with_file_name("debug_host.log");
    let exe = std::env::current_exe().map_err(|error| FailureDetail {
        kind: "debug_host_spawn_failed".to_string(),
        command: Some("session start".to_string()),
        message: Some(format!("could not resolve godot-qa executable: {error}")),
        expected: Some(json!("resolvable current executable")),
        actual: Some(json!("unavailable")),
        ..FailureDetail::default()
    })?;
    let mut command = std::process::Command::new(&exe);
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
    if let Some(scene) = project_main_scene(config) {
        command.arg("--scene").arg(scene);
    }
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

/// Stop the debug-host sidecar (if any) and remove its record. Safe to call when no
/// hot-reload session is running.
fn stop_debug_host(config: &ProjectConfig) {
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

fn stop_launched_session_process(
    config: &ProjectConfig,
    record: &SessionRecord,
) -> Option<FailureDetail> {
    if record.target == SessionTarget::Android {
        return Some(FailureDetail {
            kind: "android_cleanup_unavailable".to_string(),
            command: Some("session start".to_string()),
            path: Some(session_json_path(config).to_string_lossy().into_owned()),
            message: Some("Android session cleanup requires the Android process adapter".to_string()),
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

fn pick_port() -> Result<u16, FailureDetail> {
    match std::net::TcpListener::bind(("127.0.0.1", 0)) {
        Ok(listener) => listener
            .local_addr()
            .map(|addr| addr.port())
            .map_err(|error| artifact_write_failure(Path::new("127.0.0.1"), &error)),
        Err(error) if error.kind() == std::io::ErrorKind::PermissionDenied => {
            // The kernel refused an ephemeral bind (sandboxed CI). Pick a random
            // port in the unprivileged range instead of a time-derived one, which
            // collides whenever two starts land in the same nanosecond window.
            Ok(random_unprivileged_port())
        }
        Err(error) => Err(artifact_write_failure(Path::new("127.0.0.1"), &error)),
    }
}

/// Random port in the unprivileged, non-ephemeral range (20000..=59999) for the
/// bind-refused fallback path in [`pick_port`].
fn random_unprivileged_port() -> u16 {
    20_000 + (crate::random::random_u16() % 40_000)
}

fn write_json(path: &Path, payload: &Value) -> Result<(), FailureDetail> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| artifact_write_failure(parent, &error))?;
    }
    let content = serde_json::to_string_pretty(payload).unwrap_or_else(|_| "{}".to_string());
    fs::write(path, content).map_err(|error| artifact_write_failure(path, &error))
}

fn update_latest_link(target: &Path, latest_path: &Path) -> Result<(), FailureDetail> {
    if let Some(parent) = latest_path.parent() {
        fs::create_dir_all(parent).map_err(|error| artifact_write_failure(parent, &error))?;
    }
    #[cfg(unix)]
    {
        // Rotate the link atomically: create the new symlink under a unique sibling temp
        // name, then rename it over `latest`. rename(2) replaces an existing symlink/file in
        // one step, which avoids the EEXIST that bites when `latest` is a *dangling* symlink
        // (Path::exists follows the link and would skip cleanup) and avoids the TOCTOU race
        // between concurrent `session start` invocations. A pre-existing real directory
        // (legacy / non-unix fallback) must be removed first because rename cannot replace a
        // directory with a symlink; symlinks and files are left for rename to overwrite.
        if let Ok(metadata) = fs::symlink_metadata(latest_path)
            && metadata.is_dir()
            && !metadata.file_type().is_symlink()
        {
            fs::remove_dir_all(latest_path)
                .map_err(|error| artifact_write_failure(latest_path, &error))?;
        }
        let tmp_path = latest_link_tmp_path(latest_path);
        std::os::unix::fs::symlink(target, &tmp_path)
            .map_err(|error| artifact_write_failure(&tmp_path, &error))?;
        if let Err(error) = fs::rename(&tmp_path, latest_path) {
            let _ = fs::remove_file(&tmp_path);
            return Err(artifact_write_failure(latest_path, &error));
        }
    }
    #[cfg(windows)]
    godot_qa_process::directory_link::replace(target, latest_path)
        .map_err(|error| artifact_write_failure(latest_path, &error))?;
    #[cfg(not(any(unix, windows)))]
    return Err(artifact_write_failure(latest_path, &std::io::Error::other("directory links unsupported")));

    Ok(())
}

#[cfg(unix)]
fn latest_link_tmp_path(latest_path: &Path) -> PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let name = latest_path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("latest");
    // pid + nanos is NOT unique: every lane of ONE CLI process shares the pid, so
    // uniqueness rested entirely on the nanosecond clock. Two lanes rotating the
    // link inside the same tick both build `latest.tmp-<pid>-<nanos>`; one wins
    // symlink(2) and the other fails EEXIST, taking a healthy session start with
    // it. The random suffix removes the clock from the uniqueness argument.
    latest_path.with_file_name(format!(
        "{name}.tmp-{}-{nanos}-{}",
        std::process::id(),
        crate::random::random_hex(4)
    ))
}

#[cfg(test)]
mod granular_reload_reporting_tests {
    use super::*;

    #[test]
    fn resource_reload_reports_requested_with_caveat_not_applied() {
        let result = granular_reload_success(
            "session reload",
            "reload_resources",
            vec!["res://ui/theme.tres".to_string()],
            true,
        );
        assert!(result.ok);
        assert_eq!(result.data.get("status"), Some(&json!("requested")));
        // Never claim resources were "applied" (fire-and-forget, DEBUG_ENABLED-gated).
        assert!(result.data.get("applied").is_none());
        assert_eq!(
            result.data.get("requested"),
            Some(&json!(["res://ui/theme.tres"]))
        );
        assert_eq!(result.data.get("caveat"), Some(&json!(RESOURCE_RELOAD_CAVEAT)));
    }

    #[test]
    fn script_reload_reports_applied() {
        let result = granular_reload_success(
            "session reload",
            "reload_scripts",
            vec!["res://player.gd".to_string()],
            true,
        );
        assert!(result.ok);
        assert_eq!(result.data.get("status"), Some(&json!("applied")));
        assert_eq!(result.data.get("applied"), Some(&json!(["res://player.gd"])));
        assert!(result.data.get("caveat").is_none());
    }
}
