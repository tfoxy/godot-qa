// Extracted from run.rs; included by mod.rs to preserve the original module namespace.

/// Audit the layout of a single run's selected snapshot, returning either the
/// geometry failures or a single `layout_audit_missing_snapshot` failure when the
/// run recorded no snapshot to read. Shared by the single-run post-process
/// (`apply_layout_audit`) and the viewport-matrix path so BOTH fail loudly on a
/// missing snapshot instead of silently passing. `viewport` is the optional
/// viewport LABEL (the matrix cell, or a single-viewport scenario's lone
/// viewport); when absent the audit derives the viewport from the snapshot's own
/// `ui.viewport`.
fn layout_audit_failures(
    command: &str,
    scenario: &Value,
    audit_artifacts: &artifacts::RunArtifacts,
    viewport: Option<&[i64]>,
    snapshot_name: Option<&str>,
    mode: &str,
) -> Vec<FailureDetail> {
    let Some(snapshot_name) = snapshot_name else {
        return vec![layout_audit_missing_snapshot_failure(
            command,
            scenario,
            audit_artifacts,
            mode,
        )];
    };
    let (failures, _artifacts) = load_layout_failures(
        command,
        audit_artifacts,
        viewport,
        Some(snapshot_name),
        mode,
    );
    failures
}

/// The failure emitted when a scenario declares `layout_audit` but the run
/// produced no snapshot to audit — the audit reads a saved UI snapshot
/// (`ui.json`/`tree.json`/`focus.json`), so with no snapshot there is no evidence
/// and the audit can neither pass nor fail honestly. Closes the historical silent
/// pass (an empty failure set) on both the single-run and matrix paths.
fn layout_audit_missing_snapshot_failure(
    command: &str,
    scenario: &Value,
    audit_artifacts: &artifacts::RunArtifacts,
    mode: &str,
) -> FailureDetail {
    let artifacts_map = audit_artifacts
        .to_json()
        .as_object()
        .cloned()
        .unwrap_or_default();
    let mut failure = FailureDetail {
        kind: "layout_audit_missing_snapshot".to_string(),
        command: Some(command.to_string()),
        path: Some(audit_artifacts.snapshots_dir.to_string_lossy().into_owned()),
        message: Some(
            "layout_audit requires a saved UI snapshot but the run recorded none".to_string(),
        ),
        expected: Some(json!(
            "a `snapshot` step (for example `- snapshot: default`) so layout_audit can read the run's ui/tree/focus artifacts"
        )),
        actual: Some(json!("no snapshot recorded in the run result")),
        severity: Some("error".to_string()),
        ..FailureDetail::default()
    };
    failure.details.insert("mode".to_string(), json!(mode));
    failure.details.insert(
        "scenario".to_string(),
        json!(
            scenario
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or_default()
        ),
    );
    failure.details.insert(
        "remediation".to_string(),
        json!(
            "Add a `snapshot` step before the layout_audit assertion so the run saves ui.json/tree.json/focus.json for the audit to read."
        ),
    );
    failure_with_artifacts(failure, &artifacts_map)
}

/// CLI-side post-process that folds a single-run `layout_audit` (zero- or
/// one-viewport scenario) over the run's FINAL snapshot, mirroring how the
/// viewport-matrix path audits each cell's last snapshot. A no-op when the
/// scenario declares no `layout_audit` (including matrix variant scenarios, whose
/// `layout_audit` is stripped from the runtime payload upstream). Runs BEFORE
/// screenshot-match compares so both post-processes can coexist on one run.
fn apply_layout_audit(
    command: &str,
    scenario: &Value,
    artifacts: &artifacts::RunArtifacts,
    mut result: CommandResult,
) -> CommandResult {
    let Some(mode) = layout_mode_from_scenario(scenario) else {
        return result;
    };
    // A run whose result.json failed to load/validate has no trustworthy snapshot
    // list; its existing `scenario_result_invalid` failure is the actionable one,
    // so don't pile a layout failure on top (mirrors the matrix path's guard).
    if result
        .failures
        .iter()
        .any(|failure| failure.kind == "scenario_result_invalid")
    {
        return result;
    }
    let snapshot_name = result
        .data
        .get("snapshots")
        .and_then(Value::as_array)
        .and_then(|snapshots| snapshots.last())
        .and_then(Value::as_str);
    let viewports = scenario_viewports(scenario);
    let layout_failures = layout_audit_failures(
        command,
        scenario,
        artifacts,
        viewports.first().map(Vec::as_slice),
        snapshot_name,
        &mode,
    );
    if layout_failures.is_empty() {
        return result;
    }
    result.failures.extend(layout_failures);
    result.ok = false;
    result.exit_code = 1;
    result.message = format!("{command} failed");
    refresh_outcome_summary(&mut result);
    result
}

fn load_layout_failures(
    command: &str,
    variant_artifacts: &artifacts::RunArtifacts,
    viewport: Option<&[i64]>,
    snapshot_name: Option<&str>,
    mode: &str,
) -> (Vec<FailureDetail>, Option<Map<String, Value>>) {
    let Some(snapshot_name) = snapshot_name else {
        return (Vec::new(), None);
    };
    let snapshot_root = variant_artifacts.snapshots_dir.join(snapshot_name);
    let artifacts = Map::from_iter([
        (
            "root".to_string(),
            json!(snapshot_root.to_string_lossy().into_owned()),
        ),
        (
            "tree".to_string(),
            json!(
                snapshot_root
                    .join("tree.json")
                    .to_string_lossy()
                    .into_owned()
            ),
        ),
        (
            "ui".to_string(),
            json!(snapshot_root.join("ui.json").to_string_lossy().into_owned()),
        ),
        (
            "focus".to_string(),
            json!(
                snapshot_root
                    .join("focus.json")
                    .to_string_lossy()
                    .into_owned()
            ),
        ),
        (
            "errors".to_string(),
            json!(
                snapshot_root
                    .join("errors.json")
                    .to_string_lossy()
                    .into_owned()
            ),
        ),
        (
            "screenshot".to_string(),
            json!(
                snapshot_root
                    .join("screenshot.png")
                    .to_string_lossy()
                    .into_owned()
            ),
        ),
    ]);
    let snapshot_context = match load_layout_snapshot_context(command, &artifacts) {
        Ok(context) => context,
        Err(failure) => return (vec![failure], Some(artifacts)),
    };
    let audit_result = layout::audit_layout(
        &snapshot_context,
        viewport.map(<[i64]>::to_vec),
        mode,
        Some(artifacts.clone()),
    );
    let _ = (&audit_result.ok, &audit_result.mode, &audit_result.viewport);
    (audit_result.failures, Some(artifacts))
}

