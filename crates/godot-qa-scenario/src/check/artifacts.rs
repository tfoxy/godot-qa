use super::{
    BatchEvidence, CHECK_PROFILE_MAX_FRAME_SAMPLES, CHECK_PROFILE_TOP_SCRIPT_METHODS,
    CheckArtifacts, CheckTestArtifacts, CheckTestExecutionMode, DEFAULT_SLOW_SUMMARY_THRESHOLD_MS,
    ManifestTestEntry, SLOW_SUMMARY_LIMIT, TestManifest,
};
use godot_qa_core::config::ProjectConfig;
use godot_qa_core::profile;
use godot_qa_core::results::{CommandResult, FailureDetail};
use serde_json::{Map, Value, json};
use std::collections::HashMap;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

pub(super) fn write_batch_summary_artifact(
    check_artifacts: &CheckArtifacts,
    payloads: &[Value],
    failures: &mut Vec<FailureDetail>,
    command: &str,
    process_result: Option<&godot_qa_process::GodotRunResult>,
    logs: Option<&Value>,
) {
    let slow_summary = slow_check_summary_json(
        payloads,
        DEFAULT_SLOW_SUMMARY_THRESHOLD_MS,
        None,
        &check_artifacts.slow_summary_path,
    );
    let process = process_result.map(|run| {
        json!({
            "command": run.command,
            "returncode": run.returncode,
            "termination": run.termination,
            "timing": run.process_timing,
            "artifacts": {
                "logs": check_artifacts.batch_logs_path,
                "progress": check_artifacts.progress_path,
            }
        })
    });
    if let Err(error) = write_json(
        &check_artifacts.batch_summary_path,
        &json!({
            "ok": failures.is_empty(),
            "execution_mode": "batch",
            "test_count": payloads.len(),
            "tests": payloads,
            "failure_count": failures.len(),
            "failures": failures,
            "process": process,
            "logs": logs,
            "artifacts": check_artifacts.batch_json(),
            "slow_summary": slow_summary,
        }),
    ) {
        failures.push(required_artifact_write_failure(
            command,
            &check_artifacts.batch_summary_path,
            &error,
            check_artifacts,
        ));
    }
}

pub(super) fn slow_check_summary_json(
    payloads: &[Value],
    threshold_ms: u64,
    manifest: Option<&TestManifest>,
    artifact_path: &Path,
) -> Value {
    let test_thresholds = manifest
        .map(|manifest| {
            manifest
                .entries
                .iter()
                .map(|entry| (entry.id.as_str(), entry.slow_summary_threshold_ms))
                .collect::<HashMap<_, _>>()
        })
        .unwrap_or_default();
    let mut slow_tests = payloads
        .iter()
        .filter_map(|payload| {
            let duration_ms = payload.get("duration_ms").and_then(Value::as_u64)?;
            let entry_threshold_ms = payload
                .get("id")
                .and_then(Value::as_str)
                .and_then(|id| test_thresholds.get(id).copied())
                .unwrap_or(threshold_ms);
            if duration_ms < entry_threshold_ms {
                return None;
            }
            Some(json!({
                "id": payload.get("id").cloned().unwrap_or(Value::Null),
                "kind": payload.get("kind").cloned().unwrap_or(Value::Null),
                "target": payload.get("target").cloned().unwrap_or(Value::Null),
                "ok": payload.get("ok").cloned().unwrap_or(Value::Null),
                "duration_ms": duration_ms,
                "threshold_ms": entry_threshold_ms,
                "artifacts": payload.get("artifacts").cloned().unwrap_or_else(|| json!({})),
            }))
        })
        .collect::<Vec<_>>();
    slow_tests.sort_by(|left, right| {
        right
            .get("duration_ms")
            .and_then(Value::as_u64)
            .cmp(&left.get("duration_ms").and_then(Value::as_u64))
    });
    slow_tests.truncate(SLOW_SUMMARY_LIMIT);
    json!({
        "threshold_ms": threshold_ms,
        "limit": SLOW_SUMMARY_LIMIT,
        "slow_count": slow_tests.len(),
        "slow_tests": slow_tests,
        "artifacts": {
            "slow_summary": artifact_path,
        }
    })
}

