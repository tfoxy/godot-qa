#![allow(unused_imports)]

use crate::args::*;
use crate::*;
use godot_qa_core::config::{ProjectConfig, load_project_config};
use godot_qa_core::results::{CommandResult, FailureDetail};
use godot_qa_runtime as runtime;
use godot_qa_scenario::{
    artifact_query, artifact_summary,
    audio::{AudioAnalysisOptions, analyze_wav_file_with_options},
    bundle as scenario_bundle, check as scenario_check, contact_sheet as scenario_contact_sheet,
    edit as scenario_edit, init as scenario_init, lint as scenario_lint, live_gate,
    record as scenario_record, review as scenario_review, run as scenario_run, scenario_doc,
    screenshots as scenario_screenshots, time_control, visual_audit,
};
use notify::{Config as NotifyConfig, PollWatcher, RecursiveMode, Watcher};
use serde_json::{Map, Value, json};
use std::collections::BTreeSet;
use std::fmt::Write as _;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
    mpsc,
};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

pub(crate) fn session_run_script_result(
    config: &ProjectConfig,
    args: &SessionRunScriptArgs,
) -> CommandResult {
    let command = "session run-script";
    let script_path = match external_harness_path(config, command, &args.path) {
        Ok(path) => path,
        Err(failure) => {
            return CommandResult {
                ok: false,
                command: command.to_string(),
                message: "session script failed".to_string(),
                data: serde_json::Map::new(),
                failures: vec![failure],
                exit_code: 1,
            };
        }
    };
    let script = match scenario_doc::load_scenario_document(&script_path) {
        Ok(script) => script,
        Err(error) => {
            return CommandResult {
                ok: false,
                command: command.to_string(),
                message: "session script failed".to_string(),
                data: serde_json::Map::new(),
                failures: vec![FailureDetail {
                    kind: error.failure.kind,
                    command: Some(command.to_string()),
                    path: error.failure.path,
                    message: error.failure.message,
                    expected: error.failure.expected,
                    actual: error.failure.actual,
                    ..FailureDetail::default()
                }],
                exit_code: 1,
            };
        }
    };
    let Some(steps) = script.get("steps").and_then(Value::as_array) else {
        return CommandResult {
            ok: false,
            command: command.to_string(),
            message: "session script failed".to_string(),
            data: serde_json::Map::new(),
            failures: vec![FailureDetail {
                kind: "invalid_session_script".to_string(),
                command: Some(command.to_string()),
                path: Some(script_path.to_string_lossy().into_owned()),
                message: Some("session script requires a steps array".to_string()),
                expected: Some(json!("object with steps array")),
                actual: Some(script),
                ..FailureDetail::default()
            }],
            exit_code: 1,
        };
    };

    let mut step_results = Vec::new();
    for (index, step) in steps.iter().enumerate() {
        let step_result = run_session_script_step_with_retry(config, step, index);
        let mut step_data = Map::from_iter([
            ("index".to_string(), json!(index)),
            ("ok".to_string(), json!(step_result.ok)),
            ("command".to_string(), json!(step_result.command.clone())),
            ("message".to_string(), json!(step_result.message.clone())),
            ("data".to_string(), Value::Object(step_result.data.clone())),
            ("failures".to_string(), json!(step_result.failures.clone())),
        ]);
        if let Some(op) = session_script_step_name(step) {
            step_data.insert("op".to_string(), json!(op));
        }
        step_results.push(Value::Object(step_data));
        if !step_result.ok {
            let mut data = Map::from_iter([
                ("script_path".to_string(), json!(script_path)),
                ("failed_step".to_string(), json!(index)),
                ("steps".to_string(), Value::Array(step_results)),
            ]);
            if let Some(name) = script.get("name").and_then(Value::as_str) {
                data.insert("name".to_string(), json!(name));
            }
            return CommandResult {
                ok: false,
                command: command.to_string(),
                message: "session script failed".to_string(),
                data,
                failures: step_result.failures,
                exit_code: 1,
            };
        }
    }

    let mut data = Map::from_iter([
        ("script_path".to_string(), json!(script_path)),
        ("steps".to_string(), Value::Array(step_results)),
    ]);
    if let Some(name) = script.get("name").and_then(Value::as_str) {
        data.insert("name".to_string(), json!(name));
    }
    CommandResult {
        ok: true,
        command: command.to_string(),
        message: "session script completed".to_string(),
        data,
        failures: Vec::new(),
        exit_code: 0,
    }
}

pub(crate) fn run_session_script_step_with_retry(
    config: &ProjectConfig,
    step: &Value,
    index: usize,
) -> CommandResult {
    let mut retry_failures = Vec::new();
    let mut retry_backoffs = Vec::new();
    for attempt in 1..=SESSION_SCRIPT_RETRY_ATTEMPTS {
        let mut result = run_session_script_step(config, step, index);
        if result.ok
            || !session_script_result_is_retryable(&result)
            || attempt == SESSION_SCRIPT_RETRY_ATTEMPTS
        {
            if !retry_failures.is_empty() {
                let wait_aware = retry_backoffs
                    .iter()
                    .any(|backoff| *backoff > SESSION_SCRIPT_RETRY_BACKOFF_MS);
                result.data.insert(
                    "retry".to_string(),
                    json!({
                        "attempts": attempt,
                        "max_attempts": SESSION_SCRIPT_RETRY_ATTEMPTS,
                        "backoff_ms": SESSION_SCRIPT_RETRY_BACKOFF_MS,
                        "backoff_milliseconds": retry_backoffs,
                        "wait_aware": wait_aware,
                        "retryable_failure_kinds": ["session_bridge_busy", "session_connect_failed"],
                        "observed_retryable_failures": retry_failures,
                        "final_status": if result.ok { "success" } else { "failure" },
                        "exhausted": !result.ok && attempt == SESSION_SCRIPT_RETRY_ATTEMPTS,
                    }),
                );
            }
            return result;
        }
        let backoff_ms = session_script_retry_backoff_ms(&result);
        retry_failures.push(session_script_retry_failure_summary(
            &result, attempt, backoff_ms,
        ));
        retry_backoffs.push(backoff_ms);
        thread::sleep(Duration::from_millis(backoff_ms));
    }
    unreachable!("session script retry loop always returns on final attempt")
}

pub(crate) fn session_script_result_is_retryable(result: &CommandResult) -> bool {
    result
        .failures
        .iter()
        .any(session_script_failure_is_retryable)
}