fn load_layout_snapshot_context(
    command: &str,
    artifacts: &Map<String, Value>,
) -> Result<Value, FailureDetail> {
    let ui_path = path_from_artifacts(artifacts, "ui")?;
    let focus_path = path_from_artifacts(artifacts, "focus")?;
    let tree_path = path_from_artifacts(artifacts, "tree")?;
    let errors_path = path_from_artifacts(artifacts, "errors")?;
    Ok(json!({
        "tree": read_json_artifact(command, &tree_path, "tree.json", artifacts)?,
        "ui": read_json_artifact(command, &ui_path, "ui.json", artifacts)?,
        "focus": read_json_artifact(command, &focus_path, "focus.json", artifacts)?,
        "errors": read_json_artifact(command, &errors_path, "errors.json", artifacts)?,
    }))
}

fn path_from_artifacts(
    artifacts: &Map<String, Value>,
    key: &str,
) -> Result<PathBuf, FailureDetail> {
    artifacts
        .get(key)
        .and_then(Value::as_str)
        .map(PathBuf::from)
        .ok_or_else(|| FailureDetail {
            kind: "artifact_read_failed".to_string(),
            message: Some(
                "snapshot artifacts must include tree, ui, focus, and errors paths".to_string(),
            ),
            expected: Some(json!(["tree", "ui", "focus", "errors"])),
            actual: Some(json!([key])),
            severity: Some("error".to_string()),
            artifacts: Some(artifacts.clone()),
            ..FailureDetail::default()
        })
}

fn read_json_artifact(
    command: &str,
    path: &Path,
    artifact_name: &str,
    artifacts: &Map<String, Value>,
) -> Result<Value, FailureDetail> {
    let raw = fs::read_to_string(path).map_err(|_| FailureDetail {
        kind: "artifact_read_failed".to_string(),
        command: Some(command.to_string()),
        path: Some(path.to_string_lossy().into_owned()),
        message: Some(format!("could not load {artifact_name}")),
        expected: Some(json!("valid JSON artifact file")),
        actual: Some(json!("OSError")),
        severity: Some("error".to_string()),
        artifacts: Some(artifacts.clone()),
        ..FailureDetail::default()
    })?;
    serde_json::from_str(&raw).map_err(|_| FailureDetail {
        kind: "artifact_read_failed".to_string(),
        command: Some(command.to_string()),
        path: Some(path.to_string_lossy().into_owned()),
        message: Some(format!("could not load {artifact_name}")),
        expected: Some(json!("valid JSON artifact file")),
        actual: Some(json!("JSONDecodeError")),
        severity: Some("error".to_string()),
        artifacts: Some(artifacts.clone()),
        ..FailureDetail::default()
    })
}

fn load_runtime_result(
    command: &str,
    result_path: &Path,
    artifacts: &Value,
    launch_display: Option<&godot_qa_process::DisplayLaunchMetadata>,
) -> Result<Value, FailureDetail> {
    let artifacts_map = artifacts.as_object().cloned().unwrap_or_default();
    if !result_path.is_file() {
        let failure = failure_with_artifacts(
            FailureDetail {
                kind: "scenario_result_missing".to_string(),
                command: Some(command.to_string()),
                path: Some(result_path.to_string_lossy().into_owned()),
                message: Some("Godot run did not produce result.json".to_string()),
                expected: Some(json!("result.json written by qa_runner.gd")),
                actual: Some(missing_result_actual(result_path, &artifacts_map)),
                ..FailureDetail::default()
            },
            &artifacts_map,
        );
        return Err(classify_failure_detail(failure, launch_display));
    }

    let raw = fs::read_to_string(result_path).map_err(|error| {
        scenario_result_invalid_failure(
            command,
            result_path,
            &error.to_string(),
            json!("valid JSON result file"),
            json!("OSError"),
            &artifacts.as_object().cloned().unwrap_or_default(),
        )
    })?;
    let parsed = serde_json::from_str::<Value>(&raw).map_err(|error| {
        scenario_result_invalid_failure(
            command,
            result_path,
            &error.to_string(),
            json!("valid JSON result file"),
            json!("JSONDecodeError"),
            &artifacts.as_object().cloned().unwrap_or_default(),
        )
    })?;
    if !parsed.is_object() {
        return Err(scenario_result_invalid_failure(
            command,
            result_path,
            "result.json root must be an object",
            json!("JSON object"),
            json!(python_type_name(&parsed)),
            &artifacts.as_object().cloned().unwrap_or_default(),
        ));
    }
    Ok(parsed)
}

