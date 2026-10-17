use crate::artifacts;
use crate::scenario_doc;
use godot_qa_core::config::ProjectConfig;
use godot_qa_core::results::{CommandResult, FailureDetail};
use godot_qa_process;
use godot_qa_runtime::{self, SessionRecord};
use serde_json::{Map, Value, json};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const PROCESS_STOP_WAIT_TIMEOUT_SECONDS: f64 = 3.0;

#[derive(Debug, Clone, Copy)]
pub struct RecordOptions<'a> {
    pub input_script: Option<&'a str>,
    pub duration_frames: Option<i64>,
    pub headless: bool,
    pub joy_motion: JoyMotionRecordOptions,
}

/// Analog quantization thresholds for the runtime recorder's joypad-motion
/// capture, mirrored by the `qa_recorder.gd` defaults.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct JoyMotionRecordOptions {
    pub step: f64,
    pub deadzone: f64,
    pub frame_gap: i64,
}

impl Default for JoyMotionRecordOptions {
    fn default() -> Self {
        Self {
            step: 0.25,
            deadzone: 0.1,
            frame_gap: 6,
        }
    }
}

pub fn run_scenario_record(
    config: &ProjectConfig,
    name: &str,
    scene: &str,
    viewport_raw: &str,
    out: Option<&str>,
    options: RecordOptions<'_>,
    command_line: &str,
) -> CommandResult {
    let command = "scenario record";
    if options.headless {
        return failure_result(
            command,
            "unable to record scenario",
            vec![FailureDetail {
                kind: "headless_screenshot_unsupported".to_string(),
                command: Some(command.to_string()),
                message: Some(
                    "scenario record captures initial and final screenshots and requires a windowed Godot session"
                        .to_string(),
                ),
                expected: Some(json!("scenario record without --headless")),
                actual: Some(json!({"headless": true})),
                severity: Some("error".to_string()),
                ..FailureDetail::default()
            }],
            Map::new(),
        );
    }
    if options.duration_frames.is_some_and(|frames| frames < 0) {
        return failure_result(
            command,
            "unable to record scenario",
            vec![FailureDetail {
                kind: "invalid_duration_frames".to_string(),
                command: Some(command.to_string()),
                message: Some("duration_frames must be a non-negative integer".to_string()),
                expected: Some(json!("duration_frames >= 0")),
                actual: Some(json!(options.duration_frames)),
                ..FailureDetail::default()
            }],
            Map::new(),
        );
    }
    if let Some(failure) = invalid_joy_motion_options_failure(command, &options.joy_motion) {
        return failure_result(
            command,
            "unable to record scenario",
            vec![failure],
            Map::new(),
        );
    }
    if !config.project_root.exists() {
        return failure_result(
            command,
            "unable to record scenario",
            vec![FailureDetail {
                kind: "missing_project_root".to_string(),
                command: Some(command.to_string()),
                path: Some(config.project_root.to_string_lossy().into_owned()),
                message: Some("project root does not exist".to_string()),
                expected: Some(json!("existing project root")),
                actual: Some(json!("missing")),
                ..FailureDetail::default()
            }],
            Map::new(),
        );
    }
    let input_steps = match options.input_script {
        Some(script_path) => {
            let script_path = resolve_input_script_path(&config.project_root, script_path);
            match parse_input_script(&script_path, command) {
                Ok(steps) => Some(steps),
                Err(failure) => {
                    return failure_result(
                        command,
                        "unable to record scenario",
                        vec![failure],
                        Map::new(),
                    );
                }
            }
        }
        None => None,
    };
    if let Some(failure) = config.project_config_failure_for_command(command) {
        return failure_result(
            command,
            "unable to record scenario",
            vec![failure],
            Map::new(),
        );
    }
    let Some(binary) = config.godot_binary.as_deref() else {
        return failure_result(
            command,
            "unable to record scenario",
            vec![FailureDetail {
                kind: "missing_godot".to_string(),
                command: Some(command.to_string()),
                message: Some("Godot binary not found in PATH or GODOT_BIN".to_string()),
                expected: Some(json!(
                    "discoverable Godot binary via --godot-bin, GODOT_BIN, or PATH"
                )),
                actual: Some(json!("missing")),
                ..FailureDetail::default()
            }],
            Map::new(),
        );
    };
    if !(config.addon_dir.join("plugin.cfg")).exists() {
        return failure_result(
            command,
            "unable to record scenario",
            vec![FailureDetail {
                kind: "missing_addon".to_string(),
                command: Some(command.to_string()),
                path: Some(config.addon_dir.to_string_lossy().into_owned()),
                message: Some(
                    "godot_qa addon must be installed in the target project for scenario record"
                        .to_string(),
                ),
                expected: Some(json!("installed addon in target project")),
                actual: Some(json!("missing")),
                ..FailureDetail::default()
            }],
            Map::new(),
        );
    }

    let Some(viewport) = parse_viewport(viewport_raw) else {
        return failure_result(
            command,
            "unable to record scenario",
            vec![FailureDetail {
                kind: "invalid_viewport".to_string(),
                command: Some(command.to_string()),
                message: Some("viewport must use WIDTHxHEIGHT".to_string()),
                expected: Some(json!("positive WIDTHxHEIGHT viewport string")),
                actual: Some(json!(viewport_raw)),
                ..FailureDetail::default()
            }],
            Map::new(),
        );
    };

    if !scenario_doc::is_safe_scenario_name(name) {
        return failure_result(
            command,
            "unable to record scenario",
            vec![FailureDetail {
                kind: "scenario_validation_failed".to_string(),
                command: Some(command.to_string()),
                message: Some(
                    "scenario field 'name' must be a safe artifact path segment".to_string(),
                ),
                expected: Some(json!("letters, numbers, dot, underscore, or hyphen")),
                actual: Some(json!(name)),
                ..FailureDetail::default()
            }],
            Map::new(),
        );
    }

    let matches = scenario_doc::find_scenarios_by_name(&config.scenarios_dir, name);
    if !matches.is_empty() {
        return failure_result(
            command,
            "unable to record scenario",
            vec![FailureDetail {
                kind: "scenario_name_ambiguous".to_string(),
                command: Some(command.to_string()),
                path: Some(config.scenarios_dir.to_string_lossy().into_owned()),
                message: Some(format!("scenario '{name}' already exists")),
                expected: Some(json!("unique scenario name under qa/scenarios")),
                actual: Some(json!(
                    matches
                        .into_iter()
                        .map(|path| path.to_string_lossy().into_owned())
                        .collect::<Vec<_>>()
                )),
                ..FailureDetail::default()
            }],
            Map::new(),
        );
    }

    let output_path = resolve_output_path(config, name, out);
    if output_path.exists() {
        return failure_result(
            command,
            "unable to record scenario",
            vec![FailureDetail {
                kind: "scenario_output_exists".to_string(),
                command: Some(command.to_string()),
                path: Some(output_path.to_string_lossy().into_owned()),
                message: Some("scenario record output already exists".to_string()),
                expected: Some(json!("missing output path or explicit overwrite flow")),
                actual: Some(json!("existing_file")),
                ..FailureDetail::default()
            }],
            Map::new(),
        );
    }

    let started_at = timestamp_now();
    let artifacts = match artifacts::prepare_record_artifacts(
        &config.artifacts_dir,
        name,
        &config.project_root,
        scene,
        &output_path,
        command_line,
        &viewport,
        options.headless,
        None,
        Some(&started_at),
    ) {
        Ok(artifacts) => artifacts,
        Err(error) => {
            return failure_result(
                command,
                "unable to record scenario",
                vec![FailureDetail {
                    kind: "artifact_write_failed".to_string(),
                    command: Some(command.to_string()),
                    path: Some(config.artifacts_dir.to_string_lossy().into_owned()),
                    message: Some(error.to_string()),
                    expected: Some(json!(
                        "artifact metadata written under the artifact directory"
                    )),
                    actual: Some(json!("OSError")),
                    ..FailureDetail::default()
                }],
                Map::new(),
            );
        }
    };

    let session_token = session_token();
    let launch_headless = options.headless
        || config.launch_mode_headless_override().unwrap_or(false)
        || options.duration_frames.is_some()
        || options.input_script.is_some()
        || display_unavailable();
    // The recorder keeps the pre-audio-axis behavior: real audio when windowed
    // (a human watching the capture), null audio when headless (`--headless`
    // already forces the Dummy driver). `audio = !headless` reproduces the old
    // argv exactly.
    let launch_audio = !launch_headless;
    let session_record = SessionRecord {
        process_identity: None,
        protocol_version: godot_qa_runtime::PROTOCOL_VERSION.to_string(),
        session_id: artifacts.run_id.clone(),
        pid: 0,
        host: "127.0.0.1".to_string(),
        port: pick_port().unwrap_or(0),
        scene: scene.to_string(),
        viewport: viewport.clone(),
        artifact_dir: artifacts.root_dir.to_string_lossy().into_owned(),
        request_path: artifacts.request_path.to_string_lossy().into_owned(),
        token_file: godot_qa_runtime::session_token_path(config)
            .to_string_lossy()
            .into_owned(),
        started_at: started_at.clone(),
        headless: launch_headless,
        project_args: Vec::new(),
        frame_pacing: None,
        target: godot_qa_runtime::SessionTarget::Desktop,
        android: None,
    };
    if session_record.port == 0 {
        return failure_result(
            command,
            "unable to record scenario",
            vec![FailureDetail {
                kind: "artifact_write_failed".to_string(),
                command: Some(command.to_string()),
                path: Some("127.0.0.1".to_string()),
                message: Some("failed to allocate a recorder session port".to_string()),
                expected: Some(json!("available localhost TCP port")),
                actual: Some(json!("OSError")),
                ..FailureDetail::default()
            }],
            Map::new(),
        );
    }
    if let Err(error) = write_record_request(
        &artifacts.request_path,
        &session_record,
        &artifacts.to_json(),
        &output_path,
        name,
        scene,
        launch_headless,
        command_line,
        &options.joy_motion,
    ) {
        return failure_result(
            command,
            "unable to record scenario",
            vec![FailureDetail {
                kind: "artifact_write_failed".to_string(),
                command: Some(command.to_string()),
                path: Some(artifacts.request_path.to_string_lossy().into_owned()),
                message: Some(error.to_string()),
                expected: Some(json!(
                    "artifact metadata written under the artifact directory"
                )),
                actual: Some(json!("OSError")),
                ..FailureDetail::default()
            }],
            Map::new(),
        );
    }

    let mut persisted_record: Option<SessionRecord> = None;
    let result: Option<CommandResult>;
    let mut cleanup_failure: Option<FailureDetail> = None;
    let launch = godot_qa_process::launch_godot_session(
        binary,
        &config.godot_launch_prefix,
        &config.godot_windowed_launch_prefix,
        &config.project_root,
        &artifacts.request_path,
        &artifacts.logs_path,
        session_record.port,
        &session_token,
        launch_headless,
        launch_audio,
        Some(&session_record.viewport),
        None,
        &config.godot_args,
        &config.godot_project_args,
        &[],
        None,
    );
    match launch {
        Ok(launch) => {
            let persisted = SessionRecord {
                process_identity: godot_qa_process::process_identity(launch.pid),
                pid: launch.pid,
                ..session_record.clone()
            };
            persisted_record = Some(persisted.clone());

            let startup =
                godot_qa_runtime::probe_live_session_startup(config, &persisted, &session_token)
                    .and_then(|payload| {
                        godot_qa_runtime::validate_response(command, &payload)
                            .map_err(|error| error.failure)?;
                        Ok(payload)
                    });
            if let Err(failure) = startup {
                result = Some(failure_result(
                    command,
                    "unable to record scenario",
                    vec![failure],
                    Map::new(),
                ));
            } else if let Err(failure) =
                godot_qa_runtime::persist_session(config, &persisted, &session_token)
            {
                result = Some(failure_result(
                    command,
                    "unable to record scenario",
                    vec![failure],
                    Map::new(),
                ));
            } else {
                result = Some(execute_record_flow(
                    config,
                    &persisted,
                    name,
                    scene,
                    &output_path,
                    &artifacts,
                    &session_token,
                    command,
                    &options,
                    input_steps.as_deref(),
                ));
            }
        }
        Err(failure) => {
            result = Some(failure_result(
                command,
                "unable to record scenario",
                vec![failure],
                Map::new(),
            ));
        }
    }

    if let Some(record) = &persisted_record {
        if record.pid > 0
            && !godot_qa_process::wait_for_godot_process_exit(
                record.pid,
                PROCESS_STOP_WAIT_TIMEOUT_SECONDS,
            )
        {
            cleanup_failure = godot_qa_process::stop_godot_process_with_identity(
                record.pid,
                PROCESS_STOP_WAIT_TIMEOUT_SECONDS,
                record.process_identity,
            )
            .err();
        }
    }
    godot_qa_runtime::clear_session_files(config);

    let mut result = result.unwrap_or_else(|| {
        failure_result(
            command,
            "unable to record scenario",
            vec![FailureDetail {
                kind: "record_failed".to_string(),
                command: Some(command.to_string()),
                message: Some("scenario record did not produce a result".to_string()),
                ..FailureDetail::default()
            }],
            Map::new(),
        )
    });
    if let Some(cleanup_failure) = cleanup_failure {
        if result.ok {
            return failure_result(
                command,
                "unable to record scenario",
                vec![cleanup_failure],
                Map::new(),
            );
        }
        result.failures.push(cleanup_failure);
        result.ok = false;
        result.exit_code = 1;
    }
    result
}