pub(crate) fn session_script_failure_is_retryable(failure: &FailureDetail) -> bool {
    match failure.kind.as_str() {
        "session_bridge_busy" | "resource_unavailable" | "resource_busy" | "bridge_busy" => true,
        "session_connect_failed" => {
            session_script_failure_text_contains(failure, "Resource temporarily unavailable")
                || session_script_failure_text_contains(failure, "os error 11")
        }
        _ => false,
    }
}

pub(crate) fn session_script_failure_text_contains(failure: &FailureDetail, needle: &str) -> bool {
    failure
        .message
        .as_deref()
        .is_some_and(|message| message.contains(needle))
        || failure
            .actual
            .as_ref()
            .and_then(Value::as_str)
            .is_some_and(|actual| actual.contains(needle))
}

pub(crate) fn session_script_retry_backoff_ms(result: &CommandResult) -> u64 {
    result
        .failures
        .iter()
        .filter_map(session_script_wait_busy_backoff_ms)
        .max()
        .unwrap_or(SESSION_SCRIPT_RETRY_BACKOFF_MS)
}

pub(crate) fn session_script_wait_busy_backoff_ms(failure: &FailureDetail) -> Option<u64> {
    match failure.kind.as_str() {
        "session_bridge_busy" | "resource_unavailable" | "resource_busy" | "bridge_busy" => {}
        _ => return None,
    }
    let in_progress = failure
        .details
        .get("in_progress")
        .and_then(Value::as_object)?;
    if in_progress.get("command").and_then(Value::as_str) != Some(runtime::SESSION_WAIT_FRAMES) {
        return None;
    }
    let Some(frames) = in_progress.get("frames").and_then(Value::as_i64) else {
        return Some(SESSION_SCRIPT_WAIT_BUSY_UNKNOWN_BACKOFF_MS);
    };
    let wait_ms = ((frames.max(0) as f64 / SESSION_SCRIPT_WAIT_BUSY_FPS).ceil() * 1000.0) as u64
        + SESSION_SCRIPT_WAIT_BUSY_GRACE_MS;
    let elapsed_ms = in_progress
        .get("elapsed_msec")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    Some(wait_ms.saturating_sub(elapsed_ms).clamp(
        SESSION_SCRIPT_RETRY_BACKOFF_MS,
        SESSION_SCRIPT_WAIT_BUSY_MAX_BACKOFF_MS,
    ))
}

pub(crate) fn session_script_retry_failure_summary(
    result: &CommandResult,
    attempt: usize,
    backoff_ms: u64,
) -> Value {
    let failures = result
        .failures
        .iter()
        .map(|failure| {
            json!({
                "type": failure.kind,
                "command": failure.command,
                "message": failure.message,
                "actual": failure.actual,
                "details": failure.details.clone(),
            })
        })
        .collect::<Vec<_>>();
    json!({
        "attempt": attempt,
        "retry_after_ms": backoff_ms,
        "command": result.command,
        "message": result.message,
        "failures": failures,
    })
}

