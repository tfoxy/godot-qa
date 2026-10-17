// Extracted from run.rs; included by mod.rs to preserve the original module namespace.

fn apply_visual_risk_analysis(
    command: &str,
    scenario: &Value,
    mut result: CommandResult,
    options: RuntimeExecutionOptions,
) -> CommandResult {
    append_time_control_warnings(command, scenario, &mut result, options);

    if !options.visual_risk.enabled {
        return result;
    }

    let mut findings = visual_risk::analyze_result_artifacts(
        &result.data,
        &visual_risk::VisualRiskOptions::default(),
    );
    for finding in &mut findings {
        finding.command = Some(command.to_string());
        finding.details.insert(
            "scenario".to_string(),
            json!(
                scenario
                    .get("name")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
            ),
        );
        finding
            .details
            .entry("rule_id".to_string())
            .or_insert_with(|| json!(finding.kind));
    }

    let findings_json = serde_json::to_value(&findings).unwrap_or_else(|_| json!([]));
    result.data.insert(
        "visual_risk".to_string(),
        json!({
            "strict": options.visual_risk.strict,
            "findings": findings_json,
        }),
    );

    if findings.is_empty() {
        return result;
    }

    if options.visual_risk.strict {
        for finding in &mut findings {
            finding.severity = Some("error".to_string());
        }
        result.failures.extend(findings);
        result.ok = false;
        result.message = "scenario run failed".to_string();
        result.exit_code = 1;
        refresh_outcome_summary(&mut result);
    } else {
        let warnings = result
            .data
            .entry("warnings".to_string())
            .or_insert_with(|| json!([]));
        if let Some(warnings) = warnings.as_array_mut() {
            warnings.extend(findings_json.as_array().cloned().unwrap_or_default());
        }
    }

    result
}

fn append_time_control_warnings(
    command: &str,
    scenario: &Value,
    result: &mut CommandResult,
    options: RuntimeExecutionOptions,
) {
    let warnings = time_control_warnings(command, scenario, options);
    if warnings.is_empty() {
        return;
    }
    result
        .data
        .insert("time_control_warnings".to_string(), json!(warnings));
    let warning_values = serde_json::to_value(&warnings).unwrap_or_else(|_| json!([]));
    let all_warnings = result
        .data
        .entry("warnings".to_string())
        .or_insert_with(|| json!([]));
    if let Some(all_warnings) = all_warnings.as_array_mut() {
        all_warnings.extend(warning_values.as_array().cloned().unwrap_or_default());
    }
}

