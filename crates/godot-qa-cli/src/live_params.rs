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

pub(crate) fn snapshot_filter_params(
    command: &str,
    args: &SnapshotFilterArgs,
) -> Result<Map<String, Value>, FailureDetail> {
    let mut params = Map::new();
    if let Some(target) = &args.target {
        let target = target.trim();
        if target.is_empty() {
            return Err(FailureDetail {
                kind: "invalid_params".to_string(),
                command: Some(command.to_string()),
                message: Some(format!("{command} requires --target to be non-empty")),
                expected: Some(json!("non-empty target locator")),
                actual: Some(json!(target)),
                ..FailureDetail::default()
            });
        }
        params.insert("target".to_string(), json!(target));
    }
    if let Some(max_depth) = args.max_depth {
        if max_depth < 0 {
            return Err(FailureDetail {
                kind: "invalid_params".to_string(),
                command: Some(command.to_string()),
                message: Some(format!("{command} requires --max-depth to be non-negative")),
                expected: Some(json!("non-negative integer max_depth")),
                actual: Some(json!(max_depth)),
                ..FailureDetail::default()
            });
        }
        params.insert("max_depth".to_string(), json!(max_depth));
    }
    if args.qa_only {
        params.insert("qa_only".to_string(), json!(true));
    }
    if args.include_metadata {
        params.insert("include_metadata".to_string(), json!(true));
    }
    let metadata_keys = normalized_metadata_keys(command, &args.metadata_key, &args.metadata_keys)?;
    if !metadata_keys.is_empty() {
        params.insert("metadata_keys".to_string(), json!(metadata_keys));
    }
    Ok(params)
}

pub(crate) fn normalized_metadata_keys(
    command: &str,
    metadata_key: &[String],
    metadata_keys: &[String],
) -> Result<Vec<String>, FailureDetail> {
    let mut keys = Vec::new();
    for raw in metadata_key.iter().chain(metadata_keys.iter()) {
        let key = raw.trim();
        if !is_safe_metadata_key(key) {
            return Err(FailureDetail {
                kind: "invalid_params".to_string(),
                command: Some(command.to_string()),
                message: Some(format!(
                    "{command} metadata keys must be non-empty and use only ASCII letters, numbers, dot, underscore, hyphen, or colon"
                )),
                expected: Some(json!("non-empty metadata key using [A-Za-z0-9._:-]")),
                actual: Some(json!(raw)),
                ..FailureDetail::default()
            });
        }
        keys.push(key.to_string());
    }
    Ok(keys)
}

pub(crate) fn is_safe_metadata_key(key: &str) -> bool {
    !key.is_empty()
        && key
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | '.' | ':'))
}

pub(crate) fn physics_inspect_params(
    args: &PhysicsInspectArgs,
) -> Result<Map<String, Value>, FailureDetail> {
    let command = "physics inspect";
    let target = args.target.trim();
    if target.is_empty() {
        return Err(FailureDetail {
            kind: "invalid_params".to_string(),
            command: Some(command.to_string()),
            message: Some("physics inspect requires a non-empty target".to_string()),
            expected: Some(json!("non-empty target locator")),
            actual: Some(json!(args.target)),
            ..FailureDetail::default()
        });
    }
    if !args.radius.is_finite() || args.radius <= 0.0 || args.radius > 4096.0 {
        return Err(FailureDetail {
            kind: "invalid_params".to_string(),
            command: Some(command.to_string()),
            message: Some("physics inspect requires --radius > 0 and <= 4096".to_string()),
            expected: Some(json!("positive finite radius <= 4096")),
            actual: Some(numeric_actual(args.radius)),
            ..FailureDetail::default()
        });
    }
    if args.frames < 1 || args.frames > 240 {
        return Err(FailureDetail {
            kind: "invalid_params".to_string(),
            command: Some(command.to_string()),
            message: Some("physics inspect requires --frames between 1 and 240".to_string()),
            expected: Some(json!("integer frames in [1, 240]")),
            actual: Some(json!(args.frames)),
            ..FailureDetail::default()
        });
    }

    let state_provider = args.state_provider.trim();
    if state_provider.is_empty() {
        return Err(FailureDetail {
            kind: "invalid_params".to_string(),
            command: Some(command.to_string()),
            message: Some("physics inspect requires --state-provider to be non-empty".to_string()),
            expected: Some(json!("scene-root or target locator")),
            actual: Some(json!(args.state_provider)),
            ..FailureDetail::default()
        });
    }

    let mut methods = Vec::new();
    for method in &args.state_method {
        let method = method.trim();
        if !is_allowed_physics_state_method(method) {
            return Err(FailureDetail {
                kind: "invalid_params".to_string(),
                command: Some(command.to_string()),
                message: Some(
                    "physics inspect state methods must be read-only QA state or diagnostics methods"
                        .to_string(),
                ),
                expected: Some(json!("get_qa_*_state, get_*_qa_state, or qa_*diagnostic*")),
                actual: Some(json!(method)),
                ..FailureDetail::default()
            });
        }
        methods.push(method.to_string());
    }

    let mut params = Map::from_iter([
        ("target".to_string(), json!(target)),
        ("radius".to_string(), json!(args.radius)),
        ("frames".to_string(), json!(args.frames)),
        ("state_provider".to_string(), json!(state_provider)),
        (
            "include_project_state".to_string(),
            json!(!args.no_project_state),
        ),
    ]);
    if !methods.is_empty() {
        params.insert("state_methods".to_string(), json!(methods));
    }
    if let Some(snapshot) = &args.snapshot {
        let snapshot = snapshot.trim();
        if snapshot.is_empty() {
            return Err(FailureDetail {
                kind: "invalid_params".to_string(),
                command: Some(command.to_string()),
                message: Some("physics inspect requires --snapshot to be non-empty".to_string()),
                expected: Some(json!("non-empty snapshot name")),
                actual: Some(json!(snapshot)),
                ..FailureDetail::default()
            });
        }
        params.insert("snapshot".to_string(), json!(snapshot));
    }
    Ok(params)
}