pub(crate) fn run_session_script_step(
    config: &ProjectConfig,
    step: &Value,
    index: usize,
) -> CommandResult {
    let Some((op, value)) = session_script_step_op(step) else {
        return invalid_session_script_step(
            index,
            "each session script step must be an object with exactly one operation",
            json!("single-key object"),
            step.clone(),
        );
    };
    match op.as_str() {
        "session_start" | "start_session" => {
            let object = value.as_object();
            let scene = object
                .and_then(|v| v.get("scene"))
                .and_then(Value::as_str)
                .map(str::to_owned);
            let viewport = object
                .and_then(|v| v.get("viewport"))
                .and_then(Value::as_str)
                .unwrap_or("1920x1080");
            let headless = object
                .and_then(|v| v.get("headless"))
                .and_then(Value::as_bool)
                .unwrap_or_else(|| config.launch_mode_headless_override().unwrap_or(false));
            // Audio axis is independent of the display axis. A script step may set
            // `audio: true|false`; otherwise fall back to the project config, then to
            // the pre-audio-axis default (real audio only when windowed).
            let audio = object
                .and_then(|v| v.get("audio"))
                .and_then(Value::as_bool)
                .or_else(|| config.audio_mode_override())
                .unwrap_or(!headless);
            let project_args = match session_script_project_args(value, index) {
                Ok(project_args) => project_args,
                Err(result) => return result,
            };
            let frame_pacing_request = match session_script_frame_pacing_request(value, index) {
                Ok(request) => request,
                Err(result) => return result,
            };
            let requested_frame_pacing = match runtime::resolve_live_frame_pacing(
                config,
                frame_pacing_request,
                "session run-script",
            ) {
                Ok(frame_pacing) => frame_pacing,
                Err(failure) => {
                    return CommandResult {
                        ok: false,
                        command: "session run-script".to_string(),
                        message: "session script failed".to_string(),
                        data: Map::from_iter([("step_index".to_string(), json!(index))]),
                        failures: vec![failure],
                        exit_code: 1,
                    };
                }
            };
            let start = runtime::session_start_result(
                config,
                scene.as_deref(),
                viewport,
                headless,
                audio,
                &project_args,
                frame_pacing_request,
                false,
            );
            if start.failures.iter().any(|failure| {
                failure.kind == "session_already_running"
                    || failure
                        .message
                        .as_deref()
                        .unwrap_or_default()
                        .contains("already running")
            }) {
                let mut status = runtime::session_status_result(config);
                if status.ok {
                    let existing_headless = status
                        .data
                        .get("headless")
                        .and_then(Value::as_bool)
                        .unwrap_or(false);
                    let existing_scene = status.data.get("scene").and_then(Value::as_str);
                    let scene_mismatch = scene
                        .as_deref()
                        .is_some_and(|requested| existing_scene != Some(requested));
                    if existing_headless != headless || scene_mismatch {
                        let _ = runtime::session_stop_result(config);
                        return runtime::session_start_result(
                            config,
                            scene.as_deref(),
                            viewport,
                            headless,
                            audio,
                            &project_args,
                            frame_pacing_request,
                            false,
                        );
                    }
                    if let Some(requested) = &requested_frame_pacing
                        && !session_frame_pacing_matches(&status, requested)
                    {
                        return session_frame_pacing_mismatch_result(index, requested, &status);
                    }
                    status.command = "session start".to_string();
                    status.message = "live session reused".to_string();
                    status.data.insert("reused".to_string(), json!(true));
                    return status;
                }
                let _ = runtime::session_stop_result(config);
                return runtime::session_start_result(
                    config,
                    scene.as_deref(),
                    viewport,
                    headless,
                    audio,
                    &project_args,
                    frame_pacing_request,
                    false,
                );
            }
            start
        }
        "session_stop" | "stop_session" => runtime::session_stop_result(config),
        "session_change_scene" => {
            let scene = value
                .as_object()
                .and_then(|object| object.get("scene"))
                .and_then(Value::as_str)
                .or_else(|| value.as_str());
            let Some(scene) = scene.filter(|scene| !scene.trim().is_empty()) else {
                return invalid_session_script_step(
                    index,
                    "session_change_scene requires a non-empty scene path",
                    json!("scene path or {scene: path}"),
                    value.clone(),
                );
            };
            runtime::run_live_protocol_command(
                config,
                "session change-scene",
                runtime::SESSION_CHANGE_SCENE,
                Map::from_iter([("scene".to_string(), json!(scene))]),
            )
        }
        "session_reload_scene" => runtime::run_live_protocol_command(
            config,
            "session reload-scene",
            runtime::SESSION_RELOAD_SCENE,
            Map::new(),
        ),
        "session_pause" | "pause_session" => runtime::run_live_protocol_command(
            config,
            "session pause",
            runtime::SESSION_PAUSE,
            Map::new(),
        ),
        "session_resume" | "resume_session" => runtime::run_live_protocol_command(
            config,
            "session resume",
            runtime::SESSION_RESUME,
            Map::new(),
        ),
        "wait" | "session_wait" => {
            let frames = script_frames(value, 1);
            runtime::run_live_protocol_command(
                config,
                "session wait",
                runtime::SESSION_WAIT_FRAMES,
                Map::from_iter([("frames".to_string(), json!(frames))]),
            )
        }
        "snapshot" => {
            let mut params = value.as_object().cloned().unwrap_or_default();
            if value.is_string() {
                params.insert(
                    "name".to_string(),
                    json!(value.as_str().unwrap_or("current").to_string()),
                );
            }
            if !params.contains_key("name") {
                params.insert("name".to_string(), json!("current"));
            }
            runtime::run_live_screenshot_protocol_command(
                config,
                "snapshot",
                runtime::SNAPSHOT_CAPTURE,
                params,
            )
        }
        "screenshot" | "screenshot_capture" => {
            let out = value
                .as_object()
                .and_then(|v| v.get("out"))
                .and_then(Value::as_str)
                .or_else(|| value.as_str());
            let Some(out) = out else {
                return invalid_session_script_step(
                    index,
                    "screenshot step requires an out path",
                    json!("string or {out: string}"),
                    value.clone(),
                );
            };
            scenario_screenshots::capture_screenshot(
                config,
                out,
                scenario_screenshots::ScreenshotStateGuards::default(),
            )
        }
        "screenshot_sequence" => {
            let args = match session_script_screenshot_sequence_args(value, index) {
                Ok(args) => args,
                Err(result) => return result,
            };
            let scenario_resolution =
                match resolve_screenshot_sequence_scenario(config, args.scenario.as_deref()) {
                    Ok(resolved) => resolved,
                    Err(result) => return result,
                };
            let stop_after_state = match parse_stop_after_state(
                "screenshot sequence",
                args.stop_after_state.as_deref(),
            ) {
                Ok(value) => value,
                Err(failure) => return invalid_session_script_param_failure(index, failure),
            };
            let result = scenario_screenshots::capture_sequence(
                config,
                scenario_screenshots::ScreenshotSequenceOptions {
                    frames: args.frames,
                    every: args.every,
                    seconds: args.seconds,
                    fps: args.fps,
                    out: args.out,
                    scenario: scenario_resolution.scenario,
                    scenario_path: scenario_resolution.path,
                    scenario_scene: scenario_resolution.scene,
                    state_guards: scenario_screenshots::ScreenshotStateGuards::default(),
                    stop_after_state,
                    allow_low_information: args.allow_low_information,
                    visual_risk: args.visual_risk || args.visual_risk_strict,
                    visual_risk_strict: args.visual_risk_strict,
                    contact_sheet: args.contact_sheet,
                    contact_sheet_columns: args.contact_sheet_columns,
                    contact_sheet_thumb_width: args.contact_sheet_thumb_width,
                    contact_sheet_out: args.contact_sheet_out,
                },
            );
            if result
                .failures
                .first()
                .is_some_and(|failure| failure.kind == "screenshot_sequence_args_invalid")
            {
                return invalid_session_script_param_failure(
                    index,
                    result.failures.first().cloned().unwrap(),
                );
            }
            result
        }
        "trace_record" => {
            let args = match session_script_trace_record_args(value, index) {
                Ok(args) => args,
                Err(result) => return result,
            };
            match trace_record_params(&args) {
                Ok(params) => runtime::trace_record_result(config, params),
                Err(failure) => invalid_session_script_param_failure(index, failure),
            }
        }
        "physics_collision_report" => {
            let args = match session_script_physics_collision_report_args(value, index) {
                Ok(args) => args,
                Err(result) => return result,
            };
            match physics_collision_report_params(&args) {
                Ok(params) => physics_collision_report_result(config, &args, params),
                Err(failure) => invalid_session_script_param_failure(index, failure),
            }
        }
        "audio_capture" => {
            let args = match session_script_audio_capture_args(value, index) {
                Ok(args) => args,
                Err(result) => return result,
            };
            match audio_capture_request(config, &args) {
                Ok(request) => runtime::audio_capture_result(config, request),
                Err(failure) => invalid_session_script_param_failure(index, failure),
            }
        }
        "event_trace" => {
            let args = match session_script_event_trace_args(value, index) {
                Ok(args) => args,
                Err(result) => return result,
            };
            match event_trace_params(&args) {
                Ok(params) => runtime::event_trace_result(config, params),
                Err(failure) => invalid_session_script_param_failure(index, failure),
            }
        }
        "session_profile" => {
            let args = match session_script_profile_args(value, index) {
                Ok(args) => args,
                Err(result) => return result,
            };
            if let Err(failure) = validate_session_profile_args("session profile", &args) {
                return invalid_session_script_param_failure(index, failure);
            }
            session_profile_result(config, &args)
        }
        "tree" => runtime::run_live_protocol_command(
            config,
            "tree",
            runtime::TREE_DUMP,
            value.as_object().cloned().unwrap_or_default(),
        ),
        "ui" => runtime::run_live_protocol_command(config, "ui", runtime::UI_DUMP, Map::new()),
        "node_inspect" | "inspect" => {
            let target = value
                .as_object()
                .and_then(|v| v.get("target"))
                .and_then(Value::as_str)
                .or_else(|| value.as_str());
            let Some(target) = target else {
                return invalid_session_script_step(
                    index,
                    "node_inspect step requires a target",
                    json!("string or {target: string}"),
                    value.clone(),
                );
            };
            runtime::run_live_protocol_command(
                config,
                "node inspect",
                runtime::NODE_INSPECT,
                Map::from_iter([("target".to_string(), json!(target))]),
            )
        }
        "logs" => runtime::run_live_protocol_command(
            config,
            "logs",
            runtime::LOGS_READ,
            value.as_object().cloned().unwrap_or_default(),
        ),
        "errors" => {
            runtime::run_live_protocol_command(config, "errors", runtime::ERRORS_READ, Map::new())
        }
        "input_key" => {
            let key = value
                .as_object()
                .and_then(|v| v.get("key"))
                .and_then(Value::as_str)
                .or_else(|| value.as_str());
            let Some(key) = key else {
                return invalid_session_script_step(
                    index,
                    "input_key step requires a key",
                    json!("string or {key: string}"),
                    value.clone(),
                );
            };
            runtime::run_live_protocol_command(
                config,
                "input key",
                runtime::INPUT_KEY,
                Map::from_iter([("key".to_string(), json!(key))]),
            )
        }
        "input_text" => {
            let text = value
                .as_object()
                .and_then(|v| v.get("text"))
                .and_then(Value::as_str)
                .or_else(|| value.as_str());
            let Some(text) = text else {
                return invalid_session_script_step(
                    index,
                    "input_text step requires a text",
                    json!("string or {text: string}"),
                    value.clone(),
                );
            };
            runtime::run_live_protocol_command(
                config,
                "input type",
                runtime::INPUT_TEXT,
                Map::from_iter([("text".to_string(), json!(text))]),
            )
        }
        "input_joy_button" => {
            let button = value
                .as_object()
                .and_then(|v| v.get("button"))
                .and_then(Value::as_str)
                .or_else(|| value.as_str());
            let Some(button) = button else {
                return invalid_session_script_step(
                    index,
                    "input_joy_button step requires a button",
                    json!(
                        "string or {button: string, optional device: integer, optional frames: integer}"
                    ),
                    value.clone(),
                );
            };
            let device = match script_non_negative_int_field(
                value,
                index,
                "input_joy_button",
                "device",
                0,
            ) {
                Ok(device) => device,
                Err(result) => return result,
            };
            let frames = match script_non_negative_int_field(
                value,
                index,
                "input_joy_button",
                "frames",
                1,
            ) {
                Ok(frames) => frames,
                Err(result) => return result,
            };
            runtime::run_live_protocol_command(
                config,
                "input joy button",
                runtime::INPUT_JOY_BUTTON,
                Map::from_iter([
                    ("button".to_string(), json!(button)),
                    ("device".to_string(), json!(device)),
                    ("frames".to_string(), json!(frames)),
                ]),
            )
        }
        "input_joy_motion" => {
            let Some(object) = value.as_object() else {
                return invalid_session_script_step(
                    index,
                    "input_joy_motion step requires an object",
                    json!("{axis, value, optional device, optional frames}"),
                    value.clone(),
                );
            };
            let Some(axis) = object.get("axis").and_then(Value::as_str) else {
                return invalid_session_script_step(
                    index,
                    "input_joy_motion step requires axis",
                    json!("axis string"),
                    value.clone(),
                );
            };
            let Some(axis_value) = object.get("value").and_then(Value::as_f64) else {
                return invalid_session_script_step(
                    index,
                    "input_joy_motion step requires numeric value",
                    json!("number"),
                    value.clone(),
                );
            };
            let device = match script_non_negative_int_field(
                value,
                index,
                "input_joy_motion",
                "device",
                0,
            ) {
                Ok(device) => device,
                Err(result) => return result,
            };
            let frames = match script_non_negative_int_field(
                value,
                index,
                "input_joy_motion",
                "frames",
                1,
            ) {
                Ok(frames) => frames,
                Err(result) => return result,
            };
            runtime::run_live_protocol_command(
                config,
                "input joy motion",
                runtime::INPUT_JOY_MOTION,
                Map::from_iter([
                    ("axis".to_string(), json!(axis)),
                    ("value".to_string(), json!(axis_value)),
                    ("device".to_string(), json!(device)),
                    ("frames".to_string(), json!(frames)),
                ]),
            )
        }
        "input_joy_state" => {
            // Open-loop batch joypad input: forward the whole `inputs` list and
            // optional `frames` to input.joy_state, which validates each entry
            // and emits them in one frame. The CLI only guards the outer shape.
            let inputs = value
                .as_object()
                .and_then(|object| object.get("inputs"))
                .and_then(Value::as_array)
                .filter(|list| !list.is_empty());
            let Some(inputs) = inputs else {
                return invalid_session_script_step(
                    index,
                    "input_joy_state step requires a non-empty inputs list",
                    json!("{inputs: non-empty list of {device, motion|button}, optional frames}"),
                    value.clone(),
                );
            };
            let frames =
                match script_non_negative_int_field(value, index, "input_joy_state", "frames", 1) {
                    Ok(frames) => frames,
                    Err(result) => return result,
                };
            runtime::run_live_protocol_command(
                config,
                "input joy state",
                runtime::INPUT_JOY_STATE,
                Map::from_iter([
                    ("inputs".to_string(), Value::Array(inputs.clone())),
                    ("frames".to_string(), json!(frames)),
                ]),
            )
        }
        "joy_connect" | "joy_disconnect" => {
            let params = value.as_object();
            let devices = params
                .and_then(|object| object.get("devices"))
                .and_then(Value::as_array)
                .filter(|devices| !devices.is_empty());
            let Some(devices) = devices else {
                return invalid_session_script_step(
                    index,
                    "joy connection step requires a non-empty devices list",
                    json!("{devices: non-empty list of device IDs}"),
                    value.clone(),
                );
            };
            let mut payload = Map::from_iter([("devices".to_string(), json!(devices))]);
            if op == "joy_connect"
                && let Some(name) = params.and_then(|object| object.get("name"))
            {
                payload.insert("name".to_string(), name.clone());
            }
            runtime::run_live_protocol_command(
                config,
                if op == "joy_connect" {
                    "setup joy connect"
                } else {
                    "setup joy disconnect"
                },
                if op == "joy_connect" {
                    runtime::SETUP_JOY_CONNECT
                } else {
                    runtime::SETUP_JOY_DISCONNECT
                },
                payload,
            )
        }
        "input_tap" | "input_hold" | "input_release" => {
            let action = value
                .as_object()
                .and_then(|v| v.get("action"))
                .and_then(Value::as_str)
                .or_else(|| value.as_str());
            let Some(action) = action else {
                return invalid_session_script_step(
                    index,
                    "input action step requires an action",
                    json!("string or {action: string}"),
                    value.clone(),
                );
            };
            let (command_name, protocol) = match op.as_str() {
                "input_hold" => ("input hold", runtime::INPUT_HOLD_ACTION),
                "input_release" => ("input release", runtime::INPUT_RELEASE_ACTION),
                _ => ("input tap", runtime::INPUT_TAP_ACTION),
            };
            let mut params = Map::from_iter([("action".to_string(), json!(action))]);
            if op == "input_hold" {
                params.insert("frames".to_string(), json!(script_frames(value, 1)));
            }
            runtime::run_live_protocol_command(config, command_name, protocol, params)
        }
        "input_click" => {
            let target = value
                .as_object()
                .and_then(|v| v.get("target"))
                .and_then(Value::as_str)
                .or_else(|| value.as_str());
            let Some(target) = target else {
                return invalid_session_script_step(
                    index,
                    "input_click step requires a target",
                    json!("string or {target: string}"),
                    value.clone(),
                );
            };
            runtime::run_live_protocol_command(
                config,
                "input click",
                runtime::INPUT_CLICK_TARGET,
                Map::from_iter([("target".to_string(), json!(target))]),
            )
        }
        "call_method" => {
            let Some(object) = value.as_object() else {
                return invalid_session_script_step(
                    index,
                    "call_method step requires an object",
                    json!("{target: string, method: qa_* string, optional args: array}"),
                    value.clone(),
                );
            };
            let Some(target) = object.get("target").and_then(Value::as_str) else {
                return invalid_session_script_step(
                    index,
                    "call_method step requires target",
                    json!("target locator string"),
                    value.clone(),
                );
            };
            let Some(method) = object.get("method").and_then(Value::as_str) else {
                return invalid_session_script_step(
                    index,
                    "call_method step requires method",
                    json!("qa_ method name string"),
                    value.clone(),
                );
            };
            if !method.starts_with("qa_") {
                return invalid_session_script_step(
                    index,
                    "call_method step field 'method' must name a qa_ fixture method",
                    json!("method name beginning with qa_"),
                    json!(method),
                );
            }
            if object.get("args").is_some_and(|args| !args.is_array()) {
                return invalid_session_script_step(
                    index,
                    "call_method step field 'args' must be a list when present",
                    json!("array"),
                    object.get("args").cloned().unwrap_or(Value::Null),
                );
            }
            let mut params = Map::from_iter([
                ("target".to_string(), json!(target)),
                ("method".to_string(), json!(method)),
            ]);
            if let Some(args) = object.get("args") {
                params.insert("args".to_string(), args.clone());
            }
            runtime::run_live_protocol_command(
                config,
                "session run-script call_method",
                runtime::SETUP_CALL_METHOD,
                params,
            )
        }
        "fixture_call" => {
            let Some(object) = value.as_object() else {
                return invalid_session_script_step(
                    index,
                    "fixture_call step requires an object",
                    json!("{driver: string, action: string, optional params: object}"),
                    value.clone(),
                );
            };
            let Some(driver) = object
                .get("driver")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
            else {
                return invalid_session_script_step(
                    index,
                    "fixture_call step field 'driver' must be a non-empty string",
                    json!("non-empty fixture driver id"),
                    object.get("driver").cloned().unwrap_or(Value::Null),
                );
            };
            let Some(action) = object
                .get("action")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
            else {
                return invalid_session_script_step(
                    index,
                    "fixture_call step field 'action' must be a non-empty string",
                    json!("non-empty fixture action"),
                    object.get("action").cloned().unwrap_or(Value::Null),
                );
            };
            if object
                .get("params")
                .is_some_and(|params| !params.is_object())
            {
                return invalid_session_script_step(
                    index,
                    "fixture_call step field 'params' must be an object when present",
                    json!("object"),
                    object.get("params").cloned().unwrap_or(Value::Null),
                );
            }
            let mut params = Map::from_iter([
                ("driver".to_string(), json!(driver)),
                ("action".to_string(), json!(action)),
            ]);
            if let Some(call_params) = object.get("params") {
                params.insert("params".to_string(), call_params.clone());
            }
            runtime::run_live_protocol_command(
                config,
                "session run-script fixture_call",
                runtime::FIXTURE_CALL,
                params,
            )
        }
        "assert" => run_session_script_assert(config, value, index),
        assertion_name if assertion_protocol(assertion_name).is_some() => {
            let assertion = Map::from_iter([(assertion_name.to_string(), value.clone())]);
            run_session_script_assert(config, &Value::Object(assertion), index)
        }
        _ => invalid_session_script_step(
            index,
            "unsupported session script operation",
            json!(SESSION_SCRIPT_OPS.as_slice()),
            json!(op),
        ),
    }
}