fn missing_result_actual(result_path: &Path, artifacts: &Map<String, Value>) -> Value {
    let mut actual = Map::from_iter([("result".to_string(), json!("missing"))]);
    actual.insert(
        "result_path".to_string(),
        json!(result_path.to_string_lossy().into_owned()),
    );
    let Some(logs_path) = artifacts
        .get("logs_path")
        .and_then(Value::as_str)
        .map(PathBuf::from)
    else {
        return Value::Object(actual);
    };
    actual.insert(
        "logs_path".to_string(),
        json!(logs_path.to_string_lossy().into_owned()),
    );
    if let Ok(log_text) = fs::read_to_string(&logs_path) {
        for (key, value) in crash_context_from_logs(&log_text) {
            actual.insert(key, value);
        }
    }
    Value::Object(actual)
}

fn crash_context_from_logs(log_text: &str) -> Map<String, Value> {
    let lines = log_text.lines().map(str::to_string).collect::<Vec<_>>();
    let relevant = lines
        .iter()
        .filter(|line| is_relevant_godot_log_line(line))
        .rev()
        .take(40)
        .cloned()
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect::<Vec<_>>();
    let script_backtrace = lines
        .iter()
        .filter(|line| {
            let trimmed = line.trim_start();
            trimmed.starts_with("SCRIPT ERROR") || trimmed.starts_with("at:")
        })
        .cloned()
        .collect::<Vec<_>>();
    let crash_signal = lines
        .iter()
        .find(|line| is_crash_signal_line(line))
        .cloned();

    let mut context = Map::new();
    if let Some(crash_signal) = crash_signal {
        context.insert("crash_signal".to_string(), json!(crash_signal));
    }
    if !script_backtrace.is_empty() {
        context.insert("script_backtrace".to_string(), json!(script_backtrace));
    }
    context.insert("last_relevant_log_lines".to_string(), json!(relevant));
    context
}

fn is_relevant_godot_log_line(line: &str) -> bool {
    let trimmed = line.trim_start();
    trimmed.starts_with("SCRIPT ERROR")
        || trimmed.starts_with("ERROR:")
        || trimmed.starts_with("WARNING:")
        || trimmed.starts_with("at:")
        || trimmed.starts_with('[')
        || trimmed.contains("Backtrace")
        || trimmed.contains("backtrace")
        || is_crash_signal_line(line)
}

fn is_crash_signal_line(line: &str) -> bool {
    let lower = line.to_ascii_lowercase();
    lower.contains("signal")
        || lower.contains("sigsegv")
        || lower.contains("sigabrt")
        || lower.contains("crash")
        || lower.contains("segmentation fault")
}

