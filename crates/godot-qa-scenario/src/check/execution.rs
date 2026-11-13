use super::artifacts::{
    append_check_progress_event, ensure_writable_artifact_dir, failure_artifacts,
    prepare_test_artifacts, started_at_now, value_as_object_map, write_batch_summary_artifact,
    write_json,
};
use super::result::{
    load_runtime_test_result, normalized_failure, validate_runtime_test_result_with_evidence,
};
use super::{
    BatchEvidence, CHECK_PROFILE_MAX_FRAME_SAMPLES, CheckArtifacts, CheckTestArtifacts,
    CheckTestRunOptions, ManifestTestEntry, TestManifest, batch_deadline,
};
use godot_qa_core::config::ProjectConfig;
use godot_qa_core::results::FailureDetail;
use serde_json::{Map, Value, json};
use std::fs;
use std::path::Path;
use std::time::{Duration, Instant};

pub(super) fn run_manifest_test(
    config: &ProjectConfig,
    test_entry: &ManifestTestEntry,
    check_artifacts: &CheckArtifacts,
    command: &str,
    options: CheckTestRunOptions,
) -> (Value, Vec<FailureDetail>) {
    let artifacts = match prepare_test_artifacts(check_artifacts, test_entry) {
        Ok(artifacts) => artifacts,
        Err(error) => {
            let failure = FailureDetail {
                kind: "artifact_write_failed".to_string(),
                command: Some(command.to_string()),
                path: Some(check_artifacts.tests_dir.to_string_lossy().into_owned()),
                target: Some(test_entry.id.clone()),
                message: Some(error.to_string()),
                expected: Some(json!(
                    "artifact metadata written under the artifact directory"
                )),
                actual: Some(json!("OSError")),
                ..FailureDetail::default()
            };
            return (
                json!({
                    "id": test_entry.id,
                    "kind": test_entry.kind,
                    "target": test_entry.target,
                    "tags": test_entry.tags,
                    "ok": false,
                    "duration_ms": Value::Null,
                    "runtime_errors": [],
                    "artifacts": {},
                    "failures": [failure.clone()],
                }),
                vec![failure],
            );
        }
    };

    let request = test_check_request(
        test_entry,
        &artifacts,
        &check_artifacts.run_id,
        command,
        options,
    );
    if let Err(error) = write_json(&artifacts.request_path, &request) {
        let failure = FailureDetail {
            kind: "artifact_write_failed".to_string(),
            command: Some(command.to_string()),
            path: Some(artifacts.request_path.to_string_lossy().into_owned()),
            target: Some(test_entry.id.clone()),
            message: Some(error.to_string()),
            expected: Some(json!(
                "artifact metadata written under the artifact directory"
            )),
            actual: Some(json!("OSError")),
            artifacts: value_as_object_map(artifacts.to_json()),
            ..FailureDetail::default()
        };
        return (
            json!({
                "id": test_entry.id,
                "kind": test_entry.kind,
                "target": test_entry.target,
                "tags": test_entry.tags,
                "ok": false,
                "duration_ms": Value::Null,
                "runtime_errors": [],
                "artifacts": artifacts.to_json(),
                "failures": [failure.clone()],
            }),
            vec![failure],
        );
    }

    let started_at = started_at_now();
    let progress_started = Instant::now();
    let _ = append_check_progress_event(
        &check_artifacts.progress_path,
        Some(&artifacts.progress_path),
        test_entry,
        "start",
        &started_at,
        None,
        None,
        None,
        &artifacts,
    );

    let effective_timeout = options.timeout_override.or(test_entry.timeout);
    let process_result = match godot_qa_process::launch_godot_check_with_timeout_and_fixed_fps(
        config.godot_binary.as_deref().unwrap_or_default(),
        &config.godot_launch_prefix,
        config.godot_joypad_isolation,
        &config.godot_args,
        &config.godot_project_args,
        &config.project_root,
        &artifacts.request_path,
        &artifacts.logs_path,
        effective_timeout,
        test_entry
            .frame_pacing
            .as_ref()
            .map(|frame_pacing| frame_pacing.fixed_fps),
    ) {
        Ok(run) => run,
        Err(failure) => {
            let normalized = normalized_failure(&test_entry.id, &artifacts, &failure);
            let _ = append_check_progress_event(
                &check_artifacts.progress_path,
                Some(&artifacts.progress_path),
                test_entry,
                "finish",
                &started_at,
                Some(progress_started.elapsed().as_millis() as u64),
                Some(false),
                Some(1),
                &artifacts,
            );
            return (
                json!({
                    "id": test_entry.id,
                    "kind": test_entry.kind,
                    "target": test_entry.target,
                    "tags": test_entry.tags,
                    "ok": false,
                    "duration_ms": Value::Null,
                    "runtime_errors": [],
                    "artifacts": artifacts.to_json(),
                    "failures": [normalized.clone()],
                }),
                vec![normalized],
            );
        }
    };

    if process_result.timed_out {
        let failure = check_test_timeout_failure(
            test_entry,
            &artifacts,
            &process_result,
            progress_started.elapsed().as_millis() as u64,
            effective_timeout,
            None,
        );
        let _ = append_check_progress_event(
            &check_artifacts.progress_path,
            Some(&artifacts.progress_path),
            test_entry,
            "finish",
            &started_at,
            Some(progress_started.elapsed().as_millis() as u64),
            Some(false),
            Some(1),
            &artifacts,
        );
        return (
            test_failure_payload(test_entry, artifacts.to_json(), &failure),
            vec![failure],
        );
    }

    let runtime_result = match load_runtime_test_result(test_entry, &artifacts, None) {
        Ok(result) => result,
        Err(failure) => {
            let _ = append_check_progress_event(
                &check_artifacts.progress_path,
                Some(&artifacts.progress_path),
                test_entry,
                "finish",
                &started_at,
                Some(progress_started.elapsed().as_millis() as u64),
                Some(false),
                Some(1),
                &artifacts,
            );
            return (
                json!({
                    "id": test_entry.id,
                    "kind": test_entry.kind,
                    "target": test_entry.target,
                    "tags": test_entry.tags,
                    "ok": false,
                    "duration_ms": Value::Null,
                    "runtime_errors": [],
                    "artifacts": artifacts.to_json(),
                    "failures": [failure.clone()],
                }),
                vec![failure],
            );
        }
    };

    let (mut payload, mut failures) = validate_runtime_test_result_with_evidence(
        test_entry,
        runtime_result,
        &artifacts,
        &process_result,
        None,
        options,
    );
    let (log_failures, log_warnings, ignored_count) = check_log_diagnostics(
        &artifacts.logs_path,
        &process_result,
        &config.log_ignore_patterns,
        Some(&test_entry.id),
        artifacts.to_json(),
    );
    failures.extend(log_failures);
    payload["ok"] = json!(payload["ok"] == true && failures.is_empty());
    payload["failures"] = json!(failures);
    payload["warnings"] = json!(log_warnings);
    payload["logs"] = json!({"ignored_count": ignored_count, "ignore_patterns_applied": config.log_ignore_patterns.len()});
    let _ = append_check_progress_event(
        &check_artifacts.progress_path,
        Some(&artifacts.progress_path),
        test_entry,
        "finish",
        &started_at,
        Some(progress_started.elapsed().as_millis() as u64),
        Some(failures.is_empty()),
        Some(failures.len()),
        &artifacts,
    );
    (payload, failures)
}

