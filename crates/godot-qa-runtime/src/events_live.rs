// Extracted from lib.rs; included by lib.rs to preserve the original crate namespace.

pub fn trace_record_result(config: &ProjectConfig, params: Map<String, Value>) -> CommandResult {
    let command = "trace record";
    let loaded = match load_session_record_with_source(config, command) {
        Ok(loaded) => loaded,
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
    let record = loaded.record.clone();
    let live_result = run_live_protocol_command_with_loaded(command, TRACE_RECORD, params, loaded, None);
    if !live_result.ok {
        return live_result;
    }

    let mut data = live_result.data;
    let trace_id = data
        .get("trace_id")
        .and_then(Value::as_str)
        .filter(|id| safe_artifact_id(id))
        .map(str::to_string)
        .unwrap_or_else(unique_session_id);
    let artifact_path = PathBuf::from(&record.artifact_dir)
        .join("traces")
        .join(trace_id)
        .join("trace.json");
    if let Err(mut failure) = write_json(&artifact_path, &Value::Object(data.clone())) {
        failure.command = Some(command.to_string());
        return CommandResult {
            ok: false,
            command: command.to_string(),
            message: "trace recording failed".to_string(),
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
    artifacts.insert("trace".to_string(), json!(artifact_path));
    data.insert("artifacts".to_string(), Value::Object(artifacts));

    // Mirror `audio capture`: when a `stop_after_state` predicate was requested but never
    // matched before the frame budget elapsed, surface it as a structured failure.
    let mut failures = Vec::new();
    if data.get("stop_condition_timed_out").and_then(Value::as_bool) == Some(true) {
        failures.push(FailureDetail {
            kind: "trace_record.stop_condition_timeout".to_string(),
            command: Some(command.to_string()),
            message: Some(
                "trace record stop_after_state did not match before the trace window ended"
                    .to_string(),
            ),
            expected: Some(json!("stop_after_state matched before the frame budget elapsed")),
            actual: Some(
                data.get("stop_after_state")
                    .cloned()
                    .unwrap_or_else(|| json!("not_matched")),
            ),
            ..FailureDetail::default()
        });
    }
    let ok = failures.is_empty();
    CommandResult {
        ok,
        command: command.to_string(),
        message: if ok {
            "trace recording completed".to_string()
        } else {
            "trace recording failed".to_string()
        },
        data,
        failures,
        exit_code: if ok { 0 } else { 1 },
    }
}

pub fn event_trace_result(config: &ProjectConfig, params: Map<String, Value>) -> CommandResult {
    let command = "event trace";
    let strict_warnings = params
        .get("strict_warnings")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let pairing_config = match pairing_config_from_params(&params, &config.project_root, command) {
        Ok(config) => config,
        Err(failure) => {
            return CommandResult {
                ok: false,
                command: command.to_string(),
                message: "event trace failed".to_string(),
                data: Map::new(),
                failures: vec![failure],
                exit_code: 1,
            };
        }
    };
    let loaded = match load_session_record_with_source(config, command) {
        Ok(loaded) => loaded,
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
    let record = loaded.record.clone();
    let live_result = run_live_protocol_command_with_loaded(command, EVENT_TRACE, params, loaded, None);
    if !live_result.ok {
        return live_result;
    }

    let mut data = live_result.data;
    let trace_id = data
        .get("event_trace_id")
        .or_else(|| data.get("trace_id"))
        .and_then(Value::as_str)
        .filter(|id| safe_artifact_id(id))
        .map(str::to_string)
        .unwrap_or_else(unique_event_trace_id);
    let artifact = event_trace_artifact_from_runtime_data(&data);
    let mut summary = event_trace::summarize_event_trace(
        &artifact.events,
        pairing_config.as_ref(),
        artifact.provider_errors.len(),
    );
    merge_artifact_warnings_into_summary(&mut summary, &artifact.warnings);
    let event_dir = PathBuf::from(&record.artifact_dir)
        .join("events")
        .join(&trace_id);
    let events_path = event_dir.join("events.json");
    let summary_path = event_dir.join("summary.json");
    if let Err(mut failure) = write_json(&events_path, &json!(artifact)) {
        failure.command = Some(command.to_string());
        return CommandResult {
            ok: false,
            command: command.to_string(),
            message: "event trace failed".to_string(),
            data,
            failures: vec![failure],
            exit_code: 1,
        };
    }
    if let Err(mut failure) = write_json(&summary_path, &json!(summary)) {
        failure.command = Some(command.to_string());
        return CommandResult {
            ok: false,
            command: command.to_string(),
            message: "event trace failed".to_string(),
            data,
            failures: vec![failure],
            exit_code: 1,
        };
    }
    if let Err(mut failure) =
        update_latest_link(&event_dir, &event_dir.parent().unwrap().join("latest"))
    {
        failure.command = Some(command.to_string());
        return CommandResult {
            ok: false,
            command: command.to_string(),
            message: "event trace failed".to_string(),
            data,
            failures: vec![failure],
            exit_code: 1,
        };
    }

    data.insert("session_id".to_string(), json!(record.session_id));
    data.insert("event_trace_id".to_string(), json!(trace_id));
    data.insert("summary".to_string(), json!(summary.clone()));
    extend_with_summary_fields(&mut data, &summary);
    let mut artifacts = data
        .get("artifacts")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    artifacts.insert("events".to_string(), json!(events_path));
    artifacts.insert("summary".to_string(), json!(summary_path));
    data.insert("artifacts".to_string(), Value::Object(artifacts));

    if strict_warnings && !summary.warning_ids.is_empty() {
        return CommandResult {
            ok: false,
            command: command.to_string(),
            message: "event trace completed with strict warnings".to_string(),
            data,
            failures: vec![event_trace_strict_warning_failure(
                command,
                &summary.warning_ids,
            )],
            exit_code: 1,
        };
    }

    CommandResult {
        ok: true,
        command: command.to_string(),
        message: "event trace completed".to_string(),
        data,
        failures: Vec::new(),
        exit_code: 0,
    }
}

pub fn event_summarize_result(
    events_path: &Path,
    pairing_config_path: Option<&Path>,
    strict_warnings: bool,
) -> CommandResult {
    let command = "event summarize";
    let events_value = match fs::read_to_string(events_path)
        .map_err(|error| artifact_write_failure(events_path, &error))
        .and_then(|contents| {
            serde_json::from_str::<Value>(&contents).map_err(|error| FailureDetail {
                kind: "event_trace.invalid_events_artifact".to_string(),
                command: Some(command.to_string()),
                path: Some(events_path.to_string_lossy().into_owned()),
                message: Some(format!("events artifact must be valid JSON: {error}")),
                expected: Some(json!("event trace events.json")),
                actual: Some(json!("invalid JSON")),
                ..FailureDetail::default()
            })
        }) {
        Ok(value) => value,
        Err(mut failure) => {
            failure.command = Some(command.to_string());
            return CommandResult {
                ok: false,
                command: command.to_string(),
                message: "event summarize failed".to_string(),
                data: Map::new(),
                failures: vec![failure],
                exit_code: 1,
            };
        }
    };
    let pairing_config = match pairing_config_path {
        Some(path) => match pairing_config_from_path(path, command) {
            Ok(config) => Some(config),
            Err(failure) => {
                return CommandResult {
                    ok: false,
                    command: command.to_string(),
                    message: "event summarize failed".to_string(),
                    data: Map::new(),
                    failures: vec![failure],
                    exit_code: 1,
                };
            }
        },
        None => None,
    };
    let artifact = event_trace_artifact_from_value(&events_value);
    let mut summary = event_trace::summarize_event_trace(
        &artifact.events,
        pairing_config.as_ref(),
        artifact.provider_errors.len(),
    );
    merge_artifact_warnings_into_summary(&mut summary, &artifact.warnings);
    let mut data = Map::new();
    data.insert("summary".to_string(), json!(summary.clone()));
    extend_with_summary_fields(&mut data, &summary);
    data.insert(
        "artifacts".to_string(),
        json!({
            "events": events_path,
        }),
    );
    if strict_warnings && !summary.warning_ids.is_empty() {
        return CommandResult {
            ok: false,
            command: command.to_string(),
            message: "event summarize completed with strict warnings".to_string(),
            data,
            failures: vec![event_trace_strict_warning_failure(
                command,
                &summary.warning_ids,
            )],
            exit_code: 1,
        };
    }
    CommandResult {
        ok: true,
        command: command.to_string(),
        message: "event summarize completed".to_string(),
        data,
        failures: Vec::new(),
        exit_code: 0,
    }
}

pub fn telemetry_record_result(
    config: &ProjectConfig,
    params: Map<String, Value>,
    out: Option<&Path>,
) -> CommandResult {
    let command = "telemetry record";
    let loaded = match load_session_record_with_source(config, command) {
        Ok(loaded) => loaded,
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
    let record = loaded.record.clone();
    let live_result =
        run_live_protocol_command_with_loaded(command, TELEMETRY_RECORD, params, loaded, None);
    if !live_result.ok {
        return live_result;
    }

    let mut data = live_result.data;
    let telemetry_id = data
        .get("telemetry_id")
        .and_then(Value::as_str)
        .filter(|id| safe_artifact_id(id))
        .map(str::to_string)
        .unwrap_or_else(unique_telemetry_id);
    // The raw rows are persisted as the JSONL artifact instead of being echoed
    // through the command result, so long recordings stay readable in --json.
    let rows: Vec<Value> = data
        .remove("rows")
        .and_then(|value| value.as_array().cloned())
        .unwrap_or_default();
    let telemetry_dir = PathBuf::from(&record.artifact_dir)
        .join("telemetry")
        .join(&telemetry_id);
    let rows_path = telemetry_dir.join("telemetry.jsonl");
    let recording_config = data
        .get("config")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    let manifest = json!({
        "telemetry_id": telemetry_id,
        "session_id": record.session_id,
        "scene": data
            .get("session")
            .and_then(|session| session.get("scene"))
            .cloned()
            .unwrap_or(Value::Null),
        "provider": recording_config.get("provider").cloned().unwrap_or(Value::Null),
        "target": recording_config.get("target").cloned().unwrap_or(Value::Null),
        "method": recording_config.get("method").cloned().unwrap_or(Value::Null),
        "frames": recording_config.get("frames").cloned().unwrap_or(Value::Null),
        "every": recording_config.get("every").cloned().unwrap_or(Value::Null),
        "rows": rows.len(),
        "truncated": data
            .get("truncated")
            .cloned()
            .unwrap_or(Value::Bool(false)),
        "provider_error_count": data
            .get("provider_error_count")
            .cloned()
            .unwrap_or_else(|| json!(0)),
        "artifacts": {
            "rows": rows_path,
        },
    });
    let manifest_path = telemetry_dir.join("manifest.json");
    if let Err(mut failure) = write_jsonl_rows(&rows_path, &rows) {
        failure.command = Some(command.to_string());
        return telemetry_write_failure_result(command, data, failure);
    }
    if let Err(mut failure) = write_json(&manifest_path, &manifest) {
        failure.command = Some(command.to_string());
        return telemetry_write_failure_result(command, data, failure);
    }
    if let Err(mut failure) = update_latest_link(
        &telemetry_dir,
        &telemetry_dir.parent().unwrap().join("latest"),
    ) {
        failure.command = Some(command.to_string());
        return telemetry_write_failure_result(command, data, failure);
    }

    let mut artifacts = data
        .get("artifacts")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    artifacts.insert("rows".to_string(), json!(rows_path));
    artifacts.insert("manifest".to_string(), json!(manifest_path));
    if let Some(out) = out {
        if let Some(parent) = out.parent()
            && let Err(error) = fs::create_dir_all(parent)
        {
            let mut failure = artifact_write_failure(out, &error);
            failure.command = Some(command.to_string());
            return telemetry_write_failure_result(command, data, failure);
        }
        if let Err(error) = fs::copy(&rows_path, out) {
            let mut failure = artifact_write_failure(out, &error);
            failure.command = Some(command.to_string());
            return telemetry_write_failure_result(command, data, failure);
        }
        artifacts.insert("out".to_string(), json!(out));
    }
    data.insert("session_id".to_string(), json!(record.session_id));
    data.insert("telemetry_id".to_string(), json!(telemetry_id));
    data.insert("row_count".to_string(), json!(rows.len()));
    data.insert("artifacts".to_string(), Value::Object(artifacts));

    CommandResult {
        ok: true,
        command: command.to_string(),
        message: "telemetry recording completed".to_string(),
        data,
        failures: Vec::new(),
        exit_code: 0,
    }
}

fn telemetry_write_failure_result(
    command: &str,
    data: Map<String, Value>,
    failure: FailureDetail,
) -> CommandResult {
    CommandResult {
        ok: false,
        command: command.to_string(),
        message: "telemetry recording failed".to_string(),
        data,
        failures: vec![failure],
        exit_code: 1,
    }
}

fn write_jsonl_rows(path: &Path, rows: &[Value]) -> Result<(), FailureDetail> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| artifact_write_failure(path, &error))?;
    }
    let mut contents = String::new();
    for row in rows {
        contents.push_str(&serde_json::to_string(row).unwrap_or_else(|_| "null".to_string()));
        contents.push('\n');
    }
    fs::write(path, contents).map_err(|error| artifact_write_failure(path, &error))
}

fn pairing_config_from_params(
    params: &Map<String, Value>,
    project_root: &Path,
    command: &str,
) -> Result<Option<event_trace::EventTraceConfig>, FailureDetail> {
    let Some(path) = params
        .get("pairing_config")
        .and_then(Value::as_str)
        .filter(|path| !path.trim().is_empty())
    else {
        return Ok(None);
    };
    pairing_config_from_path(
        resolve_project_relative_path(project_root, Path::new(path)).as_path(),
        command,
    )
    .map(Some)
}

fn resolve_project_relative_path(project_root: &Path, path: &Path) -> PathBuf {
    if path.is_absolute() || path.exists() {
        return path.to_path_buf();
    }
    project_root.join(path)
}

fn pairing_config_from_path(
    path: &Path,
    command: &str,
) -> Result<event_trace::EventTraceConfig, FailureDetail> {
    let contents = fs::read_to_string(path).map_err(|error| FailureDetail {
        kind: "event_trace.invalid_pairing_config".to_string(),
        command: Some(command.to_string()),
        path: Some(path.to_string_lossy().into_owned()),
        message: Some(format!(
            "event trace pairing config could not be read: {error}"
        )),
        expected: Some(json!("readable pairing config JSON file")),
        actual: Some(json!("unreadable file")),
        ..FailureDetail::default()
    })?;
    event_trace::parse_pairing_config(&contents).map_err(|message| FailureDetail {
        kind: "event_trace.invalid_pairing_config".to_string(),
        command: Some(command.to_string()),
        path: Some(path.to_string_lossy().into_owned()),
        message: Some(message),
        expected: Some(json!("valid event trace pairing config")),
        actual: Some(json!("invalid pairing config")),
        ..FailureDetail::default()
    })
}

fn event_trace_artifact_from_runtime_data(
    data: &Map<String, Value>,
) -> event_trace::EventTraceArtifact {
    let value = data
        .get("event_trace")
        .or_else(|| data.get("artifact"))
        .unwrap_or(&Value::Object(data.clone()))
        .clone();
    event_trace_artifact_from_value(&value)
}

fn event_trace_artifact_from_value(value: &Value) -> event_trace::EventTraceArtifact {
    if let Ok(artifact) = serde_json::from_value::<event_trace::EventTraceArtifact>(value.clone()) {
        return artifact;
    }
    let object = value.as_object();
    let events = object
        .and_then(|object| object.get("events"))
        .and_then(Value::as_array)
        .cloned()
        .or_else(|| value.as_array().cloned())
        .unwrap_or_default();
    let provider_errors = object
        .and_then(|object| object.get("provider_errors"))
        .cloned()
        .and_then(|value| serde_json::from_value(value).ok())
        .unwrap_or_default();
    let mut artifact = event_trace::normalize_event_trace_artifact(&events, provider_errors);
    let runtime_warnings: Vec<event_trace::EventTraceWarning> = object
        .and_then(|object| object.get("warnings"))
        .cloned()
        .and_then(|value| serde_json::from_value(value).ok())
        .unwrap_or_default();
    artifact.warnings.extend(runtime_warnings);
    artifact
}

fn extend_with_summary_fields(
    data: &mut Map<String, Value>,
    summary: &event_trace::EventTraceSummary,
) {
    let Value::Object(summary_object) = json!(summary) else {
        return;
    };
    for (key, value) in summary_object {
        data.insert(key, value);
    }
}

fn merge_artifact_warnings_into_summary(
    summary: &mut event_trace::EventTraceSummary,
    artifact_warnings: &[event_trace::EventTraceWarning],
) {
    for warning in artifact_warnings {
        if !summary.warning_ids.contains(&warning.id) {
            summary.warning_ids.push(warning.id.clone());
        }
        if !summary.warnings.iter().any(|existing| existing == warning) {
            summary.warnings.push(warning.clone());
        }
    }
    summary.warning_ids.sort();
    summary.warning_ids.dedup();
}

fn event_trace_strict_warning_failure(command: &str, warning_ids: &[String]) -> FailureDetail {
    FailureDetail {
        kind: "event_trace.strict_warnings".to_string(),
        command: Some(command.to_string()),
        message: Some(
            "event trace warnings are failures because --strict-warnings was set".to_string(),
        ),
        expected: Some(json!("no event trace warnings")),
        actual: Some(json!(warning_ids)),
        ..FailureDetail::default()
    }
}

fn update_live_latest_link(
    config: &ProjectConfig,
    artifact_root: &Path,
) -> Result<(), FailureDetail> {
    // The link lives only under the target project's artifacts dir. It is intentionally
    // not mirrored into the tool repo (config.repo_root): that mirror leaked live artifacts
    // into the godot-qa repo whenever --project pointed at an external project.
    let latest_path = config.artifacts_dir.join("live").join("latest");
    update_latest_link(artifact_root, &latest_path)
}

fn artifact_write_failure(path: &Path, error: &std::io::Error) -> FailureDetail {
    FailureDetail {
        kind: "artifact_write_failed".to_string(),
        path: Some(path.to_string_lossy().into_owned()),
        message: Some(error.to_string()),
        expected: Some(json!("Godot logs written to the artifact path")),
        actual: Some(json!("OSError")),
        ..FailureDetail::default()
    }
}

fn redact_project_args_for_metadata(project_args: &[String]) -> Vec<String> {
    let mut redacted = Vec::with_capacity(project_args.len());
    let mut redact_next = false;
    for arg in project_args {
        if redact_next {
            redacted.push(redacted_arg_value(arg));
            redact_next = false;
            continue;
        }
        if let Some((key, value)) = arg.split_once('=')
            && is_secret_bearing_key(key)
        {
            redacted.push(format!("{key}={}", redacted_arg_value(value)));
            continue;
        }
        if is_secret_bearing_key(arg) {
            redacted.push(arg.clone());
            redact_next = true;
            continue;
        }
        redacted.push(arg.clone());
    }
    redacted
}

fn redacted_arg_value(value: &str) -> String {
    format!("<redacted:{}>", value.chars().count())
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
}

fn unique_session_id() -> String {
    // Append 4 random hex chars so two ids minted in the same millisecond do not
    // collide and silently overwrite each other's artifact dir / token file.
    format!(
        "session-{}-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis(),
        crate::random::random_hex(2)
    )
}

fn unique_event_trace_id() -> String {
    format!(
        "event-trace-{}-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis(),
        crate::random::random_hex(2)
    )
}

fn unique_telemetry_id() -> String {
    format!(
        "telemetry-{}-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis(),
        crate::random::random_hex(2)
    )
}

fn safe_artifact_id(value: &str) -> bool {
    !value.is_empty()
        && value != "."
        && value != ".."
        && value
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.'))
}

fn unique_profile_id() -> String {
    format!(
        "profile-{}-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis(),
        crate::random::random_hex(2)
    )
}

fn unique_started_at() -> String {
    format!(
        "{}Z",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs()
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use godot_qa_core::config::ProjectConfig;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn assert_json_path_eq(actual: &Value, expected: &Path) {
        let actual = Path::new(actual.as_str().unwrap());
        if actual.exists() && expected.exists() {
            assert_eq!(fs::canonicalize(actual).unwrap(), fs::canonicalize(expected).unwrap());
        } else {
            assert_eq!(actual, expected);
        }
    }

    #[test]
    fn unique_session_id_has_millis_and_random_hex_suffix() {
        let id = unique_session_id();
        let rest = id.strip_prefix("session-").expect("session- prefix");
        let (millis, suffix) = rest.rsplit_once('-').expect("millis-suffix shape");
        assert!(
            !millis.is_empty() && millis.chars().all(|ch| ch.is_ascii_digit()),
            "millis segment should be digits: {id}"
        );
        assert_eq!(suffix.len(), 4, "expected 4 hex suffix chars: {id}");
        assert!(
            suffix
                .chars()
                .all(|ch| ch.is_ascii_hexdigit() && !ch.is_ascii_uppercase()),
            "suffix should be lowercase hex: {id}"
        );
    }

    #[test]
    fn unique_ids_carry_varying_random_suffixes_in_a_tight_loop() {
        // The random suffix kills same-millisecond collisions: even when ids share
        // the millisecond prefix, generating a batch must surface several distinct
        // suffixes (a constant suffix would prove the randomness was dropped). We
        // assert entropy is present rather than full distinctness because 16 bits
        // of suffix cannot guarantee zero birthday collisions across a large batch.
        let suffix_of = |id: &str| id.rsplit_once('-').unwrap().1.to_string();
        let mut session_suffixes = std::collections::HashSet::new();
        let mut trace_suffixes = std::collections::HashSet::new();
        let mut profile_suffixes = std::collections::HashSet::new();
        for _ in 0..64 {
            session_suffixes.insert(suffix_of(&unique_session_id()));
            trace_suffixes.insert(suffix_of(&unique_event_trace_id()));
            profile_suffixes.insert(suffix_of(&unique_profile_id()));
        }
        assert!(session_suffixes.len() > 1, "session id suffix did not vary");
        assert!(
            trace_suffixes.len() > 1,
            "event-trace id suffix did not vary"
        );
        assert!(profile_suffixes.len() > 1, "profile id suffix did not vary");
    }

    #[test]
    fn build_request_matches_python_contract() {
        let request = build_request(
            SESSION_STATUS,
            Map::new(),
            Some("cmd-001"),
            Some("secret-token"),
        );
        assert_eq!(
            request,
            json!({
                "protocol_version": "0.1",
                "command": "session.status",
                "params": {},
                "token": "secret-token",
                "id": "cmd-001",
            })
        );
    }

    #[test]
    fn visual_audit_targets_request_uses_protocol_constant_and_target_array() {
        let mut params = Map::new();
        params.insert(
            "targets".to_string(),
            json!(["qa:player", "qa:progress_ui"]),
        );
        params.insert("include_state".to_string(), json!(true));
        params.insert(
            "state_methods".to_string(),
            json!(["get_qa_visual_state", "get_qa_readability_state"]),
        );

        let request = build_request(
            VISUAL_AUDIT_TARGETS,
            params,
            Some("cmd-visual-001"),
            Some("secret-token"),
        );

        assert_eq!(VISUAL_AUDIT_TARGETS, "visual.audit_targets");
        assert_eq!(
            request,
            json!({
                "protocol_version": PROTOCOL_VERSION,
                "command": "visual.audit_targets",
                "params": {
                    "targets": ["qa:player", "qa:progress_ui"],
                    "include_state": true,
                    "state_methods": ["get_qa_visual_state", "get_qa_readability_state"],
                },
                "token": "secret-token",
                "id": "cmd-visual-001",
            })
        );
    }

    #[test]
    fn visual_audit_targets_response_shape_is_accepted_by_live_validation() {
        let response = json!({
            "protocol_version": PROTOCOL_VERSION,
            "ok": true,
            "data": {
                "target_count": 1,
                "viewport": {
                    "rect": {"x": 0.0, "y": 0.0, "width": 800.0, "height": 600.0},
                    "size": {"x": 800.0, "y": 600.0, "length": 1000.0},
                    "coordinate_space": "viewport_2d"
                },
                "targets": [
                    {
                        "locator": "qa:player",
                        "resolved": "path:/root/Main/Player",
                        "path": "/root/Main/Player",
                        "visible": true,
                        "inside_viewport": true,
                        "viewport": {
                            "rect": {"x": 0.0, "y": 0.0, "width": 800.0, "height": 600.0},
                            "size": {"x": 800.0, "y": 600.0, "length": 1000.0},
                            "coordinate_space": "viewport_2d"
                        },
                        "visual_evidence": {
                            "coordinate_space": "viewport_2d",
                            "rect": [10.0, 20.0, 30.0, 40.0],
                            "rect_source": "control_global_rect",
                            "target_hidden_supported": true
                        },
                        "state": {
                            "available": true,
                            "methods": {
                                "get_qa_visual_state": {
                                    "available": true,
                                    "value": {"contrast_hint": "readable"}
                                }
                            }
                        }
                    }
                ]
            }
        });

        validate_response(VISUAL_AUDIT_TARGETS, &response).unwrap();
    }

    #[test]
    fn validate_response_rejects_non_object_payload() {
        let failure = validate_response("tree", &json!(["bad-payload"]))
            .unwrap_err()
            .failure;
        assert_eq!(
            serde_json::to_value(failure).unwrap(),
            json!({
                "type": "protocol_error",
                "command": "tree",
                "message": "live session response must be a JSON object",
                "expected": "JSON object response",
                "actual": "list",
            })
        );
    }

    #[test]
    fn session_path_never_falls_back_to_a_nested_sample_project() {
        let root = temp_root("session-no-sample-fallback");
        let config = test_config(&root);
        let sample = root.join("examples/sample_project/.qa/session.json");
        fs::create_dir_all(sample.parent().unwrap()).unwrap();
        fs::write(&sample, "{}").unwrap();
        assert_eq!(session_json_path(&config), root.join(".qa/session.json"));
        assert_eq!(load_session_record(&config, "session status").unwrap_err().kind, "session_not_found");
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn load_session_record_reports_missing_file() {
        let root = temp_root("missing-session");
        let config = test_config(&root);
        let failure = load_session_record(&config, "session status").unwrap_err();
        assert_eq!(
            serde_json::to_value(failure).unwrap(),
            json!({
                "type": "session_not_found",
                "command": "session status",
                "path": session_json_path(&config),
                "message": "live session file was not found",
                "expected": "existing .qa/session.json",
                "actual": "missing",
            })
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn legacy_session_metadata_defaults_to_desktop_target() {
        let value = json!({
            "protocol_version": "0.1",
            "session_id": "legacy",
            "pid": 123,
            "host": "127.0.0.1",
            "port": 1234,
            "scene": "res://main.tscn",
            "viewport": [1920, 1080],
            "artifact_dir": "/tmp/legacy",
            "request_path": "/tmp/legacy/request.json",
            "token_file": "/tmp/legacy/token",
            "started_at": "0Z"
        });
        let record: SessionRecord = serde_json::from_value(value).unwrap();
        assert_eq!(record.target, SessionTarget::Desktop);
        assert!(record.android.is_none());
    }

    #[test]
    fn android_liveness_never_treats_device_pid_as_a_host_pid() {
        let root = temp_root("android-liveness");
        let config = test_config(&root);
        let mut record = test_session_record(&config, &root, 1234);
        record.target = SessionTarget::Android;
        record.pid = std::process::id() as i64;
        record.android = Some(AndroidSessionMetadata {
            device_serial: "device-1".to_string(),
            package: "org.example.qa".to_string(),
            preset: "Android QA".to_string(),
            device_pid: std::process::id() as i64,
            device_port: 32400,
            forwarded_host_port: 32500,
            adb_path: "/sdk/platform-tools/adb".to_string(),
            device_artifact_root: "user://godot_qa/live/session-test".to_string(),
            logcat_pid: None,
            logcat_process_identity: None,
            deployment_log_path: "/tmp/deploy.log".to_string(),
            logcat_log_path: "/tmp/logcat.txt".to_string(),
        });
        assert_eq!(session_liveness(&record), SessionLiveness::Unavailable);
        let _ = fs::remove_dir_all(root);
    }

    #[cfg(unix)]
    #[test]
    fn android_cleanup_is_exact_and_serial_scoped() {
        use std::os::unix::fs::PermissionsExt;

        let root = temp_root("android-cleanup");
        let config = test_config(&root);
        let calls = root.join("adb-calls");
        let adb = root.join("adb");
        fs::write(
            &adb,
            format!(
                "#!/bin/sh\nprintf '%s|' \"$@\" >> '{0}'\nprintf '\\n' >> '{0}'\ncase \"$*\" in *'forward --list'*) printf 'device-1 tcp:32500 tcp:32400\\n';; esac\n",
                calls.display(),
            ),
        )
        .unwrap();
        let mut permissions = fs::metadata(&adb).unwrap().permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&adb, permissions).unwrap();

        let mut record = test_session_record(&config, &root, 1234);
        record.session_id = "session-cleanup".to_string();
        record.target = SessionTarget::Android;
        record.android = Some(AndroidSessionMetadata {
            device_serial: "device-1".to_string(),
            package: "org.example.qa".to_string(),
            preset: "Android QA".to_string(),
            device_pid: 4321,
            device_port: 32400,
            forwarded_host_port: 32500,
            adb_path: adb.to_string_lossy().into_owned(),
            device_artifact_root: "user://godot_qa/live/session-cleanup".to_string(),
            logcat_pid: None,
            logcat_process_identity: None,
            deployment_log_path: root.join("deploy.log").to_string_lossy().into_owned(),
            logcat_log_path: root.join("logcat.txt").to_string_lossy().into_owned(),
        });

        assert!(android_start_cleanup(&record, true, true).is_empty());
        let calls = fs::read_to_string(calls).unwrap();
        assert!(calls.contains("-s|device-1|shell|am|force-stop|org.example.qa|"));
        assert!(calls.contains("-s|device-1|forward|--remove|tcp:32500|"));
        assert!(calls.contains(
            "-s|device-1|shell|run-as|org.example.qa|rm|-rf|files/godot_qa/live/session-cleanup|"
        ));
        assert!(calls.contains(
            "-s|device-1|shell|run-as|org.example.qa|rm|-f|files/godot_qa/android_live_bootstrap.json|"
        ));
        assert!(!calls.contains("forward|--remove-all|"));
        let _ = fs::remove_dir_all(root);
    }

    #[cfg(unix)]
    #[test]
    fn failed_android_cleanup_persists_recovery_metadata() {
        use std::os::unix::fs::PermissionsExt;

        let root = temp_root("android-cleanup-recovery");
        let config = test_config(&root);
        let adb = root.join("adb");
        fs::write(
            &adb,
            "#!/bin/sh\nprintf 'error: device offline\\n' >&2\nexit 1\n",
        )
        .unwrap();
        let mut permissions = fs::metadata(&adb).unwrap().permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&adb, permissions).unwrap();

        let mut record = test_session_record(&config, &root, 1234);
        record.session_id = "session-recovery".to_string();
        record.target = SessionTarget::Android;
        record.android = Some(AndroidSessionMetadata {
            device_serial: "offline-device".to_string(),
            package: "org.example.qa".to_string(),
            preset: "Android QA".to_string(),
            device_pid: 4321,
            device_port: 32400,
            forwarded_host_port: 32500,
            adb_path: adb.to_string_lossy().into_owned(),
            device_artifact_root: "user://godot_qa/live/session-recovery".to_string(),
            logcat_pid: None,
            logcat_process_identity: None,
            deployment_log_path: root.join("deploy.log").to_string_lossy().into_owned(),
            logcat_log_path: root.join("logcat.txt").to_string_lossy().into_owned(),
        });

        let result = android_start_failure_after_cleanup(
            &config,
            FailureDetail {
                kind: "android_probe_failed".to_string(),
                ..FailureDetail::default()
            },
            Map::new(),
            &record,
            "recovery-token",
            true,
            true,
        );
        assert!(!result.ok);
        assert_eq!(result.data.get("metadata_retained"), Some(&json!(true)));
        assert!(session_json_path(&config).is_file());
        assert_eq!(
            fs::read_to_string(session_token_path(&config)).unwrap(),
            "recovery-token"
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn android_start_rejects_unsupported_options_before_backend_dispatch() {
        let root = temp_root("android-option-rejection");
        let config = test_config(&root);
        let project_args = Vec::new();
        let base = SessionStartRequest {
            target: SessionTarget::Android,
            scene: None,
            viewport: Some("800x600"),
            headless: false,
            audio: true,
            no_audio: false,
            project_args: &project_args,
            frame_pacing_request: None,
            fastest: false,
            hot_reload: false,
            preset: Some("Android QA"),
            device: None,
            package: None,
            no_apk_cache: false,
        };
        let result = session_start_target_result(&config, base.clone());
        assert!(!result.ok);
        assert_eq!(result.failures[0].kind, "android_option_unsupported");
        assert_eq!(result.failures[0].actual, Some(json!("800x600")));

        let missing_preset = session_start_target_result(
            &config,
            SessionStartRequest {
                viewport: None,
                preset: None,
                ..base
            },
        );
        assert_eq!(missing_preset.failures[0].kind, "android_preset_required");
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn android_live_scene_paths_are_packaged_and_traversal_free() {
        assert!(is_valid_android_live_scene("res://ui/main.tscn"));
        assert!(is_valid_android_live_scene("res://ui/main.scn"));
        for invalid in [
            "",
            "/tmp/main.tscn",
            "user://main.tscn",
            "res://../main.tscn",
            "res://ui\\main.tscn",
            "res://ui/main.gd",
        ] {
            assert!(!is_valid_android_live_scene(invalid), "{invalid}");
        }
    }

    #[test]
    fn load_session_record_prefers_primary_session_file() {
        let root = temp_root("primary-session-wins");
        let artifact_root = root.join("external-artifacts/live/artifact-session");
        let config = test_config_with_artifacts(&root, root.join("external-artifacts"));
        let mut primary = test_session_record(&config, &root, 1234);
        primary.session_id = "primary-session".to_string();
        let mut artifact = primary.clone();
        artifact.session_id = "artifact-session".to_string();
        artifact.artifact_dir = artifact_root.to_string_lossy().into_owned();
        artifact.request_path = artifact_root
            .join("request.json")
            .to_string_lossy()
            .into_owned();

        write_test_session_record(&session_json_path(&config), &primary);
        write_test_session_record(&artifact_root.join("session.json"), &artifact);

        let loaded = load_session_record(&config, "session status").unwrap();

        assert_eq!(loaded.session_id, "primary-session");
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn load_session_record_uses_direct_artifact_session_root() {
        let root = temp_root("direct-artifact-session");
        let artifact_root = root.join("external-artifacts/live/session-direct");
        let config = test_config_with_artifacts(&root, artifact_root.clone());
        let mut record = test_session_record(&config, &root, 1234);
        record.session_id = "session-direct".to_string();
        record.artifact_dir = artifact_root.to_string_lossy().into_owned();
        record.request_path = artifact_root
            .join("request.json")
            .to_string_lossy()
            .into_owned();
        write_test_session_record(&artifact_root.join("session.json"), &record);

        let loaded = load_session_record(&config, "session reload").unwrap();

        assert_eq!(loaded.session_id, "session-direct");
        assert_eq!(loaded.artifact_dir, artifact_root.to_string_lossy());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn load_session_record_scans_default_artifacts_when_token_still_exists() {
        let root = temp_root("default-artifact-session");
        let config = test_config(&root);
        let artifact_root = root.join(".qa/artifacts/live/session-default");
        let mut record = test_session_record(&config, &root, 1234);
        record.session_id = "session-default".to_string();
        record.artifact_dir = artifact_root.to_string_lossy().into_owned();
        record.request_path = artifact_root
            .join("request.json")
            .to_string_lossy()
            .into_owned();
        fs::write(session_token_path(&config), "token-test").unwrap();
        write_test_session_record(&artifact_root.join("session.json"), &record);

        let loaded = load_session_record(&config, "session reload").unwrap();

        assert_eq!(loaded.session_id, "session-default");
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn load_session_record_scans_artifacts_root_for_newest_live_session() {
        let root = temp_root("artifact-root-newest");
        let artifacts_root = root.join("external-artifacts");
        let config = test_config_with_artifacts(&root, artifacts_root.clone());
        let old_root = artifacts_root.join("live/session-old");
        let new_root = artifacts_root.join("live/session-new");
        let mut old = test_session_record(&config, &root, 1234);
        old.session_id = "session-old".to_string();
        old.started_at = "2026-05-01T00:00:00Z".to_string();
        old.artifact_dir = old_root.to_string_lossy().into_owned();
        old.request_path = old_root.join("request.json").to_string_lossy().into_owned();
        let mut new = old.clone();
        new.session_id = "session-new".to_string();
        new.started_at = "2026-05-02T00:00:00Z".to_string();
        new.artifact_dir = new_root.to_string_lossy().into_owned();
        new.request_path = new_root.join("request.json").to_string_lossy().into_owned();
        write_test_session_record(&old_root.join("session.json"), &old);
        write_test_session_record(&new_root.join("session.json"), &new);

        let loaded = load_session_record(&config, "session list").unwrap();

        assert_eq!(loaded.session_id, "session-new");
        assert_eq!(loaded.artifact_dir, new_root.to_string_lossy());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn load_session_record_recovers_legacy_artifact_request() {
        let root = temp_root("legacy-artifact-session");
        let artifact_root = root.join("external-artifacts/live/session-legacy");
        let config = test_config_with_artifacts(&root, artifact_root.clone());
        fs::create_dir_all(&artifact_root).unwrap();
        fs::write(
            artifact_root.join("request.json"),
            serde_json::to_string_pretty(&json!({
                "session": {
                    "id": "session-legacy",
                    "started_at": "2026-05-03T00:00:00Z",
                    "host": "127.0.0.1",
                    "port": 4567,
                    "scene": "res://legacy.tscn",
                    "viewport": [1024, 768],
                    "headless": false,
                    "project_args": ["--seed", "123"]
                },
                "artifacts": {
                    "root": artifact_root,
                    "request": artifact_root.join("request.json")
                }
            }))
            .unwrap(),
        )
        .unwrap();
        fs::write(
            artifact_root.join("meta.json"),
            serde_json::to_string_pretty(&json!({
                "session_id": "session-legacy",
                "scene": "res://legacy.tscn"
            }))
            .unwrap(),
        )
        .unwrap();

        let loaded = load_session_record(&config, "session reload").unwrap();

        assert_eq!(loaded.session_id, "session-legacy");
        assert_eq!(loaded.pid, 0);
        assert_eq!(loaded.port, 4567);
        assert_eq!(loaded.scene, "res://legacy.tscn");
        assert_eq!(
            loaded.token_file,
            session_token_path(&config).to_string_lossy()
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn load_session_record_reports_artifact_lookup_paths_when_override_missing() {
        let root = temp_root("artifact-session-missing");
        let artifact_root = root.join("external-artifacts");
        let config = test_config_with_artifacts(&root, artifact_root.clone());

        let failure = load_session_record(&config, "session reload").unwrap_err();

        assert_eq!(failure.kind, "session_not_found");
        assert_eq!(
            failure.path.as_deref(),
            Some(artifact_root.to_str().unwrap())
        );
        assert_eq!(
            failure.expected,
            Some(json!(
                "existing .qa/session.json or live session metadata under --artifact-dir"
            ))
        );
        assert_eq!(
            failure.actual,
            Some(json!({
                "primary": session_json_path(&config),
                "artifact_dir": artifact_root,
            }))
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn persist_session_writes_recoverable_artifact_session_copy() {
        let root = temp_root("persist-artifact-session");
        let config = test_config(&root);
        let record = test_session_record(&config, &root, 1234);

        persist_session(&config, &record, "token-test").unwrap();

        assert!(session_json_path(&config).exists());
        assert!(
            Path::new(&record.artifact_dir)
                .join("session.json")
                .exists()
        );
        assert_eq!(
            fs::read_to_string(session_token_path(&config)).unwrap(),
            "token-test"
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn prepare_live_session_records_headless_flag() {
        let root = temp_root("prepare-headless");
        let config = test_config(&root);

        let prepared = prepare_live_session(
            &config,
            "res://ui/main_menu.tscn",
            vec![1280, 720],
            true,
            &["--token".to_string(), "abcdef".to_string()],
            None,
        )
        .unwrap();

        assert!(prepared.record.headless);
        assert_eq!(
            prepared.record.project_args,
            vec!["--token".to_string(), "<redacted:6>".to_string()]
        );
        let request: Value =
            serde_json::from_str(&fs::read_to_string(&prepared.request_path).unwrap()).unwrap();
        assert_eq!(request["session"]["headless"], true);
        assert_eq!(
            request["session"]["project_args"],
            json!(["--token", "<redacted:6>"])
        );
        assert_json_path_eq(
            &request["artifacts"]["screenshots_dir"],
            &root
                .join(".qa/artifacts/live")
                .join(&prepared.record.session_id)
                .join("screenshots"),
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn android_bootstrap_keeps_token_out_of_host_request_artifacts() {
        let root = temp_root("android-bootstrap-secrecy");
        let config = test_config(&root);
        let project_args = vec![
            "--qa-user=robot".to_string(),
            "--password=secret".to_string(),
        ];
        let prepared = prepare_live_session(
            &config,
            "res://main.tscn",
            vec![0, 0],
            false,
            &project_args,
            None,
        )
        .unwrap();
        let request = fs::read_to_string(&prepared.request_path).unwrap();
        let meta = fs::read_to_string(
            Path::new(&prepared.record.artifact_dir).join("meta.json"),
        )
        .unwrap();
        assert!(!request.contains(&prepared.token));
        assert!(!meta.contains(&prepared.token));
        assert!(!request.contains("--password=secret"));
        assert!(!meta.contains("--password=secret"));

        let bootstrap_bytes =
            android_bootstrap_bytes(&prepared, &project_args, "res://main.tscn", None).unwrap();
        let bootstrap: Value = serde_json::from_slice(&bootstrap_bytes).unwrap();
        assert_eq!(bootstrap["version"], json!(1));
        assert_eq!(bootstrap["session"]["token"], json!(prepared.token));
        assert_eq!(bootstrap["session"]["project_args"], json!(project_args));
        let keys = bootstrap.as_object().unwrap();
        assert_eq!(keys.len(), 2);
        assert!(keys.contains_key("version"));
        assert!(keys.contains_key("session"));
        // A `None` mode omits the field and is byte-identical to the historical payload.
        assert!(!bootstrap["session"].as_object().unwrap().contains_key("mode"));
        assert_eq!(
            bootstrap_bytes,
            serde_json::to_vec(&json!({
                "version": 1,
                "session": {
                    "id": prepared.record.session_id,
                    "token": prepared.token,
                    "port": prepared.record.port,
                    "scene": "res://main.tscn",
                    "started_at": prepared.record.started_at,
                    "project_args": project_args,
                }
            }))
            .unwrap()
        );

        // `Some(mode)` adds a `mode` key without disturbing the rest of the payload.
        let scenario_bytes =
            android_bootstrap_bytes(&prepared, &project_args, "", Some("scenario")).unwrap();
        let scenario: Value = serde_json::from_slice(&scenario_bytes).unwrap();
        assert_eq!(scenario["session"]["mode"], json!("scenario"));
        assert_eq!(scenario["session"]["scene"], json!(""));
        assert_eq!(scenario["session"]["token"], json!(prepared.token));
        let _ = fs::remove_dir_all(root);
    }

    #[cfg(unix)]
    struct DeployFixture {
        base: PathBuf,
        config: ProjectConfig,
        prepared: PreparedSession,
        adb: PathBuf,
        godot: PathBuf,
        calls: PathBuf,
        deployment_log: PathBuf,
        logcat_log: PathBuf,
        apk_path: PathBuf,
        device: godot_qa_process::android::AndroidDevice,
        preset: godot_qa_process::android::AndroidExportPreset,
    }

    // A fake adb (logging every invocation, answering `devices -l`/`pidof`/`forward --list`
    // and consuming the bootstrap stdin) plus a fake exporting Godot and a prepared
    // sceneless session, wired for the full Android deploy stage sequence.
    #[cfg(unix)]
    fn deploy_fixture(label: &str) -> DeployFixture {
        let base = temp_root(label);
        let project = base.join("project");
        fs::create_dir_all(project.join("scenes")).unwrap();
        fs::write(project.join("project.godot"), b"config_version=5\n").unwrap();
        fs::write(project.join("scenes/main.tscn"), b"[gd_scene]\n").unwrap();
        let config = ProjectConfig {
            project_root: project.clone(),
            artifacts_dir: project.join(".qa/artifacts"),
            ..test_config(&base)
        };
        let prepared = prepare_live_session(&config, "", vec![0, 0], false, &[], None).unwrap();
        let port = prepared.record.port;
        let calls = base.join("adb-calls");
        // The logcat sidecar must stay alive (like real `adb logcat`) so its cmdline
        // still names `logcat` when stop_logcat verifies process ownership.
        let adb = write_executable(
            &base.join("adb"),
            &format!(
                "#!/bin/sh\ncase \"$*\" in *logcat*) sleep 5; exit 0;; esac\nprintf '%s|' \"$@\" >> '{calls}'\nprintf '\\n' >> '{calls}'\ncase \"$*\" in *'devices -l'*) printf 'List of devices attached\\ndevice-1 device\\n';; *'forward --list'*) printf 'device-1 tcp:{port} tcp:{port}\\n';; *pidof*) printf '4321\\n';; *umask*) cat > /dev/null;; esac\n",
                calls = calls.display(),
                port = port,
            ),
        );
        let godot = working_fake_godot(&base, &base.join("export-marker.log"), "FRESH-APK");
        let artifact_root = PathBuf::from(&prepared.record.artifact_dir);
        DeployFixture {
            deployment_log: artifact_root.join("deployment/deployment.log"),
            logcat_log: artifact_root.join("logs/logcat.txt"),
            apk_path: artifact_root.join("deployment/session-debug.apk"),
            device: godot_qa_process::android::AndroidDevice {
                serial: "device-1".to_string(),
                state: "device".to_string(),
                description: std::collections::BTreeMap::new(),
            },
            preset: godot_qa_process::android::AndroidExportPreset {
                name: "Android QA".to_string(),
                package: "org.example.qa".to_string(),
                export_path: None,
            },
            adb,
            godot,
            calls,
            config,
            prepared,
            base,
        }
    }

    #[cfg(unix)]
    fn deploy_request<'a>(fx: &'a DeployFixture) -> AndroidDeployRequest<'a> {
        AndroidDeployRequest {
            config: &fx.config,
            godot: fx.godot.to_str().unwrap(),
            adb: fx.adb.to_str().unwrap(),
            device: &fx.device,
            preset: &fx.preset,
            prepared: &fx.prepared,
            bootstrap_scene: "",
            bootstrap_mode: Some("scenario"),
            no_apk_cache: false,
            effective_project_args: &[],
            deployment_log: &fx.deployment_log,
            logcat_log: &fx.logcat_log,
            apk_path: &fx.apk_path,
        }
    }

    #[cfg(unix)]
    #[test]
    fn android_deploy_stages_run_in_order_and_fill_the_record() {
        let fx = deploy_fixture("deploy-order");
        let request = deploy_request(&fx);
        let outcome = android_deploy_stages(&request, &|_record, _token| {
            Ok(json!({"ok": true, "data": {"viewport": [1080, 1920]}}))
        })
        .expect("deploy succeeds with an Ok probe");

        assert_eq!(outcome.record.target, SessionTarget::Android);
        let android = outcome.record.android.as_ref().unwrap();
        assert_eq!(android.device_pid, 4321);
        assert!(android.logcat_pid.is_some());
        assert_eq!(outcome.record.viewport, vec![1080, 1920]);
        assert_eq!(outcome.apk_cache.status, "miss");

        let calls = fs::read_to_string(&fx.calls).expect("calls");
        let index = |needle: &str| {
            calls
                .find(needle)
                .unwrap_or_else(|| panic!("missing {needle} in:\n{calls}"))
        };
        // Ordered stages: revalidate -> install -> bootstrap -> forward -> launch -> pid.
        assert!(index("devices|-l|") < index("install|-r|"));
        assert!(index("install|-r|") < index("umask 077"));
        assert!(index("umask 077") < index("forward|--no-rebind|"));
        assert!(index("forward|--no-rebind|") < index("am|force-stop|"));
        assert!(index("am|force-stop|") < index("am|start|"));
        assert!(index("am|start|") < index("pidof|"));
        let _ = fs::remove_dir_all(&fx.base);
    }

    #[cfg(unix)]
    #[test]
    fn android_deploy_probe_failure_reports_cleanup_state_and_runs_scoped_cleanup() {
        let fx = deploy_fixture("deploy-probe-fail");
        let request = deploy_request(&fx);
        let error = android_deploy_stages(&request, &|_record, _token| {
            Err(FailureDetail {
                kind: "android_probe_failed".to_string(),
                ..FailureDetail::default()
            })
        })
        .expect_err("an Err probe fails the deploy");

        assert_eq!(error.failure.kind, "android_probe_failed");
        assert!(error.forwarded, "the port was forwarded before the probe");
        assert!(error.launched, "the app was launched before the probe");
        assert!(error.needs_cleanup, "a post-bootstrap failure must be cleaned up");
        assert!(error.record.android.as_ref().unwrap().logcat_pid.is_some());

        // Honor forwarded/launched: force-stop, exact forward remove, private session +
        // bootstrap removal all issue against the same device.
        fs::write(&fx.calls, b"").unwrap();
        assert!(android_start_cleanup(&error.record, error.forwarded, error.launched).is_empty());
        let calls = fs::read_to_string(&fx.calls).expect("calls");
        assert!(calls.contains("am|force-stop|org.example.qa|"), "{calls}");
        assert!(calls.contains("forward|--remove|"), "{calls}");
        assert!(
            calls.contains("run-as|org.example.qa|rm|-rf|files/godot_qa/live/"),
            "{calls}"
        );
        assert!(
            calls.contains(
                "run-as|org.example.qa|rm|-f|files/godot_qa/android_live_bootstrap.json|"
            ),
            "{calls}"
        );
        let _ = fs::remove_dir_all(&fx.base);
    }

    #[cfg(unix)]
    #[test]
    fn android_scenario_session_carries_the_token_in_memory_without_a_singleton() {
        let fx = deploy_fixture("scenario-session-start");
        let request = AndroidScenarioSessionRequest {
            preset: "Android QA",
            device: Some("device-1"),
            package: None,
            project_args: &[],
            no_apk_cache: false,
            deployment_log: fx.deployment_log.clone(),
            logcat_log: fx.logcat_log.clone(),
        };
        let latest_before = fs::read_link(fx.config.artifacts_dir.join("live/latest")).ok();
        assert!(latest_before.is_some(), "fixture prepared a latest link");
        let handle = deploy_android_scenario_session(
            &fx.config,
            &request,
            fx.godot.to_str().unwrap(),
            fx.adb.to_str().unwrap(),
            &fx.device,
            &fx.preset,
            &|_record, _token| Ok(json!({"ok": true, "data": {"viewport": [1080, 1920]}})),
        )
        .expect("scenario session deploys");

        assert_eq!(handle.record.target, SessionTarget::Android);
        let android = handle.record.android.as_ref().expect("android metadata");
        assert_eq!(android.package, "org.example.qa");
        assert_eq!(android.device_pid, 4321);
        // The token is carried in memory so pooled_session_command can authenticate.
        assert!(!handle.token.is_empty());
        assert_eq!(
            handle.record.token_file,
            Path::new(&handle.record.artifact_dir)
                .join("session.token")
                .to_string_lossy()
        );
        // Self-described inside its own artifact dir, never the singleton .qa/session.json.
        assert!(
            !session_json_path(&fx.config).exists(),
            "singleton session file must not be written"
        );
        assert!(
            Path::new(&handle.record.artifact_dir)
                .join("session.json")
                .is_file()
        );
        // Pooled sessions never rotate the shared `live/latest` convenience link:
        // K Android lanes would only fight over a link nothing resolves them through.
        assert_eq!(
            fs::read_link(fx.config.artifacts_dir.join("live/latest")).ok(),
            latest_before,
            "pooled android session must not rotate live/latest"
        );
        let _ = fs::remove_dir_all(&fx.base);
    }

    #[cfg(unix)]
    #[test]
    fn stop_android_scenario_session_issues_exact_scoped_cleanup() {
        use std::os::unix::fs::PermissionsExt;

        let root = temp_root("scenario-session-stop");
        let config = test_config(&root);
        let calls = root.join("adb-calls");
        let adb = root.join("adb");
        fs::write(
            &adb,
            format!(
                "#!/bin/sh\ncase \"$*\" in *logcat*) exit 0;; esac\nprintf '%s|' \"$@\" >> '{0}'\nprintf '\\n' >> '{0}'\ncase \"$*\" in *'forward --list'*) printf 'device-1 tcp:32500 tcp:32400\\n';; esac\n",
                calls.display(),
            ),
        )
        .unwrap();
        let mut permissions = fs::metadata(&adb).unwrap().permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&adb, permissions).unwrap();

        let mut record = test_session_record(&config, &root, 32500);
        record.session_id = "scenario-stop".to_string();
        record.target = SessionTarget::Android;
        record.android = Some(AndroidSessionMetadata {
            device_serial: "device-1".to_string(),
            package: "org.example.qa".to_string(),
            preset: "Android QA".to_string(),
            device_pid: 4321,
            device_port: 32400,
            forwarded_host_port: 32500,
            adb_path: adb.to_string_lossy().into_owned(),
            device_artifact_root: "user://godot_qa/live/scenario-stop".to_string(),
            logcat_pid: None,
            logcat_process_identity: None,
            deployment_log_path: root.join("deploy.log").to_string_lossy().into_owned(),
            logcat_log_path: root.join("logcat.txt").to_string_lossy().into_owned(),
        });
        let handle = PooledSessionHandle {
            record,
            token: "scenario-token".to_string(),
        };

        let failures = stop_android_scenario_session(&config, &handle);
        assert!(failures.is_empty(), "{failures:?}");
        let calls = fs::read_to_string(calls).expect("calls");
        assert!(calls.contains("am|force-stop|org.example.qa|"), "{calls}");
        assert!(calls.contains("forward|--remove|tcp:32500|"), "{calls}");
        assert!(
            calls.contains("run-as|org.example.qa|rm|-rf|files/godot_qa/live/scenario-stop|"),
            "{calls}"
        );
        assert!(
            calls.contains(
                "run-as|org.example.qa|rm|-f|files/godot_qa/android_live_bootstrap.json|"
            ),
            "{calls}"
        );
        let _ = fs::remove_dir_all(root);
    }

    #[cfg(unix)]
    #[test]
    fn android_transport_round_trips_scalar_edit_set_property_unchanged() {
        use std::os::unix::fs::PermissionsExt;

        let root = temp_root("android-edit-set-property");
        let config = test_config(&root);

        // A fake adb that answers the `pidof` liveness query with the recorded device pid
        // and logs every invocation. A scalar edit round-trip must never shell out to it:
        // there are no artifact paths to pull and no file-transport fallback on Android.
        let calls = root.join("adb-calls");
        let adb = root.join("adb");
        fs::write(
            &adb,
            format!(
                "#!/bin/sh\nprintf '%s|' \"$@\" >> '{0}'\nprintf '\\n' >> '{0}'\ncase \"$*\" in *pidof*) printf '4321\\n';; esac\n",
                calls.display(),
            ),
        )
        .unwrap();
        let mut permissions = fs::metadata(&adb).unwrap().permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&adb, permissions).unwrap();

        // A desktop-identical `edit.set_property` ok response: the live-edit ledger entry
        // documented in docs/runtime-bridge-protocol.md. Every value is a scalar, so the
        // Android transport must return it byte-for-byte.
        let data = json!({
            "sequence": 1,
            "op": "set_property",
            "target": "qa:start_button",
            "resolved": "path:/root/MainMenu/MenuButtons/StartButton",
            "scene": "res://ui/main_menu.tscn",
            "property": "visible",
            "before": true,
            "after": false,
            "exportable": true,
        });
        let responses = vec![json!({
            "protocol_version": PROTOCOL_VERSION,
            "ok": true,
            "data": data.clone(),
        })];
        let (port, received, handle) = spawn_profile_server(responses);

        let mut record = test_session_record(&config, &root, port);
        record.target = SessionTarget::Android;
        record.android = Some(AndroidSessionMetadata {
            device_serial: "device-1".to_string(),
            package: "org.example.qa".to_string(),
            preset: "Android QA".to_string(),
            device_pid: 4321,
            device_port: 32400,
            forwarded_host_port: port,
            adb_path: adb.to_string_lossy().into_owned(),
            device_artifact_root: "user://godot_qa/live/session-test".to_string(),
            logcat_pid: None,
            logcat_process_identity: None,
            deployment_log_path: root.join("deploy.log").to_string_lossy().into_owned(),
            logcat_log_path: root.join("logcat.txt").to_string_lossy().into_owned(),
        });
        persist_session(&config, &record, "token-test").unwrap();

        // Params mirror the CLI mapping in crates/godot-qa-scenario/src/edit.rs::run_edit_live_set.
        let params = Map::from_iter([
            ("target".to_string(), json!("qa:start_button")),
            ("property".to_string(), json!("visible")),
            ("value_input".to_string(), json!("false")),
        ]);
        let result = run_live_protocol_command(&config, "edit-live set", EDIT_SET_PROPERTY, params);

        handle.join().unwrap();
        assert!(result.ok, "{:?}", result.failures);
        // The forwarded TCP bridge answered exactly once: no file-transport fallback.
        assert_eq!(received.load(std::sync::atomic::Ordering::SeqCst), 1);
        // Scalar edit params/results pass through the Android transport unchanged, so the
        // data is identical to the desktop response shape.
        assert_eq!(Value::Object(result.data), data);
        // The scalar round-trip never shells out to adb: no artifact pulls, no liveness probe.
        assert!(
            !calls.exists(),
            "Android transport must not invoke adb for a scalar edit round-trip"
        );

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn resolve_live_frame_pacing_uses_explicit_fixed_fps() {
        let root = temp_root("live-frame-pacing-explicit");
        let config = test_config(&root);

        let frame_pacing = resolve_live_frame_pacing(&config, Some(Some(120)), "session start")
            .unwrap()
            .unwrap();

        assert_eq!(frame_pacing.fixed_fps, 120);
        assert_eq!(frame_pacing.source, "session");
        assert_eq!(frame_pacing.project_setting, None);
        assert_eq!(
            frame_pacing.to_json()["generated_godot_args"],
            json!(["--fixed-fps", "120", "--disable-vsync"])
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn resolve_live_frame_pacing_derives_from_project_settings() {
        let root = temp_root("live-frame-pacing-project");
        fs::write(
            root.join("project.godot"),
            "[application]\nrun/max_fps=144\n\n[physics]\ncommon/physics_ticks_per_second=60\n",
        )
        .unwrap();
        let config = test_config(&root);

        let frame_pacing = resolve_live_frame_pacing(&config, Some(None), "session start")
            .unwrap()
            .unwrap();

        assert_eq!(frame_pacing.fixed_fps, 144);
        assert_eq!(frame_pacing.source, "project_setting");
        assert_eq!(
            frame_pacing.project_setting.as_deref(),
            Some(PROJECT_MAX_FPS_KEY)
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn resolve_live_frame_pacing_rejects_invalid_fixed_fps() {
        let root = temp_root("live-frame-pacing-invalid");
        let config = test_config(&root);

        let failure =
            resolve_live_frame_pacing(&config, Some(Some(0)), "session start").unwrap_err();

        assert_eq!(failure.kind, "invalid_params");
        assert_eq!(failure.actual, Some(json!(0)));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn prepare_live_session_records_frame_pacing_metadata() {
        let root = temp_root("prepare-frame-pacing");
        let config = test_config(&root);
        let frame_pacing = SessionFramePacing {
            fixed_fps: 60,
            source: "session".to_string(),
            project_setting: None,
        };

        let prepared = prepare_live_session(
            &config,
            "res://ui/main_menu.tscn",
            vec![1280, 720],
            true,
            &[],
            Some(frame_pacing.clone()),
        )
        .unwrap();

        assert_eq!(prepared.record.frame_pacing, Some(frame_pacing));
        let request: Value =
            serde_json::from_str(&fs::read_to_string(&prepared.request_path).unwrap()).unwrap();
        assert_eq!(request["session"]["frame_pacing"]["mode"], json!("fastest"));
        assert_eq!(request["session"]["frame_pacing"]["fixed_fps"], json!(60));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn live_screenshot_command_rejects_headless_session_before_connecting() {
        let root = temp_root("headless-screenshot-command");
        let config = test_config(&root);
        let record = SessionRecord {
            process_identity: None,
            protocol_version: PROTOCOL_VERSION.to_string(),
            session_id: "session-test".to_string(),
            pid: 999_999,
            host: "127.0.0.1".to_string(),
            port: 9,
            scene: "res://ui/main_menu.tscn".to_string(),
            viewport: vec![1280, 720],
            artifact_dir: root
                .join(".qa/artifacts/live/session-test")
                .to_string_lossy()
                .into_owned(),
            request_path: root
                .join(".qa/artifacts/live/session-test/request.json")
                .to_string_lossy()
                .into_owned(),
            token_file: session_token_path(&config).to_string_lossy().into_owned(),
            started_at: "0Z".to_string(),
            headless: true,
            project_args: Vec::new(),
            frame_pacing: None,
            target: SessionTarget::Desktop,
            android: None,
        };
        fs::write(
            session_json_path(&config),
            serde_json::to_string_pretty(&record).unwrap(),
        )
        .unwrap();

        let result = run_live_screenshot_protocol_command(
            &config,
            "snapshot",
            SNAPSHOT_CAPTURE,
            Map::from_iter([("name".to_string(), json!("current"))]),
        );

        assert!(!result.ok);
        assert_eq!(result.failures[0].kind, "headless_screenshot_unsupported");
        assert_eq!(
            result.failures[0].actual,
            Some(json!({
                "session_id": "session-test",
                "headless": true,
            }))
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn parse_project_main_scene_reads_application_setting() {
        assert_eq!(
            parse_project_main_scene(
                r#"
[application]
config/name="demo"
run/main_scene="res://ui/main_menu.tscn"

[display]
window/size/viewport_width=1280
"#
            )
            .as_deref(),
            Some("res://ui/main_menu.tscn")
        );
        assert_eq!(
            parse_project_main_scene(
                r#"
[display]
run/main_scene="res://wrong.tscn"

[application]
config/name="demo"
"#
            ),
            None
        );
    }

    #[test]
    fn live_audio_prepare_session_records_audio_dir_and_latest_path() {
        let root = temp_root("live-audio-prepare");
        let config = test_config(&root);
        let prepared =
            prepare_live_session(&config, "res://main.tscn", vec![800, 600], true, &[], None)
                .unwrap();
        let artifact_root = PathBuf::from(&prepared.record.artifact_dir);

        assert!(artifact_root.join("audio").is_dir());
        let meta: Value =
            serde_json::from_str(&fs::read_to_string(artifact_root.join("meta.json")).unwrap())
                .unwrap();
        assert_eq!(meta["audio_dir"], json!(artifact_root.join("audio")));
        let request: Value =
            serde_json::from_str(&fs::read_to_string(artifact_root.join("request.json")).unwrap())
                .unwrap();
        assert_eq!(
            request["artifacts"]["audio_dir"],
            json!(artifact_root.join("audio"))
        );
        assert!(config.artifacts_dir.join("live/latest/audio").is_dir());
    }

    #[test]
    fn live_audio_prepare_session_links_latest_only_under_project_artifacts() {
        let root = temp_root("live-audio-prepare-project-latest");
        let project_root = root.join("examples/sample_project");
        fs::create_dir_all(&project_root).unwrap();
        let config = test_config_with_artifacts(&root, project_root.join(".qa/artifacts"));
        let prepared =
            prepare_live_session(&config, "res://main.tscn", vec![800, 600], true, &[], None)
                .unwrap();
        let artifact_root = PathBuf::from(&prepared.record.artifact_dir);

        assert!(config.artifacts_dir.join("live/latest/audio").is_dir());
        assert_eq!(
            config
                .artifacts_dir
                .join("live/latest")
                .canonicalize()
                .unwrap(),
            artifact_root.canonicalize().unwrap()
        );
        // The link must not be mirrored into the tool repo (config.repo_root == root here).
        assert!(!root.join(".qa/artifacts/live").exists());
    }

    #[test]
    fn live_audio_validate_wav_path_rejects_escape() {
        let root = temp_root("live-audio-security");
        let audio_dir = root.join(".qa/artifacts/live/session/audio");
        fs::create_dir_all(&audio_dir).unwrap();
        let outside_dir = root.join("outside");
        fs::create_dir_all(&outside_dir).unwrap();
        let outside_wav = outside_dir.join("clip.wav");
        fs::write(
            &outside_wav,
            wav_pcm16_for_runtime_audio(8_000, &[0.0, 0.1]),
        )
        .unwrap();
        let requested = audio_dir.join("clip.wav");

        let failure =
            validate_live_audio_wav_path("audio capture", &audio_dir, &requested, &outside_wav)
                .unwrap_err();

        assert_eq!(failure.kind, "invalid_params");
    }

    #[test]
    fn live_audio_analysis_writes_shared_artifact() {
        let root = temp_root("live-audio-analysis");
        let audio_dir = root.join(".qa/artifacts/live/session/audio");
        fs::create_dir_all(&audio_dir).unwrap();
        let wav = audio_dir.join("clip.wav");
        fs::write(
            &wav,
            wav_pcm16_for_runtime_audio(8_000, &[0.0, 0.2, -0.2, 0.0]),
        )
        .unwrap();
        let analysis =
            analyze_wav_file_with_options(&wav, AudioAnalysisOptions::default()).unwrap();
        let analysis_path = wav.with_extension("analysis.json");
        write_audio_analysis_artifact(&analysis_path, &analysis).unwrap();

        assert!(analysis_path.is_file());
        assert_eq!(analysis["sample_rate"], 8_000);
    }

    #[test]
    fn live_audio_bridge_failure_maps_to_structured_capture_warning() {
        let failure = FailureDetail {
            kind: "audio_capture.unavailable".to_string(),
            message: Some("mixed-bus capture is unavailable".to_string()),
            ..FailureDetail::default()
        };

        let result = live_audio_unavailable_result("audio capture", failure);

        assert!(!result.ok);
        assert_eq!(result.exit_code, 1);
        assert_eq!(result.failures[0].kind, "audio_capture.unavailable");
        assert_eq!(
            result.data["warnings"][0]["id"],
            json!("audio_capture.unavailable")
        );
    }

    #[test]
    fn live_audio_capture_bridge_busy_retries() {
        let root = temp_root("live-audio-busy-retry");
        let config = test_config(&root);
        let record = test_session_record(&config, &root, 0);
        let audio_dir = PathBuf::from(&record.artifact_dir).join("audio");
        fs::create_dir_all(&audio_dir).unwrap();
        let wav = audio_dir.join("capture.wav");
        fs::write(&wav, wav_pcm16_for_runtime_audio(8_000, &[0.0, 0.2, -0.2])).unwrap();
        let responses = vec![
            json!({
                "protocol_version": PROTOCOL_VERSION,
                "ok": false,
                "error": {
                    "type": "session_bridge_busy",
                    "message": "QA bridge is already processing a live command",
                    "data": {
                        "in_progress": {
                            "command": AUDIO_CAPTURE,
                            "duration_seconds": 0.05
                        }
                    }
                }
            }),
            json!({
                "protocol_version": PROTOCOL_VERSION,
                "ok": true,
                "data": {
                    "name": "capture",
                    "effective_duration_seconds": 0.05,
                    "artifacts": {"wav": wav},
                    "warnings": []
                }
            }),
        ];
        let (port, received, handle) = spawn_profile_server(responses);
        let mut record = record;
        record.port = port;
        persist_session(&config, &record, "token-test").unwrap();

        let result = audio_capture_result(
            &config,
            LiveAudioCaptureRequest {
                seconds: 0.05,
                max_seconds: 1.0,
                bus: None,
                name: "capture".to_string(),
                out: None,
                analyze: false,
                strict_warnings: false,
                stop_after_state: None,
            },
        );

        handle.join().unwrap();
        assert!(result.ok, "{:?}", result.failures);
        assert_eq!(received.load(std::sync::atomic::Ordering::SeqCst), 2);
        assert_eq!(result.data["retry"]["attempts"], json!(2));
        assert_eq!(result.data["retry"]["final_status"], json!("success"));
        assert_json_path_eq(&result.data["artifacts"]["wav"], &wav);

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn live_audio_capture_stop_after_state_success_preserves_artifact() {
        let root = temp_root("live-audio-stop-success");
        let config = test_config(&root);
        let record = test_session_record(&config, &root, 0);
        let audio_dir = PathBuf::from(&record.artifact_dir).join("audio");
        fs::create_dir_all(&audio_dir).unwrap();
        let wav = audio_dir.join("capture.wav");
        fs::write(&wav, wav_pcm16_for_runtime_audio(8_000, &[0.0, 0.2, -0.2])).unwrap();
        let responses = vec![json!({
            "protocol_version": PROTOCOL_VERSION,
            "ok": true,
            "data": {
                "name": "capture",
                "effective_duration_seconds": 0.05,
                "artifacts": {"wav": wav},
                "warnings": [],
                "stop_condition_matched": true,
                "stop_condition_timed_out": false,
                "stop_after_state": {
                    "matched": true,
                    "sample_count": 1,
                    "sample_every_frames": 1
                }
            }
        })];
        let (port, _received, handle) = spawn_profile_server(responses);
        let mut record = record;
        record.port = port;
        persist_session(&config, &record, "token-test").unwrap();

        let result = audio_capture_result(
            &config,
            LiveAudioCaptureRequest {
                seconds: 0.05,
                max_seconds: 1.0,
                bus: None,
                name: "capture".to_string(),
                out: None,
                analyze: false,
                strict_warnings: false,
                stop_after_state: Some(json!({
                    "target": "qa:audio",
                    "source": "method",
                    "method": "get_qa_audio_state",
                    "matches": [{"path": "ready", "operator": "equals", "expected": true}]
                })),
            },
        );

        handle.join().unwrap();
        assert!(result.ok, "{:?}", result.failures);
        assert_eq!(result.data["stop_condition_matched"], json!(true));
        assert_eq!(result.data["stop_after_state"]["matched"], json!(true));
        assert_json_path_eq(&result.data["artifacts"]["wav"], &wav);

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn live_audio_capture_stop_after_state_timeout_fails_with_wav_artifact() {
        let root = temp_root("live-audio-stop-timeout");
        let config = test_config(&root);
        let record = test_session_record(&config, &root, 0);
        let audio_dir = PathBuf::from(&record.artifact_dir).join("audio");
        fs::create_dir_all(&audio_dir).unwrap();
        let wav = audio_dir.join("capture.wav");
        fs::write(&wav, wav_pcm16_for_runtime_audio(8_000, &[0.0, 0.2, -0.2])).unwrap();
        let responses = vec![json!({
            "protocol_version": PROTOCOL_VERSION,
            "ok": true,
            "data": {
                "name": "capture",
                "effective_duration_seconds": 0.05,
                "artifacts": {"wav": wav},
                "warnings": [],
                "stop_condition_matched": false,
                "stop_condition_timed_out": true,
                "stop_after_state": {
                    "matched": false,
                    "sample_count": 3,
                    "sample_every_frames": 1
                }
            }
        })];
        let (port, _received, handle) = spawn_profile_server(responses);
        let mut record = record;
        record.port = port;
        persist_session(&config, &record, "token-test").unwrap();

        let result = audio_capture_result(
            &config,
            LiveAudioCaptureRequest {
                seconds: 0.05,
                max_seconds: 1.0,
                bus: None,
                name: "capture".to_string(),
                out: None,
                analyze: false,
                strict_warnings: false,
                stop_after_state: Some(json!({
                    "target": "qa:audio",
                    "source": "method",
                    "method": "get_qa_audio_state",
                    "matches": [{"path": "ready", "operator": "equals", "expected": true}]
                })),
            },
        );

        handle.join().unwrap();
        assert!(!result.ok);
        assert_eq!(result.failures[0].kind, "audio_capture.stop_condition_timeout");
        assert_eq!(result.data["stop_condition_timed_out"], json!(true));
        assert_json_path_eq(&result.data["artifacts"]["wav"], &wav);

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn live_audio_capture_copies_to_out_path() {
        let root = temp_root("live-audio-out-copy");
        let config = test_config(&root);
        let record = test_session_record(&config, &root, 0);
        let audio_dir = PathBuf::from(&record.artifact_dir).join("audio");
        fs::create_dir_all(&audio_dir).unwrap();
        // The runtime writes the canonical artifact under the controlled audio_dir.
        let controlled_wav = audio_dir.join("sfx.wav");
        fs::write(
            &controlled_wav,
            wav_pcm16_for_runtime_audio(8_000, &[0.0, 0.2, -0.2]),
        )
        .unwrap();
        let responses = vec![json!({
            "protocol_version": PROTOCOL_VERSION,
            "ok": true,
            "data": {
                "name": "sfx",
                "effective_duration_seconds": 0.05,
                "artifacts": {"wav": controlled_wav},
                "warnings": []
            }
        })];
        let (port, _received, handle) = spawn_profile_server(responses);
        let mut record = record;
        record.port = port;
        persist_session(&config, &record, "token-test").unwrap();

        let out = root.join(".qa/artifacts/manual/sfx_capture.wav");
        let result = audio_capture_result(
            &config,
            LiveAudioCaptureRequest {
                seconds: 0.05,
                max_seconds: 1.0,
                bus: None,
                name: "sfx".to_string(),
                out: Some(out.clone()),
                analyze: false,
                strict_warnings: false,
                stop_after_state: None,
            },
        );

        handle.join().unwrap();
        assert!(result.ok, "{:?}", result.failures);
        // `--out` receives a copy; the controlled artifact stays canonical.
        assert!(out.is_file(), "expected --out copy at {}", out.display());
        assert_eq!(result.data["artifacts"]["out"], json!(out));
        assert_json_path_eq(&result.data["artifacts"]["wav"], &controlled_wav);
        assert!(controlled_wav.is_file());

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn live_audio_capture_copies_analysis_to_out_path() {
        let root = temp_root("live-audio-out-analysis-copy");
        let config = test_config(&root);
        let record = test_session_record(&config, &root, 0);
        let audio_dir = PathBuf::from(&record.artifact_dir).join("audio");
        fs::create_dir_all(&audio_dir).unwrap();
        let controlled_wav = audio_dir.join("sfx.wav");
        fs::write(
            &controlled_wav,
            wav_pcm16_for_runtime_audio(8_000, &[0.0, 0.2, -0.2, 0.0]),
        )
        .unwrap();
        let responses = vec![json!({
            "protocol_version": PROTOCOL_VERSION,
            "ok": true,
            "data": {
                "name": "sfx",
                "effective_duration_seconds": 0.05,
                "artifacts": {"wav": controlled_wav},
                "warnings": []
            }
        })];
        let (port, _received, handle) = spawn_profile_server(responses);
        let mut record = record;
        record.port = port;
        persist_session(&config, &record, "token-test").unwrap();

        let out = root.join(".qa/artifacts/manual/sfx_capture.wav");
        let result = audio_capture_result(
            &config,
            LiveAudioCaptureRequest {
                seconds: 0.05,
                max_seconds: 1.0,
                bus: None,
                name: "sfx".to_string(),
                out: Some(out.clone()),
                analyze: true,
                strict_warnings: false,
                stop_after_state: None,
            },
        );

        handle.join().unwrap();
        assert!(result.ok, "{:?}", result.failures);
        let out_analysis = out.with_extension("analysis.json");
        assert!(out.is_file());
        assert!(
            out_analysis.is_file(),
            "expected analysis copy at {}",
            out_analysis.display()
        );
        assert_eq!(result.data["artifacts"]["out"], json!(out));
        assert_eq!(result.data["artifacts"]["out_analysis"], json!(out_analysis));
        // The canonical analysis stays alongside the controlled WAV.
        assert!(controlled_wav.with_extension("analysis.json").is_file());

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn resolve_session_start_scene_prefers_explicit_scene_then_project_main_scene() {
        let root = temp_root("session-start-main-scene");
        let config = test_config(&root);
        fs::write(
            root.join("project.godot"),
            "[application]\nrun/main_scene=\"res://main.tscn\"\n",
        )
        .unwrap();

        assert_eq!(
            resolve_session_start_scene(&config, Some(" res://override.tscn ")).unwrap(),
            "res://override.tscn"
        );
        assert_eq!(
            resolve_session_start_scene(&config, None).unwrap(),
            "res://main.tscn"
        );
        // An EXPLICIT empty (or whitespace-only) scene requests a sceneless boot
        // (warm bundle path) and must NOT fall back to the project main scene,
        // otherwise the game's main scene stays live alongside the per-scenario
        // executor scene and diverges warm from cold under drive_policy input.
        assert_eq!(resolve_session_start_scene(&config, Some("")).unwrap(), "");
        assert_eq!(resolve_session_start_scene(&config, Some("   ")).unwrap(), "");

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn temporal_live_error_fields_are_preserved() {
        let error_data = Map::from_iter([
            ("type".to_string(), json!("assertion_failed")),
            ("message".to_string(), json!("temporal assertion failed")),
            (
                "data".to_string(),
                json!({
                    "expected": {"max": 32.0, "window_frames": 12},
                    "actual": {"max": 81.25, "sample_frame": 17},
                    "target": "qa:demo_actor",
                    "severity": "error",
                    "path": "qa/scenarios/temporal_assertions_failure_demo.json",
                    "artifacts": {"temporal_trace": ".qa/artifacts/run/temporal/trace.jsonl"}
                }),
            ),
        ]);

        let failure = failure_detail_from_live_error("assert max-delta-position", &error_data);
        assert_eq!(failure.kind, "assertion_failed");
        assert_eq!(failure.target.as_deref(), Some("qa:demo_actor"));
        assert_eq!(failure.severity.as_deref(), Some("error"));
        assert_eq!(
            failure.path.as_deref(),
            Some("qa/scenarios/temporal_assertions_failure_demo.json")
        );
        assert_eq!(
            failure.expected,
            Some(json!({"max": 32.0, "window_frames": 12}))
        );
        assert_eq!(
            failure.actual,
            Some(json!({"max": 81.25, "sample_frame": 17}))
        );
        assert_eq!(
            failure.artifacts.unwrap()["temporal_trace"],
            json!(".qa/artifacts/run/temporal/trace.jsonl")
        );
    }

    #[test]
    fn live_profile_error_classification() {
        let token_error = Map::from_iter([
            ("type".to_string(), json!("invalid_session_token")),
            ("message".to_string(), json!("token rejected")),
            (
                "data".to_string(),
                json!({"actual": "invalid_session_token"}),
            ),
        ]);
        let token_failure = classify_live_profile_runtime_failure(
            "session profile",
            &token_error,
            LiveProfileAttempt {
                attempt: 1,
                retryable: false,
            },
        );
        assert_eq!(token_failure.failure.kind, "session_token_invalid");
        assert!(!token_failure.attempt.retryable);
        assert_eq!(token_failure.failure.details["retryable"], json!(false));

        let timeout_failure = classify_live_profile_failure(
            "session profile",
            FailureDetail {
                kind: "session_connect_failed".to_string(),
                actual: Some(json!("TimeoutError")),
                ..FailureDetail::default()
            },
        );
        assert_eq!(timeout_failure.failure.kind, "session_connect_timeout");
        assert!(!timeout_failure.attempt.retryable);

        let unreachable_failure = classify_live_profile_failure(
            "session profile",
            FailureDetail {
                kind: "session_connect_failed".to_string(),
                actual: Some(json!("ConnectionRefusedError")),
                ..FailureDetail::default()
            },
        );
        assert_eq!(unreachable_failure.failure.kind, "session_unreachable");
        assert!(!unreachable_failure.attempt.retryable);

        let busy_error = Map::from_iter([
            ("type".to_string(), json!("resource_unavailable")),
            ("message".to_string(), json!("bridge is busy")),
            ("data".to_string(), json!({"actual": "profiling_active"})),
        ]);
        let generic_live_failure = failure_detail_from_live_error("session inspect", &busy_error);
        assert_eq!(generic_live_failure.kind, "resource_unavailable");
        assert!(!generic_live_failure.details.contains_key("retryable"));

        let busy_failure = classify_live_profile_runtime_failure(
            "session profile",
            &busy_error,
            LiveProfileAttempt {
                attempt: 1,
                retryable: false,
            },
        );
        assert_eq!(busy_failure.failure.kind, "session_bridge_busy");
        assert!(busy_failure.attempt.retryable);
        assert_eq!(busy_failure.failure.details["retryable"], json!(true));

        let stale_failure = classify_live_profile_transport_failure(
            &test_config(&temp_root("profile-stale-session")),
            &SessionRecord {
                process_identity: None,
                protocol_version: PROTOCOL_VERSION.to_string(),
                session_id: "session-test".to_string(),
                pid: -1,
                host: "127.0.0.1".to_string(),
                port: 1,
                scene: "res://main.tscn".to_string(),
                viewport: vec![640, 360],
                artifact_dir: "/tmp/godot-qa-missing/.qa/artifacts/live/session-test".to_string(),
                request_path: "/tmp/godot-qa-missing/request.json".to_string(),
                token_file: "/tmp/godot-qa-missing/.qa/session.token".to_string(),
                started_at: "2026-05-06T00:00:00Z".to_string(),
                headless: true,
                project_args: Vec::new(),
                frame_pacing: None,
                target: SessionTarget::Desktop,
                android: None,
            },
            "session profile",
            FailureDetail {
                kind: "session_connect_failed".to_string(),
                actual: Some(json!("ConnectionRefusedError")),
                ..FailureDetail::default()
            },
        );
        assert_eq!(stale_failure.failure.kind, "stale_session");
        assert!(!stale_failure.attempt.retryable);
    }

    #[test]
    fn live_wait_request_timeout_scales_with_frames() {
        assert_eq!(
            live_request_timeout(
                SESSION_WAIT_FRAMES,
                &Map::from_iter([("frames".to_string(), json!(0))])
            ),
            Duration::from_secs(5)
        );
        assert_eq!(
            live_request_timeout(
                SESSION_WAIT_FRAMES,
                &Map::from_iter([("frames".to_string(), json!(60))])
            ),
            Duration::from_secs(7)
        );
        assert_eq!(
            live_request_timeout(
                SESSION_WAIT_FRAMES,
                &Map::from_iter([("frames".to_string(), json!(420))])
            ),
            Duration::from_secs(19)
        );
        assert_eq!(
            live_request_timeout(
                SESSION_WAIT_FRAMES,
                &Map::from_iter([("frames".to_string(), json!(999999))])
            ),
            Duration::from_secs(120)
        );
        assert_eq!(
            live_request_timeout(SESSION_PROFILE, &Map::new()),
            Duration::from_secs(26)
        );
    }

    #[test]
    fn live_profile_request_timeout_scales_with_sample_window() {
        assert_eq!(
            live_request_timeout(
                SESSION_PROFILE,
                &Map::from_iter([("frames".to_string(), json!(600))])
            ),
            Duration::from_secs(80)
        );
        assert_eq!(
            live_request_timeout(
                SESSION_PROFILE,
                &Map::from_iter([("seconds".to_string(), json!(10.0))])
            ),
            Duration::from_secs(30)
        );
        assert_eq!(
            live_request_timeout(
                SESSION_PROFILE,
                &Map::from_iter([("frames".to_string(), json!(999999))])
            ),
            Duration::from_secs(120)
        );
    }

    #[test]
    fn event_trace_request_timeout_scales_with_frames() {
        assert_eq!(
            live_request_timeout(
                EVENT_TRACE,
                &Map::from_iter([("frames".to_string(), json!(300))])
            ),
            Duration::from_secs(50)
        );
        assert_eq!(
            live_request_timeout(
                EVENT_TRACE,
                &Map::from_iter([("frames".to_string(), json!(600))])
            ),
            Duration::from_secs(80)
        );
        assert_eq!(
            live_request_timeout(
                EVENT_TRACE,
                &Map::from_iter([("frames".to_string(), json!(3600))])
            ),
            Duration::from_secs(120)
        );
    }

    #[test]
    fn live_command_busy_backoff_uses_in_progress_window() {
        let failure = FailureDetail {
            kind: "session_bridge_busy".to_string(),
            details: Map::from_iter([(
                "in_progress".to_string(),
                json!({
                    "command": EVENT_TRACE,
                    "frames": 600,
                    "elapsed_msec": 1000
                }),
            )]),
            ..FailureDetail::default()
        };

        let backoff = live_command_busy_backoff_ms(&failure).unwrap();

        assert!(backoff >= 19_000, "{backoff}");
    }

    #[test]
    fn live_profile_retry_backoff_uses_in_progress_profile_window() {
        let failure = FailureDetail {
            kind: "session_bridge_busy".to_string(),
            details: Map::from_iter([(
                "in_progress".to_string(),
                json!({
                    "command": SESSION_PROFILE,
                    "frames": 600,
                    "elapsed_msec": 1000
                }),
            )]),
            ..FailureDetail::default()
        };

        let (backoff, wait_aware) =
            live_profile_retry_backoff_ms(&failure, &LiveProfileRetryPolicy::default(), 1);

        assert!(wait_aware);
        assert!(backoff >= 79_000, "{backoff}");
    }

    #[test]
    fn session_stop_terminates_when_bridge_is_busy() {
        let root = temp_root("stop-bridge-busy");
        let config = test_config(&root);
        let responses = vec![json!({
            "protocol_version": PROTOCOL_VERSION,
            "ok": false,
            "error": {
                "type": "session_bridge_busy",
                "message": "QA bridge is already processing a live command",
                "data": {
                    "in_progress": {
                        "command": TRACE_RECORD,
                        "frames": 300
                    }
                }
            }
        })];
        let (port, _received, handle) = spawn_profile_server(responses);
        let mut child = std::process::Command::new("sleep")
            .arg("30")
            .spawn()
            .unwrap();
        let mut record = test_session_record(&config, &root, port);
        record.pid = child.id() as i64;
        record.process_identity = godot_qa_process::process_identity(record.pid);
        persist_session(&config, &record, "token-test").unwrap();

        let result = session_stop_result(&config);

        handle.join().unwrap();
        let _ = child.wait();
        assert!(result.ok, "{:?}", result.failures);
        assert_eq!(
            result.message,
            "terminated live session after bridge request failed"
        );
        assert!(!session_json_path(&config).exists());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn trace_record_request_timeout_scales_with_frames() {
        assert_eq!(
            live_request_timeout(
                TRACE_RECORD,
                &Map::from_iter([("frames".to_string(), json!(300))])
            ),
            Duration::from_secs(80)
        );
        assert_eq!(
            live_request_timeout(
                TRACE_RECORD,
                &Map::from_iter([("frames".to_string(), json!(3600))])
            ),
            Duration::from_secs(120)
        );
    }

    #[test]
    fn live_profile_bridge_busy_retries() {
        let root = temp_root("profile-busy-retry");
        let config = test_config(&root);
        let responses = vec![
            json!({
                "protocol_version": PROTOCOL_VERSION,
                "ok": false,
                "error": {
                    "type": "resource_unavailable",
                    "message": "bridge is busy",
                    "data": {"actual": "profiling_active"}
                }
            }),
            json!({
                "protocol_version": PROTOCOL_VERSION,
                "ok": true,
                "data": valid_runtime_profile_payload()
            }),
        ];
        let (port, received, handle) = spawn_profile_server(responses);
        let record = test_session_record(&config, &root, port);
        persist_session(&config, &record, "token-test").unwrap();

        let result = session_profile_result(&config, None, Some(0.05), 120, 5);

        handle.join().unwrap();
        assert!(result.ok, "{:?}", result.failures);
        assert_eq!(received.load(std::sync::atomic::Ordering::SeqCst), 2);
        assert_eq!(result.data["retry"]["attempts"], json!(2));
        assert_eq!(result.data["retry"]["max_attempts"], json!(3));
        assert_eq!(result.data["retry"]["backoff_milliseconds"], json!([25]));
        assert_eq!(result.data["retry"]["wait_aware"], json!(false));
        assert_eq!(
            result.data["retry"]["retryable_failure_kinds"],
            json!(LIVE_PROFILE_RETRYABLE_FAILURE_KINDS)
        );
        assert_eq!(
            result.data["retry"]["observed_retryable_failures"][0]["retry_after_ms"],
            json!(25)
        );
        assert_eq!(
            result.data["retry"]["observed_retryable_failures"][0]["wait_aware"],
            json!(false)
        );
        assert_eq!(result.data["retry"]["final_status"], json!("success"));
        assert_eq!(result.data["retry"]["exhausted"], json!(false));
        assert_eq!(
            result.data["retry"]["sample_window"]["seconds"],
            json!(0.05)
        );
        assert!(result.data["artifacts"]["profile"].as_str().is_some());
        assert!(PathBuf::from(result.data["artifacts"]["profile"].as_str().unwrap()).exists());

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn direct_live_command_bridge_busy_retries() {
        let root = temp_root("direct-busy-retry");
        let config = test_config(&root);
        let responses = vec![
            json!({
                "protocol_version": PROTOCOL_VERSION,
                "ok": false,
                "error": {
                    "type": "resource_unavailable",
                    "message": "QA bridge is already processing a live command",
                    "data": {
                        "in_progress": {
                            "command": SESSION_WAIT_FRAMES,
                            "frames": 60
                        }
                    }
                }
            }),
            json!({
                "protocol_version": PROTOCOL_VERSION,
                "ok": true,
                "data": {
                    "snapshot": "after_wait",
                    "artifacts": {"snapshot": "/tmp/snapshot"}
                }
            }),
        ];
        let (port, received, handle) = spawn_profile_server(responses);
        let record = test_session_record(&config, &root, port);
        persist_session(&config, &record, "token-test").unwrap();

        let result = run_live_protocol_command(&config, "snapshot", SNAPSHOT_CAPTURE, Map::new());

        handle.join().unwrap();
        assert!(result.ok, "{:?}", result.failures);
        assert_eq!(received.load(std::sync::atomic::Ordering::SeqCst), 2);
        assert_eq!(result.data["retry"]["attempts"], json!(2));
        assert_eq!(result.data["retry"]["max_attempts"], json!(3));
        assert_eq!(
            result.data["retry"]["retryable_failure_kinds"],
            json!(LIVE_COMMAND_RETRYABLE_FAILURE_KINDS)
        );
        assert_eq!(result.data["retry"]["final_status"], json!("success"));
        assert_eq!(result.data["retry"]["exhausted"], json!(false));

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn direct_live_command_assertion_failure_is_not_retried() {
        let root = temp_root("direct-assertion-no-retry");
        let config = test_config(&root);
        let responses = vec![json!({
            "protocol_version": PROTOCOL_VERSION,
            "ok": false,
            "error": {
                "type": "assertion_failed",
                "message": "target was not visible",
                "data": {"target": "qa:missing"}
            }
        })];
        let (port, received, handle) = spawn_profile_server(responses);
        let record = test_session_record(&config, &root, port);
        persist_session(&config, &record, "token-test").unwrap();

        let result =
            run_live_protocol_command(&config, "assert visible", ASSERT_VISIBLE, Map::new());

        handle.join().unwrap();
        assert!(!result.ok);
        assert_eq!(received.load(std::sync::atomic::Ordering::SeqCst), 1);
        assert_eq!(result.failures[0].kind, "assertion_failed");
        assert!(!result.data.contains_key("retry"));

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn direct_live_retry_classification_is_limited_to_transient_failures() {
        let temporary_connect = FailureDetail {
            kind: "session_connect_failed".to_string(),
            message: Some("Resource temporarily unavailable (os error 11)".to_string()),
            ..FailureDetail::default()
        };
        assert!(live_command_failure_is_retryable(&temporary_connect));

        let refused_connect = FailureDetail {
            kind: "session_connect_failed".to_string(),
            actual: Some(json!("ConnectionRefusedError")),
            ..FailureDetail::default()
        };
        assert!(!live_command_failure_is_retryable(&refused_connect));

        let stale_session = FailureDetail {
            kind: "stale_session".to_string(),
            ..FailureDetail::default()
        };
        assert!(!live_command_failure_is_retryable(&stale_session));

        let assertion = FailureDetail {
            kind: "assertion_failed".to_string(),
            ..FailureDetail::default()
        };
        assert!(!live_command_failure_is_retryable(&assertion));
    }

    #[test]
    fn live_profile_malformed_payload_is_not_retried() {
        let root = temp_root("profile-malformed-no-retry");
        let config = test_config(&root);
        let responses = vec![json!({
            "protocol_version": PROTOCOL_VERSION,
            "ok": true,
            "data": {
                "sample_window": {"seconds": 0.05},
                "duration_ms": 50.0,
                "frame_times_ms": "not-array",
                "monitor_samples": []
            }
        })];
        let (port, received, handle) = spawn_profile_server(responses);
        let record = test_session_record(&config, &root, port);
        persist_session(&config, &record, "token-test").unwrap();

        let result = session_profile_result(&config, None, Some(0.05), 120, 5);

        handle.join().unwrap();
        assert!(!result.ok);
        assert_eq!(received.load(std::sync::atomic::Ordering::SeqCst), 1);
        assert_eq!(result.failures[0].kind, "profile_result_invalid");
        assert!(!result.data.contains_key("retry"));

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn event_trace_result_writes_events_summary_and_artifact_paths() {
        let root = temp_root("event-trace-result");
        let config = test_config(&root);
        let responses = vec![json!({
            "protocol_version": PROTOCOL_VERSION,
            "ok": true,
            "data": {
                "event_trace_id": "trace-a",
                "events": [
                    {
                        "event_id": "spawn-1",
                        "frame": 1,
                        "time_msec": 0.0,
                        "category": "gameplay",
                        "event_type": "gameplay.spawn",
                        "payload": {"actor": "player"}
                    },
                    {
                        "event_id": "sfx-1",
                        "frame": 2,
                        "time_msec": 16.0,
                        "category": "sfx",
                        "event_type": "sfx.spawn",
                        "payload": {}
                    }
                ]
            }
        })];
        let (port, received, handle) = spawn_profile_server(responses);
        let record = test_session_record(&config, &root, port);
        persist_session(&config, &record, "token-test").unwrap();

        let result = event_trace_result(
            &config,
            Map::from_iter([
                ("target".to_string(), json!("qa:event_source")),
                ("provider".to_string(), json!("get_qa_event_stream_state")),
                ("frames".to_string(), json!(2)),
                ("sample_every_frames".to_string(), json!(1)),
            ]),
        );

        handle.join().unwrap();
        assert!(result.ok, "{:?}", result.failures);
        assert_eq!(received.load(std::sync::atomic::Ordering::SeqCst), 1);
        assert_eq!(result.data["total_event_count"], json!(2));
        let events_path = PathBuf::from(result.data["artifacts"]["events"].as_str().unwrap());
        let summary_path = PathBuf::from(result.data["artifacts"]["summary"].as_str().unwrap());
        assert!(events_path.ends_with("events/trace-a/events.json"));
        assert!(summary_path.ends_with("events/trace-a/summary.json"));
        assert!(events_path.exists());
        assert!(summary_path.exists());
        assert!(
            PathBuf::from(&record.artifact_dir)
                .join("events/latest")
                .exists()
        );

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn event_trace_result_retries_transient_live_bridge_failures() {
        let root = temp_root("event-trace-result-retry");
        let config = test_config(&root);
        let responses = vec![
            json!({
                "protocol_version": PROTOCOL_VERSION,
                "ok": false,
                "error": {
                    "type": "resource_unavailable",
                    "message": "QA bridge is already processing a live command",
                    "data": {
                        "in_progress": {
                            "command": SESSION_WAIT_FRAMES,
                            "frames": 60
                        }
                    }
                }
            }),
            json!({
                "protocol_version": PROTOCOL_VERSION,
                "ok": true,
                "data": {
                    "event_trace_id": "trace-retry",
                    "events": [
                        {
                            "event_id": "shore-1",
                            "frame": 3,
                            "time_msec": 33.0,
                            "category": "collision",
                            "event_type": "collision_bank_or_shore",
                            "payload": {"surface": "shore"}
                        }
                    ]
                }
            }),
        ];
        let (port, received, handle) = spawn_profile_server(responses);
        let record = test_session_record(&config, &root, port);
        persist_session(&config, &record, "token-test").unwrap();

        let result = event_trace_result(
            &config,
            Map::from_iter([
                ("target".to_string(), json!("qa:event_source")),
                ("provider".to_string(), json!("get_qa_event_stream_state")),
                ("frames".to_string(), json!(300)),
                ("sample_every_frames".to_string(), json!(1)),
            ]),
        );

        handle.join().unwrap();
        assert!(result.ok, "{:?}", result.failures);
        assert_eq!(received.load(std::sync::atomic::Ordering::SeqCst), 2);
        assert_eq!(result.data["retry"]["attempts"], json!(2));
        assert_eq!(
            result.data["retry"]["retryable_failure_kinds"],
            json!(LIVE_COMMAND_RETRYABLE_FAILURE_KINDS)
        );
        assert_eq!(result.data["retry"]["final_status"], json!("success"));
        assert_eq!(result.data["total_event_count"], json!(1));
        let events_path = PathBuf::from(result.data["artifacts"]["events"].as_str().unwrap());
        assert!(events_path.ends_with("events/trace-retry/events.json"));
        assert!(events_path.exists());

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn event_trace_result_strict_warnings_converts_to_failure_after_writing_artifacts() {
        let root = temp_root("event-trace-result-strict");
        let config = test_config(&root);
        let responses = vec![json!({
            "protocol_version": PROTOCOL_VERSION,
            "ok": true,
            "data": {
                "event_trace_id": "trace-empty",
                "events": []
            }
        })];
        let (port, _received, handle) = spawn_profile_server(responses);
        let record = test_session_record(&config, &root, port);
        persist_session(&config, &record, "token-test").unwrap();

        let result = event_trace_result(
            &config,
            Map::from_iter([
                ("target".to_string(), json!("qa:event_source")),
                ("provider".to_string(), json!("get_qa_event_stream_state")),
                ("frames".to_string(), json!(2)),
                ("sample_every_frames".to_string(), json!(1)),
                ("strict_warnings".to_string(), json!(true)),
            ]),
        );

        handle.join().unwrap();
        assert!(!result.ok);
        assert_eq!(result.exit_code, 1);
        assert_eq!(result.failures[0].kind, "event_trace.strict_warnings");
        assert_eq!(
            result.data["warning_ids"],
            json!(["event_trace.no_events_recorded"])
        );
        let events_path = PathBuf::from(result.data["artifacts"]["events"].as_str().unwrap());
        let summary_path = PathBuf::from(result.data["artifacts"]["summary"].as_str().unwrap());
        assert!(events_path.exists());
        assert!(summary_path.exists());

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn trace_record_result_retries_transient_live_bridge_failures() {
        let root = temp_root("trace-record-result-retry");
        let config = test_config(&root);
        let responses = vec![
            json!({
                "protocol_version": PROTOCOL_VERSION,
                "ok": false,
                "error": {
                    "type": "resource_unavailable",
                    "message": "QA bridge is already processing a live command",
                    "data": {
                        "in_progress": {
                            "command": TRACE_RECORD,
                            "frames": 300
                        }
                    }
                }
            }),
            json!({
                "protocol_version": PROTOCOL_VERSION,
                "ok": true,
                "data": {
                    "trace_id": "trace-record-retry",
                    "samples": [
                        {
                            "frame": 1,
                            "time_msec": 16.0,
                            "target": {"requested": "qa:raft_0"}
                        }
                    ]
                }
            }),
        ];
        let (port, received, handle) = spawn_profile_server(responses);
        let record = test_session_record(&config, &root, port);
        persist_session(&config, &record, "token-test").unwrap();

        let result = trace_record_result(
            &config,
            Map::from_iter([
                ("targets".to_string(), json!(["qa:raft_0"])),
                ("frames".to_string(), json!(300)),
                ("sample_every_frames".to_string(), json!(1)),
                ("include_physics".to_string(), json!(true)),
            ]),
        );

        handle.join().unwrap();
        assert!(result.ok, "{:?}", result.failures);
        assert_eq!(received.load(std::sync::atomic::Ordering::SeqCst), 2);
        assert_eq!(result.data["retry"]["attempts"], json!(2));
        assert_eq!(
            result.data["retry"]["retryable_failure_kinds"],
            json!(LIVE_COMMAND_RETRYABLE_FAILURE_KINDS)
        );
        assert_eq!(result.data["retry"]["final_status"], json!("success"));
        let trace_path = PathBuf::from(result.data["artifacts"]["trace"].as_str().unwrap());
        assert!(trace_path.ends_with("traces/trace-record-retry/trace.json"));
        assert!(trace_path.exists());

        let _ = fs::remove_dir_all(root);
    }

    fn valid_runtime_profile_payload() -> Value {
        json!({
            "sample_window": {"seconds": 0.05},
            "duration_ms": 50.0,
            "frame_times_ms": [16.0, 17.5, 15.25],
            "frame_samples": [
                {"frame": 1, "frame_time_ms": 16.0},
                {"frame": 2, "frame_time_ms": 17.5}
            ],
            "monitor_samples": [
                {"time_ms": 0.0, "Performance.TIME_FPS": 60.0},
                {"time_ms": 16.0, "Performance.TIME_FPS": 58.0}
            ],
            "unsupported_monitors": [],
            "node_process_counts": {},
            "resource_activity": {"status": "unavailable"},
            "top_script_methods": {"status": "unsupported"}
        })
    }

    #[test]
    fn telemetry_record_writes_jsonl_manifest_and_out_copy() {
        let root = temp_root("telemetry-record-artifacts");
        let config = test_config(&root);
        let record = test_session_record(&config, &root, 0);
        let responses = vec![json!({
            "protocol_version": PROTOCOL_VERSION,
            "ok": true,
            "data": {
                "telemetry_id": "telemetry-fixed",
                "session": {"scene": "res://main.tscn"},
                "config": {"provider": "difficulty", "frames": 4, "every": 2, "max_rows": 3600},
                "rows": [
                    {"frame": 10, "t_ms": 100, "value": {"difficulty": 1.0}},
                    {"frame": 12, "t_ms": 133, "value": {"difficulty": 1.5}}
                ],
                "row_count": 2,
                "truncated": false,
                "provider_error_count": 0
            }
        })];
        let (port, received, handle) = spawn_profile_server(responses);
        let mut record = record;
        record.port = port;
        persist_session(&config, &record, "token-test").unwrap();

        let out = root.join(".qa/artifacts/manual/difficulty.jsonl");
        let params = Map::from_iter([
            ("provider".to_string(), json!("difficulty")),
            ("frames".to_string(), json!(4)),
            ("every".to_string(), json!(2)),
        ]);
        let result = telemetry_record_result(&config, params, Some(&out));

        handle.join().unwrap();
        assert!(result.ok, "{:?}", result.failures);
        assert_eq!(received.load(std::sync::atomic::Ordering::SeqCst), 1);
        assert_eq!(result.data["telemetry_id"], json!("telemetry-fixed"));
        assert_eq!(result.data["row_count"], json!(2));
        assert!(
            !result.data.contains_key("rows"),
            "raw rows must live in the JSONL artifact, not the result payload"
        );

        let rows_path = PathBuf::from(result.data["artifacts"]["rows"].as_str().unwrap());
        let telemetry_dir = PathBuf::from(&record.artifact_dir)
            .join("telemetry")
            .join("telemetry-fixed");
        assert_eq!(rows_path, telemetry_dir.join("telemetry.jsonl"));
        let jsonl = fs::read_to_string(&rows_path).unwrap();
        let lines: Vec<&str> = jsonl.lines().collect();
        assert_eq!(lines.len(), 2);
        let first: Value = serde_json::from_str(lines[0]).unwrap();
        assert_eq!(first["frame"], json!(10));
        assert_eq!(first["value"]["difficulty"], json!(1.0));

        let manifest: Value = serde_json::from_str(
            &fs::read_to_string(telemetry_dir.join("manifest.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(manifest["rows"], json!(2));
        assert_eq!(manifest["provider"], json!("difficulty"));
        assert_eq!(manifest["frames"], json!(4));
        assert_eq!(manifest["every"], json!(2));
        assert_eq!(manifest["truncated"], json!(false));

        assert_eq!(result.data["artifacts"]["out"], json!(out));
        assert_eq!(fs::read_to_string(&out).unwrap(), jsonl);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn telemetry_record_runtime_error_maps_to_structured_failure() {
        let root = temp_root("telemetry-record-runtime-error");
        let config = test_config(&root);
        let record = test_session_record(&config, &root, 0);
        let responses = vec![json!({
            "protocol_version": PROTOCOL_VERSION,
            "ok": false,
            "error": {
                "type": "telemetry.provider_missing",
                "message": "telemetry.record target does not expose the requested method",
                "data": {"target": "qa:hud", "method": "qa_pacing"}
            }
        })];
        let (port, _received, handle) = spawn_profile_server(responses);
        let mut record = record;
        record.port = port;
        persist_session(&config, &record, "token-test").unwrap();

        let params = Map::from_iter([
            ("target".to_string(), json!("qa:hud")),
            ("method".to_string(), json!("qa_pacing")),
            ("frames".to_string(), json!(4)),
            ("every".to_string(), json!(1)),
        ]);
        let result = telemetry_record_result(&config, params, None);

        handle.join().unwrap();
        assert!(!result.ok);
        assert_eq!(result.failures[0].kind, "telemetry.provider_missing");
        let _ = fs::remove_dir_all(root);
    }

    fn spawn_profile_server(
        responses: Vec<Value>,
    ) -> (
        u16,
        std::sync::Arc<std::sync::atomic::AtomicUsize>,
        std::thread::JoinHandle<()>,
    ) {
        let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        let received = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let received_for_thread = received.clone();
        let handle = std::thread::spawn(move || {
            for response in responses {
                let (mut stream, _) = listener.accept().unwrap();
                let mut buffer = [0_u8; 4096];
                let _ = stream.read(&mut buffer).unwrap();
                received_for_thread.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                let mut payload = serde_json::to_vec(&response).unwrap();
                payload.push(b'\n');
                stream.write_all(&payload).unwrap();
            }
        });
        (port, received, handle)
    }

    fn test_session_record(config: &ProjectConfig, root: &Path, port: u16) -> SessionRecord {
        SessionRecord {
            process_identity: None,
            protocol_version: PROTOCOL_VERSION.to_string(),
            session_id: "session-test".to_string(),
            pid: std::process::id() as i64,
            host: "127.0.0.1".to_string(),
            port,
            scene: "res://ui/main_menu.tscn".to_string(),
            viewport: vec![1280, 720],
            artifact_dir: root
                .join(".qa/artifacts/live/session-test")
                .to_string_lossy()
                .into_owned(),
            request_path: root
                .join(".qa/artifacts/live/session-test/request.json")
                .to_string_lossy()
                .into_owned(),
            token_file: session_token_path(config).to_string_lossy().into_owned(),
            started_at: "2026-05-06T00:00:00Z".to_string(),
            headless: true,
            project_args: Vec::new(),
            frame_pacing: None,
            target: SessionTarget::Desktop,
            android: None,
        }
    }

    fn wav_pcm16_for_runtime_audio(sample_rate: u32, mono_samples: &[f64]) -> Vec<u8> {
        let mut data = Vec::new();
        for sample in mono_samples {
            let clamped = sample.clamp(-1.0, 1.0);
            let value = if clamped >= 1.0 {
                i16::MAX
            } else if clamped <= -1.0 {
                i16::MIN
            } else {
                (clamped * i16::MAX as f64) as i16
            };
            data.extend_from_slice(&value.to_le_bytes());
        }
        let data_len = data.len() as u32;
        let mut wav = Vec::new();
        wav.extend_from_slice(b"RIFF");
        wav.extend_from_slice(&(36 + data_len).to_le_bytes());
        wav.extend_from_slice(b"WAVEfmt ");
        wav.extend_from_slice(&16u32.to_le_bytes());
        wav.extend_from_slice(&1u16.to_le_bytes());
        wav.extend_from_slice(&1u16.to_le_bytes());
        wav.extend_from_slice(&sample_rate.to_le_bytes());
        wav.extend_from_slice(&(sample_rate * 2).to_le_bytes());
        wav.extend_from_slice(&2u16.to_le_bytes());
        wav.extend_from_slice(&16u16.to_le_bytes());
        wav.extend_from_slice(b"data");
        wav.extend_from_slice(&data_len.to_le_bytes());
        wav.extend_from_slice(&data);
        wav
    }

    fn temp_root(label: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("godot-qa-runtime-{label}-{unique}"));
        let _ = fs::create_dir_all(root.join(".qa"));
        root
    }

    fn write_test_session_record(path: &Path, record: &SessionRecord) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, serde_json::to_string_pretty(record).unwrap()).unwrap();
    }

    fn test_config_with_artifacts(root: &Path, artifacts_dir: PathBuf) -> ProjectConfig {
        ProjectConfig {
            artifacts_dir,
            ..test_config(root)
        }
    }

    #[cfg(unix)]
    #[test]
    fn update_latest_link_replaces_dangling_symlink() {
        let root = temp_root("dangling-latest");
        let live_dir = root.join(".qa/artifacts/live");
        fs::create_dir_all(&live_dir).unwrap();
        let latest = live_dir.join("latest");
        // A `latest` symlink whose target was rotated away. Path::exists() follows the link
        // and reports false, so the old code skipped cleanup and symlink() failed with EEXIST.
        std::os::unix::fs::symlink(live_dir.join("session-gone"), &latest).unwrap();

        let target = live_dir.join("session-new");
        fs::create_dir_all(&target).unwrap();
        update_latest_link(&target, &latest).expect("rotate dangling latest link");

        assert_eq!(fs::read_link(&latest).unwrap(), target);
        let _ = fs::remove_dir_all(&root);
    }

    #[cfg(unix)]
    #[test]
    fn update_latest_link_is_idempotent() {
        let root = temp_root("idempotent-latest");
        let live_dir = root.join(".qa/artifacts/live");
        fs::create_dir_all(&live_dir).unwrap();
        let latest = live_dir.join("latest");

        let first = live_dir.join("session-first");
        let second = live_dir.join("session-second");
        fs::create_dir_all(&first).unwrap();
        fs::create_dir_all(&second).unwrap();

        update_latest_link(&first, &latest).expect("first link");
        assert_eq!(fs::read_link(&latest).unwrap(), first);
        update_latest_link(&second, &latest).expect("second link");
        assert_eq!(fs::read_link(&latest).unwrap(), second);

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn update_live_latest_link_does_not_touch_repo_root() {
        let project_root = temp_root("live-latest-project");
        let repo_root = temp_root("live-latest-repo");
        let artifacts_dir = project_root.join(".qa/artifacts");
        let config = ProjectConfig {
            repo_root: repo_root.clone(),
            ..test_config_with_artifacts(&project_root, artifacts_dir.clone())
        };
        let artifact_root = artifacts_dir.join("live/session-1");
        fs::create_dir_all(&artifact_root).unwrap();

        update_live_latest_link(&config, &artifact_root).expect("create live latest link");

        assert!(artifacts_dir.join("live/latest").exists());
        assert!(
            !repo_root.join(".qa/artifacts").exists(),
            "live/latest must not be mirrored into the tool repo"
        );

        let _ = fs::remove_dir_all(&project_root);
        let _ = fs::remove_dir_all(&repo_root);
    }

    #[test]
    fn apk_cache_outcome_json_shape_is_exact() {
        assert_eq!(
            ApkCacheOutcome {
                status: "hit",
                key: Some("abc".to_string()),
                reason: None,
            }
            .to_json(),
            json!({"status": "hit", "key": "abc"})
        );
        assert_eq!(
            ApkCacheOutcome {
                status: "miss",
                key: Some("def".to_string()),
                reason: None,
            }
            .to_json(),
            json!({"status": "miss", "key": "def"})
        );
        assert_eq!(
            ApkCacheOutcome {
                status: "bypassed",
                key: None,
                reason: Some("godot_version_probe_failed".to_string()),
            }
            .to_json(),
            json!({"status": "bypassed", "key": null, "reason": "godot_version_probe_failed"})
        );
    }

    #[cfg(unix)]
    struct ApkCacheFixture {
        base: PathBuf,
        config: ProjectConfig,
        apk_path: PathBuf,
        deployment_log: PathBuf,
        marker: PathBuf,
        preset: godot_qa_process::android::AndroidExportPreset,
    }

    #[cfg(unix)]
    fn apk_cache_fixture(label: &str) -> ApkCacheFixture {
        let base = temp_root(label);
        let project = base.join("project");
        fs::create_dir_all(project.join("scenes")).unwrap();
        fs::write(project.join("project.godot"), b"config_version=5\n").unwrap();
        fs::write(project.join("scenes/main.tscn"), b"[gd_scene]\n").unwrap();
        let config = ProjectConfig {
            project_root: project.clone(),
            artifacts_dir: project.join(".qa/artifacts"),
            ..test_config(&base)
        };
        // Deployment paths live under the (excluded) artifacts dir, mirroring a real session.
        let deployment = config.artifacts_dir.join("live/session-1/deployment");
        fs::create_dir_all(&deployment).unwrap();
        ApkCacheFixture {
            apk_path: deployment.join("session-debug.apk"),
            deployment_log: deployment.join("deployment.log"),
            marker: base.join("export-marker.log"),
            preset: godot_qa_process::android::AndroidExportPreset {
                name: "Android QA".to_string(),
                package: "org.example.qa".to_string(),
                export_path: None,
            },
            config,
            base,
        }
    }

    #[cfg(unix)]
    fn write_executable(path: &Path, contents: &str) -> PathBuf {
        use std::os::unix::fs::PermissionsExt;
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(path, contents).unwrap();
        let mut permissions = fs::metadata(path).unwrap().permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(path, permissions).unwrap();
        path.to_path_buf()
    }

    // Fake Godot that answers `--version` and, on export, writes `apk_content` to the last arg
    // (the APK path) and records the invocation in `marker`.
    #[cfg(unix)]
    fn working_fake_godot(base: &Path, marker: &Path, apk_content: &str) -> PathBuf {
        write_executable(
            &base.join("bin/godot"),
            &format!(
                "#!/bin/sh\ncase \"$*\" in *--version*) printf '4.3.stable.official\\n'; exit 0;; esac\nprintf 'export\\n' >> '{}'\nfor apk in \"$@\"; do :; done\nprintf '{apk_content}' > \"$apk\"\nexit 0\n",
                marker.display(),
            ),
        )
    }

    #[cfg(unix)]
    fn apk_cache_key_for(
        config: &ProjectConfig,
        godot: &Path,
        preset: &godot_qa_process::android::AndroidExportPreset,
    ) -> String {
        let excluded = android_manifest_excluded_dirs(config);
        let inputs = godot_qa_process::apk_cache::ApkCacheKeyInputs {
            godot_binary: godot.to_str().unwrap(),
            godot_version: "4.3.stable.official",
            preset_name: &preset.name,
            package: &preset.package,
            project_root: &config.project_root,
            extra_excluded_dirs: &excluded,
        };
        godot_qa_process::apk_cache::compute_cache_key(&inputs).unwrap()
    }

    #[cfg(unix)]
    fn store_fake_entry(config: &ProjectConfig, key: &str, apk_content: &[u8]) {
        let cache_root = godot_qa_process::apk_cache::cache_root(&config.project_root);
        // Source lives under the excluded `.qa` tree so it never perturbs the manifest.
        let source = config.project_root.join(".qa/tmp-source.apk");
        fs::create_dir_all(source.parent().unwrap()).unwrap();
        fs::write(&source, apk_content).unwrap();
        let meta = godot_qa_process::apk_cache::ApkCacheMeta {
            version: 1,
            key: key.to_string(),
            created_at_ms: 1,
            last_used_at_ms: 1,
            godot_binary: "godot".to_string(),
            godot_version: "4.3.stable.official".to_string(),
            preset: "Android QA".to_string(),
            package: "org.example.qa".to_string(),
            manifest_files: 2,
            apk_bytes: 0,
            tool_version: "test".to_string(),
        };
        godot_qa_process::apk_cache::store(&cache_root, key, &source, meta).unwrap();
        let _ = fs::remove_file(&source);
    }

    #[cfg(unix)]
    #[test]
    fn android_apk_cache_miss_exports_and_stores() {
        let fx = apk_cache_fixture("apk-cache-miss");
        let godot = working_fake_godot(&fx.base, &fx.marker, "FRESH-APK");

        let outcome = android_obtain_session_apk(
            &fx.config,
            godot.to_str().unwrap(),
            &fx.preset,
            false,
            &fx.apk_path,
            &fx.deployment_log,
        )
        .expect("a miss must not fail");

        assert_eq!(outcome.status, "miss");
        assert!(outcome.reason.is_none());
        let key = outcome.key.clone().expect("a miss carries a key");
        assert_eq!(key.len(), 64);
        assert!(fx.marker.exists(), "export must run on a miss");
        assert_eq!(fs::read(&fx.apk_path).unwrap(), b"FRESH-APK");
        let cache_root = godot_qa_process::apk_cache::cache_root(&fx.config.project_root);
        assert!(
            godot_qa_process::apk_cache::lookup(&cache_root, &key).is_some(),
            "the export must be stored"
        );
        let log = fs::read_to_string(&fx.deployment_log).unwrap();
        assert!(log.contains(&format!("apk-cache: stored {key}")), "log: {log}");
        let _ = fs::remove_dir_all(&fx.base);
    }

    #[cfg(unix)]
    #[test]
    fn android_apk_cache_hit_skips_export() {
        let fx = apk_cache_fixture("apk-cache-hit");
        // Succeeds on --version but exits 9 if the export branch is ever reached.
        let godot = write_executable(
            &fx.base.join("bin/godot"),
            &format!(
                "#!/bin/sh\ncase \"$*\" in *--version*) printf '4.3.stable.official\\n'; exit 0;; esac\nprintf 'export\\n' >> '{}'\nexit 9\n",
                fx.marker.display(),
            ),
        );
        let key = apk_cache_key_for(&fx.config, &godot, &fx.preset);
        store_fake_entry(&fx.config, &key, b"CACHED-APK");

        let outcome = android_obtain_session_apk(
            &fx.config,
            godot.to_str().unwrap(),
            &fx.preset,
            false,
            &fx.apk_path,
            &fx.deployment_log,
        )
        .expect("a hit must not fail");

        assert_eq!(outcome.status, "hit");
        assert_eq!(outcome.key.as_deref(), Some(key.as_str()));
        assert!(!fx.marker.exists(), "export must not run on a hit");
        assert_eq!(fs::read(&fx.apk_path).unwrap(), b"CACHED-APK");
        let log = fs::read_to_string(&fx.deployment_log).unwrap();
        assert!(log.contains(&format!("apk-cache: hit {key}")), "log: {log}");
        let _ = fs::remove_dir_all(&fx.base);
    }

    #[cfg(unix)]
    #[test]
    fn android_apk_cache_no_cache_flag_bypasses_read_but_refreshes() {
        let fx = apk_cache_fixture("apk-cache-nocache");
        let godot = working_fake_godot(&fx.base, &fx.marker, "FRESH-APK");
        let key = apk_cache_key_for(&fx.config, &godot, &fx.preset);
        store_fake_entry(&fx.config, &key, b"OLD-CACHED");

        let outcome = android_obtain_session_apk(
            &fx.config,
            godot.to_str().unwrap(),
            &fx.preset,
            true,
            &fx.apk_path,
            &fx.deployment_log,
        )
        .expect("a bypass must not fail");

        assert_eq!(outcome.status, "bypassed");
        assert_eq!(outcome.reason.as_deref(), Some("--no-apk-cache"));
        assert_eq!(outcome.key.as_deref(), Some(key.as_str()));
        assert!(fx.marker.exists(), "export still runs under --no-apk-cache");
        assert_eq!(
            fs::read(&fx.apk_path).unwrap(),
            b"FRESH-APK",
            "the fresh export is deployed, not the cache"
        );
        // The read was bypassed but the entry was still refreshed to the fresh export.
        let cache_root = godot_qa_process::apk_cache::cache_root(&fx.config.project_root);
        let entry =
            godot_qa_process::apk_cache::lookup(&cache_root, &key).expect("entry refreshed");
        assert_eq!(fs::read(&entry.apk_path).unwrap(), b"FRESH-APK");
        let log = fs::read_to_string(&fx.deployment_log).unwrap();
        assert!(
            log.contains("apk-cache: bypassed (--no-apk-cache)"),
            "log: {log}"
        );
        assert!(log.contains(&format!("apk-cache: stored {key}")), "log: {log}");
        let _ = fs::remove_dir_all(&fx.base);
    }

    #[cfg(unix)]
    #[test]
    fn android_apk_cache_corrupt_entry_degrades_to_export() {
        let fx = apk_cache_fixture("apk-cache-corrupt");
        let godot = working_fake_godot(&fx.base, &fx.marker, "FRESH-APK");
        let key = apk_cache_key_for(&fx.config, &godot, &fx.preset);
        // Plant a corrupt entry at the key the seam will consult.
        let cache_root = godot_qa_process::apk_cache::cache_root(&fx.config.project_root);
        let entry_dir = cache_root.join(&key);
        fs::create_dir_all(&entry_dir).unwrap();
        fs::write(entry_dir.join("meta.json"), b"{ not valid json").unwrap();
        fs::write(
            entry_dir.join(godot_qa_process::apk_cache::APK_FILE_NAME),
            b"garbage",
        )
        .unwrap();

        let outcome = android_obtain_session_apk(
            &fx.config,
            godot.to_str().unwrap(),
            &fx.preset,
            false,
            &fx.apk_path,
            &fx.deployment_log,
        )
        .expect("a corrupt cache must not fail the session");

        assert_eq!(outcome.status, "miss");
        assert!(fx.marker.exists(), "export runs when the entry is unusable");
        assert_eq!(fs::read(&fx.apk_path).unwrap(), b"FRESH-APK");
        // Self-heal: the corrupt entry was replaced by a valid one.
        assert!(godot_qa_process::apk_cache::lookup(&cache_root, &key).is_some());
        let log = fs::read_to_string(&fx.deployment_log).unwrap();
        assert!(log.contains(&format!("apk-cache: stored {key}")), "log: {log}");
        let _ = fs::remove_dir_all(&fx.base);
    }

    #[cfg(unix)]
    #[test]
    fn android_apk_cache_version_probe_failure_bypasses_without_storing() {
        let fx = apk_cache_fixture("apk-cache-probe-fail");
        // Fails --version (no stable key possible) but still performs the export.
        let godot = write_executable(
            &fx.base.join("bin/godot"),
            &format!(
                "#!/bin/sh\ncase \"$*\" in *--version*) exit 1;; esac\nprintf 'export\\n' >> '{}'\nfor apk in \"$@\"; do :; done\nprintf 'FRESH-APK' > \"$apk\"\nexit 0\n",
                fx.marker.display(),
            ),
        );

        let outcome = android_obtain_session_apk(
            &fx.config,
            godot.to_str().unwrap(),
            &fx.preset,
            false,
            &fx.apk_path,
            &fx.deployment_log,
        )
        .expect("a probe failure degrades rather than fails");

        assert_eq!(outcome.status, "bypassed");
        assert_eq!(outcome.reason.as_deref(), Some("godot_version_probe_failed"));
        assert!(outcome.key.is_none());
        assert!(fx.marker.exists(), "the uncached export still runs");
        assert_eq!(fs::read(&fx.apk_path).unwrap(), b"FRESH-APK");
        let cache_root = godot_qa_process::apk_cache::cache_root(&fx.config.project_root);
        assert!(
            !cache_root.exists(),
            "a probe failure must not create a cache entry"
        );
        let log = fs::read_to_string(&fx.deployment_log).unwrap();
        assert!(
            log.contains("apk-cache: bypassed (godot_version_probe_failed)"),
            "log: {log}"
        );
        let _ = fs::remove_dir_all(&fx.base);
    }

    fn test_config(root: &Path) -> ProjectConfig {
        ProjectConfig {
            project_root: root.to_path_buf(),
            repo_root: root.to_path_buf(),
            verbose: false,
            godot_binary: None,
            qa_dir: root.join("qa"),
            artifacts_dir: root.join(".qa/artifacts"),
            scenarios_dir: root.join("qa/scenarios"),
            addon_dir: root.join("addons/godot_qa"),
            repo_addon_dir: root.join("addons/godot_qa"),
            addon_identity_cache: Default::default(),
            godot_launch_prefix: Vec::new(),
        godot_windowed_launch_prefix: Vec::new(),
            godot_args: Vec::new(),
            godot_project_args: Vec::new(),
            godot_launch_mode: godot_qa_core::config::GodotLaunchMode::Auto,
            godot_audio_mode: godot_qa_core::config::GodotAudioMode::Auto,
            scenario_warm_default: false,
            scenario_warm_strict_default: false,
            scenario_jobs_default: None,
            project_config_failure: None,
            artifacts_keep_runs: None,
            scenario_schedule_default: None,
            log_ignore_patterns: Vec::new(),
            config_warnings: Vec::new(),
        }
    }
}
