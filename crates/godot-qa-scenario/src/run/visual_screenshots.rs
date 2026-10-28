// Extracted from run.rs; included by mod.rs to preserve the original module namespace.

#[cfg(test)]
fn apply_screenshot_quality(
    command: &str,
    scenario: &Value,
    result: CommandResult,
) -> CommandResult {
    apply_screenshot_quality_cached(
        command,
        scenario,
        result,
        &mut screenshots::ScreenshotAnalysisCache::default(),
    )
}

fn apply_screenshot_quality_cached(
    command: &str,
    scenario: &Value,
    mut result: CommandResult,
    cache: &mut screenshots::ScreenshotAnalysisCache,
) -> CommandResult {
    if !scenario_requests_screenshot(scenario) {
        return result;
    }

    let snapshots = result
        .data
        .get("snapshots")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    if snapshots.is_empty() {
        return result;
    }

    let artifacts = result
        .data
        .get("artifacts")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    let snapshot_artifacts = result
        .data
        .get("snapshot_artifacts")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    let mut quality_by_snapshot = Map::new();
    let mut low_information_failures = Vec::new();

    for snapshot in snapshots.iter().filter_map(Value::as_str) {
        let Some(screenshot_path) =
            screenshot_path_for_snapshot(snapshot, &artifacts, &snapshot_artifacts)
        else {
            continue;
        };
        if !screenshot_path.is_file() {
            continue;
        }
        let Ok(analysis) = cache.get(&screenshot_path) else {
            continue;
        };
        let quality = analysis.quality();
        let low_information = quality
            .get("low_information")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        quality_by_snapshot.insert(snapshot.to_string(), quality.clone());
        if !low_information {
            continue;
        }
        low_information_failures.push(failure_with_artifacts(
            FailureDetail {
                kind: "screenshot_low_information".to_string(),
                command: Some(command.to_string()),
                path: Some(screenshot_path.to_string_lossy().into_owned()),
                message: Some(
                    "screenshot appears blank, too small, or visually low-information".to_string(),
                ),
                expected: Some(json!("screenshot with useful visual evidence")),
                actual: Some(quality.clone()),
                severity: Some("error".to_string()),
                artifacts: Some(Map::from_iter([(
                    "screenshot".to_string(),
                    json!(screenshot_path.to_string_lossy().into_owned()),
                )])),
                ..FailureDetail::default()
            },
            &artifacts,
        ));
    }

    if !quality_by_snapshot.is_empty() {
        result.data.insert(
            "screenshot_quality".to_string(),
            Value::Object(quality_by_snapshot),
        );
    }
    if !low_information_failures.is_empty() {
        result.failures.extend(low_information_failures);
        result.ok = false;
        result.message = "scenario run failed".to_string();
        result.exit_code = 1;
        refresh_outcome_summary(&mut result);
    }
    result
}

fn screenshot_path_for_snapshot(
    snapshot: &str,
    artifacts: &Map<String, Value>,
    snapshot_artifacts: &Map<String, Value>,
) -> Option<PathBuf> {
    if let Some(path) = snapshot_artifacts
        .get(snapshot)
        .and_then(Value::as_object)
        .and_then(|entry| entry.get("screenshot"))
        .and_then(Value::as_str)
    {
        return Some(PathBuf::from(path));
    }
    artifacts
        .get("snapshots_dir")
        .and_then(Value::as_str)
        .map(PathBuf::from)
        .map(|snapshots_dir| snapshots_dir.join(snapshot).join("screenshot.png"))
}

