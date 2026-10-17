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

pub(crate) fn import_result(cli: &Cli) -> CommandResult {
    let resolved_binary = godot_qa_process::resolve_godot_binary(cli.godot_bin.as_deref());
    let config = load_project_config(
        cli.project.as_deref().map(Path::new),
        cli.godot_bin.as_deref(),
        cli.artifact_dir.as_deref(),
        cli.verbose,
        resolved_binary,
    );
    let command = "import";
    let mut failures = Vec::new();
    if let Some(failure) = config.project_config_failure_for_command(command) {
        failures.push(failure);
    }
    if config.godot_binary.is_none() {
        failures.push(missing_godot_failure(command));
    }
    if !config.project_root.exists() {
        failures.push(missing_project_root_failure(command, &config.project_root));
    }
    if config.project_root.exists() {
        if let Some(failure) = ensure_writable_artifact_dir(&config.artifacts_dir, command) {
            failures.push(failure);
        }
    }
    if !failures.is_empty() {
        return CommandResult {
            ok: false,
            command: command.to_string(),
            message: "project import failed".to_string(),
            data: serde_json::Map::from_iter([(
                "project_root".to_string(),
                json!(config.project_root),
            )]),
            failures,
            exit_code: 1,
        };
    }

    let artifacts = prepare_import_artifacts(&config, command);
    let started = std::time::Instant::now();
    let completed = godot_qa_process::launch_godot_import_with_args(
        config.godot_binary.as_deref().unwrap_or_default(),
        &config.godot_launch_prefix,
        &config.godot_args,
        &config.project_root,
        &artifacts.logs_path,
    );

    let result = match completed {
        Ok(run) if run.returncode == 0 => CommandResult {
            ok: true,
            command: command.to_string(),
            message: "project import completed".to_string(),
            data: serde_json::Map::from_iter([
                ("project_root".to_string(), json!(config.project_root)),
                ("artifacts".to_string(), artifacts.to_json()),
                (
                    "duration_ms".to_string(),
                    json!(started.elapsed().as_millis() as u64),
                ),
            ]),
            failures: Vec::new(),
            exit_code: 0,
        },
        Ok(run) => CommandResult {
            ok: false,
            command: command.to_string(),
            message: "project import failed".to_string(),
            data: serde_json::Map::from_iter([
                ("project_root".to_string(), json!(config.project_root)),
                ("artifacts".to_string(), artifacts.to_json()),
                (
                    "duration_ms".to_string(),
                    json!(started.elapsed().as_millis() as u64),
                ),
            ]),
            failures: vec![FailureDetail {
                kind: "godot_import_failed".to_string(),
                command: Some(godot_qa_process::format_command(&run.command)),
                path: Some(artifacts.logs_path.to_string_lossy().into_owned()),
                expected: Some(json!("Godot import exits with status 0")),
                actual: Some(json!(run.returncode)),
                ..FailureDetail::default()
            }],
            exit_code: 1,
        },
        Err(failure) => CommandResult {
            ok: false,
            command: command.to_string(),
            message: "project import failed".to_string(),
            data: serde_json::Map::from_iter([
                ("project_root".to_string(), json!(config.project_root)),
                ("artifacts".to_string(), artifacts.to_json()),
            ]),
            failures: vec![failure],
            exit_code: 1,
        },
    };
    let result = append_config_warnings(result, &config);
    let _ = write_result_artifact(&artifacts.result_path, &result);
    result
}

#[derive(Debug)]
pub(crate) struct ImportArtifacts {
    root_dir: std::path::PathBuf,
    meta_path: std::path::PathBuf,
    result_path: std::path::PathBuf,
    logs_path: std::path::PathBuf,
}

impl ImportArtifacts {
    fn to_json(&self) -> serde_json::Value {
        json!({
            "root": self.root_dir,
            "meta": self.meta_path,
            "result": self.result_path,
            "logs": self.logs_path,
        })
    }
}

pub(crate) fn prepare_import_artifacts(config: &ProjectConfig, command: &str) -> ImportArtifacts {
    let root_dir = config.artifacts_dir.join("import").join(unique_run_id());
    let _ = std::fs::create_dir_all(&root_dir);
    let artifacts = ImportArtifacts {
        root_dir: root_dir.clone(),
        meta_path: root_dir.join("meta.json"),
        result_path: root_dir.join("result.json"),
        logs_path: root_dir.join("logs.txt"),
    };
    let _ = std::fs::write(
        &artifacts.meta_path,
        serde_json::to_string_pretty(&json!({
            "tool": "godot-qa",
            "project_root": config.project_root,
            "command": command,
        }))
        .unwrap_or_else(|_| "{}".to_string()),
    );
    artifacts
}

pub(crate) fn unique_run_id() -> String {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis().to_string())
        .unwrap_or_else(|_| "0".to_string())
}

pub(crate) fn write_result_artifact(path: &Path, result: &CommandResult) -> io::Result<()> {
    std::fs::write(
        path,
        serde_json::to_string_pretty(result).unwrap_or_else(|_| "{}".to_string()),
    )
}