pub(super) fn check_profile_summary_json(payloads: &[Value], artifacts: &CheckArtifacts) -> Value {
    let mut frame_samples = Vec::new();
    let mut monitor_samples = Vec::new();
    let mut unsupported_monitors = Vec::new();
    let mut per_test_timings = Vec::new();
    let mut top_script_methods = Vec::new();
    for payload in payloads {
        let test_id = payload.get("id").cloned().unwrap_or(Value::Null);
        if let Some(profile) = payload.get("profile").and_then(Value::as_object) {
            frame_samples.extend(
                profile
                    .get("frame_times_ms")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .filter_map(Value::as_f64),
            );
            monitor_samples.extend(
                profile
                    .get("monitor_samples")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .cloned(),
            );
            unsupported_monitors.extend(
                profile
                    .get("unsupported_monitors")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .cloned(),
            );
            top_script_methods.push(profile::top_script_methods_or_empty(
                profile.get("top_script_methods"),
            ));
            for timing in profile
                .get("step_timings")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
            {
                let mut timing = timing.clone();
                if let Some(timing_object) = timing.as_object_mut() {
                    timing_object
                        .entry("test_id".to_string())
                        .or_insert(test_id.clone());
                }
                per_test_timings.push(timing);
            }
        }
        per_test_timings.push(json!({
            "test_id": test_id,
            "label": "check_test_total",
            "duration_ms": payload.get("duration_ms").cloned().unwrap_or(Value::Null),
        }));
    }
    let percentiles = profile::profile_percentiles(&frame_samples);
    let mut slow_tests = payloads
        .iter()
        .filter_map(|payload| {
            Some(json!({
                "id": payload.get("id")?.clone(),
                "ok": payload.get("ok").cloned().unwrap_or(Value::Null),
                "duration_ms": payload.get("duration_ms").cloned().unwrap_or(Value::Null),
            }))
        })
        .collect::<Vec<_>>();
    slow_tests.sort_by(|left, right| {
        right
            .get("duration_ms")
            .and_then(Value::as_u64)
            .cmp(&left.get("duration_ms").and_then(Value::as_u64))
    });
    slow_tests.truncate(SLOW_SUMMARY_LIMIT);
    json!({
        "frame_time_ms": profile::profile_percentiles_json(&percentiles),
        "frame_time_percentiles_ms": profile::profile_percentiles_json(&percentiles),
        "monitor_counters": profile::monitor_counter_summaries(&monitor_samples),
        "monitor_sample_count": monitor_samples.len(),
        "unsupported_monitors": unsupported_monitors,
        "top_script_methods": profile::aggregate_top_script_methods(&top_script_methods, CHECK_PROFILE_TOP_SCRIPT_METHODS),
        "sample_limits": {
            "max_frame_samples": CHECK_PROFILE_MAX_FRAME_SAMPLES,
            "top_script_methods": CHECK_PROFILE_TOP_SCRIPT_METHODS,
        },
        "slow_tests": slow_tests,
        "per_test_timings": per_test_timings.iter().take(CHECK_PROFILE_MAX_FRAME_SAMPLES).cloned().collect::<Vec<_>>(),
        "per_test_timing_count": per_test_timings.len(),
        "artifacts": {
            "profile": artifacts.profile_path,
        },
    })
}