pub(super) fn check_log_diagnostics(
    logs: &Path,
    process: &godot_qa_process::GodotRunResult,
    ignore_patterns: &[String],
    target: Option<&str>,
    artifacts: Value,
) -> (Vec<FailureDetail>, Vec<FailureDetail>, usize) {
    let scan = godot_qa_core::godot_logs::scan_output(
        logs,
        &process.stdout,
        &process.stderr,
        ignore_patterns,
    );
    let (warnings, failures) = scan
        .diagnostics("check", logs, target)
        .into_iter()
        .map(|mut diagnostic| {
            diagnostic.artifacts = value_as_object_map(artifacts.clone());
            diagnostic
        })
        .partition(|diagnostic| diagnostic.severity.as_deref() == Some("warning"));
    (failures, warnings, scan.ignored_count)
}

pub(super) fn run_manifest_tests_batch(
    config: &ProjectConfig,
    manifest: &TestManifest,
    selected_indices: &[usize],
    check_artifacts: &CheckArtifacts,
    command: &str,
    options: CheckTestRunOptions,
) -> (Vec<Value>, Vec<FailureDetail>) {
    let pacing_groups = selected_batch_pacing_groups(manifest, selected_indices);
    if pacing_groups.len() > 1 {
        let mut payloads = Vec::new();
        let mut failures = Vec::new();
        for (group_index, (_, group_indices)) in pacing_groups.into_iter().enumerate() {
            let mut group_artifacts = check_artifacts.clone();
            group_artifacts.batch_request_path = check_artifacts
                .root_dir
                .join("batch")
                .join(format!("request-{group_index:02}.json"));
            group_artifacts.batch_logs_path = check_artifacts
                .root_dir
                .join("batch")
                .join(format!("logs-{group_index:02}.txt"));
            group_artifacts.batch_summary_path = check_artifacts
                .root_dir
                .join("batch")
                .join(format!("summary-{group_index:02}.json"));
            let (group_payloads, group_failures) = run_manifest_tests_batch(
                config,
                manifest,
                &group_indices,
                &group_artifacts,
                command,
                options,
            );
            payloads.extend(group_payloads);
            failures.extend(group_failures);
        }
        write_batch_summary_artifact(
            check_artifacts,
            &payloads,
            &mut failures,
            command,
            None,
            None,
        );
        return (payloads, failures);
    }

    let mut prepared = Vec::new();
    let mut payloads = Vec::new();
    let mut failures = Vec::new();
    for index in selected_indices {
        let test_entry = &manifest.entries[*index];
        match prepare_test_artifacts(check_artifacts, test_entry) {
            Ok(artifacts) => {
                let request = test_check_request(
                    test_entry,
                    &artifacts,
                    &check_artifacts.run_id,
                    command,
                    options,
                );
                if let Err(error) = write_json(&artifacts.request_path, &request) {
                    let failure = FailureDetail {
                        kind: "artifact_write_failed".to_string(),
                        command: Some(command.to_string()),
                        path: Some(artifacts.request_path.to_string_lossy().into_owned()),
                        target: Some(test_entry.id.clone()),
                        message: Some(error.to_string()),
                        expected: Some(json!(
                            "artifact metadata written under the artifact directory"
                        )),
                        actual: Some(json!("OSError")),
                        artifacts: value_as_object_map(artifacts.to_json()),
                        ..FailureDetail::default()
                    };
                    payloads.push(test_failure_payload(
                        test_entry,
                        artifacts.to_json(),
                        &failure,
                    ));
                    failures.push(failure);
                } else {
                    prepared.push((test_entry, artifacts, request));
                }
            }
            Err(error) => {
                let failure = FailureDetail {
                    kind: "artifact_write_failed".to_string(),
                    command: Some(command.to_string()),
                    path: Some(check_artifacts.tests_dir.to_string_lossy().into_owned()),
                    target: Some(test_entry.id.clone()),
                    message: Some(error.to_string()),
                    expected: Some(json!(
                        "artifact metadata written under the artifact directory"
                    )),
                    actual: Some(json!("OSError")),
                    ..FailureDetail::default()
                };
                payloads.push(test_failure_payload(test_entry, json!({}), &failure));
                failures.push(failure);
            }
        }
    }

    if prepared.is_empty() {
        write_batch_summary_artifact(
            check_artifacts,
            &payloads,
            &mut failures,
            command,
            None,
            None,
        );
        return (payloads, failures);
    }

    let batch_request = batch_check_request(&prepared, check_artifacts, command, options);
    if let Err(error) = write_json(&check_artifacts.batch_request_path, &batch_request) {
        let failure = FailureDetail {
            kind: "artifact_write_failed".to_string(),
            command: Some(command.to_string()),
            path: Some(
                check_artifacts
                    .batch_request_path
                    .to_string_lossy()
                    .into_owned(),
            ),
            message: Some(error.to_string()),
            expected: Some(json!("batch request written under the artifact directory")),
            actual: Some(json!("OSError")),
            artifacts: value_as_object_map(check_artifacts.batch_json()),
            ..FailureDetail::default()
        };
        for (test_entry, artifacts, _) in &prepared {
            let normalized = normalized_failure(&test_entry.id, artifacts, &failure);
            payloads.push(test_failure_payload(
                test_entry,
                artifacts.to_json(),
                &normalized,
            ));
            failures.push(normalized);
        }
        write_batch_summary_artifact(
            check_artifacts,
            &payloads,
            &mut failures,
            command,
            None,
            None,
        );
        return (payloads, failures);
    }

    let budgets = prepared
        .iter()
        .map(|(entry, _, _)| (*entry, options.timeout_override.or(entry.timeout)))
        .collect::<Vec<_>>();
    // Leave unbounded entries unbounded. Finite batches get startup/teardown slack.
    let effective_timeout = budgets
        .iter()
        .try_fold(Duration::from_secs(3), |sum, (_, budget)| {
            sum.checked_add((*budget)?)
        })
        .filter(|budget| Instant::now().checked_add(*budget).is_some());
    let mut deadline_monitor =
        batch_deadline::BatchDeadlineMonitor::new(&check_artifacts.progress_path, budgets);
    let batch_started = Instant::now();
    let batch_fixed_fps = prepared
        .first()
        .and_then(|(test_entry, _, _)| test_entry.frame_pacing.as_ref())
        .map(|frame_pacing| frame_pacing.fixed_fps);
    let process_result = match godot_qa_process::launch_godot_check_with_timeout_monitor(
        config.godot_binary.as_deref().unwrap_or_default(),
        &config.godot_launch_prefix,
        config.godot_joypad_isolation,
        &config.godot_args,
        &config.godot_project_args,
        &config.project_root,
        &check_artifacts.batch_request_path,
        &check_artifacts.batch_logs_path,
        effective_timeout,
        batch_fixed_fps,
        Some(&mut || deadline_monitor.poll()),
    ) {
        Ok(run) => run,
        Err(failure) => {
            for (test_entry, artifacts, _) in &prepared {
                let normalized = normalized_failure(&test_entry.id, artifacts, &failure);
                payloads.push(test_failure_payload(
                    test_entry,
                    artifacts.to_json(),
                    &normalized,
                ));
                failures.push(normalized);
            }
            write_batch_summary_artifact(
                check_artifacts,
                &payloads,
                &mut failures,
                command,
                None,
                None,
            );
            return (payloads, failures);
        }
    };

    let batch_evidence = BatchEvidence {
        logs_path: &check_artifacts.batch_logs_path,
        summary_path: &check_artifacts.batch_summary_path,
    };
    if process_result.timed_out {
        let runtime_results = prepared
            .iter()
            .map(|(entry, artifacts, _)| {
                load_runtime_test_result(entry, artifacts, Some(batch_evidence))
            })
            .collect::<Vec<_>>();
        let progress = fs::read_to_string(&check_artifacts.progress_path).unwrap_or_default();
        let progress_active_index = progress
            .lines()
            .rev()
            .filter_map(|line| serde_json::from_str::<Value>(line).ok())
            .find_map(|event| {
                let state = event["event"].as_str()?;
                if !matches!(state, "start" | "heartbeat" | "finish") {
                    return None;
                }
                prepared
                    .iter()
                    .position(|(entry, _, _)| {
                        event["id"] == entry.id
                            && event["kind"] == entry.kind
                            && event["target"] == entry.target
                    })
                    .map(|index| (index, state != "finish"))
            })
            .and_then(|(index, active)| active.then_some(index));
        let active_index = deadline_monitor
            .timed_out_index
            .or(progress_active_index)
            .or_else(|| {
                runtime_results
                    .iter()
                    .zip(&prepared)
                    .position(|(result, (entry, _, _))| {
                        !result.as_ref().is_ok_and(|value| {
                            value["id"] == entry.id
                                && value["kind"] == entry.kind
                                && value["target"] == entry.target
                        })
                    })
            });
        let mut validation_process_result = process_result.clone();
        validation_process_result.returncode = 0;
        for (index, ((test_entry, artifacts, _), runtime_result)) in
            prepared.iter().zip(runtime_results).enumerate()
        {
            if Some(index) == active_index {
                let failure = check_test_timeout_failure(
                    test_entry,
                    artifacts,
                    &process_result,
                    batch_started.elapsed().as_millis() as u64,
                    effective_timeout,
                    Some(batch_evidence),
                );
                payloads.push(test_failure_payload(
                    test_entry,
                    artifacts.to_json(),
                    &failure,
                ));
                failures.push(failure);
            } else {
                match runtime_result {
                    Ok(runtime_result) => {
                        let (payload, test_failures) = validate_runtime_test_result_with_evidence(
                            test_entry,
                            runtime_result,
                            artifacts,
                            &validation_process_result,
                            Some(batch_evidence),
                            options,
                        );
                        payloads.push(payload);
                        failures.extend(test_failures);
                    }
                    Err(failure)
                        if index < active_index.unwrap_or(prepared.len())
                            && artifacts.result_path.is_file() =>
                    {
                        payloads.push(test_failure_payload(
                            test_entry,
                            artifacts.to_json(),
                            &failure,
                        ));
                        failures.push(failure);
                    }
                    Err(_) => {}
                }
            }
        }
        if active_index.is_none() {
            // All tests finished; a shutdown hang must not replace a completed payload.
            let (test_entry, artifacts, _) = prepared.last().unwrap();
            let mut failure = check_test_timeout_failure(
                test_entry,
                artifacts,
                &process_result,
                batch_started.elapsed().as_millis() as u64,
                effective_timeout,
                Some(batch_evidence),
            );
            failure.message = Some(
                "Godot batch check exceeded the process timeout after all tests finished"
                    .to_string(),
            );
            failure.path = Some(
                check_artifacts
                    .batch_logs_path
                    .to_string_lossy()
                    .into_owned(),
            );
            failure.actual.as_mut().unwrap()["progress"] = json!(check_artifacts.progress_path);
            failure
                .artifacts
                .as_mut()
                .unwrap()
                .insert("progress".to_string(), json!(check_artifacts.progress_path));
            failures.push(failure);
        }
        write_batch_summary_artifact(
            check_artifacts,
            &payloads,
            &mut failures,
            command,
            Some(&process_result),
            None,
        );
        return (payloads, failures);
    }
    let (log_failures, log_warnings, log_ignored_count) = check_log_diagnostics(
        &check_artifacts.batch_logs_path,
        &process_result,
        &config.log_ignore_patterns,
        None,
        check_artifacts.batch_json(),
    );
    failures.extend(log_failures);
    let batch_log_evidence = json!({
        "warnings": log_warnings,
        "ignored_count": log_ignored_count,
        "ignore_patterns_applied": config.log_ignore_patterns.len(),
    });
    let mut validation_process_result = process_result.clone();
    validation_process_result.returncode = 0;
    for (test_entry, artifacts, _) in &prepared {
        match load_runtime_test_result(test_entry, artifacts, Some(batch_evidence)) {
            Ok(runtime_result) => {
                let (payload, test_failures) = validate_runtime_test_result_with_evidence(
                    test_entry,
                    runtime_result,
                    artifacts,
                    &validation_process_result,
                    Some(batch_evidence),
                    options,
                );
                payloads.push(payload);
                failures.extend(test_failures);
            }
            Err(failure) => {
                payloads.push(test_failure_payload(
                    test_entry,
                    artifacts.to_json(),
                    &failure,
                ));
                failures.push(failure);
            }
        }
    }
    if process_result.returncode != 0 && failures.is_empty() {
        if let Some((test_entry, artifacts, _)) = prepared.first() {
            let failure = FailureDetail {
                kind: "godot_run_failed".to_string(),
                command: Some(godot_qa_process::format_command(&process_result.command)),
                path: Some(
                    check_artifacts
                        .batch_logs_path
                        .to_string_lossy()
                        .into_owned(),
                ),
                target: Some(test_entry.id.clone()),
                expected: Some(json!("Godot batch check exits with status 0")),
                actual: Some(json!(process_result.returncode)),
                artifacts: failure_artifacts(artifacts, Some(batch_evidence)),
                ..FailureDetail::default()
            };
            if let Some(payload) = payloads.first_mut()
                && let Some(payload_object) = payload.as_object_mut()
            {
                payload_object.insert("ok".to_string(), json!(false));
                payload_object.insert(
                    "failures".to_string(),
                    serde_json::to_value(vec![failure.clone()]).unwrap_or_else(|_| json!([])),
                );
            }
            failures.push(failure);
        }
    }
    write_batch_summary_artifact(
        check_artifacts,
        &payloads,
        &mut failures,
        command,
        Some(&process_result),
        Some(&batch_log_evidence),
    );
    (payloads, failures)
}