fn apply_screenshot_matches(
    command: &str,
    scenario: &Value,
    scenario_path: &Path,
    config: &ProjectConfig,
    artifacts: &artifacts::RunArtifacts,
    runtime_result: CommandResult,
    cache: &mut screenshots::ScreenshotAnalysisCache,
) -> CommandResult {
    let screenshot_assertions = screenshot_match_assertions(scenario);
    if screenshot_assertions.is_empty() {
        return runtime_result;
    }

    let snapshot_artifacts = runtime_result
        .data
        .get("snapshot_artifacts")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    let mut screenshot_failures = Vec::new();

    for (index, screenshot_assertion) in screenshot_assertions.iter().enumerate() {
        let baseline = screenshot_assertion
            .get("baseline")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let baseline_resolution =
            resolve_screenshot_baseline_path(config, scenario_path, baseline, command);
        let baseline_path = match baseline_resolution {
            Ok(path) => path,
            Err(failure) => {
                screenshot_failures.push(failure);
                continue;
            }
        };

        let snapshot_name = screenshot_assertion
            .get("snapshot")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let actual_path = snapshot_artifacts
            .get(snapshot_name)
            .and_then(Value::as_object)
            .and_then(|entry| entry.get("screenshot"))
            .and_then(Value::as_str)
            .map(Path::new)
            .map(Path::to_path_buf)
            .unwrap_or_else(|| {
                artifacts
                    .snapshots_dir
                    .join(snapshot_name)
                    .join("screenshot.png")
            });
        let label = format!("screenshot-{:02}-{snapshot_name}", index + 1);
        let compare_result = screenshots::compare_screenshot_files_cached(
            command,
            &actual_path,
            &baseline_path,
            screenshot_assertion
                .get("threshold")
                .and_then(Value::as_f64)
                .unwrap_or(0.01),
            Some(
                &artifacts
                    .root_dir
                    .join("diffs")
                    .join(format!("{label}.diff.png")),
            ),
            Some(
                &artifacts
                    .root_dir
                    .join("diffs")
                    .join(format!("{label}.metrics.json")),
            ),
            cache,
        );
        if compare_result.ok {
            continue;
        }
        let Some(failure) = compare_result.failures.into_iter().next() else {
            continue;
        };
        let compare_artifacts = compare_result
            .data
            .get("artifacts")
            .and_then(Value::as_object)
            .cloned()
            .unwrap_or_default();
        let mut failure_artifacts = Map::new();
        failure_artifacts.insert(
            "actual".to_string(),
            json!(actual_path.to_string_lossy().into_owned()),
        );
        failure_artifacts.insert(
            "baseline".to_string(),
            json!(baseline_path.to_string_lossy().into_owned()),
        );
        for (key, value) in compare_artifacts {
            failure_artifacts.insert(key, value);
        }
        screenshot_failures.push(FailureDetail {
            kind: failure.kind,
            command: Some(command.to_string()),
            message: failure.message,
            expected: failure.expected,
            actual: failure.actual,
            artifacts: Some(failure_artifacts),
            ..FailureDetail::default()
        });
    }

    let mut final_failures = runtime_result.failures.clone();
    final_failures.extend(screenshot_failures);
    let final_ok = runtime_result.ok && final_failures.is_empty();
    let mut data = runtime_result.data;
    let outcome_reason = outcome_reason_for(final_ok, &final_failures);
    data.insert(
        "outcome".to_string(),
        outcome_summary_json(
            if final_ok { "pass" } else { "fail" },
            outcome_reason,
            Some(&final_failures),
        ),
    );
    CommandResult {
        ok: final_ok,
        command: runtime_result.command,
        message: if final_ok {
            "scenario run passed".to_string()
        } else {
            "scenario run failed".to_string()
        },
        data,
        failures: final_failures,
        exit_code: if final_ok { 0 } else { 1 },
    }
}

fn refresh_outcome_summary(result: &mut CommandResult) {
    let reason = outcome_reason_for(result.ok, &result.failures);
    result.data.insert(
        "outcome".to_string(),
        outcome_summary_json(
            if result.ok { "pass" } else { "fail" },
            reason,
            Some(&result.failures),
        ),
    );
}

fn write_result_artifact(path: &Path, result: &CommandResult) -> io::Result<()> {
    fs::write(
        path,
        serde_json::to_string_pretty(result).unwrap_or_else(|_| "{}".to_string()),
    )
}