pub(crate) fn physics_overlay_params(
    args: &PhysicsOverlayArgs,
) -> Result<Map<String, Value>, FailureDetail> {
    let command = "physics overlay";
    match (&args.scenario, &args.snapshot) {
        (Some(scenario), Some(snapshot)) => {
            if scenario.trim().is_empty() || snapshot.trim().is_empty() {
                return Err(FailureDetail {
                    kind: "invalid_params".to_string(),
                    command: Some(command.to_string()),
                    message: Some(
                        "physics overlay review mode requires non-empty --scenario and --snapshot"
                            .to_string(),
                    ),
                    expected: Some(json!("non-empty scenario and snapshot names")),
                    actual: Some(json!({"scenario": scenario, "snapshot": snapshot})),
                    ..FailureDetail::default()
                });
            }
        }
        (Some(_), None) | (None, Some(_)) => {
            return Err(FailureDetail {
                kind: "invalid_params".to_string(),
                command: Some(command.to_string()),
                message: Some(
                    "physics overlay requires --scenario and --snapshot together for review mode"
                        .to_string(),
                ),
                expected: Some(json!("both --scenario and --snapshot, or neither")),
                actual: Some(json!({"scenario": args.scenario, "snapshot": args.snapshot})),
                ..FailureDetail::default()
            });
        }
        (None, None) => {}
    }
    let target = args.target.as_deref().unwrap_or("").trim();
    if args.viewport {
        if !target.is_empty() {
            return Err(FailureDetail {
                kind: "invalid_params".to_string(),
                command: Some(command.to_string()),
                message: Some(
                    "physics overlay accepts either a target or --viewport, not both".to_string(),
                ),
                expected: Some(json!("target locator or --viewport")),
                actual: Some(json!({"target": target, "viewport": args.viewport})),
                ..FailureDetail::default()
            });
        }
    } else if target.is_empty() {
        return Err(FailureDetail {
            kind: "invalid_params".to_string(),
            command: Some(command.to_string()),
            message: Some("physics overlay requires a non-empty target or --viewport".to_string()),
            expected: Some(json!("non-empty target locator or --viewport")),
            actual: Some(json!(args.target)),
            ..FailureDetail::default()
        });
    }
    if !args.radius.is_finite() || args.radius <= 0.0 || args.radius > 4096.0 {
        return Err(FailureDetail {
            kind: "invalid_params".to_string(),
            command: Some(command.to_string()),
            message: Some("physics overlay requires --radius > 0 and <= 4096".to_string()),
            expected: Some(json!("positive finite radius <= 4096")),
            actual: Some(numeric_actual(args.radius)),
            ..FailureDetail::default()
        });
    }
    if args.frames < 1 || args.frames > 240 {
        return Err(FailureDetail {
            kind: "invalid_params".to_string(),
            command: Some(command.to_string()),
            message: Some("physics overlay requires --frames between 1 and 240".to_string()),
            expected: Some(json!("integer frames in [1, 240]")),
            actual: Some(json!(args.frames)),
            ..FailureDetail::default()
        });
    }
    if args.out.as_os_str().is_empty() {
        return Err(FailureDetail {
            kind: "invalid_params".to_string(),
            command: Some(command.to_string()),
            message: Some("physics overlay requires --out".to_string()),
            expected: Some(json!("output PNG path")),
            actual: Some(json!("")),
            ..FailureDetail::default()
        });
    }
    let extension = args
        .out
        .extension()
        .and_then(|extension| extension.to_str())
        .unwrap_or("");
    if !extension.eq_ignore_ascii_case("png") {
        return Err(FailureDetail {
            kind: "invalid_params".to_string(),
            command: Some(command.to_string()),
            message: Some("physics overlay --out must end in .png".to_string()),
            expected: Some(json!("*.png output path")),
            actual: Some(json!(args.out)),
            ..FailureDetail::default()
        });
    }
    let state_provider = args.state_provider.trim();
    if state_provider.is_empty() {
        return Err(FailureDetail {
            kind: "invalid_params".to_string(),
            command: Some(command.to_string()),
            message: Some("physics overlay requires --state-provider to be non-empty".to_string()),
            expected: Some(json!("scene-root or target locator")),
            actual: Some(json!(args.state_provider)),
            ..FailureDetail::default()
        });
    }
    let mut methods = Vec::new();
    for method in &args.state_method {
        let method = method.trim();
        if !is_allowed_physics_state_method(method) {
            return Err(FailureDetail {
                kind: "invalid_params".to_string(),
                command: Some(command.to_string()),
                message: Some(
                    "physics overlay state methods must be read-only QA state or diagnostics methods"
                        .to_string(),
                ),
                expected: Some(json!("get_qa_*_state, get_*_qa_state, or qa_*diagnostic*")),
                actual: Some(json!(method)),
                ..FailureDetail::default()
            });
        }
        methods.push(method.to_string());
    }
    if args.max_shapes < 1 || args.max_shapes > 4096 {
        return Err(FailureDetail {
            kind: "invalid_params".to_string(),
            command: Some(command.to_string()),
            message: Some("physics overlay requires --max-shapes between 1 and 4096".to_string()),
            expected: Some(json!("integer max shapes in [1, 4096]")),
            actual: Some(json!(args.max_shapes)),
            ..FailureDetail::default()
        });
    }

    let mut params = Map::from_iter([
        ("target".to_string(), json!(target)),
        ("viewport".to_string(), json!(args.viewport)),
        ("radius".to_string(), json!(args.radius)),
        ("frames".to_string(), json!(args.frames)),
        ("include_aabb".to_string(), json!(args.include_aabb)),
        ("include_contacts".to_string(), json!(args.include_contacts)),
        ("state_provider".to_string(), json!(state_provider)),
        (
            "include_project_state".to_string(),
            json!(!args.no_project_state),
        ),
        ("max_shapes".to_string(), json!(args.max_shapes)),
        ("strict".to_string(), json!(args.strict)),
    ]);
    if !methods.is_empty() {
        params.insert("state_methods".to_string(), json!(methods));
    }
    Ok(params)
}