fn execute_record_flow(
    config: &ProjectConfig,
    record: &SessionRecord,
    name: &str,
    scene: &str,
    output_path: &Path,
    artifacts: &artifacts::RecordArtifacts,
    session_token: &str,
    command: &str,
    options: &RecordOptions<'_>,
    input_steps: Option<&[InputScriptStep]>,
) -> CommandResult {
    let initial = match send_validated_request(
        config,
        record,
        godot_qa_runtime::SNAPSHOT_CAPTURE,
        Map::from_iter([("name".to_string(), json!("initial"))]),
        session_token,
        command,
    ) {
        Ok(data) => data,
        Err(failure) => {
            return failure_result(
                command,
                "unable to record scenario",
                vec![failure],
                Map::new(),
            );
        }
    };
    let _ = initial;

    if let Err(failure) =
        run_scripted_recording(config, record, session_token, command, options, input_steps)
    {
        return failure_result(
            command,
            "unable to record scenario",
            vec![failure],
            Map::new(),
        );
    }

    if let Err(failure) = send_validated_request(
        config,
        record,
        godot_qa_runtime::SNAPSHOT_CAPTURE,
        Map::from_iter([("name".to_string(), json!("final"))]),
        session_token,
        command,
    ) {
        return failure_result(
            command,
            "unable to record scenario",
            vec![failure],
            Map::new(),
        );
    }
    let export_payload = match send_validated_request(
        config,
        record,
        godot_qa_runtime::RECORD_EXPORT,
        Map::new(),
        session_token,
        command,
    ) {
        Ok(data) => data,
        Err(failure) => {
            return failure_result(
                command,
                "unable to record scenario",
                vec![failure],
                Map::new(),
            );
        }
    };

    let suggestions = suggest_recorder_assertions(&export_payload, artifacts);
    let mut candidate = build_candidate_scenario(
        name,
        scene,
        &export_payload,
        "initial",
        "final",
        &suggestions,
    );
    if let Some(review) = candidate.get_mut("review").and_then(Value::as_object_mut) {
        review.insert(
            "artifacts".to_string(),
            json!({"recording": artifacts.recording_path}),
        );
    }
    let recording_payload = recording_payload(
        name,
        scene,
        output_path,
        &artifacts.to_json(),
        &export_payload,
        "initial",
        "final",
        &suggestions,
    );

    if let Some(parent) = output_path.parent() {
        if let Err(error) = fs::create_dir_all(parent) {
            return failure_result(
                command,
                "unable to record scenario",
                vec![FailureDetail {
                    kind: "artifact_write_failed".to_string(),
                    command: Some(command.to_string()),
                    path: Some(parent.to_string_lossy().into_owned()),
                    message: Some(error.to_string()),
                    expected: Some(json!(
                        "artifact metadata written under the artifact directory"
                    )),
                    actual: Some(json!("OSError")),
                    ..FailureDetail::default()
                }],
                Map::new(),
            );
        }
    }
    for (path, payload) in [
        (
            artifacts.recording_path.as_path(),
            recording_payload.clone(),
        ),
        (artifacts.candidate_path.as_path(), candidate.clone()),
        (output_path, candidate.clone()),
    ] {
        if let Err(error) = artifacts::write_json(path, &payload) {
            return failure_result(
                command,
                "unable to record scenario",
                vec![FailureDetail {
                    kind: "artifact_write_failed".to_string(),
                    command: Some(command.to_string()),
                    path: Some(path.to_string_lossy().into_owned()),
                    message: Some(error.to_string()),
                    expected: Some(json!(
                        "artifact metadata written under the artifact directory"
                    )),
                    actual: Some(json!("OSError")),
                    ..FailureDetail::default()
                }],
                Map::new(),
            );
        }
    }

    let _ = send_validated_request(
        config,
        record,
        godot_qa_runtime::SESSION_STOP,
        Map::new(),
        session_token,
        command,
    );

    let snapshots = json!({
        "initial": artifacts.snapshots_dir.join("initial"),
        "final": artifacts.snapshots_dir.join("final"),
    });
    let input_moments = export_payload
        .get("input_moments")
        .cloned()
        .filter(Value::is_array)
        .unwrap_or_else(|| json!([]));
    let data = Map::from_iter([
        ("scenario".to_string(), json!(name)),
        ("scene".to_string(), json!(scene)),
        ("scenario_path".to_string(), json!(output_path)),
        ("artifacts".to_string(), artifacts.to_json()),
        ("snapshots".to_string(), snapshots),
        ("input_moments".to_string(), input_moments),
        ("suggested_assertions".to_string(), json!(suggestions)),
        ("review_required".to_string(), json!(true)),
    ]);
    let result = CommandResult {
        ok: true,
        command: command.to_string(),
        message: "scenario recording complete".to_string(),
        data: data.clone(),
        failures: Vec::new(),
        exit_code: 0,
    };
    let _ = artifacts::write_json(
        &artifacts.result_path,
        &json!({
            "ok": true,
            "scenario": name,
            "scene": scene,
            "scenario_path": output_path,
            "artifacts": artifacts.to_json(),
            "snapshots": {
                "initial": artifacts.snapshots_dir.join("initial"),
                "final": artifacts.snapshots_dir.join("final"),
            },
            "suggested_assertions": suggestions,
            "review_required": true,
        }),
    );
    result
}

fn send_validated_request(
    config: &ProjectConfig,
    record: &SessionRecord,
    protocol_command: &str,
    params: Map<String, Value>,
    session_token: &str,
    command: &str,
) -> Result<Value, FailureDetail> {
    let payload = godot_qa_runtime::send_live_request(
        config,
        record,
        protocol_command,
        params,
        Some(session_token),
    )?;
    godot_qa_runtime::validate_response(command, &payload)
        .map_err(|error| error.failure.clone())?;
    if payload.get("ok").and_then(Value::as_bool).unwrap_or(false) {
        return Ok(payload.get("data").cloned().unwrap_or_else(|| json!({})));
    }

    let error = payload
        .get("error")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    let data = error
        .get("data")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    Err(FailureDetail {
        kind: error
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or("runtime_error")
            .to_string(),
        command: Some(command.to_string()),
        path: data.get("path").and_then(Value::as_str).map(str::to_string),
        message: error
            .get("message")
            .and_then(Value::as_str)
            .map(str::to_string),
        expected: data.get("expected").cloned(),
        actual: data.get("actual").cloned(),
        artifacts: data.get("artifacts").and_then(Value::as_object).cloned(),
        ..FailureDetail::default()
    })
}

