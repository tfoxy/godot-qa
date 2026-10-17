// Extracted from lib.rs; included by lib.rs to preserve the original crate namespace.

pub fn audio_capture_result(
    config: &ProjectConfig,
    request: LiveAudioCaptureRequest,
) -> CommandResult {
    let command = "audio capture";
    let record = match load_session_record(config, command) {
        Ok(record) => record,
        Err(failure) => return live_audio_unavailable_result(command, failure),
    };
    let audio_dir = Path::new(&record.artifact_dir).join("audio");
    if let Err(error) = fs::create_dir_all(&audio_dir) {
        return live_audio_artifact_error(command, &audio_dir, error);
    }
    // The runtime always writes the controlled artifact under audio_dir; `--out` is a
    // CLI-side copy destination handled after the runtime returns (see cli-spec.md).
    let controlled_wav_path = audio_dir.join(format!("{}.wav", request.name));
    let mut params = Map::from_iter([
        ("duration_seconds".to_string(), json!(request.seconds)),
        (
            "effective_duration_seconds".to_string(),
            json!(request.seconds.min(request.max_seconds)),
        ),
        ("max_seconds".to_string(), json!(request.max_seconds)),
        ("name".to_string(), json!(request.name)),
        ("output_path".to_string(), json!(controlled_wav_path)),
    ]);
    if let Some(bus) = &request.bus {
        params.insert("bus".to_string(), json!(bus));
    }
    if let Some(stop_after_state) = &request.stop_after_state {
        params.insert("stop_after_state".to_string(), stop_after_state.clone());
    }
    live_audio_request_result(
        config,
        &record,
        command,
        AUDIO_CAPTURE,
        params,
        &audio_dir,
        &controlled_wav_path,
        request.out.as_deref(),
        request.analyze,
        request.strict_warnings,
        json!(request.seconds.min(request.max_seconds)),
    )
}

pub fn audio_export_result(
    config: &ProjectConfig,
    request: LiveAudioExportRequest,
) -> CommandResult {
    let command = "audio export";
    let record = match load_session_record(config, command) {
        Ok(record) => record,
        Err(failure) => return live_audio_unavailable_result(command, failure),
    };
    let audio_dir = Path::new(&record.artifact_dir).join("audio");
    if let Err(error) = fs::create_dir_all(&audio_dir) {
        return live_audio_artifact_error(command, &audio_dir, error);
    }
    // The runtime always writes the controlled artifact under audio_dir; `--out` is a
    // CLI-side copy destination handled after the runtime returns (see cli-spec.md).
    let controlled_wav_path = audio_dir.join(format!("{}.wav", request.name));
    let params = Map::from_iter([
        ("target".to_string(), json!(request.target)),
        ("method".to_string(), json!(request.method)),
        ("args".to_string(), request.args),
        ("name".to_string(), json!(request.name)),
        ("output_path".to_string(), json!(controlled_wav_path)),
    ]);
    live_audio_request_result(
        config,
        &record,
        command,
        AUDIO_EXPORT,
        params,
        &audio_dir,
        &controlled_wav_path,
        request.out.as_deref(),
        request.analyze,
        request.strict_warnings,
        Value::Null,
    )
}

