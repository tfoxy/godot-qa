#![allow(unused_imports)]

use crate::args::*;
use crate::*;
use godot_qa_core::config::{ProjectConfig, load_project_config};
use godot_qa_core::results::{CommandResult, FailureDetail};
use godot_qa_runtime as runtime;
use godot_qa_scenario::{
    artifact_query, artifact_summary,
    audio::{AudioAnalysisOptions, DecodedWav},
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

pub(crate) fn audio_analyze_result(args: &AudioAnalyzeArgs) -> CommandResult {
    let command = "audio analyze";
    let options = AudioAnalysisOptions {
        window_seconds: args.window_seconds.unwrap_or(1.0),
        hop_seconds: args.hop_seconds.unwrap_or(0.5),
    };
    let decoded = DecodedWav::read(&args.wav);
    match decoded.and_then(|wav| wav.analyze(options).map(|analysis| (wav, analysis))) {
        Ok((wav, analysis)) => {
            let warnings = analysis
                .get("warnings")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            let warning_ids = analysis
                .get("warning_ids")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            let mut failures = if args.strict_warnings && !warnings.is_empty() {
                warning_ids
                    .iter()
                    .filter_map(Value::as_str)
                    .map(|warning_id| FailureDetail {
                        kind: warning_id.to_string(),
                        command: Some(command.to_string()),
                        path: Some(args.wav.to_string_lossy().into_owned()),
                        severity: Some("warning".to_string()),
                        expected: Some(json!("no audio analysis warnings")),
                        actual: Some(json!(warning_id)),
                        ..FailureDetail::default()
                    })
                    .collect::<Vec<_>>()
            } else {
                Vec::new()
            };
            let mut artifacts = json!({ "wav": args.wav });
            if let Some(png) = &args.spectrogram {
                match render_analyze_spectrogram(command, &wav, png) {
                    Ok(()) => artifacts["spectrogram"] = json!(png),
                    Err(failure) => failures.push(failure),
                }
            }
            let ok = failures.is_empty();
            CommandResult {
                ok,
                command: command.to_string(),
                message: if ok {
                    "audio analysis completed".to_string()
                } else {
                    "audio analysis completed with failures".to_string()
                },
                data: Map::from_iter([
                    ("artifacts".to_string(), artifacts),
                    ("analysis".to_string(), analysis),
                    ("warnings".to_string(), json!(warnings)),
                ]),
                failures,
                exit_code: if ok { 0 } else { 1 },
            }
        }
        Err(error) => {
            let failure_kind = if error.code == "audio_analysis_invalid_options"
                || error.code == "audio_analysis_window_limit_exceeded"
                || error.code == "audio_read_failed"
            {
                error.code
            } else {
                "audio_analysis.invalid_wav"
            };
            CommandResult {
                ok: false,
                command: command.to_string(),
                message: "audio analysis failed".to_string(),
                data: Map::from_iter([
                    ("artifacts".to_string(), json!({ "wav": args.wav })),
                    ("warnings".to_string(), json!([])),
                ]),
                failures: vec![FailureDetail {
                    kind: failure_kind.to_string(),
                    message: Some(error.message),
                    command: Some(command.to_string()),
                    path: Some(args.wav.to_string_lossy().into_owned()),
                    expected: Some(json!("valid bounded WAV analysis input")),
                    actual: Some(json!(error.code)),
                    ..FailureDetail::default()
                }],
                exit_code: 1,
            }
        }
    }
}

fn render_analyze_spectrogram(
    command: &str,
    wav: &DecodedWav,
    png: &Path,
) -> Result<(), FailureDetail> {
    if png.extension().and_then(|ext| ext.to_str()) != Some("png") {
        return Err(FailureDetail {
            kind: "invalid_params".to_string(),
            command: Some(command.to_string()),
            message: Some("audio analyze --spectrogram must be a .png path".to_string()),
            path: Some(png.to_string_lossy().into_owned()),
            expected: Some(json!(".png output path")),
            actual: Some(json!(png)),
            ..FailureDetail::default()
        });
    }
    wav.write_spectrogram(png).map_err(|error| FailureDetail {
        kind: error.code.to_string(),
        command: Some(command.to_string()),
        message: Some(error.message),
        path: Some(png.to_string_lossy().into_owned()),
        expected: Some(json!("writable spectrogram PNG")),
        actual: Some(json!(error.code)),
        ..FailureDetail::default()
    })
}

pub(crate) fn audio_capture_request(
    config: &ProjectConfig,
    args: &AudioCaptureArgs,
) -> Result<runtime::LiveAudioCaptureRequest, FailureDetail> {
    if !args.live {
        return Err(invalid_params_failure(
            "audio capture",
            "audio capture requires --live for this spec",
            json!("--live"),
            json!("missing"),
        ));
    }
    validate_audio_duration(args.seconds, args.max_seconds, "audio capture")?;
    validate_audio_name(&args.name, "audio capture")?;
    let out = match &args.out {
        Some(out) => Some(validate_audio_out_path(config, out, "audio capture")?),
        None => None,
    };
    let stop_after_state =
        parse_stop_after_state("audio capture", args.stop_after_state.as_deref())?;
    Ok(runtime::LiveAudioCaptureRequest {
        seconds: args.seconds.min(args.max_seconds),
        max_seconds: args.max_seconds,
        bus: args.bus.clone(),
        name: args.name.clone(),
        out,
        analyze: args.analyze,
        strict_warnings: args.strict_warnings,
        stop_after_state,
    })
}

/// Parse and validate a `--stop-after-state` predicate shared by `audio capture`,
/// `trace record`, and `screenshot sequence`. `command` is threaded through so failure
/// payloads name the invoking command rather than a hard-coded one.
pub(crate) fn parse_stop_after_state(
    command: &str,
    raw: Option<&str>,
) -> Result<Option<Value>, FailureDetail> {
    let Some(raw) = raw.map(str::trim).filter(|raw| !raw.is_empty()) else {
        return Ok(None);
    };
    let value = serde_json::from_str::<Value>(raw).map_err(|error| {
        invalid_params_failure(
            command,
            &format!("{command} --stop-after-state must be valid JSON: {error}"),
            json!("JSON object using state_matches fields"),
            json!(raw),
        )
    })?;
    if !value.is_object() {
        return Err(invalid_params_failure(
            command,
            &format!("{command} --stop-after-state must be a JSON object"),
            json!("JSON object using state_matches fields"),
            value,
        ));
    }
    validate_stop_after_state_value(command, &value)?;
    Ok(Some(value))
}

pub(crate) fn validate_stop_after_state_value(
    command: &str,
    value: &Value,
) -> Result<(), FailureDetail> {
    let Some(object) = value.as_object() else {
        return Err(invalid_params_failure(
            command,
            &format!("{command} stop_after_state must be an object"),
            json!("state_matches object"),
            value.clone(),
        ));
    };
    let target_ok = object
        .get("target")
        .and_then(Value::as_str)
        .is_some_and(|target| !target.trim().is_empty());
    if !target_ok {
        return Err(invalid_params_failure(
            command,
            &format!("{command} stop_after_state requires a non-empty target"),
            json!("non-empty target"),
            object.get("target").cloned().unwrap_or(Value::Null),
        ));
    }
    let has_matches = object
        .get("matches")
        .and_then(Value::as_array)
        .is_some_and(|matches| !matches.is_empty());
    let has_single_check = object
        .get("path")
        .and_then(Value::as_str)
        .is_some_and(|path| !path.trim().is_empty())
        && object
            .get("operator")
            .and_then(Value::as_str)
            .is_some_and(|operator| !operator.trim().is_empty());
    if !has_matches && !has_single_check {
        return Err(invalid_params_failure(
            command,
            &format!("{command} stop_after_state requires matches or path/operator"),
            json!("matches array or path/operator check"),
            value.clone(),
        ));
    }
    if let Some(sample_every_frames) = object.get("sample_every_frames")
        && sample_every_frames
            .as_i64()
            .is_none_or(|frames| frames <= 0)
    {
        return Err(invalid_params_failure(
            command,
            &format!("{command} stop_after_state sample_every_frames must be positive"),
            json!("positive integer sample_every_frames"),
            sample_every_frames.clone(),
        ));
    }
    Ok(())
}

pub(crate) fn audio_export_request(
    config: &ProjectConfig,
    args: &AudioExportArgs,
) -> Result<runtime::LiveAudioExportRequest, FailureDetail> {
    validate_audio_name(&args.name, "audio export")?;
    if args.target.trim().is_empty() {
        return Err(invalid_params_failure(
            "audio export",
            "audio export requires a non-empty --target",
            json!("non-empty target"),
            json!(args.target),
        ));
    }
    if !args.method.starts_with("qa_") {
        return Err(invalid_params_failure(
            "audio export",
            "audio export --method must name a qa_ fixture method",
            json!("method starting with qa_"),
            json!(args.method),
        ));
    }
    let export_args = match &args.args {
        Some(raw) => serde_json::from_str::<Value>(raw).map_err(|error| {
            invalid_params_failure(
                "audio export",
                &format!("audio export --args must be valid JSON: {error}"),
                json!("JSON array"),
                json!(raw),
            )
        })?,
        None => json!([]),
    };
    if !export_args.is_array() {
        return Err(invalid_params_failure(
            "audio export",
            "audio export --args must be a JSON array",
            json!("JSON array"),
            export_args,
        ));
    }
    let out = match &args.out {
        Some(out) => Some(validate_audio_out_path(config, out, "audio export")?),
        None => None,
    };
    Ok(runtime::LiveAudioExportRequest {
        target: args.target.clone(),
        method: args.method.clone(),
        args: export_args,
        name: args.name.clone(),
        out,
        analyze: args.analyze,
        strict_warnings: args.strict_warnings,
    })
}

pub(crate) fn audio_render_request(
    config: &ProjectConfig,
    args: &AudioRenderArgs,
) -> Result<runtime::LiveAudioRenderRequest, FailureDetail> {
    validate_audio_name(&args.name, "audio render")?;
    if args.target.trim().is_empty() {
        return Err(invalid_params_failure(
            "audio render",
            "audio render requires a non-empty --target",
            json!("non-empty target"),
            json!(args.target),
        ));
    }
    if !args.method.starts_with("qa_") {
        return Err(invalid_params_failure(
            "audio render",
            "audio render --method must name a qa_ fixture method",
            json!("method starting with qa_"),
            json!(args.method),
        ));
    }
    validate_audio_duration(args.seconds, args.max_seconds, "audio render")?;
    if args.mix_rate == 0 {
        return Err(invalid_params_failure(
            "audio render",
            "audio render --mix-rate must be a positive sample rate",
            json!("positive mix rate"),
            json!(args.mix_rate),
        ));
    }
    let render_args = match &args.args {
        Some(raw) => serde_json::from_str::<Value>(raw).map_err(|error| {
            invalid_params_failure(
                "audio render",
                &format!("audio render --args must be valid JSON: {error}"),
                json!("JSON array"),
                json!(raw),
            )
        })?,
        None => json!([]),
    };
    if !render_args.is_array() {
        return Err(invalid_params_failure(
            "audio render",
            "audio render --args must be a JSON array",
            json!("JSON array"),
            render_args,
        ));
    }
    let out = match &args.out {
        Some(out) => Some(validate_audio_out_path(config, out, "audio render")?),
        None => None,
    };
    Ok(runtime::LiveAudioRenderRequest {
        target: args.target.clone(),
        method: args.method.clone(),
        args: render_args,
        seconds: args.seconds.min(args.max_seconds),
        max_seconds: args.max_seconds,
        mix_rate: args.mix_rate,
        name: args.name.clone(),
        out,
        analyze: args.analyze,
        strict_warnings: args.strict_warnings,
    })
}

pub(crate) fn validate_audio_duration(
    seconds: f64,
    max_seconds: f64,
    command: &str,
) -> Result<(), FailureDetail> {
    if !seconds.is_finite() || seconds <= 0.0 {
        return Err(invalid_params_failure(
            command,
            "audio capture requires a positive finite --seconds value",
            json!("positive finite seconds"),
            json!(seconds),
        ));
    }
    if !max_seconds.is_finite() || max_seconds <= 0.0 {
        return Err(invalid_params_failure(
            command,
            "audio capture requires a positive finite --max-seconds value",
            json!("positive finite max seconds"),
            json!(max_seconds),
        ));
    }
    if seconds > max_seconds {
        return Err(FailureDetail {
            kind: "audio_capture.duration_too_long".to_string(),
            command: Some(command.to_string()),
            message: Some("audio capture duration exceeds the effective maximum".to_string()),
            expected: Some(json!({ "seconds_lte": max_seconds })),
            actual: Some(json!(seconds)),
            ..FailureDetail::default()
        });
    }
    Ok(())
}

pub(crate) fn validate_audio_name(name: &str, command: &str) -> Result<(), FailureDetail> {
    let valid = !name.is_empty()
        && name.len() <= 96
        && name
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '_' || ch == '-');
    if valid {
        Ok(())
    } else {
        Err(invalid_params_failure(
            command,
            "audio artifact name must contain only ASCII letters, digits, '_' or '-'",
            json!("safe artifact name"),
            json!(name),
        ))
    }
}