fn validate_runtime_command_result(
    command: &str,
    scenario: &Value,
    base_data: &mut Map<String, Value>,
    result_path: &Path,
    artifacts: &Value,
    process_result: &GodotRunResult,
    runtime_result: &Value,
    logs_path: &Path,
    log_ignore_patterns: &[String],
) -> CommandResult {
    let artifacts_map = artifacts.as_object().cloned().unwrap_or_default();
    let runtime_result_object = runtime_result.as_object().cloned().unwrap_or_default();
    if let Some(trial) = runtime_result_object.get("asset_trial") {
        base_data.insert("asset_trial".to_string(), trial.clone());
    }
    if let Some(soak) = runtime_result_object.get("soak") {
        base_data.insert("soak".to_string(), soak.clone());
    }
    base_data.insert("process".to_string(), process_summary_json(process_result));
    if command != "scenario profile" && runtime_result_object.contains_key("profile") {
        return persist_result(
            result_path,
            invalid_result(
                command,
                base_data.clone(),
                result_path,
                "result.json field 'profile' is only valid for scenario profile",
                json!("no profile field for non-profile commands"),
                json!("profile payload"),
                &artifacts_map,
            ),
        );
    }

    let raw_ok = match runtime_result_object.get("ok") {
        Some(Value::Bool(value)) => *value,
        Some(value) => {
            return persist_result(
                result_path,
                invalid_result(
                    command,
                    base_data.clone(),
                    result_path,
                    "result.json field 'ok' must be a boolean",
                    json!("JSON boolean"),
                    json!(python_type_name(value)),
                    &artifacts_map,
                ),
            );
        }
        None => false,
    };

    let raw_snapshots = match runtime_result_object.get("snapshots") {
        Some(Value::Array(value)) => value.clone(),
        Some(value) => {
            return persist_result(
                result_path,
                invalid_result(
                    command,
                    base_data.clone(),
                    result_path,
                    "result.json field 'snapshots' must be an array",
                    json!("JSON array"),
                    json!(python_type_name(value)),
                    &artifacts_map,
                ),
            );
        }
        None => Vec::new(),
    };
    let invalid_snapshot_types: Vec<String> = raw_snapshots
        .iter()
        .filter(|value| !value.is_string())
        .map(python_type_name)
        .collect();
    if !invalid_snapshot_types.is_empty() {
        return persist_result(
            result_path,
            invalid_result(
                command,
                base_data.clone(),
                result_path,
                "result.json snapshots entries must be strings",
                json!("JSON string entries in snapshots array"),
                json!(invalid_snapshot_types),
                &artifacts_map,
            ),
        );
    }

    match runtime_result_object.get("duration_ms") {
        Some(Value::Number(value)) if value.as_i64().is_some_and(|ms| ms >= 0) => {
            base_data.insert("duration_ms".to_string(), json!(value.as_i64().unwrap()));
        }
        Some(Value::Null) | None => {
            base_data.insert("duration_ms".to_string(), Value::Null);
        }
        Some(value) => {
            return persist_result(
                result_path,
                invalid_result(
                    command,
                    base_data.clone(),
                    result_path,
                    "result.json field 'duration_ms' must be a non-negative integer",
                    json!("non-negative integer"),
                    json!(python_type_name(value)),
                    &artifacts_map,
                ),
            );
        }
    }
    if let Some(display_server) = runtime_result_object.get("display_server") {
        base_data.insert("display_server".to_string(), display_server.clone());
    }
    // Always-on coarse engine phase timing (scene_load_ms, steps_ms, ...) —
    // additive and advisory, so carry it through without validation.
    if let Some(timing) = runtime_result_object.get("timing") {
        if timing.is_object() {
            base_data.insert("timing".to_string(), timing.clone());
        }
    }
    if let Some(time_control) = runtime_result_object.get("time_control") {
        let Some(_) = time_control.as_object() else {
            return persist_result(
                result_path,
                invalid_result(
                    command,
                    base_data.clone(),
                    result_path,
                    "result.json field 'time_control' must be an object",
                    json!("JSON object"),
                    json!(python_type_name(time_control)),
                    &artifacts_map,
                ),
            );
        };
        base_data.insert("time_control".to_string(), time_control.clone());
    }
    for field in ["godot_args", "project_args"] {
        if let Some(value) = runtime_result_object.get(field) {
            let Some(values) = value.as_array() else {
                return persist_result(
                    result_path,
                    invalid_result(
                        command,
                        base_data.clone(),
                        result_path,
                        &format!("result.json field '{field}' must be an array"),
                        json!("JSON array"),
                        json!(python_type_name(value)),
                        &artifacts_map,
                    ),
                );
            };
            let invalid_arg_types: Vec<String> = values
                .iter()
                .filter(|value| !value.is_string())
                .map(python_type_name)
                .collect();
            if !invalid_arg_types.is_empty() {
                return persist_result(
                    result_path,
                    invalid_result(
                        command,
                        base_data.clone(),
                        result_path,
                        &format!("result.json {field} entries must be strings"),
                        json!("JSON string entries in argument array"),
                        json!(invalid_arg_types),
                        &artifacts_map,
                    ),
                );
            }
            base_data.insert(field.to_string(), value.clone());
        }
    }
    base_data.insert(
        "launch_display".to_string(),
        display_launch_metadata_json(&process_result.display),
    );
    base_data.insert("snapshots".to_string(), Value::Array(raw_snapshots.clone()));

    if let Some(raw_artifacts) = runtime_result_object.get("artifacts") {
        let Some(raw_artifacts) = raw_artifacts.as_object() else {
            return persist_result(
                result_path,
                invalid_result(
                    command,
                    base_data.clone(),
                    result_path,
                    "result.json field 'artifacts' must be an object",
                    json!("JSON object"),
                    json!(python_type_name(raw_artifacts)),
                    &artifacts_map,
                ),
            );
        };
        let mut invalid_values = Map::new();
        for (key, value) in raw_artifacts {
            if !value.is_string() {
                invalid_values.insert(key.clone(), json!(python_type_name(value)));
            }
        }
        if !invalid_values.is_empty() {
            return persist_result(
                result_path,
                invalid_result(
                    command,
                    base_data.clone(),
                    result_path,
                    "result.json artifacts values must be strings",
                    json!("JSON string values in artifacts object"),
                    Value::Object(invalid_values),
                    &artifacts_map,
                ),
            );
        }
        let merged_artifacts = {
            let mut merged = artifacts_map.clone();
            for (key, value) in raw_artifacts {
                merged.insert(key.clone(), value.clone());
            }
            merged
        };
        base_data.insert("artifacts".to_string(), Value::Object(merged_artifacts));
    }

    if let Some(raw_snapshot_artifacts) = runtime_result_object.get("snapshot_artifacts") {
        let validated_snapshot_artifacts = match validate_snapshot_artifacts(
            command,
            result_path,
            raw_snapshot_artifacts,
            &artifacts_map,
        ) {
            Ok(value) => value,
            Err(failure) => {
                return persist_result(
                    result_path,
                    CommandResult {
                        ok: false,
                        command: command.to_string(),
                        message: "scenario run failed".to_string(),
                        data: base_data.clone(),
                        failures: vec![failure],
                        exit_code: 1,
                    },
                );
            }
        };
        base_data.insert(
            "snapshot_artifacts".to_string(),
            Value::Object(validated_snapshot_artifacts),
        );
    }

    if let Some(node_inspections) = runtime_result_object.get("node_inspections") {
        let Some(node_inspections) = node_inspections.as_object() else {
            return persist_result(
                result_path,
                invalid_result(
                    command,
                    base_data.clone(),
                    result_path,
                    "result.json field 'node_inspections' must be an object",
                    json!("JSON object keyed by step index and target"),
                    json!(python_type_name(node_inspections)),
                    &artifacts_map,
                ),
            );
        };
        let invalid_entries: Map<String, Value> = node_inspections
            .iter()
            .filter(|(_, value)| !value.is_object())
            .map(|(key, value)| (key.clone(), json!(python_type_name(value))))
            .collect();
        if !invalid_entries.is_empty() {
            return persist_result(
                result_path,
                invalid_result(
                    command,
                    base_data.clone(),
                    result_path,
                    "result.json node_inspections entries must be objects",
                    json!("JSON object entries with node inspection evidence"),
                    Value::Object(invalid_entries),
                    &artifacts_map,
                ),
            );
        }
        base_data.insert(
            "node_inspections".to_string(),
            Value::Object(node_inspections.clone()),
        );
    }

    if let Some(physics_overlays) = runtime_result_object.get("physics_overlays") {
        let Some(physics_overlays) = physics_overlays.as_object() else {
            return persist_result(
                result_path,
                invalid_result(
                    command,
                    base_data.clone(),
                    result_path,
                    "result.json field 'physics_overlays' must be an object",
                    json!("JSON object keyed by step index, name, or target"),
                    json!(python_type_name(physics_overlays)),
                    &artifacts_map,
                ),
            );
        };
        let invalid_entries: Map<String, Value> = physics_overlays
            .iter()
            .filter(|(_, value)| !value.is_object())
            .map(|(key, value)| (key.clone(), json!(python_type_name(value))))
            .collect();
        if !invalid_entries.is_empty() {
            return persist_result(
                result_path,
                invalid_result(
                    command,
                    base_data.clone(),
                    result_path,
                    "result.json physics_overlays entries must be objects",
                    json!("JSON object entries with physics overlay evidence"),
                    Value::Object(invalid_entries),
                    &artifacts_map,
                ),
            );
        }
        base_data.insert(
            "physics_overlays".to_string(),
            Value::Object(physics_overlays.clone()),
        );
    }

    if let Some(policy_drives) = runtime_result_object.get("policy_drives") {
        let Some(policy_drives) = policy_drives.as_object() else {
            return persist_result(
                result_path,
                invalid_result(
                    command,
                    base_data.clone(),
                    result_path,
                    "result.json field 'policy_drives' must be an object",
                    json!("JSON object keyed by step index"),
                    json!(python_type_name(policy_drives)),
                    &artifacts_map,
                ),
            );
        };
        let invalid_entries: Map<String, Value> = policy_drives
            .iter()
            .filter(|(_, value)| !value.is_object())
            .map(|(key, value)| (key.clone(), json!(python_type_name(value))))
            .collect();
        if !invalid_entries.is_empty() {
            return persist_result(
                result_path,
                invalid_result(
                    command,
                    base_data.clone(),
                    result_path,
                    "result.json policy_drives entries must be objects",
                    json!("JSON object entries with drive_policy evidence"),
                    Value::Object(invalid_entries),
                    &artifacts_map,
                ),
            );
        }
        base_data.insert(
            "policy_drives".to_string(),
            Value::Object(policy_drives.clone()),
        );
    }

    if let Some(state_records) = runtime_result_object.get("state_records") {
        let Some(state_records) = state_records.as_object() else {
            return persist_result(
                result_path,
                invalid_result(
                    command,
                    base_data.clone(),
                    result_path,
                    "result.json field 'state_records' must be an object",
                    json!("JSON object keyed by record_state name"),
                    json!(python_type_name(state_records)),
                    &artifacts_map,
                ),
            );
        };
        let invalid_entries: Map<String, Value> = state_records
            .iter()
            .filter(|(_, value)| !value.is_object())
            .map(|(key, value)| (key.clone(), json!(python_type_name(value))))
            .collect();
        if !invalid_entries.is_empty() {
            return persist_result(
                result_path,
                invalid_result(
                    command,
                    base_data.clone(),
                    result_path,
                    "result.json state_records entries must be objects",
                    json!("JSON object entries with record_state evidence"),
                    Value::Object(invalid_entries),
                    &artifacts_map,
                ),
            );
        }
        base_data.insert(
            "state_records".to_string(),
            Value::Object(state_records.clone()),
        );
    }

    if runtime_result_object.contains_key("audio_artifacts") {
        let (audio_artifacts, audio_analysis) = match validate_audio_artifacts(
            command,
            result_path,
            runtime_result_object.get("audio_artifacts"),
            runtime_result_object.get("audio_analysis"),
            &artifacts_map,
        ) {
            Ok(value) => value,
            Err(failure) => {
                return persist_result(
                    result_path,
                    CommandResult {
                        ok: false,
                        command: command.to_string(),
                        message: "scenario run failed".to_string(),
                        data: base_data.clone(),
                        failures: vec![failure],
                        exit_code: 1,
                    },
                );
            }
        };
        base_data.insert(
            "audio_artifacts".to_string(),
            Value::Object(audio_artifacts),
        );
        base_data.insert("audio_analysis".to_string(), Value::Object(audio_analysis));
    }

    let mut failures = match validate_runtime_failures(
        command,
        result_path,
        runtime_result_object.get("failures"),
        raw_ok,
        &artifacts_map,
    ) {
        Ok(failures) => failures,
        Err(failure) => {
            return persist_result(
                result_path,
                CommandResult {
                    ok: false,
                    command: command.to_string(),
                    message: "scenario run failed".to_string(),
                    data: base_data.clone(),
                    failures: vec![failure],
                    exit_code: 1,
                },
            );
        }
    };
    let runtime_failure_count = failures.len();
    if let Some(truncation) = &process_result.output_truncation {
        failures.push(failure_with_artifacts(
            FailureDetail {
                kind: "output_capture_truncated".to_string(),
                command: Some(command.to_string()),
                path: Some(logs_path.to_string_lossy().into_owned()),
                message: Some("Godot output exceeded the bounded log review; inspect the retained raw streams".to_string()),
                expected: Some(json!("stdout and stderr each at most 8 MiB")),
                actual: Some(json!(truncation)),
                ..FailureDetail::default()
            },
            &artifacts_map,
        ));
    }
    let (log_failures, log_ignored_count) =
        runtime_log_failures(command, logs_path, &artifacts_map, log_ignore_patterns);
    base_data.insert(
        "runtime_result".to_string(),
        runtime_result_summary_json(
            result_path,
            if raw_ok && runtime_failure_count == 0 {
                "passed"
            } else {
                "failed"
            },
            Some(raw_ok),
            runtime_failure_count,
        ),
    );
    base_data.insert(
        "logs".to_string(),
        log_status_summary_json(
            logs_path,
            &log_failures,
            log_ignored_count,
            log_ignore_patterns.len(),
        ),
    );
    if let Some(progress_path) = artifacts_map
        .get("progress")
        .or_else(|| artifacts_map.get("progress_path"))
        .and_then(Value::as_str)
    {
        base_data.insert(
            "progress".to_string(),
            progress_evidence_summary_json(progress_path),
        );
        let slow_summary_path = artifacts_map
            .get("slow_summary")
            .and_then(Value::as_str)
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                result_path
                    .parent()
                    .unwrap_or_else(|| Path::new("."))
                    .join("slow-summary.json")
            });
        let slow_summary = slow_scenario_summary_json(scenario, progress_path, &slow_summary_path);
        if let Err(error) = write_json(&slow_summary_path, &slow_summary) {
            eprintln!(
                "godot-qa: warning: failed to write slow-summary artifact {}: {error}",
                slow_summary_path.display()
            );
        }
        base_data.insert("slow_summary".to_string(), slow_summary);
    }
    failures.extend(log_failures);

    let allows_wrapper_exit = raw_ok
        && runtime_failure_count == 0
        && failures.is_empty()
        && known_wrapper_exit_codes(scenario).contains(&process_result.returncode);
    let mut ok =
        raw_ok && (process_result.returncode == 0 || allows_wrapper_exit) && failures.is_empty();
    if process_result.returncode != 0 && failures.is_empty() {
        if allows_wrapper_exit {
            base_data.insert(
                "process".to_string(),
                process_summary_with_warning_json(process_result, result_path, &artifacts_map),
            );
        } else {
            failures.push(classify_failure_detail(
                failure_with_artifacts(
                    FailureDetail {
                        kind: "godot_run_failed".to_string(),
                        command: Some(command.to_string()),
                        path: Some(result_path.to_string_lossy().into_owned()),
                        message: Some("Godot exited with a non-zero status".to_string()),
                        expected: Some(json!(0)),
                        actual: Some(json!(process_result.returncode)),
                        ..FailureDetail::default()
                    },
                    &artifacts_map,
                ),
                None,
            ));
            ok = false;
        }
    }
    let outcome_reason = outcome_reason_for(ok, &failures);
    let pass_with_warning = ok && process_result.returncode != 0 && allows_wrapper_exit;
    base_data.insert(
        "outcome".to_string(),
        if pass_with_warning {
            outcome_warning_summary_json()
        } else {
            outcome_summary_json(
                if ok { "pass" } else { "fail" },
                outcome_reason,
                Some(&failures),
            )
        },
    );

    CommandResult {
        ok,
        command: command.to_string(),
        message: if pass_with_warning {
            format!("{command} passed with warning")
        } else if ok {
            format!("{command} passed")
        } else {
            format!("{command} failed")
        },
        data: base_data.clone(),
        failures,
        exit_code: if ok { 0 } else { 1 },
    }
}