pub fn audio_render_result(
    config: &ProjectConfig,
    request: LiveAudioRenderRequest,
) -> CommandResult {
    let command = "audio render";
    let record = match load_session_record(config, command) {
        Ok(record) => record,
        Err(failure) => return live_audio_unavailable_result(command, failure),
    };
    let audio_dir = Path::new(&record.artifact_dir).join("audio");
    if let Err(error) = fs::create_dir_all(&audio_dir) {
        return live_audio_artifact_error(command, &audio_dir, error);
    }
    // The runtime owns the bake loop: it drives the project's qa_ voice-fill method for the
    // requested duration and writes the controlled WAV under audio_dir; `--out` is a CLI copy.
    let controlled_wav_path = audio_dir.join(format!("{}.wav", request.name));
    let effective_seconds = request.seconds.min(request.max_seconds);
    let params = Map::from_iter([
        ("target".to_string(), json!(request.target)),
        ("method".to_string(), json!(request.method)),
        ("args".to_string(), request.args),
        ("duration_seconds".to_string(), json!(request.seconds)),
        (
            "effective_duration_seconds".to_string(),
            json!(effective_seconds),
        ),
        ("max_seconds".to_string(), json!(request.max_seconds)),
        ("mix_rate".to_string(), json!(request.mix_rate)),
        ("name".to_string(), json!(request.name)),
        ("output_path".to_string(), json!(controlled_wav_path)),
    ]);
    live_audio_request_result(
        config,
        &record,
        command,
        AUDIO_RENDER,
        params,
        &audio_dir,
        &controlled_wav_path,
        request.out.as_deref(),
        request.analyze,
        request.strict_warnings,
        json!(effective_seconds),
    )
}

