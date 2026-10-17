// Extracted from run.rs; included by mod.rs to preserve the original module namespace.

fn validate_snapshot_artifacts(
    command: &str,
    result_path: &Path,
    value: &Value,
    artifacts: &Map<String, Value>,
) -> Result<Map<String, Value>, FailureDetail> {
    let Some(snapshot_artifacts) = value.as_object() else {
        return Err(scenario_result_invalid_failure(
            command,
            result_path,
            "result.json field 'snapshot_artifacts' must be an object",
            json!("JSON object"),
            json!(python_type_name(value)),
            artifacts,
        ));
    };

    let mut validated = Map::new();
    let mut invalid_values = Map::new();
    for (snapshot_name, snapshot_value) in snapshot_artifacts {
        let Some(snapshot_entry) = snapshot_value.as_object() else {
            invalid_values.insert(
                snapshot_name.clone(),
                json!(python_type_name(snapshot_value)),
            );
            continue;
        };
        let mut invalid_entry = Map::new();
        for (artifact_name, path_value) in snapshot_entry {
            if !path_value.is_string() {
                invalid_entry.insert(artifact_name.clone(), json!(python_type_name(path_value)));
            }
        }
        if invalid_entry.is_empty() {
            validated.insert(snapshot_name.clone(), Value::Object(snapshot_entry.clone()));
        } else {
            invalid_values.insert(snapshot_name.clone(), Value::Object(invalid_entry));
        }
    }

    if !invalid_values.is_empty() {
        return Err(scenario_result_invalid_failure(
            command,
            result_path,
            "result.json snapshot_artifacts entries must contain only string values",
            json!("JSON object of string path values"),
            Value::Object(invalid_values),
            artifacts,
        ));
    }
    Ok(validated)
}

fn validate_audio_artifacts(
    command: &str,
    result_path: &Path,
    raw_audio_artifacts: Option<&Value>,
    raw_audio_analysis: Option<&Value>,
    artifacts: &Map<String, Value>,
) -> Result<(Map<String, Value>, Map<String, Value>), FailureDetail> {
    let Some(raw_audio_artifacts) = raw_audio_artifacts else {
        return Ok((Map::new(), Map::new()));
    };
    let Some(audio_artifact_map) = raw_audio_artifacts.as_object() else {
        return Err(scenario_result_invalid_failure(
            command,
            result_path,
            "result.json field 'audio_artifacts' must be an object",
            json!("JSON object"),
            json!(python_type_name(raw_audio_artifacts)),
            artifacts,
        ));
    };
    match raw_audio_analysis {
        Some(Value::Object(_)) => {}
        Some(value) => {
            return Err(scenario_result_invalid_failure(
                command,
                result_path,
                "result.json field 'audio_analysis' must be an object when present",
                json!("JSON object"),
                json!(python_type_name(value)),
                artifacts,
            ));
        }
        None => {}
    };
    let Some(audio_dir) = artifacts
        .get("audio_dir")
        .and_then(Value::as_str)
        .map(PathBuf::from)
    else {
        return Err(scenario_result_invalid_failure(
            command,
            result_path,
            "result.json audio_artifacts require artifacts.audio_dir",
            json!("artifact map containing audio_dir"),
            json!("missing audio_dir"),
            artifacts,
        ));
    };
    let audio_root = audio_dir.canonicalize().map_err(|error| {
        scenario_result_invalid_failure(
            command,
            result_path,
            &format!("could not resolve artifacts.audio_dir: {error}"),
            json!("existing controlled audio artifact directory"),
            json!(audio_dir.to_string_lossy().into_owned()),
            artifacts,
        )
    })?;

    let mut validated_artifacts = Map::new();
    let mut validated_analysis = Map::new();
    for (name, value) in audio_artifact_map {
        let Some(entry) = value.as_object() else {
            return Err(scenario_result_invalid_failure(
                command,
                result_path,
                "result.json audio_artifacts entries must be objects",
                json!("JSON object with string wav path"),
                json!({ name: python_type_name(value) }),
                artifacts,
            ));
        };
        let wav_value = entry.get("wav").ok_or_else(|| {
            scenario_result_invalid_failure(
                command,
                result_path,
                "result.json audio_artifacts entries must include a wav path",
                json!("audio artifact object with string field 'wav'"),
                json!(name),
                artifacts,
            )
        })?;
        let Some(wav_path_str) = wav_value.as_str() else {
            return Err(scenario_result_invalid_failure(
                command,
                result_path,
                "result.json audio_artifacts wav values must be strings",
                json!("JSON string path"),
                json!({ name: python_type_name(wav_value) }),
                artifacts,
            ));
        };
        let wav_path = PathBuf::from(wav_path_str);
        if wav_path
            .components()
            .any(|component| matches!(component, Component::ParentDir))
        {
            return Err(audio_path_outside_failure(
                command,
                result_path,
                name,
                &wav_path,
                &audio_root,
                artifacts,
            ));
        }
        if !wav_path.is_file() {
            return Err(scenario_result_invalid_failure(
                command,
                result_path,
                "result.json audio_artifacts wav file was not found",
                json!("existing WAV file under artifacts.audio_dir"),
                json!({ name: wav_path.to_string_lossy().into_owned() }),
                artifacts,
            ));
        }
        let wav_canonical = wav_path.canonicalize().map_err(|error| {
            scenario_result_invalid_failure(
                command,
                result_path,
                &format!("could not resolve audio artifact WAV path: {error}"),
                json!("existing WAV file under artifacts.audio_dir"),
                json!({ name: wav_path.to_string_lossy().into_owned() }),
                artifacts,
            )
        })?;
        if !wav_canonical.starts_with(&audio_root) {
            return Err(audio_path_outside_failure(
                command,
                result_path,
                name,
                &wav_path,
                &audio_root,
                artifacts,
            ));
        }
        let analysis_path = audio_root.join(format!("{name}.analysis.json"));
        let analysis = match audio::analyze_wav_file(&wav_canonical) {
            Ok(value) => value,
            Err(error) => {
                return Err(scenario_result_invalid_failure(
                    command,
                    result_path,
                    &format!(
                        "audio artifact '{name}' is not a valid analyzable WAV: {}",
                        error.message
                    ),
                    json!("valid RIFF/WAVE audio artifact with non-empty data"),
                    json!(error.code),
                    artifacts,
                ));
            }
        };
        audio::write_audio_analysis_artifact(&analysis_path, &analysis).map_err(|error| {
            scenario_result_invalid_failure(
                command,
                result_path,
                &format!("could not write audio analysis artifact for '{name}': {error}"),
                json!("analysis JSON written under artifacts.audio_dir"),
                json!(analysis_path.to_string_lossy().into_owned()),
                artifacts,
            )
        })?;

        let mut entry = entry.clone();
        entry.insert(
            "wav".to_string(),
            json!(wav_canonical.to_string_lossy().into_owned()),
        );
        entry.insert(
            "analysis".to_string(),
            json!(analysis_path.to_string_lossy().into_owned()),
        );
        validated_artifacts.insert(name.clone(), Value::Object(entry));
        validated_analysis.insert(name.clone(), analysis);
    }
    Ok((validated_artifacts, validated_analysis))
}

