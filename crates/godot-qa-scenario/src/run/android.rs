// Android one-shot scenario execution (`scenario run --target android`).
//
// Included into the run module (see `include!` at the bottom of mod.rs), so it
// shares every cold-path building block — `prepare_run_artifacts`,
// `finalize_runtime_run`, the scenario-introspection predicates, and the result
// persistence helpers — and reuses them verbatim, exactly like warm.rs does.
//
// The flow deploys ONE ephemeral Android scenario-mode session (a sceneless
// debug-APK deploy), drives the SAME `scenario.run` bridge command the warm path
// uses, then materializes the device-private result/evidence back into the host
// `.qa/artifacts` run tree. Because the engine reads every output path from
// request.json, rewriting the request's artifact map to device `user://` paths
// makes the executor write result.json + evidence under app-private storage; the
// CLI pulls them back and hands the SAME result.json to the SAME
// `finalize_runtime_run`, so desktop and Android produce an identical verdict /
// result.json shape. Device pacing is real, so the frame budget is stripped and
// only a wall-clock timeout is enforced.

use godot_qa_runtime::android_artifacts::AndroidArtifactMapper;

/// The device-private run root beneath the session artifact root. Every host-run
/// artifact string is rewritten to live beneath this, so the executor writes into
/// app-private storage.
const ANDROID_RUN_SUBDIR: &str = "run";

/// Resolved `scenario run --target android` inputs. The rejected desktop-only
/// display/audio/visual flags are carried so option validation can reject them
/// with the shared session-start failure shape.
pub struct ScenarioRunAndroidOptions<'a> {
    pub preset: Option<&'a str>,
    pub device: Option<&'a str>,
    pub package: Option<&'a str>,
    pub no_apk_cache: bool,
    /// `--audio`: accepted (device audio is real).
    pub audio: bool,
    /// Rejected display/audio/visual flags (host cannot control the device here).
    pub no_audio: bool,
    pub headless: bool,
    pub windowed: bool,
    pub visual_risk: bool,
    pub visual_risk_strict: bool,
}

// --- Option validation -----------------------------------------------------

/// Reuse the session-start `android_option_unsupported` failure shape for a
/// display/audio/visual flag that has no meaning against a real device.
fn android_scenario_option_unsupported(option: &str, actual: Value) -> FailureDetail {
    FailureDetail {
        kind: "android_option_unsupported".to_string(),
        command: Some("scenario run".to_string()),
        message: Some(format!(
            "Android one-shot scenarios do not support `{option}`"
        )),
        expected: Some(json!("option omitted for --target android")),
        actual: Some(actual),
        ..FailureDetail::default()
    }
}

/// Validate the resolved Android options. Returns the first rejection, or the
/// non-empty preset name on success.
fn validate_android_scenario_options<'a>(
    options: &ScenarioRunAndroidOptions<'a>,
) -> Result<&'a str, FailureDetail> {
    if options.headless {
        return Err(android_scenario_option_unsupported("--headless", json!(true)));
    }
    if options.windowed {
        return Err(android_scenario_option_unsupported("--windowed", json!(true)));
    }
    if options.no_audio {
        return Err(android_scenario_option_unsupported("--no-audio", json!(true)));
    }
    if options.visual_risk {
        return Err(android_scenario_option_unsupported(
            "--visual-risk",
            json!(true),
        ));
    }
    if options.visual_risk_strict {
        return Err(android_scenario_option_unsupported(
            "--visual-risk-strict",
            json!(true),
        ));
    }
    match options.preset.map(str::trim).filter(|value| !value.is_empty()) {
        Some(preset) => Ok(preset),
        None => Err(FailureDetail {
            kind: "android_preset_required".to_string(),
            command: Some("scenario run".to_string()),
            message: Some("Android one-shot scenarios require `--preset`".to_string()),
            expected: Some(json!("non-empty Android export preset name")),
            actual: Some(json!(options.preset)),
            ..FailureDetail::default()
        }),
    }
}

/// Reject the Android-only flags (`--preset`/`--device`/`--package`/
/// `--no-apk-cache`) when the run targets desktop.
pub fn desktop_rejects_android_scenario_flags(
    preset: Option<&str>,
    device: Option<&str>,
    package: Option<&str>,
    no_apk_cache: bool,
) -> Option<FailureDetail> {
    let offending = if preset.is_some() {
        "--preset"
    } else if device.is_some() {
        "--device"
    } else if package.is_some() {
        "--package"
    } else if no_apk_cache {
        "--no-apk-cache"
    } else {
        return None;
    };
    Some(FailureDetail {
        kind: "invalid_params".to_string(),
        command: Some("scenario run".to_string()),
        message: Some(format!("`{offending}` requires --target android")),
        expected: Some(json!("--target android when using Android deployment flags")),
        actual: Some(json!(offending)),
        ..FailureDetail::default()
    })
}

// --- Scenario preflight ----------------------------------------------------