pub(crate) fn trace_record_params(
    args: &TraceRecordArgs,
) -> Result<Map<String, Value>, FailureDetail> {
    let command = "trace record";
    let mut targets = Vec::new();
    for target in &args.target {
        let target = target.trim();
        if target.is_empty() {
            return Err(FailureDetail {
                kind: "invalid_params".to_string(),
                command: Some(command.to_string()),
                message: Some("trace record requires every --target to be non-empty".to_string()),
                expected: Some(json!("non-empty target locator")),
                actual: Some(json!(args.target)),
                ..FailureDetail::default()
            });
        }
        targets.push(target.to_string());
    }
    if targets.is_empty() {
        return Err(FailureDetail {
            kind: "invalid_params".to_string(),
            command: Some(command.to_string()),
            message: Some("trace record requires at least one --target".to_string()),
            expected: Some(json!("one or more --target locators")),
            actual: Some(json!(args.target)),
            ..FailureDetail::default()
        });
    }
    if args.frames < 1 || args.frames > 3600 {
        return Err(FailureDetail {
            kind: "invalid_params".to_string(),
            command: Some(command.to_string()),
            message: Some("trace record requires --frames between 1 and 3600".to_string()),
            expected: Some(json!("integer frames in [1, 3600]")),
            actual: Some(json!(args.frames)),
            ..FailureDetail::default()
        });
    }
    if args.sample_every_frames < 1 || args.sample_every_frames > 240 {
        return Err(FailureDetail {
            kind: "invalid_params".to_string(),
            command: Some(command.to_string()),
            message: Some(
                "trace record requires --sample-every-frames between 1 and 240".to_string(),
            ),
            expected: Some(json!("integer sample interval in [1, 240]")),
            actual: Some(json!(args.sample_every_frames)),
            ..FailureDetail::default()
        });
    }

    let state_provider = args.state_provider.trim();
    if state_provider.is_empty() {
        return Err(FailureDetail {
            kind: "invalid_params".to_string(),
            command: Some(command.to_string()),
            message: Some("trace record requires --state-provider to be non-empty".to_string()),
            expected: Some(json!("scene-root or target locator")),
            actual: Some(json!(args.state_provider)),
            ..FailureDetail::default()
        });
    }

    let mut methods = Vec::new();
    for method in &args.state_method {
        let method = method.trim();
        if !is_allowed_physics_state_method(method) {
            return Err(FailureDetail {
                kind: "invalid_params".to_string(),
                command: Some(command.to_string()),
                message: Some(
                    "trace record state methods must be read-only QA state or diagnostics methods"
                        .to_string(),
                ),
                expected: Some(json!("get_qa_*_state, get_*_qa_state, or qa_*diagnostic*")),
                actual: Some(json!(method)),
                ..FailureDetail::default()
            });
        }
        methods.push(method.to_string());
    }

    let stop_after_state = parse_stop_after_state(command, args.stop_after_state.as_deref())?;

    let mut params = Map::from_iter([
        ("targets".to_string(), json!(targets)),
        ("frames".to_string(), json!(args.frames)),
        (
            "sample_every_frames".to_string(),
            json!(args.sample_every_frames),
        ),
        ("include_physics".to_string(), json!(args.physics)),
        ("state_provider".to_string(), json!(state_provider)),
        (
            "include_project_state".to_string(),
            json!(!methods.is_empty()),
        ),
        ("snapshot_start".to_string(), json!(args.snapshot_start)),
        ("snapshot_end".to_string(), json!(args.snapshot_end)),
    ]);
    if !methods.is_empty() {
        params.insert("state_methods".to_string(), json!(methods));
    }
    if let Some(stop_after_state) = stop_after_state {
        params.insert("stop_after_state".to_string(), stop_after_state);
    }
    Ok(params)
}

pub(crate) struct EventSummarizeParams {
    pub(crate) events: PathBuf,
    pub(crate) pairing_config: Option<PathBuf>,
    pub(crate) strict_warnings: bool,
}