fn python_type_name(value: &Value) -> String {
    match value {
        Value::Null => "NoneType".to_string(),
        Value::Bool(_) => "bool".to_string(),
        Value::Number(number) => {
            if number.is_i64() || number.is_u64() {
                "int".to_string()
            } else {
                "float".to_string()
            }
        }
        Value::String(_) => "str".to_string(),
        Value::Array(_) => "list".to_string(),
        Value::Object(_) => "dict".to_string(),
    }
}

fn screenshot_match_assertions(scenario: &Value) -> Vec<Value> {
    scenario
        .get("assert")
        .and_then(Value::as_array)
        .map(|assertions| {
            assertions
                .iter()
                .filter_map(|assertion| assertion.get("screenshot_match").cloned())
                .collect()
        })
        .unwrap_or_default()
}

fn scenario_requests_screenshot(scenario: &Value) -> bool {
    let saves_screenshot = scenario
        .get("snapshots")
        .and_then(Value::as_object)
        .and_then(|snapshots| snapshots.get("save_screenshot"))
        .and_then(Value::as_bool)
        .unwrap_or(false);
    saves_screenshot
        || !screenshot_match_assertions(scenario).is_empty()
        || scenario_contains_visual_assertion(scenario)
}

fn scenario_contains_visual_assertion(scenario: &Value) -> bool {
    scenario_assertions(scenario).iter().any(|assertion| {
        assertion.get("visible_pixels").is_some()
            || assertion.get("saved_snapshot_pixels").is_some()
            || assertion.get("not_occluded").is_some()
            || assertion.get("renders_above").is_some()
            || assertion.get("renders_below").is_some()
    })
}

fn scenario_contains_audio_capture(scenario: &Value) -> bool {
    scenario
        .get("steps")
        .and_then(Value::as_array)
        .is_some_and(|steps| steps.iter().any(value_contains_audio_capture))
}

fn value_contains_audio_capture(value: &Value) -> bool {
    match value {
        Value::Object(object) => {
            object.contains_key("audio_capture")
                || object.values().any(value_contains_audio_capture)
        }
        Value::Array(values) => values.iter().any(value_contains_audio_capture),
        _ => false,
    }
}

fn time_control_warnings(
    command: &str,
    scenario: &Value,
    options: RuntimeExecutionOptions,
) -> Vec<FailureDetail> {
    let Some(time_control) = scenario.get("time_control").and_then(Value::as_object) else {
        return Vec::new();
    };
    let time_scale = time_control
        .get("time_scale")
        .and_then(Value::as_f64)
        .unwrap_or(1.0);
    if time_scale <= 1.0 || has_paired_physics_settings(time_control) {
        return Vec::new();
    }

    let mut risk_sources = Vec::new();
    if scenario_requests_screenshot(scenario) {
        risk_sources.push("screenshot");
    }
    if options.visual_risk.enabled {
        risk_sources.push("visual_risk");
    }
    if scenario_flag(scenario, "audio_sensitive") {
        risk_sources.push("audio_sensitive");
    }
    if scenario_flag(scenario, "physics_precision_sensitive") {
        risk_sources.push("physics_precision_sensitive");
    }
    if risk_sources.is_empty() {
        return Vec::new();
    }

    vec![FailureDetail {
        kind: "time_control_physics_pairing_missing".to_string(),
        command: Some(command.to_string()),
        message: Some(
            "accelerated visual/audio/physics-sensitive flow requested time_scale without paired physics settings"
                .to_string(),
        ),
        expected: Some(json!(
            "time_control.physics_ticks_per_second and time_control.max_physics_steps_per_frame"
        )),
        actual: Some(json!(time_control)),
        severity: Some("warning".to_string()),
        details: serde_json::Map::from_iter([
            ("rule_id".to_string(), json!("time-control.physics_pairing_missing")),
            ("time_scale".to_string(), json!(time_scale)),
            ("risk_sources".to_string(), json!(risk_sources)),
            ("scenario".to_string(), scenario.get("name").cloned().unwrap_or(Value::Null)),
        ]),
        ..FailureDetail::default()
    }]
}