/// True when the scenario declares an explicit `determinism.fixed_fps` (a
/// process-level pacing that the device cannot honor: device pacing is real).
fn scenario_declares_fixed_fps(scenario: &Value) -> bool {
    scenario
        .get("determinism")
        .and_then(Value::as_object)
        .and_then(|determinism| determinism.get("fixed_fps"))
        .is_some()
}

/// True when the scenario declares an explicit single `viewport` (a
/// host-controlled resize meaningless at the device's native resolution). A
/// normalized document carries `viewport` as an empty object when the scenario
/// omitted one, so an empty object is not a declaration.
fn scenario_declares_viewport(scenario: &Value) -> bool {
    scenario
        .get("viewport")
        .and_then(Value::as_object)
        .is_some_and(|viewport| !viewport.is_empty())
}

/// True when the scenario declares a non-empty `viewports` matrix.
fn scenario_declares_viewport_matrix(scenario: &Value) -> bool {
    scenario
        .get("viewports")
        .and_then(Value::as_array)
        .is_some_and(|viewports| !viewports.is_empty())
}

/// Reject a scenario that cannot run faithfully as an Android one-shot. Reuses
/// the existing cold/warm scenario-introspection predicates rather than
/// re-deriving them. Everything else (audio evidence, save_screenshot, visual
/// pixel + temporal assertions, time_control, project_args, wall-clock timeouts)
/// is allowed and runs on the real device.
fn classify_android_support(scenario: &Value) -> Result<(), FailureDetail> {
    let reason = if scenario_has_godot_args(scenario) {
        Some((
            "godot_args",
            "process-launch flags cannot be applied to a deployed device build",
            json!(scenario_godot_args(scenario)),
        ))
    } else if scenario_declares_fixed_fps(scenario) {
        Some((
            "fixed_fps",
            "device pacing is real; a process-level fixed_fps cannot be enforced",
            scenario
                .get("determinism")
                .and_then(|determinism| determinism.get("fixed_fps"))
                .cloned()
                .unwrap_or(Value::Null),
        ))
    } else if layout_mode_from_scenario(scenario).is_some() {
        Some((
            "layout_matrix",
            "layout-audit matrices require host-controlled viewport resizing",
            json!(layout_mode_from_scenario(scenario)),
        ))
    } else if scenario_declares_viewport_matrix(scenario) {
        Some((
            "multi_viewport",
            "a viewports matrix requires host-controlled viewport resizing",
            scenario.get("viewports").cloned().unwrap_or(Value::Null),
        ))
    } else if scenario_declares_viewport(scenario) {
        Some((
            "viewport_override",
            "an explicit viewport requires host-controlled viewport resizing",
            scenario.get("viewport").cloned().unwrap_or(Value::Null),
        ))
    } else if !screenshot_match_assertions(scenario).is_empty() {
        Some((
            "screenshot_match",
            "host-viewport baselines are meaningless at the device resolution",
            json!(screenshot_match_assertions(scenario)),
        ))
    } else {
        None
    };
    let Some((reason, guidance, actual)) = reason else {
        return Ok(());
    };
    let mut failure = FailureDetail {
        kind: "android_scenario_unsupported".to_string(),
        command: Some("scenario run".to_string()),
        message: Some(format!(
            "scenario cannot run as an Android one-shot: {guidance}"
        )),
        expected: Some(json!(
            "scenario without process-launch flags, explicit pacing, viewport/layout matrices, or host-viewport screenshot baselines"
        )),
        actual: Some(actual),
        ..FailureDetail::default()
    };
    failure.details.insert("reason".to_string(), json!(reason));
    Err(failure)
}

// --- Session-conflict guard ------------------------------------------------

/// New kind: a live Android singleton session conflicts with a one-shot deploy
/// (the device hosts one QA app instance and the singleton `.qa/session.json`
/// records one session). A desktop live session never conflicts.
fn android_session_active_failure(record: &godot_qa_runtime::SessionRecord) -> FailureDetail {
    FailureDetail {
        kind: "android_session_active".to_string(),
        command: Some("scenario run".to_string()),
        message: Some(
            "a live Android session is active; run `session stop` first".to_string(),
        ),
        expected: Some(json!("no active Android live session")),
        actual: Some(record.to_json_value()),
        ..FailureDetail::default()
    }
}

/// Detect a live Android singleton session. Loads the singleton record and, when
/// it is an Android target, probes the recorded device PID; a matching live PID
/// is a conflict. A missing record, a desktop record, or an unreachable/stale
/// device is not a conflict.
fn android_session_active_conflict(config: &ProjectConfig) -> Option<FailureDetail> {
    let record = godot_qa_runtime::load_session_record(config, "scenario run").ok()?;
    if record.target != godot_qa_runtime::SessionTarget::Android {
        return None;
    }
    let android = record.android.as_ref()?;
    match godot_qa_process::android::query_pid(
        &android.adb_path,
        &android.device_serial,
        &android.package,
    ) {
        Ok(Some(pid)) if pid == android.device_pid => Some(android_session_active_failure(&record)),
        _ => None,
    }
}

