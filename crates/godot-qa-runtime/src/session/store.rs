use super::*;

pub fn load_session_record(
    config: &ProjectConfig,
    command: &str,
) -> Result<SessionRecord, FailureDetail> {
    load_session_record_with_source(config, command).map(|loaded| loaded.record)
}

#[derive(Debug, Clone)]
pub(super) struct LoadedSessionRecord {
    pub(super) record: SessionRecord,
    pub(super) source_path: PathBuf,
}

pub(super) fn load_session_record_with_source(
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

pub(super) fn load_session_record_from_path(
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

pub(super) fn artifact_session_fallback_enabled(config: &ProjectConfig) -> bool {
    config.artifacts_dir != config.project_root.join(".qa").join("artifacts")
        || session_token_path(config).exists()
}

pub(super) fn load_artifact_session_record(
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

pub(super) fn newest_valid_session_under(
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

pub(super) fn newest_valid_legacy_session_under(
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

pub(super) fn session_sort_key(record: &SessionRecord, path: &Path) -> String {
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

pub(super) fn recover_legacy_session_record(
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

pub(super) fn read_json_file(
    path: &Path,
    command: &str,
    expected: &str,
) -> Result<Value, FailureDetail> {
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

pub(super) fn value_string(value: Option<&Value>) -> Option<String> {
    value.and_then(Value::as_str).map(str::to_string)
}

pub(super) fn value_bool(value: Option<&Value>) -> Option<bool> {
    value.and_then(Value::as_bool)
}

pub(super) fn value_u16(value: Option<&Value>) -> Option<u16> {
    value
        .and_then(Value::as_u64)
        .filter(|port| (1..=u16::MAX as u64).contains(port))
        .map(|port| port as u16)
}

pub(super) fn value_i64_array(value: Option<&Value>) -> Option<Vec<i64>> {
    value?
        .as_array()
        .map(|items| items.iter().filter_map(Value::as_i64).collect())
}

pub(super) fn value_string_array(value: Option<&Value>) -> Option<Vec<String>> {
    value?.as_array().map(|items| {
        items
            .iter()
            .filter_map(Value::as_str)
            .map(str::to_string)
            .collect()
    })
}

pub fn session_json_path(config: &ProjectConfig) -> PathBuf {
    config.project_root.join(".qa").join("session.json")
}

pub(super) fn session_metadata_path(config: &ProjectConfig, record: &SessionRecord) -> PathBuf {
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
pub(super) struct PreparedSession {
    pub(super) record: SessionRecord,
    pub(super) request_path: PathBuf,
    pub(super) logs_path: PathBuf,
    pub(super) token: String,
}

pub(super) fn session_failure(
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

pub(super) fn read_session_token(
    record: &SessionRecord,
    command: &str,
) -> Result<String, FailureDetail> {
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
    (|| -> std::io::Result<String> {
        let mut options = fs::OpenOptions::new();
        options.read(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.custom_flags(libc::O_NOFOLLOW);
        }
        let mut file = options.open(&token_path)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            file.set_permissions(fs::Permissions::from_mode(0o600))?;
        }
        let mut contents = String::new();
        file.read_to_string(&mut contents)?;
        Ok(contents)
    })()
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
pub(super) fn session_start_context(
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
pub(super) fn prepare_live_session(
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
pub(super) fn prepare_live_session_with_latest_link(
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
        fs::create_dir_all(parent)
            .map_err(|error| locate(artifact_write_failure(parent, &error)))?;
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

pub fn clear_session_files(config: &ProjectConfig) {
    stop_debug_host(config);
    let _ = fs::remove_file(session_json_path(config));
    let _ = fs::remove_file(session_token_path(config));
}

pub(super) fn pick_port() -> Result<u16, FailureDetail> {
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
pub(super) fn random_unprivileged_port() -> u16 {
    20_000 + (crate::random::random_u16() % 40_000)
}

pub(super) fn write_json(path: &Path, payload: &Value) -> Result<(), FailureDetail> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| artifact_write_failure(parent, &error))?;
    }
    let content = serde_json::to_string_pretty(payload).unwrap_or_else(|_| "{}".to_string());
    fs::write(path, content).map_err(|error| artifact_write_failure(path, &error))
}

pub(super) fn update_latest_link(target: &Path, latest_path: &Path) -> Result<(), FailureDetail> {
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
    return Err(artifact_write_failure(
        latest_path,
        &std::io::Error::other("directory links unsupported"),
    ));

    Ok(())
}

#[cfg(unix)]
pub(super) fn latest_link_tmp_path(latest_path: &Path) -> PathBuf {
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