fn has_paired_physics_settings(time_control: &Map<String, Value>) -> bool {
    time_control.get("physics_ticks_per_second").is_some()
        && time_control.get("max_physics_steps_per_frame").is_some()
}

fn scenario_flag(scenario: &Value, key: &str) -> bool {
    scenario.get(key).and_then(Value::as_bool).unwrap_or(false)
        || scenario
            .get("metadata")
            .and_then(Value::as_object)
            .and_then(|metadata| metadata.get(key))
            .and_then(Value::as_bool)
            .unwrap_or(false)
}

fn scenario_assertions(scenario: &Value) -> Vec<&Value> {
    let mut assertions = Vec::new();
    if let Some(top_level) = scenario.get("assert").and_then(Value::as_array) {
        assertions.extend(top_level.iter());
    }
    if let Some(steps) = scenario.get("steps").and_then(Value::as_array) {
        for step in steps {
            if let Some(step_assertions) = step.get("assert").and_then(Value::as_array) {
                assertions.extend(step_assertions.iter());
            }
        }
    }
    assertions
}

fn layout_mode_from_scenario(scenario: &Value) -> Option<String> {
    scenario
        .get("assert")
        .and_then(Value::as_array)
        .and_then(|assertions| {
            assertions.iter().find_map(|assertion| {
                assertion
                    .get("layout_audit")
                    .and_then(Value::as_str)
                    .map(str::to_owned)
            })
        })
}

fn scenario_viewports(scenario: &Value) -> Vec<Vec<i64>> {
    if let Some(viewports) = scenario.get("viewports").and_then(Value::as_array) {
        let parsed: Vec<Vec<i64>> = viewports
            .iter()
            .filter_map(|viewport| {
                let viewport = viewport.as_array()?;
                Some(vec![
                    viewport.first()?.as_i64()?,
                    viewport.get(1)?.as_i64()?,
                ])
            })
            .collect();
        if !parsed.is_empty() {
            return parsed;
        }
    }

    let Some(viewport) = scenario.get("viewport").and_then(Value::as_object) else {
        return Vec::new();
    };
    match (
        viewport.get("width").and_then(Value::as_i64),
        viewport.get("height").and_then(Value::as_i64),
    ) {
        (Some(width), Some(height)) => vec![vec![width, height]],
        _ => Vec::new(),
    }
}

fn viewport_token(viewport: &[i64]) -> String {
    format!("{}x{}", viewport[0], viewport[1])
}

fn variant_scenario(scenario: &Value, viewport: &[i64]) -> Value {
    let mut variant = runtime_scenario_payload(scenario);
    if let Some(object) = variant.as_object_mut() {
        object.insert(
            "viewport".to_string(),
            json!({"width": viewport[0], "height": viewport[1]}),
        );
        object.remove("viewports");
    }
    variant
}

fn resolve_screenshot_baseline(
    config: &ProjectConfig,
    scenario_path: &Path,
    baseline: &str,
    command: &str,
) -> Result<(), FailureDetail> {
    let baselines_root = config.project_root.join("qa").join("baselines");
    let baseline_path = Path::new(baseline);
    if baseline_path.is_absolute()
        || baseline_path.components().any(|component| {
            matches!(
                component,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            )
        })
    {
        return Err(FailureDetail {
            kind: "baseline_path_unsafe".to_string(),
            command: Some(command.to_string()),
            path: Some(scenario_path.to_string_lossy().into_owned()),
            message: Some("screenshot_match baseline must stay under qa/baselines".to_string()),
            expected: Some(json!(baselines_root.to_string_lossy().into_owned())),
            actual: Some(json!(baseline)),
            ..FailureDetail::default()
        });
    }
    let resolved = config.project_root.join(baseline_path);
    let expected_root = baselines_root
        .canonicalize()
        .unwrap_or_else(|_| baselines_root.clone());
    if !resolved.starts_with(&baselines_root) {
        return Err(FailureDetail {
            kind: "baseline_path_unsafe".to_string(),
            command: Some(command.to_string()),
            path: Some(scenario_path.to_string_lossy().into_owned()),
            message: Some("screenshot_match baseline must stay under qa/baselines".to_string()),
            expected: Some(json!(expected_root.to_string_lossy().into_owned())),
            actual: Some(json!(baseline)),
            ..FailureDetail::default()
        });
    }
    if !resolved.is_file() {
        return Err(FailureDetail {
            kind: "baseline_not_found".to_string(),
            command: Some(command.to_string()),
            path: Some(resolved.to_string_lossy().into_owned()),
            message: Some("baseline screenshot file was not found".to_string()),
            expected: Some(json!("existing baseline screenshot file")),
            actual: Some(json!("missing")),
            ..FailureDetail::default()
        });
    }
    Ok(())
}