pub(crate) const SESSION_SCRIPT_OPS: [&str; 42] = [
    "session_start",
    "session_stop",
    "session_change_scene",
    "session_reload_scene",
    "session_pause",
    "pause_session",
    "session_resume",
    "resume_session",
    "wait",
    "snapshot",
    "screenshot",
    "screenshot_sequence",
    "trace_record",
    "physics_collision_report",
    "audio_capture",
    "event_trace",
    "session_profile",
    "tree",
    "ui",
    "node_inspect",
    "logs",
    "errors",
    "input_key",
    "input_text",
    "input_joy_button",
    "input_joy_motion",
    "input_joy_state",
    "joy_connect",
    "joy_disconnect",
    "input_tap",
    "input_hold",
    "input_release",
    "input_click",
    "call_method",
    "fixture_call",
    "assert",
    "state_matches",
    "exists",
    "not_exists",
    "visible",
    "hidden",
    "no_errors",
];

pub(crate) fn run_session_script_assert(
    config: &ProjectConfig,
    value: &Value,
    index: usize,
) -> CommandResult {
    let Some((name, params_value)) = session_script_step_op(value) else {
        return invalid_session_script_step(
            index,
            "assert step must contain exactly one assertion",
            json!("single-key assertion object"),
            value.clone(),
        );
    };
    let Some(protocol) = assertion_protocol(&name) else {
        return invalid_session_script_step(
            index,
            "unsupported live assertion in session script",
            json!(SESSION_SCRIPT_ASSERTIONS),
            json!(name),
        );
    };
    let params = if params_value.is_boolean() {
        Map::new()
    } else {
        params_value.as_object().cloned().unwrap_or_default()
    };
    runtime::run_live_protocol_command(config, "assert", protocol, params)
}