fn build_candidate_scenario(
    name: &str,
    scene: &str,
    export_payload: &Value,
    initial_snapshot_name: &str,
    final_snapshot_name: &str,
    suggested_assertions: &[Value],
) -> Value {
    let mut steps = vec![
        json!({"wait_frames": 5}),
        json!({"snapshot": initial_snapshot_name}),
    ];
    if let Some(events) = export_payload.get("events").and_then(Value::as_array) {
        for event in events {
            let Some(event_type) = event.get("type").and_then(Value::as_str) else {
                continue;
            };
            match event_type {
                "tap_action" => {
                    if let Some(action) = event.get("action").and_then(Value::as_str) {
                        steps.push(json!({"tap_action": action}));
                    }
                }
                "wait_frames" => {
                    if let Some(frames) = event.get("frames").and_then(Value::as_i64) {
                        steps.push(json!({"wait_frames": frames}));
                    }
                }
                "click" => {
                    if let Some(target) = event.get("target").and_then(Value::as_str) {
                        if !target.is_empty() {
                            steps.push(json!({"click": target}));
                        }
                    }
                }
                "hold_action" => {
                    if let (Some(action), Some(frames)) = (
                        event.get("action").and_then(Value::as_str),
                        event.get("frames").and_then(Value::as_i64),
                    ) {
                        steps.push(json!({"hold_action": {"action": action, "frames": frames}}));
                    }
                }
                "release_action" => {
                    if let Some(action) = event.get("action").and_then(Value::as_str) {
                        steps.push(json!({"release_action": action}));
                    }
                }
                "input_joy_button" => {
                    if let Some(step) = joy_button_step_from_event(event) {
                        steps.push(step);
                    }
                }
                "input_joy_motion" => {
                    if let Some(step) = joy_motion_step_from_event(event) {
                        steps.push(step);
                    }
                }
                "snapshot" => {
                    if let Some(snapshot_name) = event.get("name").and_then(Value::as_str) {
                        if snapshot_name != initial_snapshot_name
                            && snapshot_name != final_snapshot_name
                        {
                            steps.push(json!({"snapshot": snapshot_name}));
                        }
                    }
                }
                _ => {}
            }
        }
    }
    steps.push(json!({"snapshot": final_snapshot_name}));

    let unconverted_events = export_payload
        .get("unconverted_events")
        .cloned()
        .filter(|value| value.is_array())
        .unwrap_or_else(|| json!([]));

    json!({
        "name": name,
        "scene": scene,
        "steps": steps,
        "assert": [],
        "review": {
            "status": "needs_review",
            "generated_by": "scenario record",
            "suggested_assertions": suggested_assertions,
            "unconverted_events": unconverted_events,
        }
    })
}

// Recorded joypad events replay through the same `input_joy_button` /
// `input_joy_motion` step vocabulary the executor already validates. The
// shorthand string form is kept when only the default device 0 one-frame tap
// is needed, matching hand-written scenarios.
fn joy_button_step_from_event(event: &Value) -> Option<Value> {
    let button = event
        .get("button")
        .and_then(Value::as_str)
        .filter(|button| !button.is_empty())?;
    let device = event.get("device").and_then(Value::as_i64).unwrap_or(0);
    let frames = event.get("frames").and_then(Value::as_i64).unwrap_or(1);
    if device == 0 && frames <= 1 {
        return Some(json!({"input_joy_button": button}));
    }
    let mut params = Map::from_iter([("button".to_string(), json!(button))]);
    if device != 0 {
        params.insert("device".to_string(), json!(device));
    }
    if frames > 1 {
        params.insert("frames".to_string(), json!(frames));
    }
    Some(json!({"input_joy_button": Value::Object(params)}))
}

fn joy_motion_step_from_event(event: &Value) -> Option<Value> {
    let axis = event
        .get("axis")
        .and_then(Value::as_str)
        .filter(|axis| !axis.is_empty())?;
    let value = event.get("value").and_then(Value::as_f64)?;
    let mut params = Map::from_iter([
        ("axis".to_string(), json!(axis)),
        ("value".to_string(), json!(value)),
    ]);
    let device = event.get("device").and_then(Value::as_i64).unwrap_or(0);
    if device != 0 {
        params.insert("device".to_string(), json!(device));
    }
    Some(json!({"input_joy_motion": Value::Object(params)}))
}

fn recording_payload(
    name: &str,
    scene: &str,
    output_path: &Path,
    artifacts_json: &Value,
    export_payload: &Value,
    initial_snapshot_name: &str,
    final_snapshot_name: &str,
    suggested_assertions: &[Value],
) -> Value {
    let input_moments = export_payload
        .get("input_moments")
        .cloned()
        .filter(Value::is_array)
        .unwrap_or_else(|| json!([]));
    let snapshot_moments = input_moments
        .as_array()
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .map(|moment| {
            let mut payload = moment;
            let snapshot_base = payload
                .get("snapshot")
                .and_then(Value::as_str)
                .map(str::to_string);
            if let Some(snapshot_base) = snapshot_base {
                if let Some(obj) = payload.as_object_mut() {
                    obj.insert(
                        "snapshots".to_string(),
                        json!({
                            "pre": format!("{snapshot_base}_pre"),
                            "post": format!("{snapshot_base}_post"),
                        }),
                    );
                }
            }
            payload
        })
        .collect::<Vec<_>>();
    json!({
        "name": name,
        "scene": scene,
        "output_path": output_path,
        "artifacts": artifacts_json,
        "snapshots": {
            "initial": initial_snapshot_name,
            "final": final_snapshot_name,
        },
        "input_moments": snapshot_moments,
        "export": export_payload,
        "suggested_assertions": suggested_assertions,
    })
}

#[derive(Default)]
#[allow(dead_code)]
struct SnapshotBundle {
    name: String,
    tree: Option<Map<String, Value>>,
    ui: Option<Map<String, Value>>,
    focus: Option<Map<String, Value>>,
    errors: Option<Map<String, Value>>,
    screenshot: Option<String>,
    controls_by_locator: BTreeMap<String, SnapshotControl>,
}

#[derive(Clone, Debug)]
struct SnapshotControl {
    visible: bool,
    text: Option<String>,
    position: Option<[f64; 2]>,
    global_position: Option<[f64; 2]>,
    global_rect: Option<[f64; 4]>,
    role: Option<String>,
    bounds: Option<Map<String, Value>>,
    playfield: Option<String>,
    occluder: Option<String>,
    proximity_target: Option<String>,
    proximity_range: Option<f64>,
}

#[derive(Default)]
struct SnapshotIndex {
    by_name: BTreeMap<String, SnapshotBundle>,
    visible_by_locator: BTreeMap<String, bool>,
    position_by_locator: BTreeMap<String, Vec<f64>>,
}

impl SnapshotIndex {
    fn from_snapshots_dir(snapshots_dir: &Path) -> Self {
        let mut index = SnapshotIndex::default();
        let Ok(entries) = fs::read_dir(snapshots_dir) else {
            return index;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            let Some(name) = path
                .file_name()
                .and_then(|v| v.to_str())
                .map(str::to_string)
            else {
                continue;
            };
            let mut bundle = SnapshotBundle {
                name: name.clone(),
                tree: read_json_object(&path.join("tree.json")),
                ui: read_json_object(&path.join("ui.json")),
                focus: read_json_object(&path.join("focus.json")),
                errors: read_json_object(&path.join("errors.json")),
                screenshot: None,
                controls_by_locator: BTreeMap::new(),
            };
            let png = path.join("screenshot.png");
            if png.exists() {
                bundle.screenshot = Some(png.to_string_lossy().into_owned());
            }
            if let Some(controls) = bundle
                .ui
                .as_ref()
                .and_then(|payload| payload.get("controls"))
                .and_then(Value::as_array)
            {
                for control in controls {
                    let Some(qa_id) = control.get("qa_id").and_then(Value::as_str) else {
                        continue;
                    };
                    if qa_id.is_empty() {
                        continue;
                    }
                    let locator = format!("qa:{qa_id}");
                    let visible = control
                        .get("visible")
                        .and_then(Value::as_bool)
                        .unwrap_or(false);
                    index.visible_by_locator.insert(locator.clone(), visible);
                    if let Some(pos) = control.get("position").and_then(Value::as_array) {
                        let vals = pos.iter().filter_map(Value::as_f64).collect::<Vec<_>>();
                        if vals.len() == 2 {
                            index.position_by_locator.insert(locator.clone(), vals);
                        }
                    }
                    bundle.controls_by_locator.insert(
                        locator.clone(),
                        SnapshotControl {
                            visible,
                            text: control
                                .get("text")
                                .and_then(Value::as_str)
                                .map(str::to_string),
                            position: value_array2(control.get("position")),
                            global_position: value_array2(control.get("global_position")),
                            global_rect: value_array4(control.get("global_rect")),
                            role: control
                                .get("qa_recorder_role")
                                .and_then(Value::as_str)
                                .map(str::to_string),
                            bounds: control
                                .get("qa_recorder_bounds")
                                .and_then(Value::as_object)
                                .cloned(),
                            playfield: control
                                .get("qa_recorder_playfield")
                                .and_then(Value::as_str)
                                .map(str::to_string),
                            occluder: control
                                .get("qa_recorder_occluder")
                                .and_then(Value::as_str)
                                .map(str::to_string),
                            proximity_target: control
                                .get("qa_recorder_proximity_target")
                                .and_then(Value::as_str)
                                .map(str::to_string),
                            proximity_range: control
                                .get("qa_recorder_proximity_range")
                                .and_then(Value::as_f64),
                        },
                    );
                }
            }
            index.by_name.insert(name, bundle);
        }
        index
    }
}

fn suggest_recorder_assertions(
    export_payload: &Value,
    artifacts: &artifacts::RecordArtifacts,
) -> Vec<Value> {
    let snapshots_dir = &artifacts.snapshots_dir;
    let initial_snapshot_dir = snapshots_dir.join("initial");
    let final_snapshot_dir = snapshots_dir.join("final");
    let initial_visible = qa_visibility_lookup(&initial_snapshot_dir);
    let final_visible = qa_visibility_lookup(&final_snapshot_dir);
    let mut suggestions = Vec::new();

    let mut locators: Vec<String> = final_visible.keys().cloned().collect();
    locators.sort();
    for locator in locators {
        if final_visible.get(&locator) == Some(&true)
            && initial_visible.get(&locator) == Some(&false)
        {
            suggestions.push(json!({
                "id": format!("visible-{locator}"),
                "kind": "positive_visibility",
                "assertion": {"visible": locator},
                "reason": "target became visible between initial and final snapshots",
                "target": {"locator": locator},
            }));
        }
    }

    let focus_path = final_snapshot_dir.join("focus.json");
    if let Some(focus_owner) = read_json_object(&focus_path)
        .and_then(|payload| payload.get("focus_owner").cloned())
        .and_then(|value| value.as_str().map(str::to_string))
        .filter(|value| value.starts_with("qa:"))
    {
        suggestions.push(json!({
            "id": "focused-final-owner",
            "kind": "positive_focus",
            "assertion": {"focused": focus_owner},
            "reason": "final focus owner at recording end",
            "target": {"locator": focus_owner},
        }));
    }

    let errors_path = final_snapshot_dir.join("errors.json");
    if read_json_object(&errors_path)
        .and_then(|payload| payload.get("count").cloned())
        .and_then(|value| value.as_i64())
        == Some(0)
    {
        suggestions.push(json!({
            "id": "no-errors-final",
            "kind": "positive_no_errors",
            "assertion": {"no_errors": true},
            "reason": "final errors snapshot contained no runtime errors",
        }));
    }
    let snapshot_index = SnapshotIndex::from_snapshots_dir(snapshots_dir);
    suggestions.extend(suggest_negative_assertions(export_payload, &snapshot_index));
    suggestions
}