fn run_viewport_matrix(
    command: &str,
    result_message: &str,
    config: &ProjectConfig,
    scenario: &Value,
    scenario_path: &Path,
    aggregate_artifacts: &artifacts::RunArtifacts,
    viewports: &[Vec<i64>],
    layout_mode: Option<&str>,
    command_line: &str,
    headless_override: Option<bool>,
    audio_override: Option<bool>,
) -> CommandResult {
    // Defense in depth: routing only calls the matrix with `viewports.len() > 1`,
    // but an empty viewport list would produce zero runs and a VACUOUS pass
    // (`runs.iter().all(..)` over an empty list is true). Fail loudly instead so
    // the vacuous-pass hole is closed at the source.
    if viewports.is_empty() {
        let result = CommandResult {
            ok: false,
            command: command.to_string(),
            message: result_message.to_string(),
            data: Map::from_iter([
                (
                    "scenario".to_string(),
                    json!(
                        scenario
                            .get("name")
                            .and_then(Value::as_str)
                            .unwrap_or_default()
                    ),
                ),
                (
                    "scene".to_string(),
                    json!(
                        scenario
                            .get("scene")
                            .and_then(Value::as_str)
                            .unwrap_or_default()
                    ),
                ),
                (
                    "scenario_path".to_string(),
                    json!(scenario_path.to_string_lossy().into_owned()),
                ),
                ("artifacts".to_string(), aggregate_artifacts.to_json()),
                ("viewports".to_string(), json!(viewports)),
                ("runs".to_string(), json!([])),
            ]),
            failures: vec![FailureDetail {
                kind: "viewport_matrix_empty".to_string(),
                command: Some(command.to_string()),
                path: Some(scenario_path.to_string_lossy().into_owned()),
                message: Some(
                    "viewport matrix produced no runs; a matrix requires at least one viewport"
                        .to_string(),
                ),
                expected: Some(json!("at least one [width, height] viewport entry")),
                actual: Some(json!(viewports)),
                ..FailureDetail::default()
            }],
            exit_code: 1,
        };
        return persist_result(&aggregate_artifacts.result_path, result);
    }
    let variant_root = aggregate_artifacts.root_dir.join("variants");
    let mut runs = Vec::new();
    let mut flattened_failures = Vec::new();
    let aggregate_started_at = fs::read_to_string(&aggregate_artifacts.meta_path)
        .ok()
        .and_then(|raw| serde_json::from_str::<Value>(&raw).ok())
        .and_then(|meta| {
            meta.get("started_at")
                .and_then(Value::as_str)
                .map(str::to_owned)
        });

    for viewport in viewports {
        let viewport_token = viewport_token(viewport);
        let variant_scenario = variant_scenario(scenario, viewport);
        let mut artifact_scenario = json!({
            "name": viewport_token,
            "scene": scenario.get("scene").cloned().unwrap_or(Value::Null),
            "viewport": variant_scenario.get("viewport").cloned().unwrap_or(Value::Null),
            "steps": scenario.get("steps").cloned().unwrap_or_else(|| json!([])),
            "godot_args": json!(merged_launch_args(&config.godot_args, &scenario_godot_args(&variant_scenario))),
            "project_args": json!(merged_launch_args(&config.godot_project_args, &scenario_project_args(&variant_scenario))),
        });
        if let Some(object) = artifact_scenario.as_object_mut() {
            if let Some(snapshots) = variant_scenario.get("snapshots") {
                object.insert("snapshots".to_string(), snapshots.clone());
            }
        }
        let variant_artifacts = match artifacts::prepare_run_artifacts(
            &variant_root,
            &artifact_scenario,
            &config.project_root,
            scenario_path,
            command_line,
            config.godot_binary.as_deref(),
            &config.godot_launch_prefix,
            &config.godot_windowed_launch_prefix,
            Some(&aggregate_artifacts.run_id),
            aggregate_started_at.as_deref(),
        ) {
            Ok(artifacts) => artifacts,
            Err(error) => {
                return CommandResult {
                    ok: false,
                    command: command.to_string(),
                    message: result_message.to_string(),
                    data: Map::from_iter([
                        (
                            "scenario".to_string(),
                            json!(
                                scenario
                                    .get("name")
                                    .and_then(Value::as_str)
                                    .unwrap_or_default()
                            ),
                        ),
                        (
                            "scene".to_string(),
                            json!(
                                scenario
                                    .get("scene")
                                    .and_then(Value::as_str)
                                    .unwrap_or_default()
                            ),
                        ),
                        (
                            "scenario_path".to_string(),
                            json!(scenario_path.to_string_lossy().into_owned()),
                        ),
                        ("artifacts".to_string(), aggregate_artifacts.to_json()),
                        ("viewports".to_string(), json!(viewports)),
                        ("runs".to_string(), Value::Array(runs)),
                    ]),
                    failures: vec![FailureDetail {
                        kind: "artifact_write_failed".to_string(),
                        command: Some(command.to_string()),
                        path: Some(variant_root.to_string_lossy().into_owned()),
                        message: Some(error.to_string()),
                        expected: Some(json!(
                            "artifact metadata written under the artifact directory"
                        )),
                        actual: Some(json!("OSError")),
                        ..FailureDetail::default()
                    }],
                    exit_code: 1,
                };
            }
        };

        let variant_requires_real_visual_evidence =
            scenario_requires_windowed_media_evidence(&variant_scenario, false)
                || layout_mode.is_some();
        let variant_requires_audio = scenario_requires_audio_evidence(&variant_scenario);
        let runtime_result = execute_runtime_run(
            command,
            result_message,
            config,
            scenario_path,
            &variant_scenario,
            &variant_artifacts,
            command_line,
            false,
            selected_headless_for_scenario(
                headless_override.or_else(|| config.launch_mode_headless_override()),
                variant_requires_real_visual_evidence,
            ),
            selected_audio_for_scenario(
                audio_override.or_else(|| config.audio_mode_override()),
                variant_requires_audio,
            ),
            variant_requires_real_visual_evidence,
            RuntimeExecutionOptions {
                profile: command == "scenario profile",
                ..RuntimeExecutionOptions::default()
            },
        );

        let mut run_failures = runtime_result.failures.clone();
        if let Some(layout_mode) = layout_mode {
            if !run_failures
                .iter()
                .any(|failure| failure.kind == "scenario_result_invalid")
            {
                let snapshot_name = runtime_result
                    .data
                    .get("snapshots")
                    .and_then(Value::as_array)
                    .and_then(|snapshots| snapshots.last())
                    .and_then(Value::as_str);
                // A matrix cell with no snapshot to audit is NOT a silent pass:
                // layout_audit_failures emits `layout_audit_missing_snapshot`,
                // exactly as the single-run apply_layout_audit path does.
                run_failures.extend(layout_audit_failures(
                    command,
                    scenario,
                    &variant_artifacts,
                    Some(viewport),
                    snapshot_name,
                    layout_mode,
                ));
            }
        }

        let run_ok = runtime_result.ok && run_failures.is_empty();
        runs.push(json!({
            "viewport": viewport,
            "ok": run_ok,
            "godot_args": runtime_result.data.get("godot_args").cloned().unwrap_or_else(|| json!([])),
            "frame_pacing": runtime_result.data.get("frame_pacing").cloned().unwrap_or(Value::Null),
            "project_args": runtime_result.data.get("project_args").cloned().unwrap_or_else(|| json!([])),
            "artifacts": runtime_result.data.get("artifacts").cloned().unwrap_or_else(|| variant_artifacts.to_json()),
            "snapshot_artifacts": runtime_result.data.get("snapshot_artifacts").cloned().unwrap_or_else(|| json!({})),
            "snapshot": runtime_result
                .data
                .get("snapshots")
                .and_then(Value::as_array)
                .and_then(|snapshots| snapshots.last())
                .cloned()
                .unwrap_or(Value::Null),
            "duration_ms": runtime_result.data.get("duration_ms").cloned().unwrap_or(Value::Null),
            "display_server": runtime_result.data.get("display_server").cloned().unwrap_or(Value::Null),
            "launch_display": runtime_result.data.get("launch_display").cloned().unwrap_or(Value::Null),
            "failures": run_failures,
        }));
        flattened_failures.extend(run_failures);
    }

    let ok = flattened_failures.is_empty()
        && runs
            .iter()
            .all(|run| run.get("ok").and_then(Value::as_bool) == Some(true));
    let result = CommandResult {
        ok,
        command: command.to_string(),
        message: if ok {
            if command == "scenario run" {
                "scenario run passed".to_string()
            } else {
                "layout audit passed".to_string()
            }
        } else {
            result_message.to_string()
        },
        data: Map::from_iter([
            (
                "scenario".to_string(),
                json!(
                    scenario
                        .get("name")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                ),
            ),
            (
                "scene".to_string(),
                json!(
                    scenario
                        .get("scene")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                ),
            ),
            (
                "scenario_path".to_string(),
                json!(scenario_path.to_string_lossy().into_owned()),
            ),
            ("artifacts".to_string(), aggregate_artifacts.to_json()),
            ("viewports".to_string(), json!(viewports)),
            (
                "godot_args".to_string(),
                scenario
                    .get("godot_args")
                    .cloned()
                    .unwrap_or_else(|| json!([])),
            ),
            (
                "frame_pacing".to_string(),
                runs.iter()
                    .find_map(|run| {
                        run.get("frame_pacing")
                            .filter(|value| !value.is_null())
                            .cloned()
                    })
                    .unwrap_or(Value::Null),
            ),
            (
                "project_args".to_string(),
                scenario
                    .get("project_args")
                    .cloned()
                    .unwrap_or_else(|| json!([])),
            ),
            ("runs".to_string(), Value::Array(runs)),
        ]),
        failures: flattened_failures,
        exit_code: if ok { 0 } else { 1 },
    };
    persist_result(&aggregate_artifacts.result_path, result)
}