pub(crate) const SESSION_SCRIPT_ASSERTIONS: [&str; 10] = [
    "state_matches",
    "exists",
    "not_exists",
    "visible",
    "hidden",
    "enabled",
    "disabled",
    "focused",
    "inside_viewport",
    "no_errors",
];

pub(crate) fn assertion_protocol(name: &str) -> Option<&'static str> {
    match name {
        "state_matches" => Some(runtime::ASSERT_STATE_MATCHES),
        "exists" => Some(runtime::ASSERT_EXISTS),
        "not_exists" => Some(runtime::ASSERT_NOT_EXISTS),
        "visible" => Some(runtime::ASSERT_VISIBLE),
        "hidden" => Some(runtime::ASSERT_HIDDEN),
        "enabled" => Some(runtime::ASSERT_ENABLED),
        "disabled" => Some(runtime::ASSERT_DISABLED),
        "focused" => Some(runtime::ASSERT_FOCUSED),
        "inside_viewport" => Some(runtime::ASSERT_INSIDE_VIEWPORT),
        "no_errors" => Some(runtime::ASSERT_NO_ERRORS),
        _ => None,
    }
}

pub(crate) fn session_script_step_op(value: &Value) -> Option<(String, &Value)> {
    let object = value.as_object()?;
    if object.len() != 1 {
        return None;
    }
    object
        .iter()
        .next()
        .map(|(key, value)| (key.clone(), value))
}