fn selected_batch_pacing_groups(
    manifest: &TestManifest,
    selected_indices: &[usize],
) -> Vec<(Option<i64>, Vec<usize>)> {
    let mut groups: Vec<(Option<i64>, Vec<usize>)> = Vec::new();
    for index in selected_indices {
        let key = manifest.entries[*index]
            .frame_pacing
            .as_ref()
            .map(|frame_pacing| frame_pacing.fixed_fps);
        if let Some((_, indices)) = groups.iter_mut().find(|(existing, _)| *existing == key) {
            indices.push(*index);
        } else {
            groups.push((key, vec![*index]));
        }
    }
    groups
}

pub(super) fn batch_check_request(
    prepared: &[(&ManifestTestEntry, CheckTestArtifacts, Value)],
    check_artifacts: &CheckArtifacts,
    command: &str,
    options: CheckTestRunOptions,
) -> Value {
    let tests = prepared
        .iter()
        .map(|(test_entry, artifacts, request)| {
            let mut entry = Map::from_iter([
                ("id".to_string(), json!(test_entry.id)),
                ("kind".to_string(), json!(test_entry.kind)),
                ("target".to_string(), json!(test_entry.target)),
                ("tags".to_string(), json!(test_entry.tags)),
                ("check".to_string(), request["check"].clone()),
                ("artifacts".to_string(), artifacts.to_json()),
                ("request".to_string(), json!(artifacts.request_path)),
            ]);
            if options.profile_enabled {
                entry.insert(
                    "profiling".to_string(),
                    json!({
                        "enabled": true,
                        "max_frame_samples": CHECK_PROFILE_MAX_FRAME_SAMPLES,
                    }),
                );
            }
            Value::Object(entry)
        })
        .collect::<Vec<_>>();
    json!({
        "check": {
            "batch": true,
            "tests": tests,
        },
        "artifacts": {
            "root": check_artifacts.root_dir,
            "request": check_artifacts.batch_request_path,
            "logs": check_artifacts.batch_logs_path,
            "summary": check_artifacts.batch_summary_path,
            "progress": check_artifacts.progress_path,
        },
        "run": {
            "id": check_artifacts.run_id,
            "command": command,
            "execution_mode": "batch",
        }
    })
}