// --- Synthesized process / display metadata --------------------------------

/// Androidized display metadata: the real device renders windowed with a live
/// display backend, so the shared finalizer records the run as windowed/available
/// with an `android` driver.
fn android_display_metadata() -> godot_qa_process::DisplayLaunchMetadata {
    godot_qa_process::DisplayLaunchMetadata {
        headless: false,
        selected_mode: "windowed".to_string(),
        platform: "android".to_string(),
        linuxbsd_target: false,
        x11_display_available: false,
        wayland_display_available: false,
        selected_display_driver: Some("android".to_string()),
        project_prefers_wayland: false,
        requested_display_driver: None,
        virtual_display_used: false,
        virtual_display_command: None,
        fallback_reason: None,
        launch_prefix_used: false,
        windowed_launch_prefix_used: false,
        effective_command_prefix: Vec::new(),
        display_backend_available: true,
        startup_attempted: true,
        virtual_display_exit_code: None,
        startup_failure_kind: None,
        startup_failure_retryable: false,
        startup_failure_guidance: None,
    }
}

/// A successful synthesized process result so the shared finalizer reads the
/// device-written result.json exactly as it does for a cold process. Mirrors
/// `synth_warm_process_result`, with androidized display metadata.
fn synth_android_process_result() -> godot_qa_process::GodotRunResult {
    godot_qa_process::GodotRunResult {
        command: Vec::new(),
        returncode: 0,
        stdout: String::new(),
        stderr: String::new(),
        display: android_display_metadata(),
        pid: None,
        process_group: None,
        cleanup: None,
        infrastructure_failure_kind: None,
        timed_out: false,
        timeout_ms: None,
        output_truncation: None,
    }
}

fn project_relative_path(project_root: &Path, path: &str) -> String {
    Path::new(path)
        .strip_prefix(project_root)
        .map(|relative| relative.to_string_lossy().into_owned())
        .unwrap_or_else(|_| path.to_string())
}

/// Build the pre-finalize metadata for an Android one-shot, mirroring the desktop
/// base_data but with androidized display, a `target: "android"` marker, and an
/// `android { ... }` block. Deliberately omits `frame_pacing` (device pacing is
/// real). `cleanup_complete` is stamped later by the cleanup step.
fn android_base_data(
    config: &ProjectConfig,
    scenario_path: &Path,
    scenario: &Value,
    artifacts: &artifacts::RunArtifacts,
    audio: bool,
    record: &godot_qa_runtime::SessionRecord,
    android: &godot_qa_runtime::AndroidSessionMetadata,
) -> Map<String, Value> {
    let godot_args = merged_launch_args(&config.godot_args, &scenario_godot_args(scenario));
    let project_args =
        merged_launch_args(&config.godot_project_args, &scenario_project_args(scenario));
    let mut base_data = Map::from_iter([
        (
            "scenario".to_string(),
            json!(scenario.get("name").and_then(Value::as_str).unwrap_or_default()),
        ),
        (
            "scene".to_string(),
            json!(scenario.get("scene").and_then(Value::as_str).unwrap_or_default()),
        ),
        (
            "scenario_path".to_string(),
            json!(scenario_path.to_string_lossy().into_owned()),
        ),
        ("artifacts".to_string(), artifacts.to_json()),
    ]);
    base_data.insert("godot_args".to_string(), json!(godot_args));
    base_data.insert(
        "project_args".to_string(),
        redact_project_args_for_metadata(&json!(project_args)),
    );
    base_data.insert("headless".to_string(), json!(false));
    base_data.insert("audio".to_string(), json!(audio));
    base_data.insert(
        "launch_display".to_string(),
        display_launch_metadata_json(&android_display_metadata()),
    );
    base_data.insert("target".to_string(), json!("android"));
    base_data.insert(
        "android".to_string(),
        json!({
            "session_id": record.session_id,
            "device": android.device_serial,
            "package": android.package,
            "preset": android.preset,
            "device_pid": android.device_pid,
            "forwarded_host_port": android.forwarded_host_port,
            "device_artifact_root": android.device_artifact_root,
            "deployment_log": project_relative_path(&config.project_root, &android.deployment_log_path),
            "logcat": project_relative_path(&config.project_root, &android.logcat_log_path),
            "deployment_mode": "debug_apk",
        }),
    );
    base_data
}

// --- Request rewrite -------------------------------------------------------