fn suggest_negative_assertions(export_payload: &Value, snapshots: &SnapshotIndex) -> Vec<Value> {
    let Some(moments) = export_payload
        .get("input_moments")
        .and_then(Value::as_array)
    else {
        return Vec::new();
    };
    let mut out = Vec::new();
    let mut bounded_targets = BTreeSet::new();
    let mut overlay_pairs = BTreeSet::new();
    let mut proximity_pairs = BTreeSet::new();
    for (idx, moment) in moments.iter().enumerate() {
        let Some(locator) = moment.get("target").and_then(Value::as_str) else {
            continue;
        };
        let Some(snapshot_base) = moment.get("snapshot").and_then(Value::as_str) else {
            continue;
        };
        let pre_name = format!("{snapshot_base}_pre");
        let post_name = format!("{snapshot_base}_post");
        if !snapshots.by_name.contains_key(&pre_name) || !snapshots.by_name.contains_key(&post_name)
        {
            continue;
        }
        if moment.get("ok").and_then(Value::as_bool) == Some(false) {
            let reason = moment
                .get("error_type")
                .or_else(|| moment.get("error"))
                .and_then(Value::as_str)
                .unwrap_or("invalid_input");
            out.push(json!({
                "id": format!("negative-never-{idx}"),
                "kind": "never",
                "assertion": {
                    "never": {
                        "target": locator,
                        "window_frames": 1,
                        "property": "visible",
                        "forbidden": "true",
                        "sample_every_frames": 1
                    }
                },
                "reason": "input failed with explicit runtime error evidence",
                "target": {"locator": locator},
                "input_moment": input_moment_summary(idx, moment, snapshot_base),
                "snapshots": {"pre": pre_name, "post": post_name},
                "evidence": {
                    "error": reason,
                    "observation": "target must never become visible during the bounded input window"
                },
            }));
            continue;
        }

        if let Some(suggestion) = suggest_noop_text_assertion(
            snapshots,
            idx,
            moment,
            locator,
            snapshot_base,
            &pre_name,
            &post_name,
        ) {
            out.push(suggestion);
        }
        if let Some(suggestion) = suggest_cancellation_assertion(
            snapshots,
            idx,
            moment,
            locator,
            snapshot_base,
            &pre_name,
            &post_name,
        ) {
            out.push(suggestion);
        }
        if bounded_targets.insert(locator.to_string()) {
            if let Some(suggestion) = suggest_bounded_movement_assertion(
                snapshots,
                idx,
                moment,
                locator,
                snapshot_base,
                &pre_name,
                &post_name,
            ) {
                out.push(suggestion);
            }
        }
        if let Some(post_control) = snapshot_control(snapshots, &post_name, locator) {
            if let Some(suggestion) = suggest_visible_pixels_assertion(
                snapshots,
                idx,
                moment,
                locator,
                snapshot_base,
                &pre_name,
                &post_name,
                post_control,
            ) {
                out.push(suggestion);
            }
            if let Some(suggestion) = suggest_not_occluded_assertion(
                snapshots,
                idx,
                moment,
                locator,
                snapshot_base,
                &pre_name,
                &post_name,
                post_control,
            ) {
                out.push(suggestion);
            }
            if let Some(playfield) = post_control.playfield.as_deref() {
                if overlay_pairs.insert((locator.to_string(), playfield.to_string())) {
                    if let Some(suggestion) = suggest_overlay_separation_assertion(
                        snapshots,
                        idx,
                        moment,
                        locator,
                        playfield,
                        snapshot_base,
                        &pre_name,
                        &post_name,
                    ) {
                        out.push(suggestion);
                    }
                }
            }
            if post_control.proximity_target.is_some() && post_control.proximity_range.is_some() {
                let proximity_target = post_control.proximity_target.as_deref().unwrap_or_default();
                if proximity_pairs.insert((locator.to_string(), proximity_target.to_string())) {
                    if let Some(suggestion) = suggest_proximity_assertion(
                        snapshots,
                        idx,
                        moment,
                        locator,
                        snapshot_base,
                        &pre_name,
                        &post_name,
                    ) {
                        out.push(suggestion);
                    }
                }
            }
        }
    }
    out
}

#[allow(clippy::too_many_arguments)]
fn suggest_visible_pixels_assertion(
    snapshots: &SnapshotIndex,
    idx: usize,
    moment: &Value,
    locator: &str,
    snapshot_base: &str,
    pre_name: &str,
    post_name: &str,
    post: &SnapshotControl,
) -> Option<Value> {
    if !post.visible {
        return None;
    }
    let post_bundle = snapshots.by_name.get(post_name)?;
    let screenshot = post_bundle.screenshot.as_ref()?;
    let rect = post.global_rect?;
    if rect_area(rect) <= 0.0 {
        return None;
    }
    Some(json!({
        "id": format!("negative-visible-pixels-{idx}"),
        "kind": "negative_visibility_guard",
        "assertion": {
            "visible_pixels": {
                "target": locator,
                "snapshot": post_name,
                "min_ratio": 0.25
            }
        },
        "reason": "input left the explicit target visible in a screenshot-backed snapshot",
        "target": {"locator": locator, "role": post.role},
        "input_moment": input_moment_summary(idx, moment, snapshot_base),
        "snapshots": {"pre": pre_name, "post": post_name},
        "evidence": {
            "visible": post.visible,
            "global_rect": rect,
            "screenshot": screenshot,
        },
    }))
}

#[allow(clippy::too_many_arguments)]
fn suggest_not_occluded_assertion(
    snapshots: &SnapshotIndex,
    idx: usize,
    moment: &Value,
    locator: &str,
    snapshot_base: &str,
    pre_name: &str,
    post_name: &str,
    post: &SnapshotControl,
) -> Option<Value> {
    if !post.visible {
        return None;
    }
    let post_bundle = snapshots.by_name.get(post_name)?;
    let screenshot = post_bundle.screenshot.as_ref()?;
    let rect = post.global_rect?;
    if rect_area(rect) <= 0.0 {
        return None;
    }
    let mut assertion = Map::from_iter([
        ("target".to_string(), json!(locator)),
        ("snapshot".to_string(), json!(post_name)),
        ("min_ratio".to_string(), json!(0.0001)),
    ]);
    let mut evidence = Map::from_iter([
        ("visible".to_string(), json!(post.visible)),
        ("global_rect".to_string(), json!(rect)),
        ("screenshot".to_string(), json!(screenshot)),
    ]);
    if let Some(occluder) = post.occluder.as_deref() {
        let occluder_control = snapshot_control(snapshots, post_name, occluder)?;
        let occluder_rect = occluder_control.global_rect?;
        if rects_overlap(rect, occluder_rect) {
            return None;
        }
        assertion.insert("occluder".to_string(), json!(occluder));
        evidence.insert("occluder".to_string(), json!(occluder));
        evidence.insert("occluder_rect".to_string(), json!(occluder_rect));
    }
    Some(json!({
        "id": format!("negative-not-occluded-{idx}"),
        "kind": "negative_not_occluded",
        "assertion": {"not_occluded": assertion},
        "reason": "input left the explicit target visible and eligible for screenshot-backed occlusion review",
        "target": {"locator": locator, "role": post.role},
        "input_moment": input_moment_summary(idx, moment, snapshot_base),
        "snapshots": {"pre": pre_name, "post": post_name},
        "evidence": evidence,
    }))
}

#[allow(clippy::too_many_arguments)]
fn suggest_noop_text_assertion(
    snapshots: &SnapshotIndex,
    idx: usize,
    moment: &Value,
    locator: &str,
    snapshot_base: &str,
    pre_name: &str,
    post_name: &str,
) -> Option<Value> {
    let pre = snapshot_control(snapshots, pre_name, locator)?;
    let post = snapshot_control(snapshots, post_name, locator)?;
    if pre.text.is_none() || pre.text != post.text || !same_position(pre, post) {
        return None;
    }
    let expected = post.text.clone()?;
    let action = moment_action(moment);
    Some(json!({
        "id": format!("negative-noop-{idx}"),
        "kind": "negative_noop",
        "assertion": {"text_equals": {"target": locator, "expected": expected, "snapshot": post_name}},
        "reason": "input left the explicit text-bearing target unchanged",
        "target": {"locator": locator},
        "input_moment": input_moment_summary(idx, moment, snapshot_base),
        "snapshots": {"pre": pre_name, "post": post_name},
        "evidence": {
            "action": action,
            "pre_text": expected,
            "post_text": expected,
            "pre_position": pre.position,
            "post_position": post.position,
        },
    }))
}

#[allow(clippy::too_many_arguments)]
fn suggest_cancellation_assertion(
    snapshots: &SnapshotIndex,
    idx: usize,
    moment: &Value,
    locator: &str,
    snapshot_base: &str,
    pre_name: &str,
    post_name: &str,
) -> Option<Value> {
    let action = moment_action(moment).to_ascii_lowercase();
    if !action.contains("cancel") {
        return None;
    }
    let pre = snapshot_control(snapshots, pre_name, locator)?;
    let post = snapshot_control(snapshots, post_name, locator)?;
    let role = post.role.as_deref().unwrap_or("");
    if role.is_empty() || pre.text.is_none() || pre.text == post.text {
        return None;
    }
    let expected = post.text.clone()?;
    Some(json!({
        "id": format!("negative-cancel-{idx}"),
        "kind": "negative_cancellation_path",
        "assertion": {"text_equals": {"target": locator, "expected": expected, "snapshot": post_name}},
        "reason": "cancel-like input changed an explicit recorder target into a stable cancellation state",
        "target": {"locator": locator, "role": role},
        "input_moment": input_moment_summary(idx, moment, snapshot_base),
        "snapshots": {"pre": pre_name, "post": post_name},
        "evidence": {
            "action": action,
            "pre_text": pre.text,
            "post_text": expected,
        },
    }))
}