fn test_failure_payload(
    test_entry: &ManifestTestEntry,
    artifacts: Value,
    failure: &FailureDetail,
) -> Value {
    json!({
        "id": test_entry.id,
        "kind": test_entry.kind,
        "target": test_entry.target,
        "tags": test_entry.tags,
        "ok": false,
        "duration_ms": Value::Null,
        "runtime_errors": [],
        "artifacts": artifacts,
        "failures": [failure.clone()],
    })
}

pub(super) fn ensure_check_prerequisites(
    config: &ProjectConfig,
    command: &str,
) -> (Vec<FailureDetail>, Option<String>) {
    let mut failures = Vec::new();
    let mut version = None;
    if let Some(failure) = config.project_config_failure_for_command(command) {
        failures.push(failure);
    }
    if config.godot_binary.is_none() {
        failures.push(FailureDetail {
            kind: "missing_godot".to_string(),
            command: Some(command.to_string()),
            message: Some("Godot binary not found in PATH or GODOT_BIN".to_string()),
            expected: Some(json!(
                "discoverable Godot binary via --godot-bin, GODOT_BIN, or PATH"
            )),
            actual: Some(json!("missing")),
            ..FailureDetail::default()
        });
    } else if let Some(binary) = config.godot_binary.as_deref()
        && config.project_config_failure.is_none()
    {
        match godot_qa_process::probe_godot_version(binary, &config.godot_launch_prefix) {
            Ok(found) => version = Some(found),
            Err(failure) => failures.push(failure),
        }
    }
    if !config.project_root.exists() {
        failures.push(FailureDetail {
            kind: "missing_project_root".to_string(),
            command: Some(command.to_string()),
            path: Some(config.project_root.to_string_lossy().into_owned()),
            message: Some("project root does not exist".to_string()),
            expected: Some(json!("existing project root")),
            actual: Some(json!("missing")),
            ..FailureDetail::default()
        });
    }
    if config.project_root.exists() {
        if let Some(failure) = ensure_writable_artifact_dir(&config.artifacts_dir, command) {
            failures.push(failure);
        }
    }
    if !config.addon_dir.join("plugin.cfg").exists() {
        failures.push(FailureDetail {
            kind: "missing_addon".to_string(),
            command: Some(command.to_string()),
            path: Some(config.addon_dir.to_string_lossy().into_owned()),
            message: Some(
                "godot_qa addon must be installed in the target project for check".to_string(),
            ),
            expected: Some(json!("installed addon in target project")),
            actual: Some(json!("missing")),
            ..FailureDetail::default()
        });
    }
    (failures, version)
}