/// Load the host request.json, strip `run.frame_budget` (device pacing is real,
/// so the engine must not enforce a project-default-fps frame budget), re-persist
/// the host copy so the on-disk record matches what ran, then rewrite every
/// host-run-root artifact string to its device path. Returns the device request
/// bytes to push. `res://`/project paths outside the run root are left untouched.
fn android_scenario_request_bytes(
    mapper: &AndroidArtifactMapper,
    artifacts: &artifacts::RunArtifacts,
) -> Result<Vec<u8>, FailureDetail> {
    let raw = fs::read_to_string(&artifacts.request_path)
        .map_err(|error| android_artifact_io_failure(&artifacts.request_path, &error))?;
    let mut request = serde_json::from_str::<Value>(&raw)
        .map_err(|error| android_request_invalid_failure(&artifacts.request_path, &error))?;
    if let Some(run) = request.get_mut("run").and_then(Value::as_object_mut) {
        run.remove("frame_budget");
    }
    // Re-persist the host copy so the on-disk request matches what actually ran.
    write_json(&artifacts.request_path, &request)
        .map_err(|error| android_artifact_io_failure(&artifacts.request_path, &error))?;
    let mut device_request = request;
    mapper.map_outgoing_params(&mut device_request)?;
    Ok(serde_json::to_vec_pretty(&device_request).unwrap_or_else(|_| b"{}".to_vec()))
}

/// Push the rewritten request.json into app-private storage and create the run
/// subdirectories the executor writes into (mirrors `prepare_run_artifacts`'s
/// snapshots/temporal/audio set).
fn push_android_scenario_request(
    mapper: &AndroidArtifactMapper,
    artifacts: &artifacts::RunArtifacts,
    android: &godot_qa_runtime::AndroidSessionMetadata,
    session_id: &str,
) -> Result<(), FailureDetail> {
    let bytes = android_scenario_request_bytes(mapper, artifacts)?;
    godot_qa_process::android::write_private_file(
        &android.adb_path,
        &android.device_serial,
        &android.package,
        session_id,
        &format!("{ANDROID_RUN_SUBDIR}/request.json"),
        &bytes,
    )?;
    godot_qa_process::android::make_private_dirs(
        &android.adb_path,
        &android.device_serial,
        &android.package,
        session_id,
        &[
            &format!("{ANDROID_RUN_SUBDIR}/snapshots"),
            &format!("{ANDROID_RUN_SUBDIR}/temporal"),
            &format!("{ANDROID_RUN_SUBDIR}/audio"),
        ],
    )
}

// --- Result collection -----------------------------------------------------

/// Replace the run-level artifact paths the executor echoes verbatim (from the
/// request's `_artifacts`) with the HOST run layout everywhere they appear
/// (top-level `artifacts` and every `failures[].artifacts`). This substitutes the
/// echoed device paths — including `logs_path`/`meta_path`/`slow_summary`, which
/// never exist as device files — so the subsequent `materialize_response` only
/// pulls the real claimed evidence (snapshots, screenshots, wavs, temporal,
/// failure-attached artifacts) and never tries to `cat` a nonexistent device file.
fn substitute_echoed_run_artifacts(value: &mut Value, host_artifacts: &Map<String, Value>) {
    if let Value::Object(object) = value {
        if let Some(Value::Object(artifacts)) = object.get_mut("artifacts") {
            for (key, host_value) in host_artifacts {
                if let Some(slot) = artifacts.get_mut(key) {
                    *slot = host_value.clone();
                }
            }
        }
    }
    match value {
        Value::Object(object) => {
            for child in object.values_mut() {
                substitute_echoed_run_artifacts(child, host_artifacts);
            }
        }
        Value::Array(items) => {
            for item in items {
                substitute_echoed_run_artifacts(item, host_artifacts);
            }
        }
        _ => {}
    }
}

/// Production device-artifact puller: binary-safe `read_private_artifact`
/// (`adb exec-out run-as … cat`) into a caller-owned temp path; the mapper renames
/// it into place atomically. Mirrors transport.rs's puller closure.
fn android_scenario_puller<'a>(
    android: &'a godot_qa_runtime::AndroidSessionMetadata,
    session_id: &'a str,
) -> impl FnMut(&str, &Path) -> Result<(), FailureDetail> + 'a {
    move |device_path: &str, temporary: &Path| {
        let bytes = godot_qa_process::android::read_private_artifact(
            &android.adb_path,
            &android.device_serial,
            &android.package,
            session_id,
            device_path,
        )?;
        fs::write(temporary, bytes).map_err(|error| android_artifact_io_failure(temporary, &error))
    }
}