pub(crate) fn event_trace_params(
    args: &EventTraceArgs,
) -> Result<Map<String, Value>, FailureDetail> {
    let command = "event trace";
    let target = args.target.trim();
    if target.is_empty() {
        return Err(FailureDetail {
            kind: "invalid_params".to_string(),
            command: Some(command.to_string()),
            message: Some("event trace requires --target to be non-empty".to_string()),
            expected: Some(json!("non-empty target locator")),
            actual: Some(json!(args.target)),
            ..FailureDetail::default()
        });
    }
    let raw_provider = args.provider.trim();
    let provider = if raw_provider == target {
        "get_qa_event_stream_state"
    } else {
        raw_provider
    };
    if provider.is_empty() {
        return Err(FailureDetail {
            kind: "invalid_params".to_string(),
            command: Some(command.to_string()),
            message: Some("event trace requires --provider to be non-empty".to_string()),
            expected: Some(json!("approved event provider method")),
            actual: Some(json!(args.provider)),
            ..FailureDetail::default()
        });
    }
    if !runtime::event_trace::APPROVED_EVENT_PROVIDER_METHODS.contains(&provider) {
        return Err(FailureDetail {
            kind: "invalid_params".to_string(),
            command: Some(command.to_string()),
            message: Some(
                "event trace provider must be an approved read-only event provider method"
                    .to_string(),
            ),
            expected: Some(json!(runtime::event_trace::APPROVED_EVENT_PROVIDER_METHODS)),
            actual: Some(json!(provider)),
            ..FailureDetail::default()
        });
    }
    if args.frames < 1 || args.frames > 3600 {
        return Err(FailureDetail {
            kind: "invalid_params".to_string(),
            command: Some(command.to_string()),
            message: Some("event trace requires --frames between 1 and 3600".to_string()),
            expected: Some(json!("integer frames in [1, 3600]")),
            actual: Some(json!(args.frames)),
            ..FailureDetail::default()
        });
    }
    if args.sample_every_frames < 1 || args.sample_every_frames > 240 {
        return Err(FailureDetail {
            kind: "invalid_params".to_string(),
            command: Some(command.to_string()),
            message: Some(
                "event trace requires --sample-every-frames between 1 and 240".to_string(),
            ),
            expected: Some(json!("integer sample interval in [1, 240]")),
            actual: Some(json!(args.sample_every_frames)),
            ..FailureDetail::default()
        });
    }
    if let Some(path) = &args.pairing_config {
        if path.exists() {
            validate_event_pairing_config_path(path, command)?;
        }
    }

    let mut params = Map::from_iter([
        ("target".to_string(), json!(target)),
        ("provider".to_string(), json!(provider)),
        ("frames".to_string(), json!(args.frames)),
        (
            "sample_every_frames".to_string(),
            json!(args.sample_every_frames),
        ),
        ("strict_warnings".to_string(), json!(args.strict_warnings)),
    ]);
    if let Some(path) = &args.pairing_config {
        params.insert(
            "pairing_config".to_string(),
            json!(path.to_string_lossy().into_owned()),
        );
    }
    Ok(params)
}

pub(crate) fn telemetry_record_params(
    args: &TelemetryRecordArgs,
) -> Result<Map<String, Value>, FailureDetail> {
    let command = "telemetry record";
    let provider = args.provider.as_deref().map(str::trim).unwrap_or_default();
    let target = args.target.as_deref().map(str::trim).unwrap_or_default();
    let method = args.method.as_deref().map(str::trim).unwrap_or_default();
    if provider.is_empty() && (target.is_empty() || method.is_empty()) {
        return Err(FailureDetail {
            kind: "invalid_params".to_string(),
            command: Some(command.to_string()),
            message: Some(
                "telemetry record requires --provider or --target with --method".to_string(),
            ),
            expected: Some(json!("--provider <id> or --target <locator> --method qa_*")),
            actual: Some(json!({
                "provider": args.provider,
                "target": args.target,
                "method": args.method,
            })),
            ..FailureDetail::default()
        });
    }
    if !provider.is_empty() && !is_valid_provider_id(provider) {
        return Err(FailureDetail {
            kind: "invalid_params".to_string(),
            command: Some(command.to_string()),
            message: Some(
                "telemetry record --provider must be a registered provider id".to_string(),
            ),
            expected: Some(json!(
                "ASCII letters, numbers, '.', '_', ':', or '-' provider id"
            )),
            actual: Some(json!(args.provider)),
            ..FailureDetail::default()
        });
    }
    if !method.is_empty() && !is_allowed_telemetry_method(method) {
        return Err(FailureDetail {
            kind: "invalid_params".to_string(),
            command: Some(command.to_string()),
            message: Some(
                "telemetry record --method must be a read-only qa_ state method".to_string(),
            ),
            expected: Some(json!(
                "qa_* (non-mutating), get_qa_*_state, or get_*_qa_state"
            )),
            actual: Some(json!(method)),
            ..FailureDetail::default()
        });
    }
    if args.frames < 1 || args.frames > 3600 {
        return Err(FailureDetail {
            kind: "invalid_params".to_string(),
            command: Some(command.to_string()),
            message: Some("telemetry record requires --frames between 1 and 3600".to_string()),
            expected: Some(json!("integer frames in [1, 3600]")),
            actual: Some(json!(args.frames)),
            ..FailureDetail::default()
        });
    }
    if args.every < 1 || args.every > 240 {
        return Err(FailureDetail {
            kind: "invalid_params".to_string(),
            command: Some(command.to_string()),
            message: Some("telemetry record requires --every between 1 and 240".to_string()),
            expected: Some(json!("integer sample interval in [1, 240]")),
            actual: Some(json!(args.every)),
            ..FailureDetail::default()
        });
    }

    let mut params = Map::from_iter([
        ("frames".to_string(), json!(args.frames)),
        ("every".to_string(), json!(args.every)),
    ]);
    if !provider.is_empty() {
        params.insert("provider".to_string(), json!(provider));
    } else {
        params.insert("target".to_string(), json!(target));
        params.insert("method".to_string(), json!(method));
    }
    Ok(params)
}

fn is_valid_provider_id(provider_id: &str) -> bool {
    !provider_id.is_empty()
        && provider_id
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '.' | '_' | ':' | '-'))
}