pub(super) fn test_check_request(
    test_entry: &ManifestTestEntry,
    artifacts: &CheckTestArtifacts,
    run_id: &str,
    command: &str,
    options: CheckTestRunOptions,
) -> Value {
    let mut payload = Map::from_iter([
        ("id".to_string(), json!(test_entry.id)),
        ("kind".to_string(), json!(test_entry.kind)),
        ("target".to_string(), json!(test_entry.target)),
        ("tags".to_string(), json!(test_entry.tags)),
    ]);
    if let Some(startup_frames) = test_entry.startup_frames {
        payload.insert("startup_frames".to_string(), json!(startup_frames));
    }
    if let Some(time_control) = &test_entry.time_control {
        payload.insert("time_control".to_string(), time_control.clone());
    }
    if let Some(frame_pacing) = &test_entry.frame_pacing {
        payload.insert("frame_pacing".to_string(), frame_pacing.to_json());
    }
    let mut request = Map::from_iter([
        ("check".to_string(), Value::Object(payload)),
        ("artifacts".to_string(), artifacts.to_json()),
        (
            "run".to_string(),
            json!({
                "id": run_id,
                "command": command,
            }),
        ),
    ]);
    if options.profile_enabled {
        request.insert(
            "profiling".to_string(),
            json!({
                "enabled": true,
                "max_frame_samples": CHECK_PROFILE_MAX_FRAME_SAMPLES,
            }),
        );
    }
    Value::Object(request)
}