/// Pull + rewrite the device result.json and its claimed evidence into the host
/// run tree, then best-effort mirror the progress heartbeat. The device
/// result.json (a flat engine payload) is pulled, its echoed run-artifact map is
/// substituted for the host layout, its remaining evidence is materialized, and
/// the rewritten copy is written to the host `result_path` for the finalizer.
fn collect_android_scenario_result(
    mapper: &AndroidArtifactMapper,
    artifacts: &artifacts::RunArtifacts,
    android: &godot_qa_runtime::AndroidSessionMetadata,
    session_id: &str,
    device_run_root: &str,
) -> Result<(), FailureDetail> {
    let device_result_path = format!("{device_run_root}/result.json");
    let result_bytes = godot_qa_process::android::read_private_artifact(
        &android.adb_path,
        &android.device_serial,
        &android.package,
        session_id,
        &device_result_path,
    )?;
    let host_artifacts = artifacts
        .to_json()
        .as_object()
        .cloned()
        .unwrap_or_default();
    let mut puller = android_scenario_puller(android, session_id);
    let result = rewrite_collected_result(&result_bytes, &host_artifacts, mapper, &mut puller)?;

    write_json(&artifacts.result_path, &result)
        .map_err(|error| android_artifact_io_failure(&artifacts.result_path, &error))?;

    collect_android_progress(mapper, artifacts, android, session_id, device_run_root);
    Ok(())
}

/// The result walk (no ADB): parse the device result.json, substitute the echoed
/// run-artifact map for the host layout, then materialize the remaining claimed
/// evidence via the supplied puller. Factored out so tests can drive it with a
/// fake puller. The echoed run-level artifacts are substituted BEFORE materialize
/// so their host-only members (logs.txt, meta.json, slow-summary.json) are never
/// pulled from device; only real evidence is.
fn rewrite_collected_result(
    result_bytes: &[u8],
    host_artifacts: &Map<String, Value>,
    mapper: &AndroidArtifactMapper,
    puller: &mut impl godot_qa_runtime::android_artifacts::AndroidArtifactPuller,
) -> Result<Value, FailureDetail> {
    let mut result = serde_json::from_slice::<Value>(result_bytes)
        .map_err(|error| android_result_invalid_failure("device result.json", &error))?;
    substitute_echoed_run_artifacts(&mut result, host_artifacts);
    mapper.materialize_response(&mut result, puller)?;
    Ok(result)
}

/// Rewrite each JSON progress line's device paths to host paths WITHOUT pulling
/// (heartbeat artifacts are not yet materialized). Non-JSON lines pass through and
/// lines whose paths fail beneath-root validation are dropped.
fn rewrite_progress_lines(text: &str, mapper: &AndroidArtifactMapper) -> String {
    let mut rewritten = String::new();
    for line in text.lines() {
        if line.trim().is_empty() {
            continue;
        }
        match serde_json::from_str::<Value>(line) {
            Ok(mut value) => {
                if mapper.rewrite_paths_only(&mut value).is_err() {
                    continue;
                }
                rewritten.push_str(&serde_json::to_string(&value).unwrap_or_default());
                rewritten.push('\n');
            }
            Err(_) => {
                rewritten.push_str(line);
                rewritten.push('\n');
            }
        }
    }
    rewritten
}

/// Best-effort progress mirror: pull the device progress.jsonl, rewrite paths in
/// each line WITHOUT pulling (heartbeat artifacts are not yet materialized), and
/// write the host copy. A missing device progress file is not an error.
fn collect_android_progress(
    mapper: &AndroidArtifactMapper,
    artifacts: &artifacts::RunArtifacts,
    android: &godot_qa_runtime::AndroidSessionMetadata,
    session_id: &str,
    device_run_root: &str,
) {
    let device_progress_path = format!("{device_run_root}/progress.jsonl");
    let Ok(bytes) = godot_qa_process::android::read_private_artifact(
        &android.adb_path,
        &android.device_serial,
        &android.package,
        session_id,
        &device_progress_path,
    ) else {
        return;
    };
    let text = String::from_utf8_lossy(&bytes);
    let rewritten = rewrite_progress_lines(&text, mapper);
    let _ = fs::write(&artifacts.progress_path, rewritten);
}

// --- Failure classification ------------------------------------------------

/// The transport's own default `scenario.run` budget (seconds) when the scenario
/// declares no timeout. Mirrors `LIVE_SCENARIO_RUN_DEFAULT_TIMEOUT_SECONDS`.
const ANDROID_DEFAULT_SCENARIO_TIMEOUT_SECONDS: f64 = 120.0;

fn android_transport_budget_seconds(scenario_timeout: Option<Duration>) -> f64 {
    scenario_timeout
        .map(|timeout| timeout.as_secs_f64())
        .unwrap_or(ANDROID_DEFAULT_SCENARIO_TIMEOUT_SECONDS)
}

/// Classify a failed `scenario.run` round-trip by probing the device app state,
/// then delegating to the pure classifier. A live app points at a real timeout; a
/// gone app points at a crash; an unqueryable device points at device loss.
fn classify_android_roundtrip_failure(
    config: &ProjectConfig,
    android: &godot_qa_runtime::AndroidSessionMetadata,
    scenario_timeout: Option<Duration>,
) -> FailureDetail {
    let pid_probe = godot_qa_process::android::query_pid(
        &android.adb_path,
        &android.device_serial,
        &android.package,
    );
    android_roundtrip_failure_from_probe(config, android, scenario_timeout, pid_probe)
}

