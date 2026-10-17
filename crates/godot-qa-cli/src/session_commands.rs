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

pub(crate) fn session_profile_result(
    config: &ProjectConfig,
    args: &SessionProfileArgs,
) -> CommandResult {
    let command = "session profile";
    if let Some(id) = &args.soak_status {
        return runtime::run_live_protocol_command(
            config,
            command,
            runtime::SESSION_SOAK_STATUS,
            Map::from_iter([("run_id".into(), json!(id))]),
        );
    }
    if let Some(id) = &args.soak_stop {
        return runtime::run_live_protocol_command(
            config,
            command,
            runtime::SESSION_SOAK_STOP,
            Map::from_iter([("run_id".into(), json!(id))]),
        );
    }
    if let Some(path) = &args.soak {
        return session_soak_result(config, path, args.detach);
    }
    if let Err(failure) = validate_session_profile_args(command, args) {
        return invalid_params_result(command, failure);
    }
    let frames = if args.seconds.is_none() {
        Some(args.frames.unwrap_or(60))
    } else {
        args.frames
    };
    runtime::session_profile_result(
        config,
        frames,
        args.seconds,
        effective_session_profile_max_frame_samples(args, frames),
        args.top,
    )
}

fn session_soak_result(config: &ProjectConfig, path: &str, detach: bool) -> CommandResult {
    let command = "session profile";
    let spec = match godot_qa_core::soak::load_spec(std::path::Path::new(path), command) {
        Ok(spec) => spec,
        Err(failure) => return invalid_params_result(command, failure),
    };
    let session = runtime::session_status_result(config);
    if !session.ok {
        return session;
    }
    if session.data.get("frame_pacing").is_some()
        || session
            .data
            .get("runtime")
            .and_then(|v| v.get("time_scale"))
            .and_then(Value::as_f64)
            != Some(1.0)
    {
        return invalid_params_result(
            command,
            FailureDetail {
                kind: "soak_realtime_required".into(),
                command: Some(command.into()),
                message: Some(
                    "live soak requires realtime pacing and Engine.time_scale == 1".into(),
                ),
                expected: Some(json!({"frame_pacing":null,"time_scale":1.0})),
                actual: Some(
                    json!({"frame_pacing":session.data.get("frame_pacing"),"time_scale":session.data.get("runtime").and_then(|v|v.get("time_scale"))}),
                ),
                ..FailureDetail::default()
            },
        );
    }
    let run_id = format!(
        "soak-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis()
    );
    let started = runtime::run_live_protocol_command(
        config,
        command,
        runtime::SESSION_SOAK_START,
        Map::from_iter([
            ("run_id".into(), json!(run_id)),
            ("spec".into(), spec.clone()),
        ]),
    );
    if !started.ok || detach {
        return started;
    }
    let deadline = std::time::Instant::now()
        + std::time::Duration::from_secs_f64(
            spec["warmup_seconds"].as_f64().unwrap_or(0.0)
                + spec["duration_seconds"].as_f64().unwrap_or(600.0)
                + 60.0,
        );
    loop {
        std::thread::sleep(std::time::Duration::from_secs(1));
        let status = runtime::run_live_protocol_command(
            config,
            command,
            runtime::SESSION_SOAK_STATUS,
            Map::from_iter([("run_id".into(), json!(run_id))]),
        );
        if !status.ok {
            return status;
        }
        if status.data.get("status").and_then(Value::as_str) != Some("running") {
            if status.data.get("ok").and_then(Value::as_bool) == Some(true) {
                return status;
            }
            let mut failed = status;
            failed.ok = false;
            failed.exit_code = 1;
            failed.message = "soak failed or incomplete".into();
            failed.failures.push(FailureDetail {
                kind: "soak_budget_failed".into(),
                command: Some(command.into()),
                message: Some("soak did not complete within declared budgets".into()),
                expected: Some(json!("complete and within budgets")),
                actual: Some(json!(failed.data)),
                ..FailureDetail::default()
            });
            return failed;
        }
        if std::time::Instant::now() >= deadline {
            let mut stopped = runtime::run_live_protocol_command(
                config,
                command,
                runtime::SESSION_SOAK_STOP,
                Map::from_iter([("run_id".into(), json!(run_id))]),
            );
            stopped.ok = false;
            stopped.exit_code = 1;
            stopped.message = "soak exceeded warmup + duration + 60 seconds".into();
            stopped.failures.push(FailureDetail {
                kind: "soak_timeout".into(),
                command: Some(command.into()),
                message: Some("soak did not finish before the CLI watchdog".into()),
                expected: Some(json!("complete before watchdog")),
                actual: Some(json!(stopped.data)),
                ..FailureDetail::default()
            });
            return stopped;
        }
    }
}

pub(crate) fn effective_session_profile_max_frame_samples(
    args: &SessionProfileArgs,
    effective_frames: Option<i64>,
) -> i64 {
    args.max_frame_samples
        .unwrap_or_else(|| effective_frames.unwrap_or(120).clamp(1, 1000))
}