pub(super) fn check_profile_artifact_json(
    config: &ProjectConfig,
    manifest_path: &Path,
    selection: &Value,
    execution_mode: CheckTestExecutionMode,
    artifacts: &CheckArtifacts,
    payloads: &[Value],
) -> Value {
    let mut frame_samples = Vec::new();
    let mut monitor_samples = Vec::new();
    let mut unsupported_monitors = Vec::new();
    let mut per_test_timings = Vec::new();
    let mut top_script_methods = Vec::new();
    for payload in payloads {
        let test_id = payload.get("id").cloned().unwrap_or(Value::Null);
        if let Some(profile) = payload.get("profile").and_then(Value::as_object) {
            frame_samples.extend(
                profile
                    .get("frame_times_ms")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .filter_map(Value::as_f64),
            );
            monitor_samples.extend(
                profile
                    .get("monitor_samples")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .cloned(),
            );
            unsupported_monitors.extend(
                profile
                    .get("unsupported_monitors")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .cloned(),
            );
            top_script_methods.push(profile::top_script_methods_or_empty(
                profile.get("top_script_methods"),
            ));
            per_test_timings.extend(
                profile
                    .get("step_timings")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .cloned(),
            );
        }
        per_test_timings.push(json!({
            "test_id": test_id,
            "label": "check_test_total",
            "duration_ms": payload.get("duration_ms").cloned().unwrap_or(Value::Null),
        }));
    }
    let percentiles = profile::profile_percentiles(&frame_samples);
    json!({
        "project_root": config.project_root,
        "manifest": manifest_path,
        "selection": selection,
        "execution_mode": match execution_mode {
            CheckTestExecutionMode::Isolated => "isolated",
            CheckTestExecutionMode::Batch => "batch",
        },
        "artifacts": artifacts.to_json(),
        "sample_limits": {
            "max_frame_samples": CHECK_PROFILE_MAX_FRAME_SAMPLES,
            "top_script_methods": CHECK_PROFILE_TOP_SCRIPT_METHODS,
        },
        "frame_time_percentiles_ms": profile::profile_percentiles_json(&percentiles),
        "monitor_counters": profile::monitor_counter_summaries(&monitor_samples),
        "unsupported_monitors": unsupported_monitors,
        "top_script_methods": profile::aggregate_top_script_methods(&top_script_methods, CHECK_PROFILE_TOP_SCRIPT_METHODS),
        "per_test_timings": per_test_timings,
        "frame_times_ms": profile::bounded_frame_values(&frame_samples, CHECK_PROFILE_MAX_FRAME_SAMPLES),
        "monitor_samples": monitor_samples.into_iter().take(CHECK_PROFILE_MAX_FRAME_SAMPLES).collect::<Vec<_>>(),
        "tests": payloads.iter().map(|payload| json!({
            "id": payload.get("id").cloned().unwrap_or(Value::Null),
            "ok": payload.get("ok").cloned().unwrap_or(Value::Null),
            "duration_ms": payload.get("duration_ms").cloned().unwrap_or(Value::Null),
        })).collect::<Vec<_>>(),
    })
}

pub(super) fn write_check_profile_artifact(path: &Path, value: &Value) -> std::io::Result<()> {
    write_json(path, value)
}

pub(super) fn append_check_progress_event(
    run_progress_path: &Path,
    test_progress_path: Option<&Path>,
    test_entry: &ManifestTestEntry,
    event: &str,
    started_at: &str,
    elapsed_ms: Option<u64>,
    ok: Option<bool>,
    failure_count: Option<usize>,
    artifacts: &CheckTestArtifacts,
) -> std::io::Result<()> {
    let mut payload = Map::from_iter([
        ("event".to_string(), json!(event)),
        ("id".to_string(), json!(test_entry.id)),
        ("kind".to_string(), json!(test_entry.kind)),
        ("target".to_string(), json!(test_entry.target)),
        ("started_at".to_string(), json!(started_at)),
        ("elapsed_ms".to_string(), json!(elapsed_ms)),
        ("ok".to_string(), json!(ok)),
        ("failure_count".to_string(), json!(failure_count)),
        ("artifacts".to_string(), artifacts.to_json()),
    ]);
    if event == "start" {
        payload.insert("elapsed_ms".to_string(), Value::Null);
        payload.insert("ok".to_string(), Value::Null);
        payload.insert("failure_count".to_string(), Value::Null);
    }
    let line = serde_json::to_string(&Value::Object(payload)).map_err(std::io::Error::other)?;
    append_jsonl(run_progress_path, &line)?;
    if let Some(test_progress_path) = test_progress_path {
        append_jsonl(test_progress_path, &line)?;
    }
    Ok(())
}