/// Pure classifier (unit-testable): a still-running app after a failed round-trip
/// reuses the desktop `scenario_run_timeout`; a vanished app is a NEW
/// `android_app_crashed` (both retained host logs named); an unqueryable device
/// reuses `android_device_unavailable`.
fn android_roundtrip_failure_from_probe(
    config: &ProjectConfig,
    android: &godot_qa_runtime::AndroidSessionMetadata,
    scenario_timeout: Option<Duration>,
    pid_probe: Result<Option<i64>, FailureDetail>,
) -> FailureDetail {
    match pid_probe {
        Ok(Some(_pid)) => {
            let mut failure = FailureDetail {
                kind: "scenario_run_timeout".to_string(),
                command: Some("scenario run".to_string()),
                message: Some(
                    "Android scenario did not complete within the transport budget while the app was still running"
                        .to_string(),
                ),
                expected: Some(json!("scenario.run completes within the transport budget")),
                actual: Some(json!("transport_budget_exceeded")),
                ..FailureDetail::default()
            };
            failure.details.insert("device".to_string(), json!(android.device_serial));
            failure.details.insert("package".to_string(), json!(android.package));
            failure.details.insert(
                "transport_budget_seconds".to_string(),
                json!(android_transport_budget_seconds(scenario_timeout)),
            );
            failure.details.insert(
                "logcat".to_string(),
                json!(project_relative_path(&config.project_root, &android.logcat_log_path)),
            );
            failure
        }
        Ok(None) => {
            let mut failure = FailureDetail {
                kind: "android_app_crashed".to_string(),
                command: Some("scenario run".to_string()),
                message: Some(
                    "the Android app exited before the scenario completed; inspect the retained logcat and deployment log"
                        .to_string(),
                ),
                expected: Some(json!("the deployed app stays alive until scenario.run completes")),
                actual: Some(json!("process_exited")),
                ..FailureDetail::default()
            };
            failure.details.insert("device".to_string(), json!(android.device_serial));
            failure.details.insert("package".to_string(), json!(android.package));
            failure.details.insert(
                "logcat".to_string(),
                json!(project_relative_path(&config.project_root, &android.logcat_log_path)),
            );
            failure.details.insert(
                "deployment_log".to_string(),
                json!(project_relative_path(&config.project_root, &android.deployment_log_path)),
            );
            failure
        }
        Err(probe_failure) => {
            let mut failure = FailureDetail {
                kind: "android_device_unavailable".to_string(),
                command: Some("scenario run".to_string()),
                message: Some(
                    "the Android device became unavailable while running the scenario".to_string(),
                ),
                expected: Some(json!("reachable ADB device")),
                actual: probe_failure
                    .actual
                    .clone()
                    .or_else(|| Some(json!("unavailable"))),
                ..FailureDetail::default()
            };
            failure.details.insert("device".to_string(), json!(android.device_serial));
            failure.details.insert("package".to_string(), json!(android.package));
            failure
        }
    }
}

/// Wrap a mid-run collection failure (a claimed-evidence pull that failed) with the
/// run's device/package context. Keeps the underlying kind (e.g.
/// `android_artifact_pull_failed`) so the transport diagnostics carry through.
fn wrap_android_collection_failure(
    mut failure: FailureDetail,
    android: &godot_qa_runtime::AndroidSessionMetadata,
) -> FailureDetail {
    failure
        .details
        .entry("device".to_string())
        .or_insert_with(|| json!(android.device_serial));
    failure
        .details
        .entry("package".to_string())
        .or_insert_with(|| json!(android.package));
    failure
}

// --- Failure helpers -------------------------------------------------------

fn android_artifact_io_failure(path: &Path, error: &std::io::Error) -> FailureDetail {
    FailureDetail {
        kind: "artifact_write_failed".to_string(),
        command: Some("scenario run".to_string()),
        path: Some(path.to_string_lossy().into_owned()),
        message: Some(error.to_string()),
        expected: Some(json!("Android one-shot artifact written to disk")),
        actual: Some(json!("OSError")),
        ..FailureDetail::default()
    }
}

fn android_request_invalid_failure(path: &Path, error: &serde_json::Error) -> FailureDetail {
    FailureDetail {
        kind: "artifact_read_failed".to_string(),
        command: Some("scenario run".to_string()),
        path: Some(path.to_string_lossy().into_owned()),
        message: Some(error.to_string()),
        expected: Some(json!("valid host request.json")),
        actual: Some(json!("JSONDecodeError")),
        ..FailureDetail::default()
    }
}

fn android_result_invalid_failure(device_path: &str, error: &serde_json::Error) -> FailureDetail {
    FailureDetail {
        kind: "scenario_result_invalid".to_string(),
        command: Some("scenario run".to_string()),
        path: Some(device_path.to_string()),
        message: Some(error.to_string()),
        expected: Some(json!("valid device result.json")),
        actual: Some(json!("JSONDecodeError")),
        ..FailureDetail::default()
    }
}