fn check_test_timeout_failure(
    test_entry: &ManifestTestEntry,
    artifacts: &CheckTestArtifacts,
    process_result: &godot_qa_process::GodotRunResult,
    elapsed_ms: u64,
    configured_timeout: Option<Duration>,
    batch_evidence: Option<BatchEvidence<'_>>,
) -> FailureDetail {
    let mut artifact_map = failure_artifacts(artifacts, batch_evidence).unwrap_or_default();
    artifact_map.insert(
        "process".to_string(),
        json!({
            "timed_out": true,
            "timeout_ms": process_result.timeout_ms.or_else(|| configured_timeout.map(duration_millis_u64)),
            "elapsed_ms": elapsed_ms,
            "cleanup": process_result.cleanup.as_ref().map(|cleanup| json!({
                "status": cleanup.status,
                "failure_kind": cleanup.failure_kind,
                "pid": cleanup.pid,
                "process_group": cleanup.process_group,
                "owned": cleanup.owned,
                "message": cleanup.message,
            })),
            "infrastructure_failure_kind": process_result.infrastructure_failure_kind,
        }),
    );
    artifact_map.insert("progress".to_string(), json!(artifacts.progress_path));
    FailureDetail {
        kind: "check_test_timeout".to_string(),
        command: Some(godot_qa_process::format_command(&process_result.command)),
        path: Some(artifacts.logs_path.to_string_lossy().into_owned()),
        target: Some(test_entry.id.clone()),
        message: Some("Godot check test exceeded the configured timeout".to_string()),
        expected: Some(json!("Godot check exits with status 0 before timeout")),
        actual: Some(json!({
            "state": "timed_out",
            "elapsed_ms": elapsed_ms,
            "timeout_ms": process_result.timeout_ms.or_else(|| configured_timeout.map(duration_millis_u64)),
            "exit_code": process_result.returncode,
            "active_test": active_test_json(test_entry),
            "progress": artifacts.progress_path,
            "cleanup": process_result.cleanup.as_ref().map(|cleanup| json!({
                "status": cleanup.status,
                "failure_kind": cleanup.failure_kind,
                "pid": cleanup.pid,
                "process_group": cleanup.process_group,
                "owned": cleanup.owned,
                "message": cleanup.message,
            })),
        })),
        artifacts: Some(artifact_map),
        ..FailureDetail::default()
    }
}

fn active_test_json(test_entry: &ManifestTestEntry) -> Value {
    json!({
        "id": test_entry.id,
        "kind": test_entry.kind,
        "target": test_entry.target,
        "tags": test_entry.tags,
    })
}

fn duration_millis_u64(duration: Duration) -> u64 {
    duration.as_millis().try_into().unwrap_or(u64::MAX)
}