fn display_launch_metadata_json(display: &godot_qa_process::DisplayLaunchMetadata) -> Value {
    json!({
        "headless": display.headless,
        "selected_mode": display.selected_mode,
        "platform": display.platform,
        "linuxbsd_target": display.linuxbsd_target,
        "x11_display_available": display.x11_display_available,
        "wayland_display_available": display.wayland_display_available,
        "selected_display_driver": display.selected_display_driver,
        "project_prefers_wayland": display.project_prefers_wayland,
        "requested_display_driver": display.requested_display_driver,
        "virtual_display_used": display.virtual_display_used,
        "virtual_display_command": display.virtual_display_command,
        "fallback_reason": display.fallback_reason,
        "launch_prefix_used": display.launch_prefix_used,
        "windowed_launch_prefix_used": display.windowed_launch_prefix_used,
        "effective_command_prefix": display.effective_command_prefix,
        "display_backend_available": display.display_backend_available,
        "startup_attempted": display.startup_attempted,
        "virtual_display_exit_code": display.virtual_display_exit_code,
        "startup_failure_kind": display.startup_failure_kind,
        "startup_failure_retryable": display.startup_failure_retryable,
        "startup_failure_guidance": display.startup_failure_guidance,
        "lifecycle": {
            "startup_attempted": display.startup_attempted,
            "startup_failed": display.startup_failure_kind.is_some(),
            "virtual_display_used": display.virtual_display_used,
            "virtual_display_exit_code": display.virtual_display_exit_code,
        },
        "failure_category": display.startup_failure_kind.as_ref().map(|_| "display_infrastructure"),
        "retryable": display.startup_failure_retryable,
        "retry_guidance": display.startup_failure_guidance,
    })
}