pub(crate) fn session_script_step_name(value: &Value) -> Option<String> {
    session_script_step_op(value).map(|(name, _)| name)
}

pub(crate) fn script_frames(value: &Value, default: i64) -> i64 {
    value
        .as_object()
        .and_then(|v| v.get("frames"))
        .and_then(Value::as_i64)
        .or_else(|| value.as_i64())
        .unwrap_or(default)
}

pub(crate) fn script_non_negative_int_field(
    value: &Value,
    index: usize,
    op: &str,
    field: &str,
    default: i64,
) -> Result<i64, CommandResult> {
    let Some(object) = value.as_object() else {
        return Ok(default);
    };
    let Some(raw) = object.get(field) else {
        return Ok(default);
    };
    let Some(parsed) = raw.as_i64().filter(|value| *value >= 0) else {
        return Err(invalid_session_script_step(
            index,
            &format!("{op} step field '{field}' must be a non-negative integer"),
            json!("non-negative integer"),
            raw.clone(),
        ));
    };
    Ok(parsed)
}

pub(crate) fn session_script_object<'a>(
    value: &'a Value,
    index: usize,
    op: &str,
) -> Result<&'a Map<String, Value>, CommandResult> {
    value.as_object().ok_or_else(|| {
        invalid_session_script_step(
            index,
            &format!("{op} step requires an object payload"),
            json!("object"),
            value.clone(),
        )
    })
}

pub(crate) fn session_script_optional_string(
    object: &Map<String, Value>,
    index: usize,
    op: &str,
    field: &str,
) -> Result<Option<String>, CommandResult> {
    let Some(value) = object.get(field) else {
        return Ok(None);
    };
    value
        .as_str()
        .map(|value| Some(value.to_string()))
        .ok_or_else(|| {
            invalid_session_script_step(
                index,
                &format!("{op} step field '{field}' must be a string"),
                json!("string"),
                value.clone(),
            )
        })
}

pub(crate) fn session_script_required_string(
    object: &Map<String, Value>,
    index: usize,
    op: &str,
    field: &str,
) -> Result<String, CommandResult> {
    session_script_optional_string(object, index, op, field)?.ok_or_else(|| {
        invalid_session_script_step(
            index,
            &format!("{op} step requires field '{field}'"),
            json!("string"),
            Value::Null,
        )
    })
}

pub(crate) fn session_script_optional_i64(
    object: &Map<String, Value>,
    index: usize,
    op: &str,
    field: &str,
) -> Result<Option<i64>, CommandResult> {
    let Some(value) = object.get(field) else {
        return Ok(None);
    };
    value.as_i64().map(Some).ok_or_else(|| {
        invalid_session_script_step(
            index,
            &format!("{op} step field '{field}' must be an integer"),
            json!("integer"),
            value.clone(),
        )
    })
}

pub(crate) fn session_script_optional_f64(
    object: &Map<String, Value>,
    index: usize,
    op: &str,
    field: &str,
) -> Result<Option<f64>, CommandResult> {
    let Some(value) = object.get(field) else {
        return Ok(None);
    };
    value.as_f64().map(Some).ok_or_else(|| {
        invalid_session_script_step(
            index,
            &format!("{op} step field '{field}' must be a number"),
            json!("number"),
            value.clone(),
        )
    })
}

pub(crate) fn session_script_required_f64(
    object: &Map<String, Value>,
    index: usize,
    op: &str,
    field: &str,
) -> Result<f64, CommandResult> {
    session_script_optional_f64(object, index, op, field)?.ok_or_else(|| {
        invalid_session_script_step(
            index,
            &format!("{op} step requires field '{field}'"),
            json!("number"),
            Value::Null,
        )
    })
}

pub(crate) fn session_script_bool(object: &Map<String, Value>, field: &str, default: bool) -> bool {
    object
        .get(field)
        .and_then(Value::as_bool)
        .unwrap_or(default)
}

pub(crate) fn session_script_string_list(
    object: &Map<String, Value>,
    index: usize,
    op: &str,
    field: &str,
) -> Result<Vec<String>, CommandResult> {
    let Some(value) = object.get(field) else {
        return Ok(Vec::new());
    };
    if let Some(item) = value.as_str() {
        return Ok(vec![item.to_string()]);
    }
    let Some(items) = value.as_array() else {
        return Err(invalid_session_script_step(
            index,
            &format!("{op} step field '{field}' must be a string or string list"),
            json!("string or array of strings"),
            value.clone(),
        ));
    };
    let mut parsed = Vec::with_capacity(items.len());
    for item in items {
        let Some(item) = item.as_str() else {
            return Err(invalid_session_script_step(
                index,
                &format!("{op} step field '{field}' must contain only strings"),
                json!("array of strings"),
                item.clone(),
            ));
        };
        parsed.push(item.to_string());
    }
    Ok(parsed)
}

pub(crate) fn invalid_session_script_param_failure(
    index: usize,
    failure: FailureDetail,
) -> CommandResult {
    invalid_session_script_step(
        index,
        failure
            .message
            .as_deref()
            .unwrap_or("session script operation parameters are invalid"),
        failure.expected.clone().unwrap_or(Value::Null),
        failure.actual.clone().unwrap_or(Value::Null),
    )
}