/// Telemetry samples must stay read-only: accept the read-only QA state naming
/// contract plus plain `qa_`-prefixed provider methods, while rejecting the
/// mutating `qa_prepare_/qa_apply_/qa_mutate_/qa_set_` fixture prefixes.
fn is_allowed_telemetry_method(method: &str) -> bool {
    if method.is_empty()
        || !method
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '_')
        || method.starts_with("qa_prepare_")
        || method.starts_with("qa_apply_")
        || method.starts_with("qa_mutate_")
        || method.starts_with("qa_set_")
    {
        return false;
    }
    method.starts_with("qa_")
        || (method.starts_with("get_qa_") && method.ends_with("_state"))
        || (method.starts_with("get_") && method.ends_with("_qa_state"))
}

pub(crate) fn event_summarize_params(
    args: &EventSummarizeArgs,
) -> Result<EventSummarizeParams, FailureDetail> {
    let command = "event summarize";
    if args.events.as_os_str().is_empty() {
        return Err(FailureDetail {
            kind: "invalid_params".to_string(),
            command: Some(command.to_string()),
            message: Some("event summarize requires an events.json path".to_string()),
            expected: Some(json!("path to events.json")),
            actual: Some(json!("")),
            ..FailureDetail::default()
        });
    }
    if let Some(path) = &args.pairing_config {
        validate_event_pairing_config_path(path, command)?;
    }
    Ok(EventSummarizeParams {
        events: args.events.clone(),
        pairing_config: args.pairing_config.clone(),
        strict_warnings: args.strict_warnings,
    })
}

pub(crate) fn validate_event_pairing_config_path(
    path: &Path,
    command: &str,
) -> Result<(), FailureDetail> {
    let contents = std::fs::read_to_string(path).map_err(|error| FailureDetail {
        kind: "event_trace.invalid_pairing_config".to_string(),
        command: Some(command.to_string()),
        path: Some(path.to_string_lossy().into_owned()),
        message: Some(format!(
            "event trace pairing config could not be read: {error}"
        )),
        expected: Some(json!("readable pairing config JSON file")),
        actual: Some(json!("unreadable file")),
        ..FailureDetail::default()
    })?;
    runtime::event_trace::parse_pairing_config(&contents).map_err(|message| FailureDetail {
        kind: "event_trace.invalid_pairing_config".to_string(),
        command: Some(command.to_string()),
        path: Some(path.to_string_lossy().into_owned()),
        message: Some(message),
        expected: Some(json!("valid event trace pairing config")),
        actual: Some(json!("invalid pairing config")),
        ..FailureDetail::default()
    })?;
    Ok(())
}

pub(crate) fn physics_collision_report_params(
    args: &PhysicsCollisionReportArgs,
) -> Result<Map<String, Value>, FailureDetail> {
    let command = "physics collision-report";
    let target = args.target.as_deref().unwrap_or("").trim();
    let trace = args.from_trace.as_ref();
    let trace_empty = trace
        .map(|path| path.as_os_str().is_empty())
        .unwrap_or(false);
    if (target.is_empty() && trace.is_none()) || (!target.is_empty() && trace.is_some()) {
        return Err(FailureDetail {
            kind: "invalid_params".to_string(),
            command: Some(command.to_string()),
            message: Some(
                "physics collision-report requires either a live target or --from-trace, not both"
                    .to_string(),
            ),
            expected: Some(json!("non-empty target locator or --from-trace path")),
            actual: Some(json!({"target": target, "from_trace": trace})),
            ..FailureDetail::default()
        });
    }
    if trace_empty {
        return Err(FailureDetail {
            kind: "invalid_params".to_string(),
            command: Some(command.to_string()),
            message: Some(
                "physics collision-report requires --from-trace to be non-empty".to_string(),
            ),
            expected: Some(json!("non-empty trace path")),
            actual: Some(json!("")),
            ..FailureDetail::default()
        });
    }
    if args.frames < 1 || args.frames > 3600 {
        return Err(FailureDetail {
            kind: "invalid_params".to_string(),
            command: Some(command.to_string()),
            message: Some(
                "physics collision-report requires --frames between 1 and 3600".to_string(),
            ),
            expected: Some(json!("integer frames in [1, 3600]")),
            actual: Some(json!(args.frames)),
            ..FailureDetail::default()
        });
    }
    if args.sample_every_frames < 1 || args.sample_every_frames > 240 {
        return Err(FailureDetail {
            kind: "invalid_params".to_string(),
            command: Some(command.to_string()),
            message: Some(
                "physics collision-report requires --sample-every-frames between 1 and 240"
                    .to_string(),
            ),
            expected: Some(json!("integer sample interval in [1, 240]")),
            actual: Some(json!(args.sample_every_frames)),
            ..FailureDetail::default()
        });
    }
    let state_provider = args.state_provider.trim();
    if state_provider.is_empty() {
        return Err(FailureDetail {
            kind: "invalid_params".to_string(),
            command: Some(command.to_string()),
            message: Some(
                "physics collision-report requires --state-provider to be non-empty".to_string(),
            ),
            expected: Some(json!("scene-root or target locator")),
            actual: Some(json!(args.state_provider)),
            ..FailureDetail::default()
        });
    }
    let mut methods = Vec::new();
    for method in &args.state_method {
        let method = method.trim();
        if !is_allowed_collision_report_state_method(method) {
            return Err(FailureDetail {
                kind: "invalid_params".to_string(),
                command: Some(command.to_string()),
                message: Some(
                    "physics collision-report state methods must be approved read-only collision QA providers"
                        .to_string(),
                ),
                expected: Some(json!(collision_report_state_methods())),
                actual: Some(json!(method)),
                ..FailureDetail::default()
            });
        }
        methods.push(method.to_string());
    }
    validate_optional_non_negative_float(
        command,
        "--max-normal-retention",
        args.max_normal_retention,
    )?;
    validate_optional_non_negative_float(
        command,
        "--max-angular-velocity-spike",
        args.max_angular_velocity_spike,
    )?;
    validate_optional_non_negative_float(command, "--max-overlap-depth", args.max_overlap_depth)?;
    validate_optional_non_negative_i64(
        command,
        "--max-velocity-reversals",
        args.max_velocity_reversals,
    )?;
    validate_optional_non_negative_i64(
        command,
        "--max-same-contact-frames",
        args.max_same_contact_frames,
    )?;
    validate_optional_non_negative_i64(
        command,
        "--max-low-progress-contact-frames",
        args.max_low_progress_contact_frames,
    )?;

    let state_methods = if methods.is_empty() {
        collision_report_default_state_methods()
            .iter()
            .map(|method| (*method).to_string())
            .collect::<Vec<_>>()
    } else {
        methods
    };
    let mut params = Map::from_iter([
        ("targets".to_string(), json!([target])),
        ("frames".to_string(), json!(args.frames)),
        (
            "sample_every_frames".to_string(),
            json!(args.sample_every_frames),
        ),
        ("include_physics".to_string(), json!(args.physics)),
        ("state_provider".to_string(), json!(state_provider)),
        ("include_project_state".to_string(), json!(true)),
        ("state_methods".to_string(), json!(state_methods)),
    ]);
    if let Some(path) = trace {
        params.insert("from_trace".to_string(), json!(path));
        params.insert("targets".to_string(), json!([]));
    }
    Ok(params)
}