fn profile_result_from_runtime(
    command: &str,
    scenario: &Value,
    base_data: &mut Map<String, Value>,
    artifacts: &artifacts::RunArtifacts,
    runtime_data: &Value,
    runtime_result: &CommandResult,
) -> Result<CommandResult, CommandResult> {
    let profile = runtime_data
        .as_object()
        .and_then(|data| data.get("profile"))
        .ok_or_else(|| {
            profile_invalid_result(
                command,
                base_data,
                artifacts,
                "missing profile payload",
                json!("result.json profile object"),
                Value::Null,
            )
        })?;
    let Some(profile_object) = profile.as_object() else {
        return Err(profile_invalid_result(
            command,
            base_data,
            artifacts,
            "result.json field 'profile' must be an object",
            json!("JSON object"),
            json!(python_type_name(profile)),
        ));
    };
    let frame_samples = profile::optional_number_array(profile_object.get("frame_times_ms"))
        .map_err(|actual| {
            profile_invalid_result(
                command,
                base_data,
                artifacts,
                "profile.frame_times_ms must be an array of non-negative numbers",
                json!("JSON array of non-negative numbers"),
                actual,
            )
        })?;
    let monitor_samples = profile::optional_object_array(profile_object.get("monitor_samples"))
        .map_err(|actual| {
            profile_invalid_result(
                command,
                base_data,
                artifacts,
                "profile.monitor_samples must be an array of objects",
                json!("JSON array of JSON objects"),
                actual,
            )
        })?;
    let step_timings =
        profile::optional_object_array(profile_object.get("step_timings")).map_err(|actual| {
            profile_invalid_result(
                command,
                base_data,
                artifacts,
                "profile.step_timings must be an array of objects",
                json!("JSON array of JSON objects"),
                actual,
            )
        })?;
    let top_script_methods =
        profile::top_script_methods_or_empty(profile_object.get("top_script_methods"));
    let percentiles = profile::profile_percentiles(&frame_samples);
    let bounded_frames = profile::bounded_frame_values(&frame_samples, PROFILE_MAX_FRAME_SAMPLES);
    let artifact_path = artifacts.root_dir.join("profile.json");
    let time_control = runtime_data
        .as_object()
        .and_then(|data| data.get("time_control"))
        .cloned()
        .or_else(|| runtime_result.data.get("time_control").cloned());
    let artifact = json!({
        "scenario": scenario.get("name").and_then(Value::as_str).unwrap_or_default(),
        "scene": scenario.get("scene").and_then(Value::as_str).unwrap_or_default(),
        "duration_ms": base_data.get("duration_ms").cloned().unwrap_or(Value::Null),
        "launch_display": base_data.get("launch_display").cloned().unwrap_or(Value::Null),
        "artifacts": base_data.get("artifacts").cloned().unwrap_or_else(|| artifacts.to_json()),
        "time_control": time_control.clone().unwrap_or(Value::Null),
        "soak": runtime_data.get("soak").cloned().unwrap_or(Value::Null),
        "frame_times_ms": bounded_frames,
        "frame_time_percentiles_ms": profile::profile_percentiles_json(&percentiles),
        "monitor_samples": monitor_samples,
        "step_timings": step_timings,
        "top_script_methods": top_script_methods.clone(),
        "sample_limits": {
            "max_frame_samples": PROFILE_MAX_FRAME_SAMPLES,
            "top_script_methods": PROFILE_TOP_SCRIPT_METHODS,
        },
    });
    if let Err(error) = write_profile_artifact(&artifact_path, &artifact) {
        let artifacts_map = artifacts.to_json().as_object().cloned().unwrap_or_default();
        return Err(persist_result(
            &artifacts.result_path,
            CommandResult {
                ok: false,
                command: command.to_string(),
                message: "scenario profile failed".to_string(),
                data: base_data.clone(),
                failures: vec![failure_with_artifacts(
                    FailureDetail {
                        kind: "artifact_write_failed".to_string(),
                        command: Some(command.to_string()),
                        path: Some(artifact_path.to_string_lossy().into_owned()),
                        message: Some(error.to_string()),
                        expected: Some(json!("profile.json written under the run root")),
                        actual: Some(json!("OSError")),
                        ..FailureDetail::default()
                    },
                    &artifacts_map,
                )],
                exit_code: 1,
            },
        ));
    }
    let mut data = runtime_result.data.clone();
    let mut artifacts_json = data
        .get("artifacts")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_else(|| artifacts.to_json().as_object().cloned().unwrap_or_default());
    artifacts_json.insert(
        "profile".to_string(),
        json!(artifact_path.to_string_lossy().into_owned()),
    );
    data.insert("artifacts".to_string(), Value::Object(artifacts_json));
    data.insert(
        "profile".to_string(),
        json!({
            "frame_time_ms": profile::profile_percentiles_json(&percentiles),
            "frame_time_percentiles_ms": profile::profile_percentiles_json(&percentiles),
            "monitor_counters": profile::monitor_counter_summaries(&monitor_samples),
            "monitor_sample_count": monitor_samples.len(),
            "step_timings": step_timings,
            "step_timing_count": step_timings.len(),
            "top_script_methods": top_script_methods,
            "sample_limits": {
                "max_frame_samples": PROFILE_MAX_FRAME_SAMPLES,
                "top_script_methods": PROFILE_TOP_SCRIPT_METHODS,
            },
        }),
    );
    if let Some(time_control) = time_control {
        data.insert("time_control".to_string(), time_control);
    }
    Ok(CommandResult {
        ok: true,
        command: command.to_string(),
        message: "scenario profile passed".to_string(),
        data,
        failures: Vec::new(),
        exit_code: 0,
    })
}