#[allow(clippy::too_many_arguments)]
fn suggest_bounded_movement_assertion(
    snapshots: &SnapshotIndex,
    idx: usize,
    moment: &Value,
    locator: &str,
    snapshot_base: &str,
    pre_name: &str,
    post_name: &str,
) -> Option<Value> {
    let pre = snapshot_control(snapshots, pre_name, locator)?;
    let post = snapshot_control(snapshots, post_name, locator)?;
    let bounds = post.bounds.as_ref().or(pre.bounds.as_ref())?;
    let max = max_bound_from_metadata(bounds)?;
    let pre_position = pre.position.or(pre.global_position)?;
    let post_position = post.position.or(post.global_position)?;
    let delta = distance2(pre_position, post_position);
    if delta > max {
        return None;
    }
    Some(json!({
        "id": format!("negative-bounded-movement-{idx}"),
        "kind": "negative_bounded_movement",
        "assertion": {
            "within_limits": {
                "target": locator,
                "window_frames": 1,
                "property": "position_delta",
                "min": 0,
                "max": max,
                "sample_every_frames": 1
            }
        },
        "reason": "target movement stayed within explicit recorder bounds metadata",
        "target": {"locator": locator, "role": post.role},
        "input_moment": input_moment_summary(idx, moment, snapshot_base),
        "snapshots": {"pre": pre_name, "post": post_name},
        "evidence": {
            "pre_position": pre_position,
            "post_position": post_position,
            "observed_delta": delta,
            "bounds": bounds,
        },
    }))
}

#[allow(clippy::too_many_arguments)]
fn suggest_overlay_separation_assertion(
    snapshots: &SnapshotIndex,
    idx: usize,
    moment: &Value,
    locator: &str,
    playfield: &str,
    snapshot_base: &str,
    pre_name: &str,
    post_name: &str,
) -> Option<Value> {
    let overlay = snapshot_control(snapshots, post_name, locator)?;
    let playfield_control = snapshot_control(snapshots, post_name, playfield)?;
    if overlay.role.as_deref() != Some("debug_overlay") || !overlay.visible {
        return None;
    }
    let overlay_rect = overlay.global_rect?;
    let playfield_rect = playfield_control.global_rect?;
    if rects_overlap(overlay_rect, playfield_rect) {
        return None;
    }
    Some(json!({
        "id": format!("negative-overlay-separation-{idx}"),
        "kind": "negative_overlay_separation",
        "assertion": {"no_overlap": {"a": locator, "b": playfield}},
        "reason": "debug overlay became visible while remaining outside the explicit playfield target",
        "target": {"locator": locator, "role": overlay.role},
        "input_moment": input_moment_summary(idx, moment, snapshot_base),
        "snapshots": {"pre": pre_name, "post": post_name},
        "evidence": {
            "overlay_rect": overlay_rect,
            "playfield": playfield,
            "playfield_rect": playfield_rect,
            "overlay_visible": overlay.visible,
            "screenshot": snapshots.by_name.get(post_name).and_then(|bundle| bundle.screenshot.clone()),
        },
    }))
}

#[allow(clippy::too_many_arguments)]
fn suggest_proximity_assertion(
    snapshots: &SnapshotIndex,
    idx: usize,
    moment: &Value,
    locator: &str,
    snapshot_base: &str,
    pre_name: &str,
    post_name: &str,
) -> Option<Value> {
    let target = snapshot_control(snapshots, post_name, locator)?;
    let actor_locator = target.proximity_target.as_deref()?;
    let actor = snapshot_control(snapshots, post_name, actor_locator)?;
    let range = target.proximity_range?;
    let target_position = target
        .global_position
        .or_else(|| rect_center(target.global_rect?))?;
    let actor_position = actor
        .global_position
        .or_else(|| rect_center(actor.global_rect?))?;
    let observed_distance = distance2(actor_position, target_position);
    let threshold_key = if observed_distance <= range {
        "max"
    } else {
        "min"
    };
    Some(json!({
        "id": format!("negative-proximity-{idx}"),
        "kind": "negative_proximity_requirement",
        "assertion": {
            "distance": {
                "a": actor_locator,
                "b": locator,
                threshold_key: range,
            }
        },
        "reason": "interaction target declares an explicit proximity target and range",
        "target": {"locator": locator, "role": target.role},
        "input_moment": input_moment_summary(idx, moment, snapshot_base),
        "snapshots": {"pre": pre_name, "post": post_name},
        "evidence": {
            "actor": actor_locator,
            "actor_position": actor_position,
            "target_position": target_position,
            "observed_distance": observed_distance,
            "range": range,
            "threshold": threshold_key,
        },
    }))
}

fn snapshot_control<'a>(
    snapshots: &'a SnapshotIndex,
    snapshot_name: &str,
    locator: &str,
) -> Option<&'a SnapshotControl> {
    snapshots
        .by_name
        .get(snapshot_name)?
        .controls_by_locator
        .get(locator)
}

fn moment_action(moment: &Value) -> &str {
    moment
        .get("params")
        .and_then(Value::as_object)
        .and_then(|params| params.get("action"))
        .and_then(Value::as_str)
        .unwrap_or("")
}

fn input_moment_summary(idx: usize, moment: &Value, snapshot_base: &str) -> Value {
    json!({
        "index": idx,
        "command": moment.get("command").and_then(Value::as_str).unwrap_or(""),
        "action": moment_action(moment),
        "frame": moment.get("frame").and_then(Value::as_i64),
        "snapshot": snapshot_base,
    })
}

fn same_position(pre: &SnapshotControl, post: &SnapshotControl) -> bool {
    pre.position == post.position && pre.global_position == post.global_position
}

fn max_bound_from_metadata(bounds: &Map<String, Value>) -> Option<f64> {
    ["max_delta", "max_distance", "max_right", "max"]
        .into_iter()
        .filter_map(|key| bounds.get(key).and_then(Value::as_f64))
        .find(|value| value.is_finite() && *value >= 0.0)
}

fn value_array2(value: Option<&Value>) -> Option<[f64; 2]> {
    let values = value?.as_array()?;
    if values.len() != 2 {
        return None;
    }
    Some([values[0].as_f64()?, values[1].as_f64()?])
}

fn value_array4(value: Option<&Value>) -> Option<[f64; 4]> {
    let values = value?.as_array()?;
    if values.len() != 4 {
        return None;
    }
    Some([
        values[0].as_f64()?,
        values[1].as_f64()?,
        values[2].as_f64()?,
        values[3].as_f64()?,
    ])
}

fn distance2(a: [f64; 2], b: [f64; 2]) -> f64 {
    let dx = b[0] - a[0];
    let dy = b[1] - a[1];
    (dx * dx + dy * dy).sqrt()
}

fn rect_center(rect: [f64; 4]) -> Option<[f64; 2]> {
    Some([rect[0] + rect[2] / 2.0, rect[1] + rect[3] / 2.0])
}

fn rect_area(rect: [f64; 4]) -> f64 {
    rect[2].max(0.0) * rect[3].max(0.0)
}

fn rects_overlap(a: [f64; 4], b: [f64; 4]) -> bool {
    let a_right = a[0] + a[2];
    let a_bottom = a[1] + a[3];
    let b_right = b[0] + b[2];
    let b_bottom = b[1] + b[3];
    a[0] < b_right && a_right > b[0] && a[1] < b_bottom && a_bottom > b[1]
}

fn qa_visibility_lookup(snapshot_dir: &Path) -> std::collections::BTreeMap<String, bool> {
    let mut lookup = std::collections::BTreeMap::new();
    let ui_path = snapshot_dir.join("ui.json");
    let Some(controls) = read_json_object(&ui_path)
        .and_then(|payload| payload.get("controls").cloned())
        .and_then(|value| value.as_array().cloned())
    else {
        return lookup;
    };
    for entry in controls {
        let Some(qa_id) = entry.get("qa_id").and_then(Value::as_str) else {
            continue;
        };
        if qa_id.is_empty() {
            continue;
        }
        let visible = entry
            .get("visible")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        lookup.insert(format!("qa:{qa_id}"), visible);
    }
    lookup
}

fn read_json_object(path: &Path) -> Option<Map<String, Value>> {
    fs::read_to_string(path)
        .ok()
        .and_then(|contents| serde_json::from_str::<Value>(&contents).ok())
        .and_then(|value| value.as_object().cloned())
}

fn wait_for_recording_end(record: &SessionRecord) {
    loop {
        if godot_qa_process::wait_for_godot_process_exit(record.pid, 0.1) {
            break;
        }
        thread::sleep(Duration::from_millis(100));
    }
}

fn run_scripted_recording(
    config: &ProjectConfig,
    record: &SessionRecord,
    session_token: &str,
    command: &str,
    options: &RecordOptions<'_>,
    input_steps: Option<&[InputScriptStep]>,
) -> Result<(), FailureDetail> {
    let mut current_frame = 0_i64;
    if let Some(steps) = input_steps {
        for step in steps {
            if step.frame < current_frame {
                continue;
            }
            let delta = step.frame - current_frame;
            if delta > 0 {
                send_validated_request(
                    config,
                    record,
                    godot_qa_runtime::SESSION_WAIT_FRAMES,
                    Map::from_iter([("frames".to_string(), json!(delta))]),
                    session_token,
                    command,
                )?;
            }
            current_frame = step.frame;
            if let Some(snapshot) = step.snapshot.as_deref() {
                let _ = send_validated_request(
                    config,
                    record,
                    godot_qa_runtime::SNAPSHOT_CAPTURE,
                    Map::from_iter([("name".to_string(), json!(format!("{snapshot}_pre")))]),
                    session_token,
                    command,
                )?;
            }
            let mut params = step.params.clone();
            if let Some(snapshot) = step.snapshot.as_deref() {
                params.insert("snapshot".to_string(), json!(snapshot));
            }
            let _ = send_validated_request(
                config,
                record,
                &step.command,
                params,
                session_token,
                command,
            )?;
            if let Some(snapshot) = step.snapshot.as_deref() {
                let _ = send_validated_request(
                    config,
                    record,
                    godot_qa_runtime::SNAPSHOT_CAPTURE,
                    Map::from_iter([("name".to_string(), json!(format!("{snapshot}_post")))]),
                    session_token,
                    command,
                )?;
            }
        }
    }

    if let Some(duration_frames) = options.duration_frames {
        let remaining = duration_frames - current_frame;
        if remaining > 0 {
            let _ = send_validated_request(
                config,
                record,
                godot_qa_runtime::SESSION_WAIT_FRAMES,
                Map::from_iter([("frames".to_string(), json!(remaining))]),
                session_token,
                command,
            )?;
        }
    } else if options.input_script.is_some() {
        // Scripted mode without explicit duration should still be bounded.
        return Ok(());
    } else {
        wait_for_recording_end(record);
    }
    Ok(())
}

#[derive(Debug, Clone)]
struct InputScriptStep {
    frame: i64,
    command: String,
    params: Map<String, Value>,
    snapshot: Option<String>,
}

