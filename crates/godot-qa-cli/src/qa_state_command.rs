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
    screenshots as scenario_screenshots, time_control, visual_audit, visual_isolate,
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

pub(crate) fn qa_state_validate_result(cli: &Cli, args: &QaStateValidateArgs) -> CommandResult {
    let request = qa_state::QaStateValidateRequest {
        schema: args.schema.clone(),
        input: args.input.clone(),
        trace: args.trace.clone(),
        state_path: args.state_path.clone(),
        target: args.target.clone(),
        method: args.method.clone(),
        provider: args.provider.clone(),
    };
    if let Some(provider) = &args.provider {
        let live_result = runtime::run_live_protocol_command(
            &runtime_config(cli),
            qa_state::COMMAND,
            runtime::QA_STATE_READ,
            serde_json::Map::from_iter([("provider".to_string(), json!(provider))]),
        );
        if !live_result.ok {
            return live_result;
        }
        let Some(state) = live_result.data.get("state").cloned() else {
            return CommandResult {
                ok: false,
                command: qa_state::COMMAND.to_string(),
                message: "QA state live read failed".to_string(),
                data: serde_json::Map::from_iter([
                    ("provider".to_string(), json!(provider)),
                    ("schema".to_string(), json!(args.schema)),
                ]),
                failures: vec![FailureDetail {
                    kind: "invalid_state_source".to_string(),
                    command: Some(qa_state::COMMAND.to_string()),
                    message: Some("qa_state.read response did not include data.state".to_string()),
                    expected: Some(json!("live response data.state")),
                    actual: Some(json!(live_result.data)),
                    target: Some(provider.clone()),
                    details: serde_json::Map::from_iter([(
                        "provider".to_string(),
                        json!(provider),
                    )]),
                    ..FailureDetail::default()
                }],
                exit_code: 1,
            };
        };
        return qa_state::qa_state_validate_value_result(request, state);
    }
    if let (Some(target), Some(method)) = (&args.target, &args.method) {
        let live_result = runtime::run_live_protocol_command(
            &runtime_config(cli),
            qa_state::COMMAND,
            runtime::QA_STATE_READ,
            serde_json::Map::from_iter([
                ("target".to_string(), json!(target)),
                ("method".to_string(), json!(method)),
            ]),
        );
        if !live_result.ok {
            return live_result;
        }
        let Some(state) = live_result.data.get("state").cloned() else {
            return CommandResult {
                ok: false,
                command: qa_state::COMMAND.to_string(),
                message: "QA state live read failed".to_string(),
                data: serde_json::Map::from_iter([
                    ("target".to_string(), json!(target)),
                    ("method".to_string(), json!(method)),
                    ("schema".to_string(), json!(args.schema)),
                ]),
                failures: vec![FailureDetail {
                    kind: "invalid_state_source".to_string(),
                    command: Some(qa_state::COMMAND.to_string()),
                    message: Some("qa_state.read response did not include data.state".to_string()),
                    expected: Some(json!("live response data.state")),
                    actual: Some(json!(live_result.data)),
                    target: Some(target.clone()),
                    details: serde_json::Map::from_iter([("method".to_string(), json!(method))]),
                    ..FailureDetail::default()
                }],
                exit_code: 1,
            };
        };
        return qa_state::qa_state_validate_value_result(request, state);
    }
    qa_state::qa_state_validate_result(request)
}
