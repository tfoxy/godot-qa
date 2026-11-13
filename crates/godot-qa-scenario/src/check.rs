use godot_qa_core::config::ProjectConfig;
use godot_qa_core::profile;
use godot_qa_core::results::{CommandResult, FailureDetail};
use serde_json::{Map, Value, json};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use crate::frame_pacing::FramePacing;

mod artifacts;
mod batch_deadline;
mod execution;
mod manifest;
mod result;

use artifacts::{
    artifact_write_failed, check_profile_artifact_json, check_profile_summary_json,
    persist_check_result, prepare_check_artifacts, required_artifact_write_failure,
    slow_check_summary_json, write_check_profile_artifact, write_json,
};
use execution::{
    check_log_diagnostics, ensure_check_prerequisites, run_manifest_test, run_manifest_tests_batch,
};
use manifest::{load_test_manifest, resolve_check_test_selection, selected_batch_opt_out_failures};

const CHECK_STARTUP_FRAMES: i64 = 5;
const CHECK_PROFILE_MAX_FRAME_SAMPLES: usize = profile::PROFILE_MAX_FRAME_SAMPLES;
const CHECK_PROFILE_TOP_SCRIPT_METHODS: usize = 10;
const TEST_MANIFEST_RELATIVE_PATH: &str = "qa/tests/manifest.json";
const DEFAULT_SLOW_SUMMARY_THRESHOLD_MS: u64 = 1_000;
const SLOW_SUMMARY_LIMIT: usize = 10;

#[derive(Debug, Clone)]
struct CheckArtifacts {
    run_id: String,
    root_dir: PathBuf,
    meta_path: PathBuf,
    result_path: PathBuf,
    progress_path: PathBuf,
    import_logs_path: PathBuf,
    tests_dir: PathBuf,
    batch_request_path: PathBuf,
    batch_logs_path: PathBuf,
    batch_summary_path: PathBuf,
    slow_summary_path: PathBuf,
    profile_path: PathBuf,
}

impl CheckArtifacts {
    fn to_json(&self) -> Value {
        json!({
            "root": self.root_dir,
            "meta": self.meta_path,
            "result": self.result_path,
            "progress": self.progress_path,
            "slow_summary": self.slow_summary_path,
        })
    }

    fn batch_json(&self) -> Value {
        json!({
            "request": self.batch_request_path,
            "logs": self.batch_logs_path,
            "summary": self.batch_summary_path,
            "progress": self.progress_path,
        })
    }
}

#[derive(Debug, Clone)]
struct CheckTestArtifacts {
    root_dir: PathBuf,
    meta_path: PathBuf,
    request_path: PathBuf,
    result_path: PathBuf,
    logs_path: PathBuf,
    progress_path: PathBuf,
}

impl CheckTestArtifacts {
    fn to_json(&self) -> Value {
        json!({
            "root": self.root_dir,
            "meta": self.meta_path,
            "request": self.request_path,
            "result": self.result_path,
            "logs": self.logs_path,
            "progress": self.progress_path,
        })
    }
}

#[derive(Debug, Clone, Copy)]
struct BatchEvidence<'a> {
    logs_path: &'a Path,
    summary_path: &'a Path,
}

#[derive(Debug, Clone)]
struct ManifestTestEntry {
    id: String,
    artifact_slug: String,
    kind: String,
    target: String,
    tags: Vec<String>,
    batchable: bool,
    startup_frames: Option<i64>,
    time_control: Option<Value>,
    frame_pacing: Option<FramePacing>,
    timeout: Option<Duration>,
    slow_summary_threshold_ms: u64,
}

#[derive(Debug, Clone)]
struct TestManifest {
    entries: Vec<ManifestTestEntry>,
    profiles: HashMap<String, ManifestProfile>,
    slow_summary_threshold_ms: u64,
}