pub(crate) fn surface_audit_params(
    args: &SurfaceAuditArgs,
) -> Result<Map<String, Value>, FailureDetail> {
    let provider = args.provider.as_deref().unwrap_or_default().trim();
    let provider_id = args.provider_id.as_deref().unwrap_or_default().trim();
    if provider.is_empty() && provider_id.is_empty() {
        return Err(invalid_surface_param(
            "surface audit requires a non-empty provider target or --provider-id",
            json!("non-empty provider target locator or provider id"),
            json!({"provider": provider, "provider_id": provider_id}),
        ));
    }
    if !args.sample_step.is_finite() || args.sample_step <= 0.0 {
        return Err(invalid_surface_param(
            "surface audit requires --sample-step to be positive and finite",
            json!("positive finite sample step"),
            numeric_actual(args.sample_step),
        ));
    }
    if !args.actor_radius.is_finite() || args.actor_radius <= 0.0 {
        return Err(invalid_surface_param(
            "surface audit requires --actor-radius to be positive and finite",
            json!("positive finite actor radius"),
            numeric_actual(args.actor_radius),
        ));
    }
    validate_optional_positive_surface_float("--min-spawn-clearance", args.min_spawn_clearance)?;
    validate_optional_positive_surface_float("--min-corridor-width", args.min_corridor_width)?;

    let provider_method = args.provider_method.trim();
    if provider_id.is_empty() && !surface_audit_provider_methods().contains(&provider_method) {
        return Err(invalid_surface_param(
            "surface audit provider method must be one of the approved read-only surface probe methods",
            json!(surface_audit_provider_methods()),
            json!(provider_method),
        ));
    }

    let center = trimmed_optional(args.center.as_deref());
    let region_size = trimmed_optional(args.region_size.as_deref());
    let route = trimmed_optional(args.route.as_deref());
    let left_region = trimmed_optional(args.left_region.as_deref());
    let right_region = trimmed_optional(args.right_region.as_deref());
    let mirror_axis = trimmed_optional(args.mirror_axis.as_deref());

    if args.center.is_some() && center.is_none() {
        return Err(invalid_surface_param(
            "surface audit requires --center to be non-empty",
            json!("non-empty center locator"),
            json!(args.center),
        ));
    }
    if args.route.is_some() && route.is_none() {
        return Err(invalid_surface_param(
            "surface audit requires --route to be non-empty",
            json!("comma-separated route locators"),
            json!(args.route),
        ));
    }
    if args.left_region.is_some() && left_region.is_none() {
        return Err(invalid_surface_param(
            "surface audit requires --left-region to be non-empty",
            json!("non-empty left region locator"),
            json!(args.left_region),
        ));
    }
    if args.right_region.is_some() && right_region.is_none() {
        return Err(invalid_surface_param(
            "surface audit requires --right-region to be non-empty",
            json!("non-empty right region locator"),
            json!(args.right_region),
        ));
    }
    if args.mirror_axis.is_some() && mirror_axis.is_none() {
        return Err(invalid_surface_param(
            "surface audit requires --mirror-axis to be non-empty",
            json!("non-empty mirror axis locator"),
            json!(args.mirror_axis),
        ));
    }

    let spawn_mode = center.is_some() || region_size.is_some();
    if spawn_mode && (center.is_none() || region_size.is_none()) {
        return Err(invalid_surface_param(
            "surface audit local region mode requires --center and --region-size together",
            json!("both --center and --region-size"),
            json!({"center": &args.center, "region_size": &args.region_size}),
        ));
    }
    let mirror_mode = left_region.is_some() || right_region.is_some() || mirror_axis.is_some();
    if mirror_mode && (left_region.is_none() || right_region.is_none() || mirror_axis.is_none()) {
        return Err(invalid_surface_param(
            "surface audit mirror mode requires --left-region, --right-region, and --mirror-axis together",
            json!("all mirror region inputs"),
            json!({
                "left_region": &args.left_region,
                "right_region": &args.right_region,
                "mirror_axis": &args.mirror_axis
            }),
        ));
    }
    let route_mode = route.is_some();
    let mode_count = [spawn_mode, route_mode, mirror_mode]
        .iter()
        .filter(|mode| **mode)
        .count();
    if mode_count != 1 {
        return Err(invalid_surface_param(
            "surface audit requires exactly one bounded audit mode: local region, route, or mirror",
            json!("one of --center with --region-size, --route, or mirror region inputs"),
            json!({
                "center": &args.center,
                "region_size": &args.region_size,
                "route": &args.route,
                "left_region": &args.left_region,
                "right_region": &args.right_region,
                "mirror_axis": &args.mirror_axis
            }),
        ));
    }

    let mut params = Map::from_iter([
        ("sample_step".to_string(), json!(args.sample_step)),
        ("actor_radius".to_string(), json!(args.actor_radius)),
        ("provider_method".to_string(), json!(provider_method)),
        ("strict_warnings".to_string(), json!(args.strict_warnings)),
    ]);
    if !provider_id.is_empty() {
        params.insert("provider_id".to_string(), json!(provider_id));
    } else {
        params.insert("provider".to_string(), json!(provider));
    }
    if let Some(value) = args.min_spawn_clearance {
        params.insert("min_spawn_clearance".to_string(), json!(value));
    }
    if let Some(value) = args.min_corridor_width {
        params.insert("min_corridor_width".to_string(), json!(value));
    }
    if let Some(path) = &args.out {
        if path.as_os_str().is_empty() {
            return Err(invalid_surface_param(
                "surface audit requires --out to be non-empty",
                json!("JSON report output path"),
                json!(""),
            ));
        }
        params.insert("out".to_string(), json!(path));
    }
    if let Some(path) = &args.heatmap_out {
        if path.as_os_str().is_empty() {
            return Err(invalid_surface_param(
                "surface audit requires --heatmap-out to be non-empty",
                json!("heatmap output path"),
                json!(""),
            ));
        }
        let extension = path
            .extension()
            .and_then(|extension| extension.to_str())
            .unwrap_or("");
        if !extension.eq_ignore_ascii_case("png") {
            return Err(invalid_surface_param(
                "surface audit --heatmap-out must end in .png",
                json!("*.png output path"),
                json!(path),
            ));
        }
        params.insert("heatmap_out".to_string(), json!(path));
    }

    if let Some(center) = center {
        let (width, height) = parse_surface_region_size(region_size.unwrap())?;
        params.insert("mode".to_string(), json!("region"));
        params.insert("center".to_string(), json!(center));
        params.insert(
            "region_size".to_string(),
            json!({"width": width, "height": height}),
        );
    } else if let Some(route) = route {
        let route_points = parse_surface_route(route)?;
        params.insert("mode".to_string(), json!("route"));
        params.insert("route".to_string(), json!(route_points));
    } else {
        params.insert("mode".to_string(), json!("mirror"));
        params.insert("left_region".to_string(), json!(left_region.unwrap()));
        params.insert("right_region".to_string(), json!(right_region.unwrap()));
        params.insert("mirror_axis".to_string(), json!(mirror_axis.unwrap()));
    }
    Ok(params)
}