fn scenario_run_early_failure(failure: FailureDetail) -> CommandResult {
    CommandResult {
        ok: false,
        command: "scenario run".to_string(),
        message: "scenario run failed".to_string(),
        data: Map::new(),
        failures: vec![failure],
        exit_code: 1,
    }
}

fn android_scenario_failure_result(
    base_data: Map<String, Value>,
    failure: FailureDetail,
    artifacts: &artifacts::RunArtifacts,
) -> CommandResult {
    let artifacts_map = artifacts.to_json().as_object().cloned().unwrap_or_default();
    CommandResult {
        ok: false,
        command: "scenario run".to_string(),
        message: "scenario run failed".to_string(),
        data: base_data,
        failures: vec![failure_with_artifacts(failure, &artifacts_map)],
        exit_code: 1,
    }
}

/// Stamp the cleanup outcome onto the finished result: record
/// `android.cleanup_complete`, append any cleanup failures, and force a non-zero /
/// `ok=false` outcome when cleanup did not fully succeed. Re-persists result.json
/// so the on-disk artifact reflects the cleanup verdict.
fn apply_android_cleanup_outcome(
    result: &mut CommandResult,
    cleanup_failures: Vec<FailureDetail>,
    artifacts: &artifacts::RunArtifacts,
) {
    let complete = cleanup_failures.is_empty();
    if let Some(android) = result.data.get_mut("android").and_then(Value::as_object_mut) {
        android.insert("cleanup_complete".to_string(), json!(complete));
    }
    if !complete {
        result.ok = false;
        result.exit_code = 1;
        result.failures.extend(cleanup_failures);
    }
    let _ = write_result_artifact(&artifacts.result_path, result);
}

// --- Run flow --------------------------------------------------------------

/// Entry point for `scenario run --target android`. Validates options, preflights
/// scenario support, guards against a live Android session, prepares the host run
/// tree, deploys an ephemeral scenario-mode session, then runs and cleans up.
pub fn run_scenario_once_android(
    config: &ProjectConfig,
    scenario_path: &Path,
    scenario: &Value,
    command_line: &str,
    options: ScenarioRunAndroidOptions<'_>,
) -> CommandResult {
    let command = "scenario run";
    if let Some(failure) = config.project_config_failure_for_command(command) {
        return scenario_run_early_failure(failure);
    }
    let preset = match validate_android_scenario_options(&options) {
        Ok(preset) => preset,
        Err(failure) => return scenario_run_early_failure(failure),
    };
    let scenario_timeout = match scenario_timeout_duration(command, scenario_path, scenario) {
        Ok(timeout) => timeout,
        Err(failure) => return scenario_run_early_failure(failure),
    };
    if let Err(failure) = classify_android_support(scenario) {
        return scenario_run_early_failure(failure);
    }
    if let Some(failure) = android_session_active_conflict(config) {
        return scenario_run_early_failure(failure);
    }

    let scenario_for_artifacts =
        scenario_with_effective_launch_args(config, &runtime_scenario_payload(scenario));
    let artifacts = match artifacts::prepare_run_artifacts(
        &config.artifacts_dir,
        &scenario_for_artifacts,
        &config.project_root,
        scenario_path,
        command_line,
        config.godot_binary.as_deref(),
        &config.godot_launch_prefix,
        &config.godot_windowed_launch_prefix,
        None,
        None,
    ) {
        Ok(artifacts) => artifacts,
        Err(error) => {
            return scenario_run_early_failure(FailureDetail {
                kind: "artifact_write_failed".to_string(),
                command: Some(command.to_string()),
                path: Some(config.artifacts_dir.to_string_lossy().into_owned()),
                message: Some(error.to_string()),
                expected: Some(json!(
                    "artifact metadata written under the artifact directory"
                )),
                actual: Some(json!("OSError")),
                ..FailureDetail::default()
            });
        }
    };

    let android_dir = artifacts.root_dir.join("android");
    let scenario_project_args = scenario_project_args(scenario);
    let request = godot_qa_runtime::AndroidScenarioSessionRequest {
        preset,
        device: options.device,
        package: options.package,
        project_args: &scenario_project_args,
        no_apk_cache: options.no_apk_cache,
        deployment_log: android_dir.join("deployment.log"),
        logcat_log: android_dir.join("logcat.txt"),
    };
    let handle = match godot_qa_runtime::start_android_scenario_session(config, &request) {
        Ok(handle) => handle,
        Err(failure) => {
            // The deploy already ran its own scoped cleanup on failure.
            let mut data = Map::from_iter([
                (
                    "scenario".to_string(),
                    json!(scenario.get("name").and_then(Value::as_str).unwrap_or_default()),
                ),
                (
                    "scene".to_string(),
                    json!(scenario.get("scene").and_then(Value::as_str).unwrap_or_default()),
                ),
                (
                    "scenario_path".to_string(),
                    json!(scenario_path.to_string_lossy().into_owned()),
                ),
                ("artifacts".to_string(), artifacts.to_json()),
                ("target".to_string(), json!("android")),
            ]);
            data.insert("headless".to_string(), json!(false));
            return persist_result(
                &artifacts.result_path,
                android_scenario_failure_result(data, failure, &artifacts),
            );
        }
    };

    let audio_override = options.audio.then_some(true);
    let audio = selected_audio_for_scenario(audio_override, scenario_requires_audio_evidence(scenario));
    execute_android_scenario_on_session(
        config,
        &handle,
        scenario_path,
        scenario,
        &artifacts,
        scenario_timeout,
        audio,
    )
}