fn profile_invalid_result(
    command: &str,
    base_data: &Map<String, Value>,
    artifacts: &artifacts::RunArtifacts,
    message: &str,
    expected: Value,
    actual: Value,
) -> CommandResult {
    let artifacts_map = artifacts.to_json().as_object().cloned().unwrap_or_default();
    persist_result(
        &artifacts.result_path,
        invalid_result(
            command,
            base_data.clone(),
            &artifacts.result_path,
            message,
            expected,
            actual,
            &artifacts_map,
        ),
    )
}

fn write_profile_artifact(path: &Path, value: &Value) -> io::Result<()> {
    fs::write(
        path,
        serde_json::to_string_pretty(value).unwrap_or_else(|_| "{}".to_string()),
    )
}

fn runtime_failure_to_detail(command: &str, value: &Map<String, Value>) -> FailureDetail {
    let mut detail = runtime_failure_extra_fields(value);
    FailureDetail {
        kind: value
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or("scenario_assertion_failed")
            .to_string(),
        command: Some(command.to_string()),
        path: value.get("path").and_then(Value::as_str).map(str::to_owned),
        message: value
            .get("message")
            .and_then(Value::as_str)
            .map(str::to_owned),
        expected: value.get("expected").cloned(),
        actual: value.get("actual").cloned(),
        target: value
            .get("target")
            .and_then(Value::as_str)
            .map(str::to_owned),
        severity: value
            .get("severity")
            .and_then(Value::as_str)
            .map(str::to_owned),
        artifacts: value.get("artifacts").and_then(Value::as_object).cloned(),
        viewport: take_i64_vec(&mut detail, "viewport"),
        a: value.get("a").and_then(Value::as_str).map(str::to_owned),
        b: value.get("b").and_then(Value::as_str).map(str::to_owned),
        rect: take_f64_vec(&mut detail, "rect"),
        rect_a: take_f64_vec(&mut detail, "rect_a"),
        rect_b: take_f64_vec(&mut detail, "rect_b"),
        details: detail,
    }
}