pub(crate) fn surface_audit_provider_methods() -> [&'static str; 4] {
    [
        "qa_surface_probe",
        "get_qa_surface_state",
        "get_surface_qa_state",
        "qa_probe_surface",
    ]
}

pub(crate) fn trimmed_optional(value: Option<&str>) -> Option<&str> {
    value.map(str::trim).filter(|value| !value.is_empty())
}

pub(crate) fn parse_surface_region_size(value: &str) -> Result<(f64, f64), FailureDetail> {
    let Some((width, height)) = value.split_once('x') else {
        return Err(invalid_surface_param(
            "surface audit requires --region-size in WIDTHxHEIGHT format",
            json!("WIDTHxHEIGHT with positive finite dimensions"),
            json!(value),
        ));
    };
    if height.contains('x') {
        return Err(invalid_surface_param(
            "surface audit requires --region-size in WIDTHxHEIGHT format",
            json!("WIDTHxHEIGHT with positive finite dimensions"),
            json!(value),
        ));
    }
    let parsed_width = width.trim().parse::<f64>().ok();
    let parsed_height = height.trim().parse::<f64>().ok();
    let Some(width) = parsed_width else {
        return Err(invalid_surface_param(
            "surface audit requires --region-size width to be numeric",
            json!("positive finite width"),
            json!(value),
        ));
    };
    let Some(height) = parsed_height else {
        return Err(invalid_surface_param(
            "surface audit requires --region-size height to be numeric",
            json!("positive finite height"),
            json!(value),
        ));
    };
    if !width.is_finite() || !height.is_finite() || width <= 0.0 || height <= 0.0 {
        return Err(invalid_surface_param(
            "surface audit requires --region-size dimensions to be positive and finite",
            json!("positive finite WIDTHxHEIGHT"),
            json!(value),
        ));
    }
    Ok((width, height))
}

pub(crate) fn parse_surface_route(value: &str) -> Result<Vec<String>, FailureDetail> {
    let points = value
        .split(',')
        .map(str::trim)
        .filter(|point| !point.is_empty())
        .map(ToString::to_string)
        .collect::<Vec<_>>();
    if points.len() < 2 || points.len() != value.split(',').count() {
        return Err(invalid_surface_param(
            "surface audit requires --route to contain at least two non-empty locators",
            json!("comma-separated route with two or more locators"),
            json!(value),
        ));
    }
    Ok(points)
}