fn resolve_input_script_path(project_root: &Path, script_path: &str) -> PathBuf {
    let path = PathBuf::from(script_path);
    if path.is_absolute() {
        path
    } else {
        project_root.join(path)
    }
}

fn parse_input_script(path: &Path, command: &str) -> Result<Vec<InputScriptStep>, FailureDetail> {
    let contents = fs::read_to_string(path).map_err(|_| FailureDetail {
        kind: "input_script_missing".to_string(),
        command: Some(command.to_string()),
        path: Some(path.to_string_lossy().into_owned()),
        message: Some("input script was not found".to_string()),
        expected: Some(json!("existing input script json file")),
        actual: Some(json!("missing")),
        ..FailureDetail::default()
    })?;
    let payload: Value = serde_json::from_str(&contents).map_err(|error| FailureDetail {
        kind: "input_script_malformed".to_string(),
        command: Some(command.to_string()),
        path: Some(path.to_string_lossy().into_owned()),
        message: Some(error.to_string()),
        expected: Some(json!("valid JSON object with steps array")),
        actual: Some(json!("parse_error")),
        ..FailureDetail::default()
    })?;
    let Some(steps_payload) = payload.get("steps").and_then(Value::as_array) else {
        return Err(FailureDetail {
            kind: "input_script_malformed".to_string(),
            command: Some(command.to_string()),
            path: Some(path.to_string_lossy().into_owned()),
            message: Some("input script must include a steps array".to_string()),
            expected: Some(json!("object containing steps[]")),
            actual: Some(payload),
            ..FailureDetail::default()
        });
    };
    let mut steps = Vec::new();
    for entry in steps_payload {
        let frame = entry.get("frame").and_then(Value::as_i64).unwrap_or(0);
        let live = entry
            .get("command")
            .and_then(Value::as_str)
            .ok_or_else(|| FailureDetail {
                kind: "input_script_malformed".to_string(),
                command: Some(command.to_string()),
                path: Some(path.to_string_lossy().into_owned()),
                message: Some("script step must include command".to_string()),
                expected: Some(json!("step.command string")),
                actual: Some(entry.clone()),
                ..FailureDetail::default()
            })?;
        if !matches!(
            live,
            godot_qa_runtime::INPUT_TAP_ACTION
                | godot_qa_runtime::INPUT_HOLD_ACTION
                | godot_qa_runtime::INPUT_RELEASE_ACTION
                | godot_qa_runtime::INPUT_CLICK_TARGET
                | godot_qa_runtime::INPUT_KEY
                | godot_qa_runtime::INPUT_TEXT
                | godot_qa_runtime::INPUT_MOUSE_CLICK
                | godot_qa_runtime::INPUT_JOY_BUTTON
                | godot_qa_runtime::INPUT_JOY_MOTION
                | godot_qa_runtime::INPUT_JOY_STATE
        ) {
            return Err(FailureDetail {
                kind: "input_script_unsupported_command".to_string(),
                command: Some(command.to_string()),
                path: Some(path.to_string_lossy().into_owned()),
                message: Some(format!("unsupported input script command: {live}")),
                expected: Some(json!([
                    godot_qa_runtime::INPUT_TAP_ACTION,
                    godot_qa_runtime::INPUT_HOLD_ACTION,
                    godot_qa_runtime::INPUT_RELEASE_ACTION,
                    godot_qa_runtime::INPUT_CLICK_TARGET,
                    godot_qa_runtime::INPUT_KEY,
                    godot_qa_runtime::INPUT_TEXT,
                    godot_qa_runtime::INPUT_MOUSE_CLICK,
                    godot_qa_runtime::INPUT_JOY_BUTTON,
                    godot_qa_runtime::INPUT_JOY_MOTION,
                    godot_qa_runtime::INPUT_JOY_STATE
                ])),
                actual: Some(json!(live)),
                ..FailureDetail::default()
            });
        }
        let params = entry
            .get("params")
            .and_then(Value::as_object)
            .cloned()
            .unwrap_or_default();
        let snapshot = entry
            .get("snapshot")
            .and_then(Value::as_str)
            .map(str::to_string);
        steps.push(InputScriptStep {
            frame,
            command: live.to_string(),
            params,
            snapshot,
        });
    }
    Ok(steps)
}

fn write_record_request(
    path: &Path,
    session_record: &SessionRecord,
    artifacts: &Value,
    output_path: &Path,
    name: &str,
    scene: &str,
    headless: bool,
    command_line: &str,
    joy_motion: &JoyMotionRecordOptions,
) -> std::io::Result<()> {
    artifacts::write_json(
        path,
        &json!({
            "recording": {
                "name": name,
                "scene": scene,
                "output_path": output_path,
                "viewport": session_record.viewport,
                "headless": headless,
            },
            "session": {
                "id": session_record.session_id,
                "started_at": session_record.started_at,
                "host": session_record.host,
                "port": session_record.port,
                "scene": session_record.scene,
                "viewport": session_record.viewport,
                "recorder_enabled": true,
                "recorder": {
                    "joy_motion_step": joy_motion.step,
                    "joy_motion_deadzone": joy_motion.deadzone,
                    "joy_motion_frame_gap": joy_motion.frame_gap,
                },
            },
            "artifacts": artifacts,
            "run": {
                "id": session_record.session_id,
                "started_at": session_record.started_at,
                "command": command_line,
            }
        }),
    )
}

fn invalid_joy_motion_options_failure(
    command: &str,
    joy_motion: &JoyMotionRecordOptions,
) -> Option<FailureDetail> {
    let (message, expected, actual) = if !(joy_motion.step > 0.0 && joy_motion.step <= 1.0) {
        (
            "joy-motion-step must be greater than 0.0 and at most 1.0",
            json!("0.0 < step <= 1.0"),
            json!(joy_motion.step),
        )
    } else if !(joy_motion.deadzone >= 0.0 && joy_motion.deadzone < 0.5) {
        (
            "joy-motion-deadzone must be at least 0.0 and below 0.5",
            json!("0.0 <= deadzone < 0.5"),
            json!(joy_motion.deadzone),
        )
    } else if !(0..=240).contains(&joy_motion.frame_gap) {
        (
            "joy-motion-frame-gap must be an integer from 0 through 240",
            json!("integer frame gap in [0, 240]"),
            json!(joy_motion.frame_gap),
        )
    } else {
        return None;
    };
    Some(FailureDetail {
        kind: "invalid_joy_motion_options".to_string(),
        command: Some(command.to_string()),
        message: Some(message.to_string()),
        expected: Some(expected),
        actual: Some(actual),
        ..FailureDetail::default()
    })
}

fn resolve_output_path(config: &ProjectConfig, name: &str, out: Option<&str>) -> PathBuf {
    match out {
        Some(raw) => {
            let path = PathBuf::from(raw);
            if path.is_absolute() {
                path
            } else {
                config.project_root.join(path)
            }
        }
        None => config
            .scenarios_dir
            .join("candidates")
            .join(format!("{name}.json")),
    }
}

fn parse_viewport(value: &str) -> Option<Vec<i64>> {
    let normalized = value.to_ascii_lowercase();
    let (width, height) = normalized.split_once('x')?;
    let width = width.parse::<i64>().ok()?;
    let height = height.parse::<i64>().ok()?;
    if width <= 0 || height <= 0 {
        return None;
    }
    Some(vec![width, height])
}

fn timestamp_now() -> String {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| format!("{}Z", duration.as_secs()))
        .unwrap_or_else(|_| "0Z".to_string())
}

fn session_token() -> String {
    // Random per-session token (32 lowercase hex chars) to match the live-session
    // token and docs/security-model.md, instead of a guessable time-derived value.
    godot_qa_runtime::random::random_hex(16)
}

fn pick_port() -> std::io::Result<u16> {
    match TcpListener::bind(("127.0.0.1", 0)) {
        Ok(listener) => listener.local_addr().map(|addr| addr.port()),
        Err(error) if error.kind() == std::io::ErrorKind::PermissionDenied => {
            // Random unprivileged port instead of a collision-prone time-derived one.
            Ok(20_000 + (godot_qa_runtime::random::random_u16() % 40_000))
        }
        Err(error) => Err(error),
    }
}

fn display_unavailable() -> bool {
    cfg!(any(
        target_os = "linux",
        target_os = "freebsd",
        target_os = "openbsd",
        target_os = "netbsd",
        target_os = "dragonfly"
    )) && std::env::var_os("DISPLAY").is_none()
        && std::env::var_os("WAYLAND_DISPLAY").is_none()
}

fn failure_result(
    command: &str,
    message: &str,
    failures: Vec<FailureDetail>,
    data: Map<String, Value>,
) -> CommandResult {
    CommandResult {
        ok: false,
        command: command.to_string(),
        message: message.to_string(),
        data,
        failures,
        exit_code: 1,
    }
}