pub(crate) fn session_script_screenshot_sequence_args(
    value: &Value,
    index: usize,
) -> Result<ScreenshotSequenceArgs, CommandResult> {
    let op = "screenshot_sequence";
    let object = session_script_object(value, index, op)?;
    Ok(ScreenshotSequenceArgs {
        frames: session_script_optional_i64(object, index, op, "frames")?,
        every: session_script_optional_i64(object, index, op, "every")?,
        seconds: session_script_optional_f64(object, index, op, "seconds")?,
        fps: session_script_optional_f64(object, index, op, "fps")?,
        out: session_script_required_string(object, index, op, "out")?,
        setup_script: None,
        require_visible: Vec::new(),
        require_hidden: Vec::new(),
        require_state: None,
        stop_after_state: session_script_stop_after_state(
            object,
            index,
            op,
            "screenshot sequence",
        )?,
        scenario: session_script_optional_string(object, index, op, "scenario")?,
        allow_low_information: session_script_bool(object, "allow_low_information", false),
        visual_risk: session_script_bool(object, "visual_risk", false),
        visual_risk_strict: session_script_bool(object, "visual_risk_strict", false),
        contact_sheet: session_script_bool(object, "contact_sheet", false),
        contact_sheet_columns: session_script_optional_i64(
            object,
            index,
            op,
            "contact_sheet_columns",
        )?,
        contact_sheet_thumb_width: session_script_optional_i64(
            object,
            index,
            op,
            "contact_sheet_thumb_width",
        )?,
        contact_sheet_out: session_script_optional_string(object, index, op, "contact_sheet_out")?,
    })
}

pub(crate) fn session_script_trace_record_args(
    value: &Value,
    index: usize,
) -> Result<TraceRecordArgs, CommandResult> {
    let op = "trace_record";
    let object = session_script_object(value, index, op)?;
    let mut target = session_script_string_list(object, index, op, "target")?;
    target.extend(session_script_string_list(object, index, op, "targets")?);
    Ok(TraceRecordArgs {
        target,
        setup_script: None,
        setup_timeout_seconds: None,
        summary_json: false,
        frames: session_script_optional_i64(object, index, op, "frames")?.unwrap_or(180),
        sample_every_frames: session_script_optional_i64(object, index, op, "sample_every_frames")?
            .unwrap_or(1),
        physics: session_script_bool(object, "physics", false),
        state_provider: session_script_optional_string(object, index, op, "state_provider")?
            .unwrap_or_else(|| "scene-root".to_string()),
        state_method: session_script_string_list(object, index, op, "state_method")?,
        snapshot_start: session_script_bool(object, "snapshot_start", false),
        snapshot_end: session_script_bool(object, "snapshot_end", false),
        stop_after_state: session_script_stop_after_state(object, index, op, "trace record")?,
    })
}

pub(crate) fn session_script_physics_collision_report_args(
    value: &Value,
    index: usize,
) -> Result<PhysicsCollisionReportArgs, CommandResult> {
    let op = "physics_collision_report";
    let object = session_script_object(value, index, op)?;
    Ok(PhysicsCollisionReportArgs {
        target: session_script_optional_string(object, index, op, "target")?,
        from_trace: session_script_optional_string(object, index, op, "from_trace")?
            .map(PathBuf::from),
        setup_script: None,
        summary_json: false,
        frames: session_script_optional_i64(object, index, op, "frames")?.unwrap_or(600),
        sample_every_frames: session_script_optional_i64(object, index, op, "sample_every_frames")?
            .unwrap_or(1),
        physics: session_script_bool(object, "physics", false),
        state_provider: session_script_optional_string(object, index, op, "state_provider")?
            .unwrap_or_else(|| "scene-root".to_string()),
        state_method: session_script_string_list(object, index, op, "state_method")?,
        max_normal_retention: session_script_optional_f64(
            object,
            index,
            op,
            "max_normal_retention",
        )?,
        max_velocity_reversals: session_script_optional_i64(
            object,
            index,
            op,
            "max_velocity_reversals",
        )?,
        max_same_contact_frames: session_script_optional_i64(
            object,
            index,
            op,
            "max_same_contact_frames",
        )?,
        max_angular_velocity_spike: session_script_optional_f64(
            object,
            index,
            op,
            "max_angular_velocity_spike",
        )?,
        max_overlap_depth: session_script_optional_f64(object, index, op, "max_overlap_depth")?,
        max_low_progress_contact_frames: session_script_optional_i64(
            object,
            index,
            op,
            "max_low_progress_contact_frames",
        )?,
        strict_warnings: session_script_bool(object, "strict_warnings", false),
    })
}

pub(crate) fn session_script_audio_capture_args(
    value: &Value,
    index: usize,
) -> Result<AudioCaptureArgs, CommandResult> {
    let op = "audio_capture";
    let object = session_script_object(value, index, op)?;
    Ok(AudioCaptureArgs {
        live: true,
        setup_script: None,
        seconds: session_script_required_f64(object, index, op, "seconds")?,
        max_seconds: session_script_optional_f64(object, index, op, "max_seconds")?.unwrap_or(60.0),
        bus: session_script_optional_string(object, index, op, "bus")?,
        name: session_script_optional_string(object, index, op, "name")?
            .unwrap_or_else(|| "capture".to_string()),
        out: session_script_optional_string(object, index, op, "out")?.map(PathBuf::from),
        analyze: session_script_bool(object, "analyze", false),
        strict_warnings: session_script_bool(object, "strict_warnings", false),
        stop_after_state: session_script_stop_after_state(object, index, op, "audio capture")?,
    })
}

fn session_script_stop_after_state(
    object: &Map<String, Value>,
    index: usize,
    op: &str,
    command: &str,
) -> Result<Option<String>, CommandResult> {
    let Some(value) = object.get("stop_after_state") else {
        return Ok(None);
    };
    if !value.is_object() {
        return Err(invalid_session_script_step(
            index,
            &format!("{op} stop_after_state must be an object"),
            json!("state_matches object"),
            value.clone(),
        ));
    }
    validate_stop_after_state_value(command, value).map_err(|failure| CommandResult {
        ok: false,
        command: "session run-script".to_string(),
        message: "session script failed".to_string(),
        data: Map::from_iter([
            ("step_index".to_string(), json!(index)),
            ("operation".to_string(), json!(op)),
        ]),
        failures: vec![failure],
        exit_code: 1,
    })?;
    Ok(Some(value.to_string()))
}

pub(crate) fn session_script_event_trace_args(
    value: &Value,
    index: usize,
) -> Result<EventTraceArgs, CommandResult> {
    let op = "event_trace";
    let object = session_script_object(value, index, op)?;
    Ok(EventTraceArgs {
        target: session_script_required_string(object, index, op, "target")?,
        provider: session_script_required_string(object, index, op, "provider")?,
        setup_script: None,
        summary_json: false,
        frames: session_script_optional_i64(object, index, op, "frames")?.unwrap_or(180),
        sample_every_frames: session_script_optional_i64(object, index, op, "sample_every_frames")?
            .unwrap_or(1),
        pairing_config: session_script_optional_string(object, index, op, "pairing_config")?
            .map(PathBuf::from),
        strict_warnings: session_script_bool(object, "strict_warnings", false),
    })
}