fn runtime_failure_extra_fields(value: &Map<String, Value>) -> Map<String, Value> {
    value
        .iter()
        .filter_map(|(key, value)| match key.as_str() {
            "type" | "message" | "path" | "expected" | "actual" | "target" | "severity"
            | "artifacts" => None,
            "a" | "b" if value.is_string() => None,
            _ => Some((key.clone(), value.clone())),
        })
        .collect()
}

fn take_i64_vec(detail: &mut Map<String, Value>, key: &str) -> Option<Vec<i64>> {
    let values = detail.get(key)?.as_array()?;
    let parsed: Option<Vec<i64>> = values.iter().map(Value::as_i64).collect();
    if parsed.is_some() {
        detail.remove(key);
    }
    parsed
}

fn take_f64_vec(detail: &mut Map<String, Value>, key: &str) -> Option<Vec<f64>> {
    let values = detail.get(key)?.as_array()?;
    let parsed: Option<Vec<f64>> = values.iter().map(Value::as_f64).collect();
    if parsed.is_some() {
        detail.remove(key);
    }
    parsed
}

/// Engine-log scan for one run's `logs.txt`.
///
/// Returns the `godot_log_error` / `godot_log_warning` failures plus how many
/// `ERROR:`/`WARNING:` lines an ignore list silenced — the built-in lists below
/// or the project's own `logs.ignore` patterns. The count is reported as
/// `data.logs.ignored_count` so a project that silences engine noise can still
/// see that it did.
fn runtime_log_failures(
    command: &str,
    logs_path: &Path,
    artifacts: &Map<String, Value>,
    log_ignore_patterns: &[String],
) -> (Vec<FailureDetail>, usize) {
    let scan = godot_qa_core::godot_logs::scan_output(logs_path, "", "", log_ignore_patterns);
    let failures = scan.entries.iter().map(|(severity, line)| {
        failure_with_artifacts(FailureDetail {
            kind: match severity {
                godot_qa_core::godot_logs::Severity::Error => "godot_log_error",
                godot_qa_core::godot_logs::Severity::Warning => "godot_log_warning",
            }.to_string(),
            command: Some(command.to_string()),
            path: Some(logs_path.to_string_lossy().into_owned()),
            message: Some("Godot runtime emitted engine log noise during scenario run".to_string()),
            expected: Some(json!("scenario logs with no engine or script WARNING:/ERROR: lines")),
            actual: Some(json!(line)),
            ..FailureDetail::default()
        }, artifacts)
    }).collect();
    (failures, scan.ignored_count)
}