#[cfg(test)]
mod tests {
    use super::{
        JoyMotionRecordOptions, RecordOptions, SnapshotIndex, parse_input_script,
        recording_payload, run_scenario_record, suggest_negative_assertions,
        suggest_recorder_assertions,
    };
    use crate::{artifacts, scenario_doc};
    use godot_qa_core::config::ProjectConfig;
    use serde_json::{Value, json};
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn scenario_record_headless_fails_before_launch() {
        let root = temp_root("record-headless");
        let config = test_config(&root);

        let result = run_scenario_record(
            &config,
            "sample_recording",
            "res://ui/main_menu.tscn",
            "1280x720",
            None,
            RecordOptions {
                input_script: None,
                duration_frames: None,
                headless: true,
                joy_motion: JoyMotionRecordOptions::default(),
            },
            "godot-qa scenario record sample_recording --scene res://ui/main_menu.tscn --headless",
        );

        assert!(!result.ok);
        assert_eq!(result.failures[0].kind, "headless_screenshot_unsupported");
        assert_eq!(result.failures[0].actual, Some(json!({"headless": true})));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn scenario_record_rejects_negative_duration_before_launch() {
        let root = temp_root("record-duration");
        let config = test_config(&root);
        let result = run_scenario_record(
            &config,
            "sample_recording",
            "res://ui/main_menu.tscn",
            "1280x720",
            None,
            RecordOptions {
                input_script: None,
                duration_frames: Some(-1),
                headless: false,
                joy_motion: JoyMotionRecordOptions::default(),
            },
            "godot-qa scenario record sample_recording --scene res://ui/main_menu.tscn --duration-frames -1",
        );
        assert!(!result.ok);
        assert_eq!(result.failures[0].kind, "invalid_duration_frames");
    }

    #[test]
    fn scenario_record_rejects_invalid_joy_motion_options_before_launch() {
        let root = temp_root("record-joy-options");
        let config = test_config(&root);
        for (joy_motion, expected_actual) in [
            (
                JoyMotionRecordOptions {
                    step: 0.0,
                    ..JoyMotionRecordOptions::default()
                },
                json!(0.0),
            ),
            (
                JoyMotionRecordOptions {
                    deadzone: 0.5,
                    ..JoyMotionRecordOptions::default()
                },
                json!(0.5),
            ),
            (
                JoyMotionRecordOptions {
                    frame_gap: 241,
                    ..JoyMotionRecordOptions::default()
                },
                json!(241),
            ),
        ] {
            let result = run_scenario_record(
                &config,
                "sample_recording",
                "res://ui/main_menu.tscn",
                "1280x720",
                None,
                RecordOptions {
                    input_script: None,
                    duration_frames: None,
                    headless: false,
                    joy_motion,
                },
                "godot-qa scenario record sample_recording --scene res://ui/main_menu.tscn",
            );
            assert!(!result.ok);
            assert_eq!(result.failures[0].kind, "invalid_joy_motion_options");
            assert_eq!(result.failures[0].actual, Some(expected_actual));
        }
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn candidate_scenario_replays_recorded_joypad_events_on_the_recorded_timeline() {
        let export = json!({
            "events": [
                {"type": "wait_frames", "frames": 24},
                {"type": "input_joy_motion", "axis": "left_x", "device": 2, "value": 0.75},
                {"type": "wait_frames", "frames": 6},
                {"type": "input_joy_button", "button": "a", "device": 0},
                {"type": "input_joy_button", "button": "right_shoulder", "device": 1, "frames": 18},
                {"type": "input_joy_motion", "axis": "left_x", "device": 2, "value": 0.0}
            ],
            "unconverted_events": [],
            "input_moments": []
        });

        let candidate = super::build_candidate_scenario(
            "joy_demo",
            "res://main.tscn",
            &export,
            "initial",
            "final",
            &[],
        );

        assert_eq!(
            candidate.get("steps"),
            Some(&json!([
                {"wait_frames": 5},
                {"snapshot": "initial"},
                {"wait_frames": 24},
                {"input_joy_motion": {"axis": "left_x", "value": 0.75, "device": 2}},
                {"wait_frames": 6},
                {"input_joy_button": "a"},
                {"input_joy_button": {"button": "right_shoulder", "device": 1, "frames": 18}},
                {"input_joy_motion": {"axis": "left_x", "value": 0.0, "device": 2}},
                {"snapshot": "final"}
            ]))
        );
        scenario_doc::validate_scenario_document(&candidate, Path::new("joy_demo.json"))
            .expect("recorded joypad candidate must validate as a runnable scenario");
    }

    #[test]
    fn candidate_scenario_skips_malformed_joypad_events() {
        let export = json!({
            "events": [
                {"type": "input_joy_button", "button": ""},
                {"type": "input_joy_button"},
                {"type": "input_joy_motion", "axis": "left_x"},
                {"type": "input_joy_motion", "value": 0.5}
            ],
            "unconverted_events": [],
            "input_moments": []
        });
        let candidate = super::build_candidate_scenario(
            "joy_demo",
            "res://main.tscn",
            &export,
            "initial",
            "final",
            &[],
        );
        assert_eq!(
            candidate.get("steps"),
            Some(&json!([
                {"wait_frames": 5},
                {"snapshot": "initial"},
                {"snapshot": "final"}
            ]))
        );
    }

    #[test]
    fn input_script_missing_failure_shape() {
        let root = temp_root("record-script-missing");
        let missing = root.join("qa/recordings/missing.json");
        let failure = parse_input_script(&missing, "scenario record").unwrap_err();
        assert_eq!(failure.kind, "input_script_missing");
        assert_eq!(failure.path, Some(missing.to_string_lossy().into_owned()));
    }

    #[test]
    fn input_script_malformed_failure_shape() {
        let root = temp_root("record-script-malformed");
        let path = root.join("qa/recordings/bad.json");
        fs::create_dir_all(path.parent().expect("parent")).expect("mkdir");
        fs::write(&path, "{not-json").expect("write bad");
        let failure = parse_input_script(&path, "scenario record").unwrap_err();
        assert_eq!(failure.kind, "input_script_malformed");
    }

    #[test]
    fn scenario_record_validates_input_script_before_godot_and_artifacts() {
        let root = temp_root("record-script-preflight");
        let config = test_config(&root);
        let result = run_scenario_record(
            &config,
            "sample_recording",
            "res://ui/main_menu.tscn",
            "1280x720",
            None,
            RecordOptions {
                input_script: Some("qa/recordings/missing.json"),
                duration_frames: Some(5),
                headless: false,
                joy_motion: JoyMotionRecordOptions::default(),
            },
            "godot-qa scenario record sample_recording --scene res://ui/main_menu.tscn --input-script qa/recordings/missing.json --duration-frames 5",
        );

        assert!(!result.ok);
        assert_eq!(result.failures[0].kind, "input_script_missing");
        assert!(
            !config.artifacts_dir.exists(),
            "input-script preflight should fail before recorder artifact setup"
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn recording_payload_includes_input_moments_and_snapshot_names() {
        let payload = recording_payload(
            "demo",
            "res://ui/main_menu.tscn",
            Path::new("qa/scenarios/candidates/demo.json"),
            &json!({"recording":"path"}),
            &json!({
                "events": [],
                "unconverted_events": [],
                "input_moments": [{
                    "command": "input.click_target",
                    "snapshot": "open_menu",
                    "ok": true
                }]
            }),
            "initial",
            "final",
            &[],
        );
        assert_eq!(
            payload.get("snapshots"),
            Some(&json!({"initial":"initial","final":"final"}))
        );
        let moments = payload
            .get("input_moments")
            .and_then(Value::as_array)
            .expect("input moments");
        assert_eq!(moments.len(), 1);
        assert_eq!(
            moments[0].get("snapshots"),
            Some(&json!({"pre":"open_menu_pre","post":"open_menu_post"}))
        );
    }

    #[test]
    fn recorder_negative_suggestions_require_target_snapshot_and_evidence() {
        let root = temp_root("recorder-negative-suggest");
        let snapshots = root.join(".qa/artifacts/recordings/demo/run/snapshots");
        fs::create_dir_all(snapshots.join("tap_pre")).expect("pre");
        fs::create_dir_all(snapshots.join("tap_post")).expect("post");
        fs::write(snapshots.join("tap_pre/ui.json"), r#"{"controls":[]}"#).expect("ui pre");
        fs::write(snapshots.join("tap_post/ui.json"), r#"{"controls":[]}"#).expect("ui post");

        let export = json!({
            "input_moments": [
                {"snapshot":"tap","target":"qa:menu","ok":false,"error":"hidden"},
                {"snapshot":"tap","ok":false,"error":"hidden"},
                {"target":"qa:menu","ok":false,"error":"hidden"},
                {"snapshot":"tap","target":"qa:menu","ok":true}
            ]
        });
        let idx = SnapshotIndex::from_snapshots_dir(&snapshots);
        let suggestions = suggest_negative_assertions(&export, &idx);
        assert_eq!(suggestions.len(), 1);
        assert_eq!(suggestions[0].get("kind"), Some(&json!("never")));
        assert!(suggestions[0].get("evidence").is_some());
        assert_suggestion_assertions_validate(&suggestions);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn recorder_negative_suggestions_use_runtime_noop_evidence() {
        let root = temp_root("recorder-negative-noop-suggest");
        let snapshots = root.join(".qa/artifacts/recordings/demo/run/snapshots");
        fs::create_dir_all(snapshots.join("invalid_action_pre")).expect("pre");
        fs::create_dir_all(snapshots.join("invalid_action_post")).expect("post");
        let ui = r#"{"controls":[{"qa_id":"qa_invalid_action_result","text":"invalid:no-op"}]}"#;
        fs::write(snapshots.join("invalid_action_pre/ui.json"), ui).expect("ui pre");
        fs::write(snapshots.join("invalid_action_post/ui.json"), ui).expect("ui post");

        let export = json!({
            "input_moments": [{
                "snapshot": "invalid_action",
                "target": "qa:qa_invalid_action_result",
                "ok": true,
                "params": {"action": "qa_invalid_action"}
            }]
        });
        let idx = SnapshotIndex::from_snapshots_dir(&snapshots);
        let suggestions = suggest_negative_assertions(&export, &idx);
        assert_eq!(suggestions.len(), 1);
        assert_eq!(suggestions[0].get("kind"), Some(&json!("negative_noop")));
        assert_eq!(
            suggestions[0].get("assertion"),
            Some(
                &json!({"text_equals": {"target": "qa:qa_invalid_action_result", "expected": "invalid:no-op", "snapshot": "invalid_action_post"}})
            )
        );
        assert_suggestion_assertions_validate(&suggestions);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn recorder_negative_suggestions_include_visual_guards_with_screenshot_evidence() {
        let root = temp_root("recorder-negative-visual-suggest");
        let snapshots = root.join(".qa/artifacts/recordings/demo/run/snapshots");
        write_snapshot_ui(
            &snapshots,
            "overlay_pre",
            json!([
                {
                    "qa_id": "qa_actor",
                    "visible": true,
                    "global_rect": [100.0, 100.0, 48.0, 48.0],
                    "qa_recorder_occluder": "qa:qa_overlay"
                },
                {
                    "qa_id": "qa_overlay",
                    "visible": true,
                    "global_rect": [300.0, 100.0, 80.0, 80.0]
                }
            ]),
        );
        write_snapshot_ui(
            &snapshots,
            "overlay_post",
            json!([
                {
                    "qa_id": "qa_actor",
                    "visible": true,
                    "global_rect": [100.0, 100.0, 48.0, 48.0],
                    "qa_recorder_occluder": "qa:qa_overlay"
                },
                {
                    "qa_id": "qa_overlay",
                    "visible": true,
                    "global_rect": [300.0, 100.0, 80.0, 80.0]
                }
            ]),
        );
        fs::write(snapshots.join("overlay_post/screenshot.png"), []).expect("screenshot");

        let export = json!({
            "input_moments": [{
                "snapshot": "overlay",
                "target": "qa:qa_actor",
                "ok": true,
                "command": "input.tap_action",
                "params": {"action": "qa_toggle_debug"}
            }]
        });
        let idx = SnapshotIndex::from_snapshots_dir(&snapshots);
        let suggestions = suggest_negative_assertions(&export, &idx);
        let kinds = suggestions
            .iter()
            .filter_map(|suggestion| suggestion.get("kind").and_then(Value::as_str))
            .collect::<Vec<_>>();
        assert!(kinds.contains(&"negative_visibility_guard"), "{kinds:?}");
        assert!(kinds.contains(&"negative_not_occluded"), "{kinds:?}");
        assert!(
            suggestions
                .iter()
                .any(|suggestion| suggestion.get("assertion")
                    == Some(
                        &json!({"visible_pixels": {"target": "qa:qa_actor", "snapshot": "overlay_post", "min_ratio": 0.25}})
                    ))
        );
        assert!(
            suggestions
                .iter()
                .any(|suggestion| suggestion.get("assertion")
                    == Some(
                        &json!({"not_occluded": {"target": "qa:qa_actor", "snapshot": "overlay_post", "min_ratio": 0.0001, "occluder": "qa:qa_overlay"}})
                    ))
        );
        assert_suggestion_assertions_validate(&suggestions);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn recorder_negative_suggestions_cover_fixture_evidence_beyond_noop() {
        let root = temp_root("recorder-negative-fixture-shaped");
        let snapshots = root.join(".qa/artifacts/recordings/demo/run/snapshots");
        write_snapshot_ui(
            &snapshots,
            "invalid_action_pre",
            json!([
                {
                    "qa_id": "qa_invalid_action_result",
                    "text": "invalid:no-op",
                    "position": [0.0, 0.0],
                    "global_position": [0.0, 0.0]
                }
            ]),
        );
        write_snapshot_ui(
            &snapshots,
            "invalid_action_post",
            json!([
                {
                    "qa_id": "qa_invalid_action_result",
                    "text": "invalid:no-op",
                    "position": [0.0, 0.0],
                    "global_position": [0.0, 0.0]
                }
            ]),
        );
        write_snapshot_ui(
            &snapshots,
            "cancel_pre",
            json!([
                {
                    "qa_id": "qa_interaction_result",
                    "qa_recorder_role": "interaction_result",
                    "text": "interaction:idle"
                }
            ]),
        );
        write_snapshot_ui(
            &snapshots,
            "cancel_post",
            json!([
                {
                    "qa_id": "qa_interaction_result",
                    "qa_recorder_role": "interaction_result",
                    "text": "interaction:cancelled"
                }
            ]),
        );
        write_snapshot_ui(
            &snapshots,
            "move_pre",
            json!([
                {
                    "qa_id": "qa_actor",
                    "qa_recorder_role": "actor",
                    "qa_recorder_bounds": {"max_right": 144.0},
                    "position": [90.0, 300.0],
                    "global_position": [210.0, 460.0]
                }
            ]),
        );
        write_snapshot_ui(
            &snapshots,
            "move_post",
            json!([
                {
                    "qa_id": "qa_actor",
                    "qa_recorder_role": "actor",
                    "qa_recorder_bounds": {"max_right": 144.0},
                    "position": [114.0, 300.0],
                    "global_position": [234.0, 460.0]
                }
            ]),
        );
        write_snapshot_ui(
            &snapshots,
            "interact_pre",
            json!([
                {
                    "qa_id": "qa_actor",
                    "global_position": [210.0, 460.0]
                },
                {
                    "qa_id": "qa_interactable",
                    "qa_recorder_role": "interactable",
                    "qa_recorder_proximity_target": "qa:qa_actor",
                    "qa_recorder_proximity_range": 240.0,
                    "global_position": [580.0, 460.0]
                }
            ]),
        );
        write_snapshot_ui(
            &snapshots,
            "interact_post",
            json!([
                {
                    "qa_id": "qa_actor",
                    "global_position": [210.0, 460.0]
                },
                {
                    "qa_id": "qa_interactable",
                    "qa_recorder_role": "interactable",
                    "qa_recorder_proximity_target": "qa:qa_actor",
                    "qa_recorder_proximity_range": 240.0,
                    "global_position": [580.0, 460.0]
                }
            ]),
        );
        write_snapshot_ui(
            &snapshots,
            "debug_overlay_on_pre",
            json!([
                {
                    "qa_id": "qa_debug_overlay",
                    "qa_recorder_role": "debug_overlay",
                    "qa_recorder_playfield": "qa:qa_playfield",
                    "visible": false,
                    "global_rect": [1420.0, 160.0, 430.0, 180.0]
                },
                {
                    "qa_id": "qa_playfield",
                    "qa_recorder_role": "playfield",
                    "visible": true,
                    "global_rect": [120.0, 160.0, 1200.0, 700.0]
                }
            ]),
        );
        write_snapshot_ui(
            &snapshots,
            "debug_overlay_on_post",
            json!([
                {
                    "qa_id": "qa_debug_overlay",
                    "qa_recorder_role": "debug_overlay",
                    "qa_recorder_playfield": "qa:qa_playfield",
                    "visible": true,
                    "global_rect": [1420.0, 160.0, 430.0, 180.0]
                },
                {
                    "qa_id": "qa_playfield",
                    "qa_recorder_role": "playfield",
                    "visible": true,
                    "global_rect": [120.0, 160.0, 1200.0, 700.0]
                }
            ]),
        );

        let export = json!({
            "input_moments": [
                {
                    "snapshot": "invalid_action",
                    "target": "qa:qa_invalid_action_result",
                    "ok": true,
                    "command": "input.tap_action",
                    "params": {"action": "qa_invalid_action"}
                },
                {
                    "snapshot": "cancel",
                    "target": "qa:qa_interaction_result",
                    "ok": true,
                    "command": "input.tap_action",
                    "params": {"action": "qa_cancel"}
                },
                {
                    "snapshot": "move",
                    "target": "qa:qa_actor",
                    "ok": true,
                    "command": "input.tap_action",
                    "params": {"action": "qa_move_right"}
                },
                {
                    "snapshot": "interact",
                    "target": "qa:qa_interactable",
                    "ok": true,
                    "command": "input.tap_action",
                    "params": {"action": "qa_interact"}
                },
                {
                    "snapshot": "debug_overlay_on",
                    "target": "qa:qa_debug_overlay",
                    "ok": true,
                    "command": "input.tap_action",
                    "params": {"action": "qa_toggle_debug"}
                }
            ]
        });
        let idx = SnapshotIndex::from_snapshots_dir(&snapshots);
        let suggestions = suggest_negative_assertions(&export, &idx);
        let kinds = suggestions
            .iter()
            .filter_map(|suggestion| suggestion.get("kind").and_then(Value::as_str))
            .collect::<Vec<_>>();
        assert!(kinds.contains(&"negative_noop"), "{kinds:?}");
        assert!(kinds.contains(&"negative_cancellation_path"), "{kinds:?}");
        assert!(kinds.contains(&"negative_bounded_movement"), "{kinds:?}");
        assert!(
            kinds.contains(&"negative_proximity_requirement"),
            "{kinds:?}"
        );
        assert!(kinds.contains(&"negative_overlay_separation"), "{kinds:?}");
        assert!(suggestions.len() > 2, "{suggestions:#?}");
        assert_suggestion_assertions_validate(&suggestions);
        let proximity_assertion = suggestions
            .iter()
            .find(|suggestion| {
                suggestion.get("kind") == Some(&json!("negative_proximity_requirement"))
            })
            .and_then(|suggestion| suggestion.get("assertion"))
            .expect("proximity assertion");
        assert_eq!(
            proximity_assertion
                .get("distance")
                .and_then(|distance| distance.get("min")),
            Some(&json!(240.0))
        );
        assert!(
            suggestions
                .iter()
                .any(|suggestion| suggestion.get("assertion")
                    == Some(
                        &json!({"no_overlap": {"a": "qa:qa_debug_overlay", "b": "qa:qa_playfield"}})
                    ))
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn recorder_negative_suggestions_are_review_only_candidate_assertions_stay_empty() {
        let root = temp_root("recorder-review-only");
        let snapshots = root.join(".qa/artifacts/recordings/demo/run/snapshots");
        fs::create_dir_all(snapshots.join("initial")).expect("initial");
        fs::create_dir_all(snapshots.join("final")).expect("final");
        fs::create_dir_all(snapshots.join("tap_pre")).expect("pre");
        fs::create_dir_all(snapshots.join("tap_post")).expect("post");
        fs::write(snapshots.join("final/errors.json"), r#"{"count":0}"#).expect("errors");
        let artifacts = artifacts::RecordArtifacts {
            root_dir: root.join(".qa/artifacts/recordings/demo/run"),
            run_id: "run".to_string(),
            meta_path: root.join("m.json"),
            request_path: root.join("r.json"),
            result_path: root.join("x.json"),
            logs_path: root.join("l.log"),
            recording_path: root.join("recording.json"),
            candidate_path: root.join("candidate.json"),
            snapshots_dir: snapshots.clone(),
        };
        let export = json!({"input_moments":[{"snapshot":"tap","target":"qa:menu","ok":false,"error":"hidden"}]});
        let suggestions = suggest_recorder_assertions(&export, &artifacts);
        assert!(!suggestions.is_empty());
        assert_suggestion_assertions_validate(&suggestions);
        let candidate = super::build_candidate_scenario(
            "n",
            "res://scene.tscn",
            &export,
            "initial",
            "final",
            &suggestions,
        );
        assert_eq!(candidate.get("assert"), Some(&json!([])));
        assert!(
            candidate
                .get("review")
                .and_then(|v| v.get("suggested_assertions"))
                .is_some()
        );
        let _ = fs::remove_dir_all(root);
    }

    fn write_snapshot_ui(snapshots: &Path, name: &str, controls: Value) {
        let dir = snapshots.join(name);
        fs::create_dir_all(&dir).expect("snapshot dir");
        fs::write(
            dir.join("ui.json"),
            json!({"controls": controls}).to_string(),
        )
        .expect("snapshot ui");
    }

    fn assert_suggestion_assertions_validate(suggestions: &[Value]) {
        let assertions = suggestions
            .iter()
            .filter_map(|suggestion| suggestion.get("assertion").cloned())
            .collect::<Vec<_>>();
        let doc = json!({
            "name": "suggested_assertions_validate",
            "scene": "res://scene.tscn",
            "steps": [],
            "assert": assertions,
        });
        scenario_doc::validate_scenario_document(&doc, Path::new("suggested.json"))
            .expect("suggested assertions should validate as scenario assertions");
    }

    fn temp_root(label: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("godot-qa-record-{label}-{unique}"));
        let _ = fs::create_dir_all(&root);
        root
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