pub(crate) fn session_script_profile_args(
    value: &Value,
    index: usize,
) -> Result<SessionProfileArgs, CommandResult> {
    let op = "session_profile";
    let object = session_script_object(value, index, op)?;
    let frames = session_script_optional_i64(object, index, op, "frames")?;
    let seconds = session_script_optional_f64(object, index, op, "seconds")?;
    if frames.is_some() && seconds.is_some() {
        return Err(invalid_session_script_step(
            index,
            "session_profile step accepts either 'frames' or 'seconds', not both",
            json!("one of frames or seconds"),
            json!({"frames": frames, "seconds": seconds}),
        ));
    }
    Ok(SessionProfileArgs {
        soak: None,
        soak_status: None,
        soak_stop: None,
        detach: false,
        frames,
        seconds,
        max_frame_samples: session_script_optional_i64(object, index, op, "max_frame_samples")?,
        top: session_script_optional_i64(object, index, op, "top")?.unwrap_or(10),
        setup_script: None,
        summary_json: false,
    })
}

pub(crate) fn session_script_project_args(
    value: &Value,
    index: usize,
) -> Result<Vec<String>, CommandResult> {
    let Some(project_args) = value.as_object().and_then(|v| v.get("project_args")) else {
        return Ok(Vec::new());
    };
    let Some(items) = project_args.as_array() else {
        return Err(invalid_session_script_step(
            index,
            "session_start project_args must be a list of non-empty strings",
            json!("array of non-empty strings"),
            project_args.clone(),
        ));
    };
    let mut args = Vec::with_capacity(items.len());
    for item in items {
        let Some(arg) = item.as_str() else {
            return Err(invalid_session_script_step(
                index,
                "session_start project_args must contain only strings",
                json!("array of strings"),
                item.clone(),
            ));
        };
        if arg.is_empty() {
            return Err(invalid_session_script_step(
                index,
                "session_start project_args must contain only non-empty strings",
                json!("non-empty string"),
                item.clone(),
            ));
        }
        args.push(arg.to_string());
    }
    Ok(args)
}

pub(crate) fn session_script_frame_pacing_request(
    value: &Value,
    index: usize,
) -> Result<Option<Option<i64>>, CommandResult> {
    let Some(object) = value.as_object() else {
        return Ok(None);
    };
    let top_fixed_fps = match object.get("fixed_fps") {
        Some(value) => Some(value.as_i64().ok_or_else(|| {
            invalid_session_script_step(
                index,
                "session_start fixed_fps must be an integer",
                json!("integer"),
                value.clone(),
            )
        })?),
        None => None,
    };
    if let Some(frame_pacing_value) = object.get("frame_pacing") {
        let Some(frame_pacing) = frame_pacing_value.as_object() else {
            return Err(invalid_session_script_step(
                index,
                "session_start frame_pacing must be an object",
                json!({"mode": "fastest", "fixed_fps": 60}),
                frame_pacing_value.clone(),
            ));
        };
        let mode = frame_pacing.get("mode").and_then(Value::as_str);
        if mode != Some("fastest") {
            return Err(invalid_session_script_step(
                index,
                "session_start frame_pacing.mode must be fastest",
                json!("fastest"),
                frame_pacing.get("mode").cloned().unwrap_or(Value::Null),
            ));
        }
        let fixed_fps = match frame_pacing.get("fixed_fps") {
            Some(value) => Some(value.as_i64().ok_or_else(|| {
                invalid_session_script_step(
                    index,
                    "session_start frame_pacing.fixed_fps must be an integer",
                    json!("integer"),
                    value.clone(),
                )
            })?),
            None => top_fixed_fps,
        };
        return Ok(Some(fixed_fps));
    }
    if let Some(fastest_value) = object.get("fastest") {
        let Some(fastest) = fastest_value.as_bool() else {
            return Err(invalid_session_script_step(
                index,
                "session_start fastest must be a boolean",
                json!("boolean"),
                fastest_value.clone(),
            ));
        };
        if fastest {
            return Ok(Some(top_fixed_fps));
        }
    }
    if top_fixed_fps.is_some() {
        return Err(invalid_session_script_step(
            index,
            "session_start fixed_fps requires fastest: true or frame_pacing.mode: fastest",
            json!({"fastest": true, "fixed_fps": 60}),
            value.clone(),
        ));
    }
    Ok(None)
}

pub(crate) fn session_frame_pacing_matches(
    status: &CommandResult,
    requested: &runtime::SessionFramePacing,
) -> bool {
    let Some(actual) = status.data.get("frame_pacing") else {
        return false;
    };
    actual.get("mode").and_then(Value::as_str) == Some("fastest")
        && actual.get("fixed_fps").and_then(Value::as_i64) == Some(requested.fixed_fps)
}

pub(crate) fn session_frame_pacing_mismatch_result(
    index: usize,
    requested: &runtime::SessionFramePacing,
    status: &CommandResult,
) -> CommandResult {
    let actual = status
        .data
        .get("frame_pacing")
        .cloned()
        .unwrap_or(Value::Null);
    CommandResult {
        ok: false,
        command: "session run-script".to_string(),
        message: "session script failed".to_string(),
        data: Map::from_iter([
            ("step_index".to_string(), json!(index)),
            ("reused".to_string(), json!(false)),
        ]),
        failures: vec![FailureDetail {
            kind: "session_frame_pacing_mismatch".to_string(),
            command: Some("session run-script".to_string()),
            message: Some(
                "active live session frame pacing does not match session_start request".to_string(),
            ),
            expected: Some(requested.to_json()),
            actual: Some(actual),
            ..FailureDetail::default()
        }],
        exit_code: 1,
    }
}

pub(crate) fn invalid_session_script_step(
    index: usize,
    message: &str,
    expected: Value,
    actual: Value,
) -> CommandResult {
    CommandResult {
        ok: false,
        command: "session run-script".to_string(),
        message: "session script failed".to_string(),
        data: Map::from_iter([("step_index".to_string(), json!(index))]),
        failures: vec![FailureDetail {
            kind: "invalid_session_script".to_string(),
            command: Some("session run-script".to_string()),
            message: Some(message.to_string()),
            expected: Some(expected),
            actual: Some(actual),
            ..FailureDetail::default()
        }],
        exit_code: 1,
    }
}