/// Post-deploy loop: request push, `scenario.run` dispatch, result/evidence
/// collection, shared finalize, and mandatory cleanup. Factored out so tests can
/// drive it with a fake TCP bridge + fake ADB against a hand-built handle, without
/// running a real deploy. Cleanup ALWAYS runs (success or failure).
fn execute_android_scenario_on_session(
    config: &ProjectConfig,
    handle: &godot_qa_runtime::PooledSessionHandle,
    scenario_path: &Path,
    scenario: &Value,
    artifacts: &artifacts::RunArtifacts,
    scenario_timeout: Option<Duration>,
    audio: bool,
) -> CommandResult {
    let mut result = run_android_scenario_core(
        config,
        handle,
        scenario_path,
        scenario,
        artifacts,
        scenario_timeout,
        audio,
    );
    let cleanup_failures = godot_qa_runtime::stop_android_scenario_session(config, handle);
    apply_android_cleanup_outcome(&mut result, cleanup_failures, artifacts);
    result
}

/// The device round-trip (no cleanup): build metadata, push the request, dispatch
/// `scenario.run`, collect the result/evidence, and finalize through the shared
/// verdict path — identical to the desktop/warm finalize.
fn run_android_scenario_core(
    config: &ProjectConfig,
    handle: &godot_qa_runtime::PooledSessionHandle,
    scenario_path: &Path,
    scenario: &Value,
    artifacts: &artifacts::RunArtifacts,
    scenario_timeout: Option<Duration>,
    audio: bool,
) -> CommandResult {
    let command = "scenario run";
    let result_message = "scenario run failed";
    let record = &handle.record;
    let Some(android) = record.android.as_ref() else {
        return persist_result(
            &artifacts.result_path,
            android_scenario_failure_result(
                Map::new(),
                FailureDetail {
                    kind: "session_file_invalid".to_string(),
                    command: Some(command.to_string()),
                    message: Some("Android session metadata is missing".to_string()),
                    expected: Some(json!("session handle with android metadata")),
                    actual: Some(json!("missing")),
                    ..FailureDetail::default()
                },
                artifacts,
            ),
        );
    };
    let session_id = record.session_id.clone();
    let base_data =
        android_base_data(config, scenario_path, scenario, artifacts, audio, record, android);

    let device_run_root = format!("{}/{ANDROID_RUN_SUBDIR}", android.device_artifact_root);
    let mapper = match AndroidArtifactMapper::new(&artifacts.root_dir, device_run_root.clone()) {
        Ok(mapper) => mapper,
        Err(failure) => {
            return persist_result(
                &artifacts.result_path,
                android_scenario_failure_result(base_data, failure, artifacts),
            );
        }
    };

    if let Err(failure) = push_android_scenario_request(&mapper, artifacts, android, &session_id) {
        return persist_result(
            &artifacts.result_path,
            android_scenario_failure_result(base_data, failure, artifacts),
        );
    }

    let mut params = Map::new();
    params.insert(
        "request_path".to_string(),
        json!(format!("{device_run_root}/request.json")),
    );
    if let Some(timeout) = scenario_timeout {
        params.insert("timeout_seconds".to_string(), json!(timeout.as_secs_f64()));
    }
    let live = godot_qa_runtime::pooled_session_command(
        handle,
        command,
        godot_qa_runtime::SCENARIO_RUN,
        params,
    );
    if !live.ok {
        let failure = classify_android_roundtrip_failure(config, android, scenario_timeout);
        return persist_result(
            &artifacts.result_path,
            android_scenario_failure_result(base_data, failure, artifacts),
        );
    }

    if let Err(failure) =
        collect_android_scenario_result(&mapper, artifacts, android, &session_id, &device_run_root)
    {
        return persist_result(
            &artifacts.result_path,
            android_scenario_failure_result(
                base_data,
                wrap_android_collection_failure(failure, android),
                artifacts,
            ),
        );
    }

    let process_result = synth_android_process_result();
    finalize_runtime_run(
        command,
        result_message,
        config,
        scenario_path,
        scenario,
        artifacts,
        false,
        false,
        RuntimeExecutionOptions {
            timeout: scenario_timeout,
            ..RuntimeExecutionOptions::default()
        },
        base_data,
        &process_result,
    )
}

include!("tests_android.rs");