pub(crate) fn validate_optional_positive_surface_float(
    flag: &str,
    value: Option<f64>,
) -> Result<(), FailureDetail> {
    if value.is_some_and(|value| !value.is_finite() || value <= 0.0) {
        return Err(invalid_surface_param(
            &format!("surface audit requires {flag} to be positive and finite"),
            json!("positive finite number"),
            numeric_actual(value.unwrap()),
        ));
    }
    Ok(())
}

pub(crate) fn invalid_surface_param(
    message: &str,
    expected: Value,
    actual: Value,
) -> FailureDetail {
    FailureDetail {
        kind: "invalid_params".to_string(),
        command: Some("surface audit".to_string()),
        message: Some(message.to_string()),
        expected: Some(expected),
        actual: Some(actual),
        ..FailureDetail::default()
    }
}

pub(crate) fn collision_report_state_methods() -> [&'static str; 5] {
    [
        "get_qa_collision_state",
        "get_collision_qa_state",
        "get_qa_physics_state",
        "get_physics_qa_state",
        "qa_collision_probe",
    ]
}

pub(crate) fn collision_report_default_state_methods() -> [&'static str; 1] {
    ["get_qa_collision_state"]
}

pub(crate) fn is_allowed_collision_report_state_method(method: &str) -> bool {
    collision_report_state_methods().contains(&method)
}

pub(crate) fn validate_optional_non_negative_float(
    command: &str,
    flag: &str,
    value: Option<f64>,
) -> Result<(), FailureDetail> {
    if value.is_some_and(|value| !value.is_finite() || value < 0.0) {
        return Err(FailureDetail {
            kind: "invalid_params".to_string(),
            command: Some(command.to_string()),
            message: Some(format!(
                "physics collision-report requires {flag} to be finite and non-negative"
            )),
            expected: Some(json!("finite non-negative number")),
            actual: Some(numeric_actual(value.unwrap())),
            ..FailureDetail::default()
        });
    }
    Ok(())
}

pub(crate) fn validate_optional_non_negative_i64(
    command: &str,
    flag: &str,
    value: Option<i64>,
) -> Result<(), FailureDetail> {
    if value.is_some_and(|value| value < 0) {
        return Err(FailureDetail {
            kind: "invalid_params".to_string(),
            command: Some(command.to_string()),
            message: Some(format!(
                "physics collision-report requires {flag} to be non-negative"
            )),
            expected: Some(json!("non-negative integer")),
            actual: Some(json!(value.unwrap())),
            ..FailureDetail::default()
        });
    }
    Ok(())
}

pub(crate) fn is_allowed_physics_state_method(method: &str) -> bool {
    if method.is_empty()
        || !method
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '_')
        || method.starts_with("qa_prepare_")
        || method.starts_with("qa_apply_")
        || method.starts_with("qa_mutate_")
        || method.starts_with("qa_set_")
    {
        return false;
    }
    if method == "qa_collision_probe" {
        return true;
    }
    (method.starts_with("get_qa_") && method.ends_with("_state"))
        || (method.starts_with("get_") && method.ends_with("_qa_state"))
        || (method.starts_with("qa_") && method.contains("diagnostic"))
}

pub(crate) fn invalid_params_result(command: &str, failure: FailureDetail) -> CommandResult {
    CommandResult {
        ok: false,
        command: command.to_string(),
        message: "live command failed".to_string(),
        data: Map::new(),
        failures: vec![failure],
        exit_code: 1,
    }
}

pub(crate) fn physics_collision_report_result(
    config: &ProjectConfig,
    args: &PhysicsCollisionReportArgs,
    trace_params: Map<String, Value>,
) -> CommandResult {
    let thresholds = collision_report::Thresholds {
        max_normal_retention: args.max_normal_retention,
        max_velocity_reversals: args.max_velocity_reversals,
        max_same_contact_frames: args.max_same_contact_frames,
        max_angular_velocity_spike: args.max_angular_velocity_spike,
        max_overlap_depth: args.max_overlap_depth,
        max_low_progress_contact_frames: args.max_low_progress_contact_frames,
    };
    if let Some(trace_path) = &args.from_trace {
        return collision_report::collision_report_from_trace_result(
            trace_path,
            args.target.as_deref(),
            thresholds,
            args.strict_warnings,
        );
    }

    let trace_result = runtime::trace_record_result(config, trace_params);
    if !trace_result.ok {
        let mut result = trace_result;
        result.command = "physics collision-report".to_string();
        return result;
    }
    let trace_path = trace_result
        .data
        .get("artifacts")
        .and_then(Value::as_object)
        .and_then(|artifacts| artifacts.get("trace"))
        .and_then(Value::as_str)
        .map(PathBuf::from);
    let Some(trace_path) = trace_path else {
        let actual_artifacts = trace_result.data.get("artifacts").cloned();
        return CommandResult {
            ok: false,
            command: "physics collision-report".to_string(),
            message: "collision report failed".to_string(),
            data: trace_result.data,
            failures: vec![FailureDetail {
                kind: "collision_report_trace_missing".to_string(),
                command: Some("physics collision-report".to_string()),
                message: Some("trace record result did not include a trace artifact".to_string()),
                expected: Some(json!("artifacts.trace path")),
                actual: Some(json!(actual_artifacts)),
                ..FailureDetail::default()
            }],
            exit_code: 1,
        };
    };
    collision_report::collision_report_from_trace_result(
        &trace_path,
        args.target.as_deref(),
        thresholds,
        args.strict_warnings,
    )
}