fn audio_path_outside_failure(
    command: &str,
    result_path: &Path,
    name: &str,
    wav_path: &Path,
    audio_root: &Path,
    artifacts: &Map<String, Value>,
) -> FailureDetail {
    scenario_result_invalid_failure(
        command,
        result_path,
        "result.json audio_artifacts wav paths must stay under artifacts.audio_dir",
        json!(audio_root.to_string_lossy().into_owned()),
        json!({ name: wav_path.to_string_lossy().into_owned() }),
        artifacts,
    )
}

fn validate_runtime_failures(
    command: &str,
    result_path: &Path,
    raw_failures: Option<&Value>,
    raw_ok: bool,
    artifacts: &Map<String, Value>,
) -> Result<Vec<FailureDetail>, FailureDetail> {
    let raw_failures = match raw_failures {
        Some(Value::Array(value)) => value,
        Some(value) => {
            return Err(scenario_result_invalid_failure(
                command,
                result_path,
                "result.json field 'failures' must be an array",
                json!("JSON array"),
                json!(python_type_name(value)),
                artifacts,
            ));
        }
        None => return Ok(Vec::new()),
    };

    let mut failures = Vec::new();
    for failure in raw_failures {
        let Some(failure_object) = failure.as_object() else {
            return Err(scenario_result_invalid_failure(
                command,
                result_path,
                "result.json failures entries must be objects",
                json!("JSON object entries in failures array"),
                json!([python_type_name(failure)]),
                artifacts,
            ));
        };
        for field_name in ["type", "message", "path", "target", "severity"] {
            if let Some(value) = failure_object.get(field_name) {
                if !value.is_null() && !value.is_string() {
                    let mut actual = Map::new();
                    actual.insert(field_name.to_string(), json!(python_type_name(value)));
                    return Err(scenario_result_invalid_failure(
                        command,
                        result_path,
                        &format!("result.json failure field '{field_name}' must be a string"),
                        json!("JSON string"),
                        Value::Object(actual),
                        artifacts,
                    ));
                }
            }
        }
        if let Some(value) = failure_object.get("artifacts") {
            let Some(artifact_map) = value.as_object() else {
                return Err(scenario_result_invalid_failure(
                    command,
                    result_path,
                    "result.json failure field 'artifacts' must be an object",
                    json!("JSON object of string paths"),
                    json!(python_type_name(value)),
                    artifacts,
                ));
            };
            let mut invalid_artifacts = Map::new();
            for (key, value) in artifact_map {
                if !value.is_string() {
                    invalid_artifacts.insert(key.clone(), json!(python_type_name(value)));
                }
            }
            if !invalid_artifacts.is_empty() {
                return Err(scenario_result_invalid_failure(
                    command,
                    result_path,
                    "result.json failure field 'artifacts' must contain only string values",
                    json!("JSON object of string paths"),
                    Value::Object(invalid_artifacts),
                    artifacts,
                ));
            }
        }
        failures.push(classify_failure_detail(
            failure_with_artifacts(
                runtime_failure_to_detail(command, failure_object),
                artifacts,
            ),
            None,
        ));
    }

    if !raw_ok && failures.is_empty() {
        return Err(scenario_result_invalid_failure(
            command,
            result_path,
            "result.json reported a failed run without actionable failures",
            json!("non-empty failures array when ok is false"),
            json!([]),
            artifacts,
        ));
    }

    Ok(failures)
}