#[allow(clippy::too_many_arguments)]
fn live_audio_request_result(
    config: &ProjectConfig,
    record: &SessionRecord,
    command: &str,
    protocol_command: &str,
    params: Map<String, Value>,
    audio_dir: &Path,
    requested_wav_path: &Path,
    out: Option<&Path>,
    analyze: bool,
    strict_warnings: bool,
    effective_duration_seconds: Value,
) -> CommandResult {
    let mut retry_failures = Vec::new();
    let max_attempts = LIVE_COMMAND_RETRY_BACKOFF_MILLIS.len() + 1;
    let mut attempt = 1;
    let object = loop {
        let response = match send_live_request(config, record, protocol_command, params.clone(), None) {
            Ok(response) => response,
                Err(failure) => {
                    let metadata_path = session_metadata_path(config, record);
                    let failure =
                        classify_session_connect_failure(record, command, &metadata_path, failure);
                let result = live_audio_unavailable_result(command, failure);
                if live_command_result_is_retryable(&result) && attempt < max_attempts {
                    let backoff_ms = live_command_retry_backoff_ms(&result, attempt);
                    retry_failures.push(live_command_retry_failure_summary(
                        &result, attempt, backoff_ms,
                    ));
                    thread::sleep(Duration::from_millis(backoff_ms));
                    attempt += 1;
                    continue;
                }
                return attach_live_audio_retry_metadata(result, attempt, max_attempts, &retry_failures);
            }
        };
        if let Err(error) = validate_response(command, &response) {
            let result = live_audio_unavailable_result(command, error.failure);
            if live_command_result_is_retryable(&result) && attempt < max_attempts {
                let backoff_ms = live_command_retry_backoff_ms(&result, attempt);
                retry_failures.push(live_command_retry_failure_summary(
                    &result, attempt, backoff_ms,
                ));
                thread::sleep(Duration::from_millis(backoff_ms));
                attempt += 1;
                continue;
            }
            return attach_live_audio_retry_metadata(result, attempt, max_attempts, &retry_failures);
        }
        let object = response.as_object().unwrap_or(&Map::new()).clone();
        if object.get("ok").and_then(Value::as_bool).unwrap_or(false) {
            break object;
        }
        let error_data = object
            .get("error")
            .and_then(Value::as_object)
            .cloned()
            .unwrap_or_default();
        let mut failure = failure_detail_from_live_error(command, &error_data);
        if command == "audio capture" && failure.kind == "runtime_error" {
            failure.kind = "audio_capture.failed".to_string();
        } else if command == "audio render" && failure.kind == "runtime_error" {
            failure.kind = "audio_render.failed".to_string();
        }
        let result = live_audio_unavailable_result(command, failure);
        if live_command_result_is_retryable(&result) && attempt < max_attempts {
            let backoff_ms = live_command_retry_backoff_ms(&result, attempt);
            retry_failures.push(live_command_retry_failure_summary(
                &result, attempt, backoff_ms,
            ));
            thread::sleep(Duration::from_millis(backoff_ms));
            attempt += 1;
            continue;
        }
        return attach_live_audio_retry_metadata(result, attempt, max_attempts, &retry_failures);
    };

    let mut data = object
        .get("data")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    if !retry_failures.is_empty() {
        data.insert(
            "retry".to_string(),
            live_command_retry_metadata(attempt, max_attempts, &retry_failures, true),
        );
    }
    let returned_wav_path = data
        .get("artifacts")
        .and_then(Value::as_object)
        .and_then(|artifacts| artifacts.get("wav"))
        .or_else(|| data.get("wav_path"))
        .and_then(Value::as_str)
        .map(PathBuf::from)
        .unwrap_or_else(|| requested_wav_path.to_path_buf());
    let approved_wav_path = match validate_live_audio_wav_path(
        command,
        audio_dir,
        requested_wav_path,
        &returned_wav_path,
    ) {
        Ok(path) => path,
        Err(failure) => return live_audio_unavailable_result(command, failure),
    };

    let mut artifacts = data
        .get("artifacts")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    artifacts.insert("root".to_string(), json!(audio_dir));
    artifacts.insert("wav".to_string(), json!(approved_wav_path));
    let mut warnings = data
        .get("warnings")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let mut failures = Vec::new();
    if analyze {
        let analysis_path = approved_wav_path.with_extension("analysis.json");
        match analyze_wav_file_with_options(&approved_wav_path, AudioAnalysisOptions::default()) {
            Ok(analysis) => {
                if let Err(error) = write_audio_analysis_artifact(&analysis_path, &analysis) {
                    return live_audio_artifact_error(command, &analysis_path, error);
                }
                artifacts.insert("analysis".to_string(), json!(analysis_path));
                if let Some(analysis_warnings) = analysis.get("warnings").and_then(Value::as_array)
                {
                    warnings.extend(analysis_warnings.iter().cloned());
                }
                if strict_warnings
                    && let Some(warning_ids) = analysis.get("warning_ids").and_then(Value::as_array)
                {
                    failures.extend(warning_ids.iter().filter_map(Value::as_str).map(|id| {
                        FailureDetail {
                            kind: id.to_string(),
                            command: Some(command.to_string()),
                            path: Some(approved_wav_path.to_string_lossy().into_owned()),
                            severity: Some("warning".to_string()),
                            expected: Some(json!("no audio analysis warnings")),
                            actual: Some(json!(id)),
                            ..FailureDetail::default()
                        }
                    }));
                }
                data.insert("analysis".to_string(), analysis);
            }
            Err(error) => {
                failures.push(FailureDetail {
                    kind: "audio_analysis.invalid_wav".to_string(),
                    command: Some(command.to_string()),
                    path: Some(approved_wav_path.to_string_lossy().into_owned()),
                    message: Some(error.message),
                    expected: Some(json!("valid bounded WAV analysis input")),
                    actual: Some(json!(error.code)),
                    ..FailureDetail::default()
                });
            }
        }
    }
    // `--out` is a CLI-controlled copy destination: the canonical artifact stays under audio_dir
    // (reported as `artifacts.wav`/`artifacts.analysis`) and the copy is reported as `artifacts.out`.
    if let Some(out) = out {
        if let Some(parent) = out.parent()
            && let Err(error) = fs::create_dir_all(parent)
        {
            return live_audio_artifact_error(command, parent, error);
        }
        if let Err(error) = fs::copy(&approved_wav_path, out) {
            return live_audio_artifact_error(command, out, error);
        }
        artifacts.insert("out".to_string(), json!(out));
        if artifacts.contains_key("analysis") {
            let out_analysis = out.with_extension("analysis.json");
            if let Err(error) =
                fs::copy(approved_wav_path.with_extension("analysis.json"), &out_analysis)
            {
                return live_audio_artifact_error(command, &out_analysis, error);
            }
            artifacts.insert("out_analysis".to_string(), json!(out_analysis));
        }
    }
    data.insert("session_id".to_string(), json!(record.session_id));
    data.insert("artifact_dir".to_string(), json!(record.artifact_dir));
    data.insert("audio_dir".to_string(), json!(audio_dir));
    data.insert(
        "effective_duration_seconds".to_string(),
        effective_duration_seconds,
    );
    data.insert("artifacts".to_string(), Value::Object(artifacts));
    data.insert("warnings".to_string(), json!(warnings));
    if data.get("stop_condition_timed_out").and_then(Value::as_bool) == Some(true) {
        failures.push(FailureDetail {
            kind: "audio_capture.stop_condition_timeout".to_string(),
            command: Some(command.to_string()),
            message: Some("audio capture stop_after_state did not match before timeout".to_string()),
            expected: Some(json!("stop_after_state matched before effective duration elapsed")),
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
            format!("{command} completed")
        } else {
            format!("{command} failed")
        },
        data,
        failures,
        exit_code: if ok { 0 } else { 1 },
    }
}

fn attach_live_audio_retry_metadata(
    mut result: CommandResult,
    attempts: usize,
    max_attempts: usize,
    retry_failures: &[Value],
) -> CommandResult {
    if !retry_failures.is_empty() {
        result.data.insert(
            "retry".to_string(),
            live_command_retry_metadata(attempts, max_attempts, retry_failures, result.ok),
        );
    }
    result
}

fn validate_live_audio_wav_path(
    command: &str,
    audio_dir: &Path,
    requested_wav_path: &Path,
    returned_wav_path: &Path,
) -> Result<PathBuf, FailureDetail> {
    let returned = returned_wav_path
        .canonicalize()
        .map_err(|error| FailureDetail {
            kind: "audio_capture.failed".to_string(),
            command: Some(command.to_string()),
            path: Some(returned_wav_path.to_string_lossy().into_owned()),
            message: Some(error.to_string()),
            expected: Some(json!("existing WAV artifact written by the live runtime")),
            actual: Some(json!("missing")),
            ..FailureDetail::default()
        })?;
    let approved_root = if requested_wav_path == returned_wav_path {
        requested_wav_path
            .parent()
            .unwrap_or(audio_dir)
            .canonicalize()
            .unwrap_or_else(|_| audio_dir.to_path_buf())
    } else {
        audio_dir
            .canonicalize()
            .unwrap_or_else(|_| audio_dir.to_path_buf())
    };
    if returned.extension().and_then(|ext| ext.to_str()) != Some("wav")
        || !returned.starts_with(&approved_root)
    {
        return Err(FailureDetail {
            kind: "invalid_params".to_string(),
            command: Some(command.to_string()),
            path: Some(returned.to_string_lossy().into_owned()),
            message: Some("live audio artifact path escaped the approved output root".to_string()),
            expected: Some(json!(approved_root)),
            actual: Some(json!(returned)),
            ..FailureDetail::default()
        });
    }
    Ok(returned)
}

fn live_audio_unavailable_result(command: &str, failure: FailureDetail) -> CommandResult {
    let mut warnings = Vec::new();
    if command == "audio capture" {
        warnings.push(json!({
            "id": "audio_capture.unavailable",
            "message": failure.message.clone().unwrap_or_else(|| "live audio capture unavailable".to_string()),
        }));
    }
    CommandResult {
        ok: false,
        command: command.to_string(),
        message: format!("{command} failed"),
        data: Map::from_iter([("warnings".to_string(), json!(warnings))]),
        failures: vec![failure],
        exit_code: 1,
    }
}

fn live_audio_artifact_error(command: &str, path: &Path, error: std::io::Error) -> CommandResult {
    CommandResult {
        ok: false,
        command: command.to_string(),
        message: format!("{command} failed"),
        data: Map::new(),
        failures: vec![FailureDetail {
            kind: "artifact_write_failed".to_string(),
            command: Some(command.to_string()),
            path: Some(path.to_string_lossy().into_owned()),
            message: Some(error.to_string()),
            expected: Some(json!("writable live audio artifact path")),
            actual: Some(json!("OSError")),
            ..FailureDetail::default()
        }],
        exit_code: 1,
    }
}