pub(crate) fn validate_session_profile_args(
    command: &str,
    args: &SessionProfileArgs,
) -> Result<(), FailureDetail> {
    let frames = if args.seconds.is_none() {
        Some(args.frames.unwrap_or(60))
    } else {
        args.frames
    };
    if let Some(frames) = frames
        && !(1..=1000).contains(&frames)
    {
        return Err(FailureDetail {
            kind: "invalid_params".to_string(),
            command: Some(command.to_string()),
            message: Some("session profile requires --frames between 1 and 1000".to_string()),
            expected: Some(json!("integer frames in range 1..=1000")),
            actual: Some(json!(frames)),
            ..FailureDetail::default()
        });
    }
    if let Some(seconds) = args.seconds
        && (!seconds.is_finite() || seconds <= 0.0 || seconds > 30.0)
    {
        return Err(FailureDetail {
            kind: "invalid_params".to_string(),
            command: Some(command.to_string()),
            message: Some("session profile requires --seconds between 0 and 30".to_string()),
            expected: Some(json!("finite seconds in range (0, 30]")),
            actual: Some(json!(seconds)),
            ..FailureDetail::default()
        });
    }
    if let Some(max_frame_samples) = args.max_frame_samples
        && !(1..=1000).contains(&max_frame_samples)
    {
        return Err(FailureDetail {
            kind: "invalid_params".to_string(),
            command: Some(command.to_string()),
            message: Some(
                "session profile requires --max-frame-samples between 1 and 1000".to_string(),
            ),
            expected: Some(json!("integer max_frame_samples in range 1..=1000")),
            actual: Some(json!(max_frame_samples)),
            ..FailureDetail::default()
        });
    }
    if !(1..=100).contains(&args.top) {
        return Err(FailureDetail {
            kind: "invalid_params".to_string(),
            command: Some(command.to_string()),
            message: Some("session profile requires --top between 1 and 100".to_string()),
            expected: Some(json!("integer top in range 1..=100")),
            actual: Some(json!(args.top)),
            ..FailureDetail::default()
        });
    }
    Ok(())
}

pub(crate) fn session_time_scale_params(
    args: &SessionTimeScaleArgs,
) -> Result<serde_json::Map<String, Value>, FailureDetail> {
    let command = "session time-scale";
    if !args.time_scale.is_finite()
        || args.time_scale <= 0.0
        || args.time_scale > time_control::MAX_TIME_SCALE
    {
        return Err(FailureDetail {
            kind: "invalid_params".to_string(),
            command: Some(command.to_string()),
            message: Some(format!(
                "session time-scale requires time_scale greater than 0.0 and <= {}",
                time_control::MAX_TIME_SCALE
            )),
            expected: Some(json!(format!(
                "positive finite number <= {}",
                time_control::MAX_TIME_SCALE
            ))),
            actual: Some(numeric_actual(args.time_scale)),
            ..FailureDetail::default()
        });
    }

    if let Some(value) = args.physics_ticks_per_second {
        if value < time_control::MIN_PHYSICS_TICKS_PER_SECOND {
            return Err(live_time_control_integer_failure(
                command,
                "physics_ticks_per_second",
                time_control::MIN_PHYSICS_TICKS_PER_SECOND,
                value,
            ));
        }
    }
    if let Some(value) = args.max_physics_steps_per_frame {
        if value < time_control::MIN_MAX_PHYSICS_STEPS_PER_FRAME {
            return Err(live_time_control_integer_failure(
                command,
                "max_physics_steps_per_frame",
                time_control::MIN_MAX_PHYSICS_STEPS_PER_FRAME,
                value,
            ));
        }
    }

    let mut params = Map::from_iter([("time_scale".to_string(), json!(args.time_scale))]);
    if let Some(value) = args.physics_ticks_per_second {
        params.insert("physics_ticks_per_second".to_string(), json!(value));
    }
    if let Some(value) = args.max_physics_steps_per_frame {
        params.insert("max_physics_steps_per_frame".to_string(), json!(value));
    }
    Ok(params)
}

pub(crate) fn live_time_control_integer_failure(
    command: &str,
    field: &str,
    minimum: i64,
    actual: i64,
) -> FailureDetail {
    FailureDetail {
        kind: "invalid_params".to_string(),
        command: Some(command.to_string()),
        message: Some(format!("session time-scale requires {field} >= {minimum}")),
        expected: Some(json!(format!("integer >= {minimum}"))),
        actual: Some(json!(actual)),
        ..FailureDetail::default()
    }
}

pub(crate) fn numeric_actual(value: f64) -> Value {
    if value.is_finite() {
        json!(value)
    } else {
        json!(value.to_string())
    }
}

pub(crate) fn session_frame_pacing_request(
    fastest: bool,
    fixed_fps: Option<i64>,
    command: &str,
) -> Result<Option<Option<i64>>, CommandResult> {
    if fastest {
        return Ok(Some(fixed_fps));
    }
    if let Some(fixed_fps) = fixed_fps {
        return Err(CommandResult {
            ok: false,
            command: command.to_string(),
            message: "unable to start live session".to_string(),
            data: Map::new(),
            failures: vec![FailureDetail {
                kind: "invalid_params".to_string(),
                command: Some(command.to_string()),
                message: Some("--fixed-fps requires --fastest".to_string()),
                expected: Some(json!("--fastest --fixed-fps <N>")),
                actual: Some(json!({"fixed_fps": fixed_fps, "fastest": false})),
                ..FailureDetail::default()
            }],
            exit_code: 1,
        });
    }
    Ok(None)
}