fn visual_support_preflight_json(display: &godot_qa_process::DisplayLaunchMetadata) -> Value {
    let real_viewport_pixels = real_viewport_pixel_support(display);
    let real_pixels_available = real_viewport_pixels
        .get("available")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    json!({
        "real_viewport_pixels": real_viewport_pixels,
        "target_hidden_capture": derived_visual_support(
            real_pixels_available,
            display,
            "target_hidden_capture",
            "real viewport pixels and runtime support for hiding the resolved target during a capture",
            "Use --windowed with DISPLAY/WAYLAND_DISPLAY available or configure a virtual display such as xvfb-run; verify the target resolves to supported 2D geometry at runtime."
        ),
        "mask_generation": derived_visual_support(
            real_pixels_available,
            display,
            "mask_generation",
            "real viewport pixels and supported viewport-space target geometry for mask generation",
            "Use --windowed with DISPLAY/WAYLAND_DISPLAY available or configure a virtual display such as xvfb-run; use QA IDs or locators that resolve to supported 2D targets."
        ),
        "saved_snapshot_object_pixels": derived_visual_support(
            real_pixels_available,
            display,
            "saved_snapshot_object_pixels",
            "a saved snapshot with real viewport pixels plus supported 2D target geometry",
            "Capture the snapshot in a real display-backed run and target a supported 2D object by locator or qa_id."
        ),
    })
}

fn real_viewport_pixel_support(display: &godot_qa_process::DisplayLaunchMetadata) -> Value {
    let available = !display.headless && display.display_backend_available;
    let reason = visual_support_reason(display, available);
    json!({
        "available": available,
        "reason": reason,
        "expected": "windowed launch with a real display backend or detected virtual display wrapper",
        "actual": display_launch_metadata_json(display),
        "remediation": if available {
            "No launch remediation required; scenario runtime screenshot quality checks still validate nonblank real pixels."
        } else if display.headless {
            "Run visual scenarios with --windowed or omit --headless when real rendered pixels are required."
        } else {
            "Set DISPLAY or WAYLAND_DISPLAY, configure a launch prefix such as xvfb-run, or install a virtual display backend before running visual scenarios."
        },
    })
}

fn derived_visual_support(
    real_pixels_available: bool,
    display: &godot_qa_process::DisplayLaunchMetadata,
    capability: &str,
    expected: &str,
    remediation: &str,
) -> Value {
    let reason = if real_pixels_available {
        "launch_supported_runtime_target_validation_required".to_string()
    } else {
        format!(
            "unavailable_real_viewport_pixels_{}",
            visual_support_reason(display, false)
        )
    };
    json!({
        "available": real_pixels_available,
        "reason": reason,
        "expected": expected,
        "actual": {
            "capability": capability,
            "real_viewport_pixels_available": real_pixels_available,
            "launch_display": display_launch_metadata_json(display),
        },
        "remediation": if real_pixels_available {
            "No launch remediation required; target-specific support is validated when a scenario or saved snapshot assertion resolves the target."
        } else {
            remediation
        },
    })
}

fn visual_support_reason(
    display: &godot_qa_process::DisplayLaunchMetadata,
    available: bool,
) -> String {
    if available {
        if display.virtual_display_used {
            "virtual_display_available".to_string()
        } else {
            "display_backend_available".to_string()
        }
    } else if display.headless {
        "forced_headless".to_string()
    } else {
        display
            .fallback_reason
            .clone()
            .unwrap_or_else(|| "display_backend_unavailable".to_string())
    }
}

fn scenario_requires_windowed_media_evidence(
    scenario: &Value,
    has_screenshot_match_assertions: bool,
) -> bool {
    scenario_requests_screenshot(scenario)
        || has_screenshot_match_assertions
        || scenario_contains_visual_assertion(scenario)
        // A `layout_audit` reads the run's UI snapshot + screenshot evidence, so —
        // like screenshot scenarios — it needs a windowed, real-pixel session. A
        // single-viewport layout audit is therefore WINDOWED-warm (not headless).
        || layout_mode_from_scenario(scenario).is_some()
}

fn scenario_requires_audio_evidence(scenario: &Value) -> bool {
    scenario_contains_audio_capture(scenario)
}

fn selected_headless_for_scenario(
    headless_override: Option<bool>,
    requires_windowed_media_evidence: bool,
) -> bool {
    headless_override.unwrap_or(!requires_windowed_media_evidence)
}

fn selected_audio_for_scenario(audio_override: Option<bool>, requires_audio: bool) -> bool {
    audio_override.unwrap_or(requires_audio)
}