#[derive(Debug, Clone)]
struct ManifestProfile {
    tags: Vec<String>,
    tests: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CheckTestSelectionRequest {
    pub test_ids: Vec<String>,
    pub tags: Vec<String>,
    pub profile: Option<String>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum CheckTestExecutionMode {
    #[default]
    Isolated,
    Batch,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CheckTestRunOptions {
    pub timeout_override: Option<Duration>,
    pub profile_enabled: bool,
}

#[derive(Debug, Clone)]
struct ResolvedCheckTestSelection {
    selected_indices: Vec<usize>,
    total_test_count: usize,
    selection: Value,
}

impl ResolvedCheckTestSelection {
    fn selected_test_count(&self) -> usize {
        self.selected_indices.len()
    }

    fn skipped_test_count(&self) -> usize {
        self.total_test_count
            .saturating_sub(self.selected_test_count())
    }
}

fn add_check_selection_metadata(
    data: &mut Map<String, Value>,
    selected_test_count: usize,
    skipped_test_count: usize,
    total_test_count: usize,
    selection: Value,
) {
    data.insert("checked_test_count".to_string(), json!(selected_test_count));
    data.insert(
        "selected_test_count".to_string(),
        json!(selected_test_count),
    );
    data.insert("skipped_test_count".to_string(), json!(skipped_test_count));
    data.insert("total_test_count".to_string(), json!(total_test_count));
    data.insert("selection".to_string(), selection);
}

pub fn run_check_tests(
    config: &ProjectConfig,
    command_line: &str,
    selection_request: CheckTestSelectionRequest,
    execution_mode: CheckTestExecutionMode,
    options: CheckTestRunOptions,
) -> CommandResult {
    let command = "check";
    let mut data = Map::from_iter([
        ("project_root".to_string(), json!(config.project_root)),
        ("mode".to_string(), json!("tests")),
    ]);
    let (failures, version) = ensure_check_prerequisites(config, command);
    if let Some(version) = version {
        data.insert(
            "godot".to_string(),
            json!({
                "binary": config.godot_binary,
                "version": version,
            }),
        );
    }
    if !failures.is_empty() {
        return CommandResult {
            ok: false,
            command: command.to_string(),
            message: "project check failed".to_string(),
            data,
            failures,
            exit_code: 1,
        };
    }

    let (manifest_path, test_manifest, manifest_failure) = load_test_manifest(config, command);
    if let Some(failure) = manifest_failure {
        return CommandResult {
            ok: false,
            command: command.to_string(),
            message: "project check failed".to_string(),
            data,
            failures: vec![failure],
            exit_code: 1,
        };
    }
    let test_manifest = test_manifest.unwrap_or(TestManifest {
        entries: Vec::new(),
        profiles: HashMap::new(),
        slow_summary_threshold_ms: DEFAULT_SLOW_SUMMARY_THRESHOLD_MS,
    });
    data.insert(
        "manifest".to_string(),
        json!(
            manifest_path
                .canonicalize()
                .unwrap_or(manifest_path.clone())
        ),
    );

    let selection = match resolve_check_test_selection(
        command,
        &manifest_path,
        &test_manifest,
        selection_request,
    ) {
        Ok(selection) => selection,
        Err(failure) => {
            add_check_selection_metadata(
                &mut data,
                0,
                0,
                test_manifest.entries.len(),
                json!({"mode": "invalid"}),
            );
            data.insert("tests".to_string(), json!([]));
            return CommandResult {
                ok: false,
                command: command.to_string(),
                message: "project check failed".to_string(),
                data,
                failures: vec![failure],
                exit_code: 1,
            };
        }
    };
    add_check_selection_metadata(
        &mut data,
        selection.selected_test_count(),
        selection.skipped_test_count(),
        selection.total_test_count,
        selection.selection.clone(),
    );

    if execution_mode == CheckTestExecutionMode::Batch {
        let batch_failures =
            selected_batch_opt_out_failures(command, &test_manifest, &selection.selected_indices);
        if !batch_failures.is_empty() {
            data.insert("checked_test_count".to_string(), json!(0));
            data.insert("tests".to_string(), json!([]));
            return CommandResult {
                ok: false,
                command: command.to_string(),
                message: "project check failed".to_string(),
                data,
                failures: batch_failures,
                exit_code: 1,
            };
        }
    }

    let artifacts =
        match prepare_check_artifacts(&config.artifacts_dir, &config.project_root, command_line) {
            Ok(artifacts) => artifacts,
            Err(error) => {
                return artifact_write_failed(
                    command,
                    &config.artifacts_dir,
                    &error.to_string(),
                    data,
                );
            }
        };

    if test_manifest.entries.is_empty() {
        let result = CommandResult {
            ok: true,
            command: command.to_string(),
            message: "project check passed".to_string(),
            data: Map::from_iter([
                ("project_root".to_string(), json!(config.project_root)),
                ("mode".to_string(), json!("tests")),
                (
                    "manifest".to_string(),
                    json!(manifest_path.canonicalize().unwrap_or(manifest_path)),
                ),
                ("artifacts".to_string(), artifacts.to_json()),
                (
                    "import".to_string(),
                    json!({
                        "ok": true,
                        "skipped": true,
                        "reason": "empty_manifest"
                    }),
                ),
                (
                    "checked_test_count".to_string(),
                    json!(selection.selected_test_count()),
                ),
                (
                    "selected_test_count".to_string(),
                    json!(selection.selected_test_count()),
                ),
                (
                    "skipped_test_count".to_string(),
                    json!(selection.skipped_test_count()),
                ),
                (
                    "total_test_count".to_string(),
                    json!(selection.total_test_count),
                ),
                ("selection".to_string(), selection.selection.clone()),
                ("tests".to_string(), json!([])),
            ]),
            failures: Vec::new(),
            exit_code: 0,
        };
        return persist_check_result(&artifacts, result);
    }

    let import_summary = json!({
        "ok": false,
        "artifacts": {
            "logs": artifacts.import_logs_path,
        }
    });

    let import_run = match godot_qa_process::launch_godot_import(
        config.godot_binary.as_deref().unwrap_or_default(),
        &config.godot_launch_prefix,
        config.godot_joypad_isolation,
        &config.project_root,
        &artifacts.import_logs_path,
    ) {
        Ok(run) if run.returncode == 0 => run,
        Ok(run) => {
            let result = CommandResult {
                ok: false,
                command: command.to_string(),
                message: "project check failed".to_string(),
                data: Map::from_iter([
                    ("project_root".to_string(), json!(config.project_root)),
                    ("mode".to_string(), json!("tests")),
                    (
                        "manifest".to_string(),
                        json!(manifest_path.canonicalize().unwrap_or(manifest_path)),
                    ),
                    ("artifacts".to_string(), artifacts.to_json()),
                    ("import".to_string(), import_summary),
                    (
                        "checked_test_count".to_string(),
                        json!(selection.selected_test_count()),
                    ),
                    (
                        "selected_test_count".to_string(),
                        json!(selection.selected_test_count()),
                    ),
                    (
                        "skipped_test_count".to_string(),
                        json!(selection.skipped_test_count()),
                    ),
                    (
                        "total_test_count".to_string(),
                        json!(selection.total_test_count),
                    ),
                    ("selection".to_string(), selection.selection.clone()),
                    ("tests".to_string(), json!([])),
                ]),
                failures: vec![FailureDetail {
                    kind: "godot_import_failed".to_string(),
                    command: Some(godot_qa_process::format_command(&run.command)),
                    path: Some(artifacts.import_logs_path.to_string_lossy().into_owned()),
                    expected: Some(json!("Godot import exits with status 0")),
                    actual: Some(json!(run.returncode)),
                    details: [("termination".to_string(), json!(run.termination))]
                        .into_iter()
                        .collect(),
                    ..FailureDetail::default()
                }],
                exit_code: 1,
            };
            return persist_check_result(&artifacts, result);
        }
        Err(failure) => {
            let result = CommandResult {
                ok: false,
                command: command.to_string(),
                message: "project check failed".to_string(),
                data: Map::from_iter([
                    ("project_root".to_string(), json!(config.project_root)),
                    ("mode".to_string(), json!("tests")),
                    (
                        "manifest".to_string(),
                        json!(manifest_path.canonicalize().unwrap_or(manifest_path)),
                    ),
                    ("artifacts".to_string(), artifacts.to_json()),
                    ("import".to_string(), import_summary),
                    (
                        "checked_test_count".to_string(),
                        json!(selection.selected_test_count()),
                    ),
                    (
                        "selected_test_count".to_string(),
                        json!(selection.selected_test_count()),
                    ),
                    (
                        "skipped_test_count".to_string(),
                        json!(selection.skipped_test_count()),
                    ),
                    (
                        "total_test_count".to_string(),
                        json!(selection.total_test_count),
                    ),
                    ("selection".to_string(), selection.selection.clone()),
                    ("tests".to_string(), json!([])),
                ]),
                failures: vec![failure],
                exit_code: 1,
            };
            return persist_check_result(&artifacts, result);
        }
    };

    let (mut import_failures, import_warnings, import_ignored_count) = check_log_diagnostics(
        &artifacts.import_logs_path,
        &import_run,
        &config.log_ignore_patterns,
        None,
        json!({"logs": artifacts.import_logs_path}),
    );
    for failure in &mut import_failures {
        failure.details.insert("phase".to_string(), json!("import"));
    }
    let import_summary = json!({"ok": import_failures.is_empty(), "warnings": import_warnings,
        "logs": {"ignored_count": import_ignored_count, "ignore_patterns_applied": config.log_ignore_patterns.len()},
        "artifacts": {"logs": artifacts.import_logs_path}});
    let (test_payloads, failures) = match execution_mode {
        CheckTestExecutionMode::Isolated => {
            let mut failures = Vec::new();
            let mut test_payloads = Vec::new();
            for index in &selection.selected_indices {
                let test_entry = &test_manifest.entries[*index];
                let (test_payload, test_failures) =
                    run_manifest_test(config, test_entry, &artifacts, command, options);
                test_payloads.push(test_payload);
                failures.extend(test_failures);
            }
            (test_payloads, failures)
        }
        CheckTestExecutionMode::Batch => run_manifest_tests_batch(
            config,
            &test_manifest,
            &selection.selected_indices,
            &artifacts,
            command,
            options,
        ),
    };

    let mut failures = failures;
    failures.extend(import_failures);
    let mut exit_code = if failures.is_empty() { 0 } else { 1 };
    let mut artifacts_json = artifacts.to_json();
    let slow_summary = slow_check_summary_json(
        &test_payloads,
        test_manifest.slow_summary_threshold_ms,
        Some(&test_manifest),
        &artifacts.slow_summary_path,
    );
    if let Err(error) = write_json(&artifacts.slow_summary_path, &slow_summary) {
        failures.push(required_artifact_write_failure(
            command,
            &artifacts.slow_summary_path,
            &error,
            &artifacts,
        ));
        exit_code = 1;
    }
    if execution_mode == CheckTestExecutionMode::Batch {
        if let Some(artifacts_object) = artifacts_json.as_object_mut() {
            artifacts_object.insert(
                "batch_summary".to_string(),
                json!(artifacts.batch_summary_path),
            );
            artifacts_object.insert(
                "batch_request".to_string(),
                json!(artifacts.batch_request_path),
            );
            artifacts_object.insert("batch_logs".to_string(), json!(artifacts.batch_logs_path));
        }
    }
    let mut profile_summary = None;
    if options.profile_enabled {
        if let Some(artifacts_object) = artifacts_json.as_object_mut() {
            artifacts_object.insert("profile".to_string(), json!(artifacts.profile_path));
        }
        let summary = check_profile_summary_json(&test_payloads, &artifacts);
        let artifact_started = Instant::now();
        let artifact = check_profile_artifact_json(
            config,
            &manifest_path,
            &selection.selection,
            execution_mode,
            &artifacts,
            &test_payloads,
        );
        if let Err(error) = write_check_profile_artifact(&artifacts.profile_path, &artifact) {
            failures.push(FailureDetail {
                kind: "artifact_write_failed".to_string(),
                command: Some(command.to_string()),
                path: Some(artifacts.profile_path.to_string_lossy().into_owned()),
                message: Some(error.to_string()),
                expected: Some(json!("profile.json written under the check artifact root")),
                actual: Some(json!("OSError")),
                ..FailureDetail::default()
            });
            exit_code = 1;
        }
        let mut summary = summary;
        if let Some(summary_object) = summary.as_object_mut() {
            summary_object.insert(
                "profile_artifact_write_ms".to_string(),
                json!(artifact_started.elapsed().as_millis() as u64),
            );
        }
        profile_summary = Some(summary);
    }
    let mut result_data = Map::from_iter([
        ("project_root".to_string(), json!(config.project_root)),
        ("mode".to_string(), json!("tests")),
        (
            "execution_mode".to_string(),
            json!(match execution_mode {
                CheckTestExecutionMode::Isolated => "isolated",
                CheckTestExecutionMode::Batch => "batch",
            }),
        ),
        (
            "manifest".to_string(),
            json!(manifest_path.canonicalize().unwrap_or(manifest_path)),
        ),
        ("artifacts".to_string(), artifacts_json),
        ("import".to_string(), import_summary),
        ("checked_test_count".to_string(), json!(test_payloads.len())),
        (
            "selected_test_count".to_string(),
            json!(selection.selected_test_count()),
        ),
        (
            "skipped_test_count".to_string(),
            json!(selection.skipped_test_count()),
        ),
        (
            "total_test_count".to_string(),
            json!(selection.total_test_count),
        ),
        ("selection".to_string(), selection.selection.clone()),
        ("tests".to_string(), Value::Array(test_payloads)),
        ("slow_summary".to_string(), slow_summary),
    ]);
    if execution_mode == CheckTestExecutionMode::Batch {
        result_data.insert(
            "batch".to_string(),
            json!({
                "artifacts": artifacts.batch_json(),
            }),
        );
    }
    if let Some(profile_summary) = profile_summary {
        result_data.insert("profile".to_string(), profile_summary);
    }
    let result = CommandResult {
        ok: failures.is_empty(),
        command: command.to_string(),
        message: if failures.is_empty() {
            "project check passed".to_string()
        } else {
            "project check failed".to_string()
        },
        data: result_data,
        failures,
        exit_code,
    };
    let _ = import_run;
    persist_check_result(&artifacts, result)
}

#[cfg(test)]
mod tests;