fn append_jsonl(path: &Path, line: &str) -> std::io::Result<()> {
    let mut file = OpenOptions::new().create(true).append(true).open(path)?;
    writeln!(file, "{line}")?;
    Ok(())
}

pub(super) fn prepare_check_artifacts(
    artifacts_root: &Path,
    project_root: &Path,
    command_line: &str,
) -> std::io::Result<CheckArtifacts> {
    let check_root = artifacts_root.join("check");
    fs::create_dir_all(&check_root)?;
    let base_run_id = unique_run_id();
    let mut candidate_run_id = base_run_id.clone();
    let mut suffix = 1;
    let root_dir = loop {
        let candidate = check_root.join(&candidate_run_id);
        match fs::create_dir(&candidate) {
            Ok(()) => break candidate,
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                candidate_run_id = format!("{base_run_id}-{suffix:02}");
                suffix += 1;
            }
            Err(error) => return Err(error),
        }
    };
    let import_dir = root_dir.join("import");
    let tests_dir = root_dir.join("tests");
    let batch_dir = root_dir.join("batch");
    fs::create_dir_all(&import_dir)?;
    fs::create_dir_all(&tests_dir)?;
    fs::create_dir_all(&batch_dir)?;
    let artifacts = CheckArtifacts {
        run_id: root_dir
            .file_name()
            .and_then(|value| value.to_str())
            .unwrap_or_default()
            .to_string(),
        root_dir: root_dir.clone(),
        meta_path: root_dir.join("meta.json"),
        result_path: root_dir.join("result.json"),
        progress_path: root_dir.join("progress.jsonl"),
        import_logs_path: import_dir.join("logs.txt"),
        tests_dir,
        batch_request_path: batch_dir.join("request.json"),
        batch_logs_path: batch_dir.join("logs.txt"),
        batch_summary_path: batch_dir.join("summary.json"),
        slow_summary_path: root_dir.join("slow-summary.json"),
        profile_path: root_dir.join("profile.json"),
    };
    write_json(
        &artifacts.meta_path,
        &json!({
            "tool": "godot-qa",
            "project_root": project_root,
            "command": command_line,
            "started_at": started_at_now(),
        }),
    )?;
    Ok(artifacts)
}

pub(super) fn prepare_test_artifacts(
    check_artifacts: &CheckArtifacts,
    test_entry: &ManifestTestEntry,
) -> std::io::Result<CheckTestArtifacts> {
    let root_dir = check_artifacts.tests_dir.join(&test_entry.artifact_slug);
    fs::create_dir_all(&root_dir)?;
    let artifacts = CheckTestArtifacts {
        root_dir: root_dir.clone(),
        meta_path: root_dir.join("meta.json"),
        request_path: root_dir.join("request.json"),
        result_path: root_dir.join("result.json"),
        logs_path: root_dir.join("logs.txt"),
        progress_path: root_dir.join("progress.jsonl"),
    };
    let mut meta = json!({
        "id": test_entry.id,
        "kind": test_entry.kind,
        "target": test_entry.target,
        "tags": test_entry.tags,
    });
    if let Some(frame_pacing) = &test_entry.frame_pacing
        && let Some(meta) = meta.as_object_mut()
    {
        meta.insert("frame_pacing".to_string(), frame_pacing.to_json());
    }
    write_json(&artifacts.meta_path, &meta)?;
    Ok(artifacts)
}

pub(super) fn failure_artifacts(
    artifacts: &CheckTestArtifacts,
    batch_evidence: Option<BatchEvidence<'_>>,
) -> Option<Map<String, Value>> {
    let mut map = value_as_object_map(artifacts.to_json()).unwrap_or_default();
    if let Some(batch_evidence) = batch_evidence {
        map.insert("batch_logs".to_string(), json!(batch_evidence.logs_path));
        map.insert(
            "batch_summary".to_string(),
            json!(batch_evidence.summary_path),
        );
    }
    Some(map)
}