pub(crate) fn validate_audio_out_path(
    config: &ProjectConfig,
    out: &Path,
    command: &str,
) -> Result<PathBuf, FailureDetail> {
    if out.is_absolute()
        || out
            .components()
            .any(|component| matches!(component, std::path::Component::ParentDir))
        || out.extension().and_then(|ext| ext.to_str()) != Some("wav")
    {
        return Err(invalid_params_failure(
            command,
            "audio --out must be a relative .wav path without '..'",
            json!("relative .wav path under .qa/artifacts"),
            json!(out),
        ));
    }
    let artifacts_prefix = Path::new(".qa").join("artifacts");
    if !out.starts_with(&artifacts_prefix) {
        return Err(invalid_params_failure(
            command,
            "audio --out must stay under .qa/artifacts",
            json!(artifacts_prefix),
            json!(out),
        ));
    }
    Ok(config.project_root.join(out))
}

pub(crate) fn invalid_params_failure(
    command: &str,
    message: &str,
    expected: Value,
    actual: Value,
) -> FailureDetail {
    FailureDetail {
        kind: "invalid_params".to_string(),
        command: Some(command.to_string()),
        message: Some(message.to_string()),
        expected: Some(expected),
        actual: Some(actual),
        ..FailureDetail::default()
    }
}