fn invalid_result(
    command: &str,
    data: Map<String, Value>,
    result_path: &Path,
    message: &str,
    expected: Value,
    actual: Value,
    artifacts: &Map<String, Value>,
) -> CommandResult {
    CommandResult {
        ok: false,
        command: command.to_string(),
        message: "scenario run failed".to_string(),
        data,
        failures: vec![scenario_result_invalid_failure(
            command,
            result_path,
            message,
            expected,
            actual,
            artifacts,
        )],
        exit_code: 1,
    }
}

fn scenario_result_invalid_failure(
    command: &str,
    result_path: &Path,
    message: &str,
    expected: Value,
    actual: Value,
    artifacts: &Map<String, Value>,
) -> FailureDetail {
    classify_failure_detail(
        failure_with_artifacts(
            FailureDetail {
                kind: "scenario_result_invalid".to_string(),
                command: Some(command.to_string()),
                path: Some(result_path.to_string_lossy().into_owned()),
                message: Some(message.to_string()),
                expected: Some(expected),
                actual: Some(actual),
                ..FailureDetail::default()
            },
            artifacts,
        ),
        None,
    )
}

struct FailureClassification {
    failure_category: &'static str,
    infrastructure_failure_kind: Option<String>,
    retryable: bool,
    retry_guidance: Option<String>,
}

fn classify_failure_detail(
    mut failure: FailureDetail,
    launch_display: Option<&godot_qa_process::DisplayLaunchMetadata>,
) -> FailureDetail {
    let classification = failure_classification(&failure.kind, launch_display);
    failure.details.insert(
        "failure_category".to_string(),
        json!(classification.failure_category),
    );
    failure
        .details
        .insert("retryable".to_string(), json!(classification.retryable));
    if let Some(kind) = classification.infrastructure_failure_kind {
        failure
            .details
            .insert("infrastructure_failure_kind".to_string(), json!(kind));
    }
    if let Some(guidance) = classification.retry_guidance {
        failure
            .details
            .insert("retry_guidance".to_string(), json!(guidance));
    }
    failure
}

fn failure_classification(
    kind: &str,
    launch_display: Option<&godot_qa_process::DisplayLaunchMetadata>,
) -> FailureClassification {
    if kind == "scenario_result_missing" {
        if let Some(display) = launch_display {
            if let Some(startup_kind) = display.startup_failure_kind.clone() {
                return FailureClassification {
                    failure_category: "display_infrastructure",
                    infrastructure_failure_kind: Some(startup_kind),
                    retryable: display.startup_failure_retryable,
                    retry_guidance: display.startup_failure_guidance.clone(),
                };
            }
        }
    }
    let failure_category = match kind {
        "godot_run_failed" | "godot_launch_failed" | "scenario_run_timeout" => "process",
        "artifact_read_failed"
        | "artifact_write_failed"
        | "scenario_result_missing"
        | "scenario_result_invalid"
        | "screenshot_low_information" => "artifact",
        "godot_log_error" | "godot_log_warning" => "log_health",
        _ => "gameplay_assertion",
    };
    FailureClassification {
        failure_category,
        infrastructure_failure_kind: None,
        retryable: false,
        retry_guidance: None,
    }
}

fn outcome_classification_json(failures: &[FailureDetail]) -> Option<Map<String, Value>> {
    let failure = failures.first()?;
    let category = failure
        .details
        .get("failure_category")
        .and_then(Value::as_str)
        .map(str::to_string)
        .unwrap_or_else(|| {
            failure_classification(&failure.kind, None)
                .failure_category
                .to_string()
        });
    let retryable = failure
        .details
        .get("retryable")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let mut classification = Map::from_iter([
        ("failure_category".to_string(), json!(category)),
        ("retryable".to_string(), json!(retryable)),
    ]);
    if let Some(kind) = failure.details.get("infrastructure_failure_kind") {
        classification.insert("infrastructure_failure_kind".to_string(), kind.clone());
    }
    if let Some(guidance) = failure.details.get("retry_guidance") {
        classification.insert("retry_guidance".to_string(), guidance.clone());
    }
    Some(classification)
}

fn process_failure_category(process_result: &GodotRunResult) -> Option<&'static str> {
    if process_result.timed_out {
        Some("process")
    } else if process_result.infrastructure_failure_kind.is_some() {
        Some("display_infrastructure")
    } else if process_result.returncode != 0 {
        Some("process")
    } else {
        None
    }
}

fn failure_with_artifacts(failure: FailureDetail, artifacts: &Map<String, Value>) -> FailureDetail {
    let mut merged = artifacts.clone();
    if let Some(existing) = failure.artifacts.clone() {
        for (key, value) in existing {
            merged.insert(key, value);
        }
    }
    FailureDetail {
        artifacts: Some(merged),
        ..failure
    }
}

fn persist_result(result_path: &Path, result: CommandResult) -> CommandResult {
    if let Err(error) = write_result_artifact(result_path, &result) {
        eprintln!(
            "godot-qa: warning: failed to write result artifact {}: {error}",
            result_path.display()
        );
    }
    result
}