pub(super) fn ensure_writable_artifact_dir(path: &Path, command: &str) -> Option<FailureDetail> {
    let error = if path.exists() && !path.is_dir() {
        Some(std::io::Error::other(format!(
            "{} is not a directory",
            path.display()
        )))
    } else if let Err(error) = fs::create_dir_all(path) {
        Some(error)
    } else {
        let probe_path = path.join(".godot-qa-write-test");
        match fs::write(&probe_path, b"") {
            Ok(_) => {
                let _ = fs::remove_file(probe_path);
                None
            }
            Err(error) => Some(error),
        }
    };
    error.map(|error| FailureDetail {
        kind: "artifact_dir_not_writable".to_string(),
        command: Some(command.to_string()),
        path: Some(path.to_string_lossy().into_owned()),
        message: Some(error.to_string()),
        expected: Some(json!("writable artifact directory")),
        actual: Some(json!(if path.exists() && !path.is_dir() {
            "NotADirectoryError"
        } else {
            match error.kind() {
                std::io::ErrorKind::PermissionDenied => "PermissionError",
                _ => "OSError",
            }
        })),
        ..FailureDetail::default()
    })
}

pub(super) fn artifact_write_failed(
    command: &str,
    path: &Path,
    message: &str,
    data: Map<String, Value>,
) -> CommandResult {
    CommandResult {
        ok: false,
        command: command.to_string(),
        message: "project check failed".to_string(),
        data,
        failures: vec![FailureDetail {
            kind: "artifact_write_failed".to_string(),
            command: Some(command.to_string()),
            path: Some(path.to_string_lossy().into_owned()),
            message: Some(message.to_string()),
            expected: Some(json!(
                "artifact metadata written under the artifact directory"
            )),
            actual: Some(json!("OSError")),
            ..FailureDetail::default()
        }],
        exit_code: 1,
    }
}

pub(super) fn write_json(path: &Path, payload: &Value) -> std::io::Result<()> {
    fs::write(
        path,
        serde_json::to_string_pretty(payload).unwrap_or_else(|_| "{}".to_string()),
    )
}

pub(super) fn required_artifact_write_failure(
    command: &str,
    path: &Path,
    error: &std::io::Error,
    artifacts: &CheckArtifacts,
) -> FailureDetail {
    FailureDetail {
        kind: "artifact_write_failed".to_string(),
        command: Some(command.to_string()),
        path: Some(path.to_string_lossy().into_owned()),
        target: Some(path.to_string_lossy().into_owned()),
        message: Some(error.to_string()),
        expected: Some(json!("required check artifact written successfully")),
        actual: Some(json!("OSError")),
        artifacts: value_as_object_map(artifacts.to_json()),
        ..FailureDetail::default()
    }
}

pub(super) fn persist_check_result(
    artifacts: &CheckArtifacts,
    mut result: CommandResult,
) -> CommandResult {
    if let Err(error) = fs::write(
        &artifacts.result_path,
        serde_json::to_string_pretty(&result).unwrap_or_else(|_| "{}".to_string()),
    ) {
        result.failures.push(required_artifact_write_failure(
            &result.command,
            &artifacts.result_path,
            &error,
            artifacts,
        ));
        result.ok = false;
        result.exit_code = 1;
        result.message = "project check failed".to_string();
    }
    result
}

pub(super) fn normalize_existing_or_join(path: &Path) -> PathBuf {
    path.canonicalize()
        .unwrap_or_else(|_| path.components().collect())
}

pub(super) fn slug_value(value: &str) -> String {
    let cleaned = value
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() {
                character.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect::<String>();
    cleaned.trim_matches('-').to_string()
}

pub(super) fn started_at_now() -> String {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| format!("{}Z", duration.as_secs()))
        .unwrap_or_else(|_| "0Z".to_string())
}

fn unique_run_id() -> String {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis().to_string())
        .unwrap_or_else(|_| "0".to_string())
}

pub(super) fn value_as_object_map(value: Value) -> Option<Map<String, Value>> {
    value.as_object().cloned()
}