fn resolve_screenshot_baseline_path(
    config: &ProjectConfig,
    scenario_path: &Path,
    baseline: &str,
    command: &str,
) -> Result<std::path::PathBuf, FailureDetail> {
    resolve_screenshot_baseline(config, scenario_path, baseline, command)?;
    Ok(config.project_root.join(baseline))
}

fn apply_saved_snapshot_pixels(
    command: &str,
    config: &ProjectConfig,
    scenario: &Value,
    artifacts: &artifacts::RunArtifacts,
    mut result: CommandResult,
    cache: &mut screenshots::ScreenshotAnalysisCache,
) -> CommandResult {
    let assertions = saved_snapshot_pixel_assertions(scenario);
    if assertions.is_empty() || !result.ok || !result.failures.is_empty() {
        return persist_result(&artifacts.result_path, result);
    }

    let scenario_name = scenario
        .get("name")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let mut outcomes = Vec::new();
    let mut contexts = std::collections::HashMap::new();
    for assertion in assertions {
        let snapshot = assertion
            .get("snapshot")
            .and_then(Value::as_str)
            .unwrap_or_default();
        // Bound large tree/UI contexts independently of the image cache.
        if contexts.len() == 2
            && !contexts.contains_key(snapshot)
            && let Some(previous) = contexts.keys().next().cloned()
        {
            contexts.remove(&previous);
        }
        let loaded = contexts.entry(snapshot.to_string()).or_insert_with(|| {
            snapshot_context::load_snapshot_context(
                &config.artifacts_dir,
                scenario_name,
                Some(&artifacts.run_id),
                Some(snapshot),
                command,
            )
            .map(|loaded| {
                let context = assertions::SavedSnapshotPixelContext::new(&loaded.snapshot_context);
                (loaded, context)
            })
        });
        let (loaded, context) = match loaded {
            Ok(loaded) => loaded,
            Err(failure) => {
                let mut failure = failure.clone();
                failure.command = Some(command.to_string());
                result.failures.push(failure);
                continue;
            }
        };
        match assertions::evaluate_saved_snapshot_pixels(
            &assertion,
            context,
            &loaded.artifacts,
            cache,
        ) {
            Ok(outcome) => {
                let mut outcome_data = Map::from_iter([
                    ("snapshot".to_string(), json!(loaded.snapshot_name)),
                    (
                        "assertion".to_string(),
                        json!({"saved_snapshot_pixels": assertion.clone()}),
                    ),
                    (
                        "artifacts".to_string(),
                        Value::Object(loaded.artifacts.clone()),
                    ),
                ]);
                for (key, value) in outcome.data {
                    outcome_data.insert(key, value);
                }
                outcomes.push(Value::Object(outcome_data));
                if let Some(mut failure) = outcome.failure {
                    failure.command = Some(command.to_string());
                    result.failures.push(failure);
                }
            }
            Err(mut failure) => {
                failure.command = Some(command.to_string());
                result.failures.push(failure);
            }
        }
    }

    if !outcomes.is_empty() {
        result
            .data
            .insert("saved_snapshot_pixels".to_string(), Value::Array(outcomes));
    }
    result.ok = result.ok && result.failures.is_empty();
    result.exit_code = if result.ok { 0 } else { 1 };
    if !result.ok {
        result.message = format!("{command} failed");
        result.data.insert(
            "outcome".to_string(),
            outcome_summary_json(
                "fail",
                outcome_reason_for(false, &result.failures),
                Some(&result.failures),
            ),
        );
    }
    persist_result(&artifacts.result_path, result)
}

fn saved_snapshot_pixel_assertions(scenario: &Value) -> Vec<Map<String, Value>> {
    scenario_assertions(scenario)
        .into_iter()
        .filter_map(|assertion| {
            assertion
                .get("saved_snapshot_pixels")
                .and_then(Value::as_object)
                .cloned()
        })
        .collect()
}

fn runtime_scenario_payload(scenario: &Value) -> Value {
    let mut payload = scenario.clone();
    if let Some(object) = payload.as_object_mut() {
        if object
            .get("viewports")
            .and_then(Value::as_array)
            .is_none_or(Vec::is_empty)
        {
            object.remove("viewports");
        }
        object.remove("launch_args");
        if let Some(assertions) = object.get_mut("assert").and_then(Value::as_array_mut) {
            strip_cli_only_runtime_assertions(assertions);
        }
        if let Some(steps) = object.get_mut("steps").and_then(Value::as_array_mut) {
            for step in steps {
                if let Some(assertions) = step.get_mut("assert").and_then(Value::as_array_mut) {
                    strip_cli_only_runtime_assertions(assertions);
                }
            }
        }
    }
    payload
}

fn strip_cli_only_runtime_assertions(assertions: &mut Vec<Value>) {
    assertions.retain(|assertion| {
        !assertion.get("layout_audit").is_some()
            && !assertion.get("screenshot_match").is_some()
            && !assertion.get("saved_snapshot_pixels").is_some()
    });
}

fn write_runtime_request(
    path: &Path,
    scenario: &Value,
    project_root: &Path,
    artifacts: &artifacts::RunArtifacts,
    command_line: &str,
    options: RuntimeExecutionOptions,
) -> io::Result<()> {
    let meta = serde_json::from_str::<Value>(&fs::read_to_string(&artifacts.meta_path)?)
        .unwrap_or_else(|_| json!({}));
    let started_at = meta.get("started_at").cloned().unwrap_or(Value::Null);
    let warnings = time_control_warnings("runtime request", scenario, options);
    let mut request = json!({
        "scenario": scenario,
        "assertion_proof": artifacts.assertion_manifest,
        "artifacts": artifacts.to_json(),
        "run": {
            "id": artifacts.run_id,
            "nonce": meta.get("run_nonce").cloned().unwrap_or(Value::Null),
            "started_at": started_at,
            "command": command_line,
            "godot_args": scenario.get("godot_args").cloned().unwrap_or_else(|| json!([])),
            "project_args": redact_project_args_for_metadata(
                &scenario.get("project_args").cloned().unwrap_or_else(|| json!([]))
            ),
        }
    });
    if let Some(trial) = scenario.get("_asset_trial") {
        request["asset_trial"] = trial.clone();
    }
    if let Some(time_control) = scenario.get("time_control") {
        if let Some(object) = request.as_object_mut() {
            object.insert("time_control".to_string(), time_control.clone());
            object.insert("time_control_warnings".to_string(), json!(warnings));
        }
        if let Some(run) = request.get_mut("run").and_then(Value::as_object_mut) {
            run.insert("time_control".to_string(), time_control.clone());
        }
    }
    if let Some(frame_budget) = scenario_frame_budget_json(scenario, project_root) {
        if let Some(run) = request.get_mut("run").and_then(Value::as_object_mut) {
            run.insert("frame_budget".to_string(), frame_budget);
        }
    }
    if options.profile {
        if let Some(object) = request.as_object_mut() {
            object.insert(
                "profiling".to_string(),
                json!({"enabled": true, "max_frame_samples": PROFILE_MAX_FRAME_SAMPLES}),
            );
        }
    }
    fs::write(
        path,
        serde_json::to_string_pretty(&request).unwrap_or_else(|_| "{}".to_string()),
    )
}
