use godot_qa_core::config::ProjectConfig;
use godot_qa_core::profile;
use godot_qa_core::results::{CommandResult, FailureDetail};
use serde_json::{Map, Value, json};
use std::collections::{HashMap, HashSet};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use crate::frame_pacing::{FramePacing, manifest_frame_pacing};
use crate::time_control::validate_time_control_for_manifest;

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

fn resolve_check_test_selection(
    command: &str,
    manifest_path: &Path,
    manifest: &TestManifest,
    request: CheckTestSelectionRequest,
) -> Result<ResolvedCheckTestSelection, FailureDetail> {
    let total_test_count = manifest.entries.len();
    let requested_tests = normalize_selection_values(
        command,
        manifest_path,
        request.test_ids,
        "check --tests --test values must be non-empty strings",
        false,
    )?;
    let requested_tags = normalize_selection_values(
        command,
        manifest_path,
        request.tags,
        "check --tests --tag values must be non-empty strings",
        true,
    )?;
    let requested_profile = request
        .profile
        .map(|profile| profile.trim().to_string())
        .filter(|profile| !profile.is_empty());

    if requested_tests.is_empty() && requested_tags.is_empty() && requested_profile.is_none() {
        return Ok(ResolvedCheckTestSelection {
            selected_indices: (0..total_test_count).collect(),
            total_test_count,
            selection: json!({
                "mode": "all",
                "tests": [],
                "tags": [],
                "profile": Value::Null,
                "selected_ids": manifest.entries.iter().map(|entry| entry.id.clone()).collect::<Vec<_>>(),
            }),
        });
    }

    let mut id_to_index = HashMap::new();
    let mut known_tags = HashSet::new();
    for (index, entry) in manifest.entries.iter().enumerate() {
        id_to_index.insert(entry.id.as_str(), index);
        for tag in &entry.tags {
            known_tags.insert(tag.as_str());
        }
    }

    let mut selected_indices = Vec::new();
    let mut selected_seen = HashSet::new();
    let mut profile_tags = Vec::new();
    let mut profile_tests = Vec::new();

    if let Some(profile_name) = requested_profile.as_deref() {
        let profile = manifest.profiles.get(profile_name).ok_or_else(|| {
            selection_failure(
                command,
                manifest_path,
                "unknown_profile",
                "check --tests profile must exist in qa/tests/manifest.json",
                json!("known profile name"),
                json!(profile_name),
                Some(profile_name.to_string()),
            )
        })?;
        profile_tags = profile.tags.clone();
        profile_tests = profile.tests.clone();
        let mut profile_seen = HashSet::new();
        for test_id in &profile_tests {
            let index = *id_to_index.get(test_id.as_str()).ok_or_else(|| {
                selection_failure(
                    command,
                    manifest_path,
                    "unknown_test",
                    "manifest profile expanded to an unknown test id",
                    json!("known test id"),
                    json!(test_id),
                    Some(profile_name.to_string()),
                )
            })?;
            if !profile_seen.insert(index) {
                return Err(selection_failure(
                    command,
                    manifest_path,
                    "ambiguous_selection",
                    "manifest profile selects the same test id more than once",
                    json!("unique profile-derived test ids"),
                    json!(test_id),
                    Some(profile_name.to_string()),
                ));
            }
            push_selected_index(&mut selected_indices, &mut selected_seen, index);
        }
        for tag in &profile_tags {
            let matching_indices = indices_for_tag(&manifest.entries, tag);
            for index in matching_indices {
                if !profile_seen.insert(index) {
                    return Err(selection_failure(
                        command,
                        manifest_path,
                        "ambiguous_selection",
                        "manifest profile tags and tests select the same test id",
                        json!("unique profile-derived test ids"),
                        json!(manifest.entries[index].id),
                        Some(profile_name.to_string()),
                    ));
                }
                push_selected_index(&mut selected_indices, &mut selected_seen, index);
            }
        }
    }

    for test_id in &requested_tests {
        let index = *id_to_index.get(test_id.as_str()).ok_or_else(|| {
            selection_failure(
                command,
                manifest_path,
                "unknown_test",
                "check --tests --test value must match a manifest test id",
                json!("known test id"),
                json!(test_id),
                Some(test_id.to_string()),
            )
        })?;
        push_selected_index(&mut selected_indices, &mut selected_seen, index);
    }

    for tag in &requested_tags {
        if !known_tags.contains(tag.as_str()) {
            return Err(selection_failure(
                command,
                manifest_path,
                "unknown_tag",
                "check --tests --tag value must match at least one manifest test tag",
                json!("known tag"),
                json!(tag),
                Some(tag.to_string()),
            ));
        }
        for index in indices_for_tag(&manifest.entries, tag) {
            push_selected_index(&mut selected_indices, &mut selected_seen, index);
        }
    }

    selected_indices.sort_unstable();
    if selected_indices.is_empty() {
        return Err(selection_failure(
            command,
            manifest_path,
            "empty_selection",
            "check --tests selection matched no manifest tests",
            json!("at least one selected test"),
            json!("empty_selection"),
            requested_profile.clone(),
        ));
    }
    let selected_ids = selected_indices
        .iter()
        .map(|index| manifest.entries[*index].id.clone())
        .collect::<Vec<_>>();
    Ok(ResolvedCheckTestSelection {
        selected_indices,
        total_test_count,
        selection: json!({
            "mode": "filtered",
            "tests": requested_tests,
            "tags": requested_tags,
            "profile": requested_profile,
            "profile_tests": profile_tests,
            "profile_tags": profile_tags,
            "selected_ids": selected_ids,
        }),
    })
}

fn normalize_selection_values(
    command: &str,
    manifest_path: &Path,
    values: Vec<String>,
    message: &str,
    reject_duplicates: bool,
) -> Result<Vec<String>, FailureDetail> {
    let mut normalized = Vec::new();
    let mut seen = HashSet::new();
    for value in values {
        let trimmed = value.trim();
        if trimmed.is_empty() {
            return Err(selection_failure(
                command,
                manifest_path,
                "invalid_selection",
                message,
                json!("non-empty string"),
                json!(value),
                None,
            ));
        }
        if reject_duplicates && !seen.insert(trimmed.to_string()) {
            return Err(selection_failure(
                command,
                manifest_path,
                "duplicate_tag",
                "check --tests --tag values must not contain duplicates",
                json!("unique tag filters"),
                json!(trimmed),
                Some(trimmed.to_string()),
            ));
        }
        if !reject_duplicates {
            seen.insert(trimmed.to_string());
        }
        normalized.push(trimmed.to_string());
    }
    if !reject_duplicates {
        normalized.sort();
        normalized.dedup();
    }
    Ok(normalized)
}

fn indices_for_tag(entries: &[ManifestTestEntry], tag: &str) -> Vec<usize> {
    entries
        .iter()
        .enumerate()
        .filter_map(|(index, entry)| {
            entry
                .tags
                .iter()
                .any(|entry_tag| entry_tag == tag)
                .then_some(index)
        })
        .collect()
}

fn selected_batch_opt_out_failures(
    command: &str,
    manifest: &TestManifest,
    selected_indices: &[usize],
) -> Vec<FailureDetail> {
    selected_indices
        .iter()
        .filter_map(|index| manifest.entries.get(*index))
        .filter(|entry| !entry.batchable)
        .map(|entry| FailureDetail {
            kind: "test_batch_opt_out".to_string(),
            command: Some(command.to_string()),
            target: Some(entry.id.clone()),
            message: Some("manifest test entry is not batchable".to_string()),
            expected: Some(json!("selected tests are batchable")),
            actual: Some(json!({"batch": false})),
            ..FailureDetail::default()
        })
        .collect()
}

fn push_selected_index(
    selected_indices: &mut Vec<usize>,
    selected_seen: &mut HashSet<usize>,
    index: usize,
) {
    if selected_seen.insert(index) {
        selected_indices.push(index);
    }
}

fn selection_failure(
    command: &str,
    manifest_path: &Path,
    actual_kind: &str,
    message: &str,
    expected: Value,
    actual: Value,
    target: Option<String>,
) -> FailureDetail {
    FailureDetail {
        kind: "test_selection_invalid".to_string(),
        command: Some(command.to_string()),
        path: Some(manifest_path.to_string_lossy().into_owned()),
        target,
        message: Some(message.to_string()),
        expected: Some(expected),
        actual: Some(json!(actual_kind)),
        details: Map::from_iter([("value".to_string(), actual)]),
        ..FailureDetail::default()
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
        let _ = write_result_artifact(&artifacts.result_path, &result);
        return result;
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
            let _ = write_result_artifact(&artifacts.result_path, &result);
            return result;
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
            let _ = write_result_artifact(&artifacts.result_path, &result);
            return result;
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
    let _ = write_json(&artifacts.slow_summary_path, &slow_summary);
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
    let _ = write_result_artifact(&artifacts.result_path, &result);
    result
}

fn run_manifest_test(
    config: &ProjectConfig,
    test_entry: &ManifestTestEntry,
    check_artifacts: &CheckArtifacts,
    command: &str,
    options: CheckTestRunOptions,
) -> (Value, Vec<FailureDetail>) {
    let artifacts = match prepare_test_artifacts(check_artifacts, test_entry) {
        Ok(artifacts) => artifacts,
        Err(error) => {
            let failure = FailureDetail {
                kind: "artifact_write_failed".to_string(),
                command: Some(command.to_string()),
                path: Some(check_artifacts.tests_dir.to_string_lossy().into_owned()),
                target: Some(test_entry.id.clone()),
                message: Some(error.to_string()),
                expected: Some(json!(
                    "artifact metadata written under the artifact directory"
                )),
                actual: Some(json!("OSError")),
                ..FailureDetail::default()
            };
            return (
                json!({
                    "id": test_entry.id,
                    "kind": test_entry.kind,
                    "target": test_entry.target,
                    "tags": test_entry.tags,
                    "ok": false,
                    "duration_ms": Value::Null,
                    "runtime_errors": [],
                    "artifacts": {},
                    "failures": [failure.clone()],
                }),
                vec![failure],
            );
        }
    };

    let request = test_check_request(
        test_entry,
        &artifacts,
        &check_artifacts.run_id,
        command,
        options,
    );
    if let Err(error) = write_json(&artifacts.request_path, &request) {
        let failure = FailureDetail {
            kind: "artifact_write_failed".to_string(),
            command: Some(command.to_string()),
            path: Some(artifacts.request_path.to_string_lossy().into_owned()),
            target: Some(test_entry.id.clone()),
            message: Some(error.to_string()),
            expected: Some(json!(
                "artifact metadata written under the artifact directory"
            )),
            actual: Some(json!("OSError")),
            artifacts: value_as_object_map(artifacts.to_json()),
            ..FailureDetail::default()
        };
        return (
            json!({
                "id": test_entry.id,
                "kind": test_entry.kind,
                "target": test_entry.target,
                "tags": test_entry.tags,
                "ok": false,
                "duration_ms": Value::Null,
                "runtime_errors": [],
                "artifacts": artifacts.to_json(),
                "failures": [failure.clone()],
            }),
            vec![failure],
        );
    }

    let started_at = started_at_now();
    let progress_started = Instant::now();
    let _ = append_check_progress_event(
        &check_artifacts.progress_path,
        Some(&artifacts.progress_path),
        test_entry,
        "start",
        &started_at,
        None,
        None,
        None,
        &artifacts,
    );

    let effective_timeout = options.timeout_override.or(test_entry.timeout);
    let process_result = match godot_qa_process::launch_godot_check_with_timeout_and_fixed_fps(
        config.godot_binary.as_deref().unwrap_or_default(),
        &config.godot_launch_prefix,
        &config.godot_args,
        &config.godot_project_args,
        &config.project_root,
        &artifacts.request_path,
        &artifacts.logs_path,
        effective_timeout,
        test_entry
            .frame_pacing
            .as_ref()
            .map(|frame_pacing| frame_pacing.fixed_fps),
    ) {
        Ok(run) => run,
        Err(failure) => {
            let normalized = normalized_failure(&test_entry.id, &artifacts, &failure);
            let _ = append_check_progress_event(
                &check_artifacts.progress_path,
                Some(&artifacts.progress_path),
                test_entry,
                "finish",
                &started_at,
                Some(progress_started.elapsed().as_millis() as u64),
                Some(false),
                Some(1),
                &artifacts,
            );
            return (
                json!({
                    "id": test_entry.id,
                    "kind": test_entry.kind,
                    "target": test_entry.target,
                    "tags": test_entry.tags,
                    "ok": false,
                    "duration_ms": Value::Null,
                    "runtime_errors": [],
                    "artifacts": artifacts.to_json(),
                    "failures": [normalized.clone()],
                }),
                vec![normalized],
            );
        }
    };

    if process_result.timed_out {
        let failure = check_test_timeout_failure(
            test_entry,
            &artifacts,
            &process_result,
            progress_started.elapsed().as_millis() as u64,
            effective_timeout,
            None,
        );
        let _ = append_check_progress_event(
            &check_artifacts.progress_path,
            Some(&artifacts.progress_path),
            test_entry,
            "finish",
            &started_at,
            Some(progress_started.elapsed().as_millis() as u64),
            Some(false),
            Some(1),
            &artifacts,
        );
        return (
            test_failure_payload(test_entry, artifacts.to_json(), &failure),
            vec![failure],
        );
    }

    let runtime_result = match load_runtime_test_result(test_entry, &artifacts, None) {
        Ok(result) => result,
        Err(failure) => {
            let _ = append_check_progress_event(
                &check_artifacts.progress_path,
                Some(&artifacts.progress_path),
                test_entry,
                "finish",
                &started_at,
                Some(progress_started.elapsed().as_millis() as u64),
                Some(false),
                Some(1),
                &artifacts,
            );
            return (
                json!({
                    "id": test_entry.id,
                    "kind": test_entry.kind,
                    "target": test_entry.target,
                    "tags": test_entry.tags,
                    "ok": false,
                    "duration_ms": Value::Null,
                    "runtime_errors": [],
                    "artifacts": artifacts.to_json(),
                    "failures": [failure.clone()],
                }),
                vec![failure],
            );
        }
    };

    let (mut payload, mut failures) = validate_runtime_test_result_with_evidence(
        test_entry,
        runtime_result,
        &artifacts,
        &process_result,
        None,
        options,
    );
    let (log_failures, log_warnings, ignored_count) = check_log_diagnostics(
        &artifacts.logs_path,
        &process_result,
        &config.log_ignore_patterns,
        Some(&test_entry.id),
        artifacts.to_json(),
    );
    failures.extend(log_failures);
    payload["ok"] = json!(payload["ok"] == true && failures.is_empty());
    payload["failures"] = json!(failures);
    payload["warnings"] = json!(log_warnings);
    payload["logs"] = json!({"ignored_count": ignored_count, "ignore_patterns_applied": config.log_ignore_patterns.len()});
    let _ = append_check_progress_event(
        &check_artifacts.progress_path,
        Some(&artifacts.progress_path),
        test_entry,
        "finish",
        &started_at,
        Some(progress_started.elapsed().as_millis() as u64),
        Some(failures.is_empty()),
        Some(failures.len()),
        &artifacts,
    );
    (payload, failures)
}

fn check_log_diagnostics(
    logs: &Path,
    process: &godot_qa_process::GodotRunResult,
    ignore_patterns: &[String],
    target: Option<&str>,
    artifacts: Value,
) -> (Vec<FailureDetail>, Vec<FailureDetail>, usize) {
    let scan = godot_qa_core::godot_logs::scan_output(
        logs,
        &process.stdout,
        &process.stderr,
        ignore_patterns,
    );
    let (warnings, failures) = scan
        .diagnostics("check", logs, target)
        .into_iter()
        .map(|mut diagnostic| {
            diagnostic.artifacts = value_as_object_map(artifacts.clone());
            diagnostic
        })
        .partition(|diagnostic| diagnostic.severity.as_deref() == Some("warning"));
    (failures, warnings, scan.ignored_count)
}

fn run_manifest_tests_batch(
    config: &ProjectConfig,
    manifest: &TestManifest,
    selected_indices: &[usize],
    check_artifacts: &CheckArtifacts,
    command: &str,
    options: CheckTestRunOptions,
) -> (Vec<Value>, Vec<FailureDetail>) {
    let pacing_groups = selected_batch_pacing_groups(manifest, selected_indices);
    if pacing_groups.len() > 1 {
        let mut payloads = Vec::new();
        let mut failures = Vec::new();
        for (group_index, (_, group_indices)) in pacing_groups.into_iter().enumerate() {
            let mut group_artifacts = check_artifacts.clone();
            group_artifacts.batch_request_path = check_artifacts
                .root_dir
                .join("batch")
                .join(format!("request-{group_index:02}.json"));
            group_artifacts.batch_logs_path = check_artifacts
                .root_dir
                .join("batch")
                .join(format!("logs-{group_index:02}.txt"));
            group_artifacts.batch_summary_path = check_artifacts
                .root_dir
                .join("batch")
                .join(format!("summary-{group_index:02}.json"));
            let (group_payloads, group_failures) = run_manifest_tests_batch(
                config,
                manifest,
                &group_indices,
                &group_artifacts,
                command,
                options,
            );
            payloads.extend(group_payloads);
            failures.extend(group_failures);
        }
        let _ = write_batch_summary_artifact(check_artifacts, &payloads, &failures, None, None);
        return (payloads, failures);
    }

    let mut prepared = Vec::new();
    let mut payloads = Vec::new();
    let mut failures = Vec::new();
    for index in selected_indices {
        let test_entry = &manifest.entries[*index];
        match prepare_test_artifacts(check_artifacts, test_entry) {
            Ok(artifacts) => {
                let request = test_check_request(
                    test_entry,
                    &artifacts,
                    &check_artifacts.run_id,
                    command,
                    options,
                );
                if let Err(error) = write_json(&artifacts.request_path, &request) {
                    let failure = FailureDetail {
                        kind: "artifact_write_failed".to_string(),
                        command: Some(command.to_string()),
                        path: Some(artifacts.request_path.to_string_lossy().into_owned()),
                        target: Some(test_entry.id.clone()),
                        message: Some(error.to_string()),
                        expected: Some(json!(
                            "artifact metadata written under the artifact directory"
                        )),
                        actual: Some(json!("OSError")),
                        artifacts: value_as_object_map(artifacts.to_json()),
                        ..FailureDetail::default()
                    };
                    payloads.push(test_failure_payload(
                        test_entry,
                        artifacts.to_json(),
                        &failure,
                    ));
                    failures.push(failure);
                } else {
                    prepared.push((test_entry, artifacts, request));
                }
            }
            Err(error) => {
                let failure = FailureDetail {
                    kind: "artifact_write_failed".to_string(),
                    command: Some(command.to_string()),
                    path: Some(check_artifacts.tests_dir.to_string_lossy().into_owned()),
                    target: Some(test_entry.id.clone()),
                    message: Some(error.to_string()),
                    expected: Some(json!(
                        "artifact metadata written under the artifact directory"
                    )),
                    actual: Some(json!("OSError")),
                    ..FailureDetail::default()
                };
                payloads.push(test_failure_payload(test_entry, json!({}), &failure));
                failures.push(failure);
            }
        }
    }

    if prepared.is_empty() {
        let _ = write_batch_summary_artifact(check_artifacts, &payloads, &failures, None, None);
        return (payloads, failures);
    }

    let batch_request = batch_check_request(&prepared, check_artifacts, command, options);
    if let Err(error) = write_json(&check_artifacts.batch_request_path, &batch_request) {
        let failure = FailureDetail {
            kind: "artifact_write_failed".to_string(),
            command: Some(command.to_string()),
            path: Some(
                check_artifacts
                    .batch_request_path
                    .to_string_lossy()
                    .into_owned(),
            ),
            message: Some(error.to_string()),
            expected: Some(json!("batch request written under the artifact directory")),
            actual: Some(json!("OSError")),
            artifacts: value_as_object_map(check_artifacts.batch_json()),
            ..FailureDetail::default()
        };
        for (test_entry, artifacts, _) in &prepared {
            let normalized = normalized_failure(&test_entry.id, artifacts, &failure);
            payloads.push(test_failure_payload(
                test_entry,
                artifacts.to_json(),
                &normalized,
            ));
            failures.push(normalized);
        }
        let _ = write_batch_summary_artifact(check_artifacts, &payloads, &failures, None, None);
        return (payloads, failures);
    }

    let effective_timeout = options.timeout_override.or_else(|| {
        prepared
            .iter()
            .filter_map(|(test_entry, _, _)| test_entry.timeout)
            .max()
    });
    let batch_started = Instant::now();
    let batch_fixed_fps = prepared
        .first()
        .and_then(|(test_entry, _, _)| test_entry.frame_pacing.as_ref())
        .map(|frame_pacing| frame_pacing.fixed_fps);
    let process_result = match godot_qa_process::launch_godot_check_with_timeout_and_fixed_fps(
        config.godot_binary.as_deref().unwrap_or_default(),
        &config.godot_launch_prefix,
        &config.godot_args,
        &config.godot_project_args,
        &config.project_root,
        &check_artifacts.batch_request_path,
        &check_artifacts.batch_logs_path,
        effective_timeout,
        batch_fixed_fps,
    ) {
        Ok(run) => run,
        Err(failure) => {
            for (test_entry, artifacts, _) in &prepared {
                let normalized = normalized_failure(&test_entry.id, artifacts, &failure);
                payloads.push(test_failure_payload(
                    test_entry,
                    artifacts.to_json(),
                    &normalized,
                ));
                failures.push(normalized);
            }
            let _ = write_batch_summary_artifact(check_artifacts, &payloads, &failures, None, None);
            return (payloads, failures);
        }
    };

    let batch_evidence = BatchEvidence {
        logs_path: &check_artifacts.batch_logs_path,
        summary_path: &check_artifacts.batch_summary_path,
    };
    if process_result.timed_out {
        if let Some((test_entry, artifacts, _)) = prepared.first() {
            let failure = check_test_timeout_failure(
                test_entry,
                artifacts,
                &process_result,
                batch_started.elapsed().as_millis() as u64,
                effective_timeout,
                Some(batch_evidence),
            );
            payloads.push(test_failure_payload(
                test_entry,
                artifacts.to_json(),
                &failure,
            ));
            failures.push(failure);
        }
        let _ = write_batch_summary_artifact(
            check_artifacts,
            &payloads,
            &failures,
            Some(&process_result),
            None,
        );
        return (payloads, failures);
    }
    let (log_failures, log_warnings, log_ignored_count) = check_log_diagnostics(
        &check_artifacts.batch_logs_path,
        &process_result,
        &config.log_ignore_patterns,
        None,
        check_artifacts.batch_json(),
    );
    failures.extend(log_failures);
    let batch_log_evidence = json!({
        "warnings": log_warnings,
        "ignored_count": log_ignored_count,
        "ignore_patterns_applied": config.log_ignore_patterns.len(),
    });
    let mut validation_process_result = process_result.clone();
    validation_process_result.returncode = 0;
    for (test_entry, artifacts, _) in &prepared {
        match load_runtime_test_result(test_entry, artifacts, Some(batch_evidence)) {
            Ok(runtime_result) => {
                let (payload, test_failures) = validate_runtime_test_result_with_evidence(
                    test_entry,
                    runtime_result,
                    artifacts,
                    &validation_process_result,
                    Some(batch_evidence),
                    options,
                );
                payloads.push(payload);
                failures.extend(test_failures);
            }
            Err(failure) => {
                payloads.push(test_failure_payload(
                    test_entry,
                    artifacts.to_json(),
                    &failure,
                ));
                failures.push(failure);
            }
        }
    }
    if process_result.returncode != 0 && failures.is_empty() {
        if let Some((test_entry, artifacts, _)) = prepared.first() {
            let failure = FailureDetail {
                kind: "godot_run_failed".to_string(),
                command: Some(godot_qa_process::format_command(&process_result.command)),
                path: Some(
                    check_artifacts
                        .batch_logs_path
                        .to_string_lossy()
                        .into_owned(),
                ),
                target: Some(test_entry.id.clone()),
                expected: Some(json!("Godot batch check exits with status 0")),
                actual: Some(json!(process_result.returncode)),
                artifacts: failure_artifacts(artifacts, Some(batch_evidence)),
                ..FailureDetail::default()
            };
            if let Some(payload) = payloads.first_mut()
                && let Some(payload_object) = payload.as_object_mut()
            {
                payload_object.insert("ok".to_string(), json!(false));
                payload_object.insert(
                    "failures".to_string(),
                    serde_json::to_value(vec![failure.clone()]).unwrap_or_else(|_| json!([])),
                );
            }
            failures.push(failure);
        }
    }
    let _ = write_batch_summary_artifact(
        check_artifacts,
        &payloads,
        &failures,
        Some(&process_result),
        Some(&batch_log_evidence),
    );
    (payloads, failures)
}

fn selected_batch_pacing_groups(
    manifest: &TestManifest,
    selected_indices: &[usize],
) -> Vec<(Option<i64>, Vec<usize>)> {
    let mut groups: Vec<(Option<i64>, Vec<usize>)> = Vec::new();
    for index in selected_indices {
        let key = manifest.entries[*index]
            .frame_pacing
            .as_ref()
            .map(|frame_pacing| frame_pacing.fixed_fps);
        if let Some((_, indices)) = groups.iter_mut().find(|(existing, _)| *existing == key) {
            indices.push(*index);
        } else {
            groups.push((key, vec![*index]));
        }
    }
    groups
}

fn batch_check_request(
    prepared: &[(&ManifestTestEntry, CheckTestArtifacts, Value)],
    check_artifacts: &CheckArtifacts,
    command: &str,
    options: CheckTestRunOptions,
) -> Value {
    let tests = prepared
        .iter()
        .map(|(test_entry, artifacts, request)| {
            let mut entry = Map::from_iter([
                ("id".to_string(), json!(test_entry.id)),
                ("kind".to_string(), json!(test_entry.kind)),
                ("target".to_string(), json!(test_entry.target)),
                ("tags".to_string(), json!(test_entry.tags)),
                ("check".to_string(), request["check"].clone()),
                ("artifacts".to_string(), artifacts.to_json()),
                ("request".to_string(), json!(artifacts.request_path)),
            ]);
            if options.profile_enabled {
                entry.insert(
                    "profiling".to_string(),
                    json!({
                        "enabled": true,
                        "max_frame_samples": CHECK_PROFILE_MAX_FRAME_SAMPLES,
                    }),
                );
            }
            Value::Object(entry)
        })
        .collect::<Vec<_>>();
    json!({
        "check": {
            "batch": true,
            "tests": tests,
        },
        "artifacts": {
            "root": check_artifacts.root_dir,
            "request": check_artifacts.batch_request_path,
            "logs": check_artifacts.batch_logs_path,
            "summary": check_artifacts.batch_summary_path,
            "progress": check_artifacts.progress_path,
        },
        "run": {
            "id": check_artifacts.run_id,
            "command": command,
            "execution_mode": "batch",
        }
    })
}

fn test_failure_payload(
    test_entry: &ManifestTestEntry,
    artifacts: Value,
    failure: &FailureDetail,
) -> Value {
    json!({
        "id": test_entry.id,
        "kind": test_entry.kind,
        "target": test_entry.target,
        "tags": test_entry.tags,
        "ok": false,
        "duration_ms": Value::Null,
        "runtime_errors": [],
        "artifacts": artifacts,
        "failures": [failure.clone()],
    })
}

fn write_batch_summary_artifact(
    check_artifacts: &CheckArtifacts,
    payloads: &[Value],
    failures: &[FailureDetail],
    process_result: Option<&godot_qa_process::GodotRunResult>,
    logs: Option<&Value>,
) -> std::io::Result<()> {
    let slow_summary = slow_check_summary_json(
        payloads,
        DEFAULT_SLOW_SUMMARY_THRESHOLD_MS,
        None,
        &check_artifacts.slow_summary_path,
    );
    let process = process_result.map(|run| {
        json!({
            "command": run.command,
            "returncode": run.returncode,
            "termination": run.termination,
            "timing": run.process_timing,
            "artifacts": {
                "logs": check_artifacts.batch_logs_path,
                "progress": check_artifacts.progress_path,
            }
        })
    });
    write_json(
        &check_artifacts.batch_summary_path,
        &json!({
            "ok": failures.is_empty(),
            "execution_mode": "batch",
            "test_count": payloads.len(),
            "tests": payloads,
            "failure_count": failures.len(),
            "failures": failures,
            "process": process,
            "logs": logs,
            "artifacts": check_artifacts.batch_json(),
            "slow_summary": slow_summary,
        }),
    )
}

fn slow_check_summary_json(
    payloads: &[Value],
    threshold_ms: u64,
    manifest: Option<&TestManifest>,
    artifact_path: &Path,
) -> Value {
    let test_thresholds = manifest
        .map(|manifest| {
            manifest
                .entries
                .iter()
                .map(|entry| (entry.id.as_str(), entry.slow_summary_threshold_ms))
                .collect::<HashMap<_, _>>()
        })
        .unwrap_or_default();
    let mut slow_tests = payloads
        .iter()
        .filter_map(|payload| {
            let duration_ms = payload.get("duration_ms").and_then(Value::as_u64)?;
            let entry_threshold_ms = payload
                .get("id")
                .and_then(Value::as_str)
                .and_then(|id| test_thresholds.get(id).copied())
                .unwrap_or(threshold_ms);
            if duration_ms < entry_threshold_ms {
                return None;
            }
            Some(json!({
                "id": payload.get("id").cloned().unwrap_or(Value::Null),
                "kind": payload.get("kind").cloned().unwrap_or(Value::Null),
                "target": payload.get("target").cloned().unwrap_or(Value::Null),
                "ok": payload.get("ok").cloned().unwrap_or(Value::Null),
                "duration_ms": duration_ms,
                "threshold_ms": entry_threshold_ms,
                "artifacts": payload.get("artifacts").cloned().unwrap_or_else(|| json!({})),
            }))
        })
        .collect::<Vec<_>>();
    slow_tests.sort_by(|left, right| {
        right
            .get("duration_ms")
            .and_then(Value::as_u64)
            .cmp(&left.get("duration_ms").and_then(Value::as_u64))
    });
    slow_tests.truncate(SLOW_SUMMARY_LIMIT);
    json!({
        "threshold_ms": threshold_ms,
        "limit": SLOW_SUMMARY_LIMIT,
        "slow_count": slow_tests.len(),
        "slow_tests": slow_tests,
        "artifacts": {
            "slow_summary": artifact_path,
        }
    })
}

fn check_profile_summary_json(payloads: &[Value], artifacts: &CheckArtifacts) -> Value {
    let mut frame_samples = Vec::new();
    let mut monitor_samples = Vec::new();
    let mut unsupported_monitors = Vec::new();
    let mut per_test_timings = Vec::new();
    let mut top_script_methods = Vec::new();
    for payload in payloads {
        let test_id = payload.get("id").cloned().unwrap_or(Value::Null);
        if let Some(profile) = payload.get("profile").and_then(Value::as_object) {
            frame_samples.extend(
                profile
                    .get("frame_times_ms")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .filter_map(Value::as_f64),
            );
            monitor_samples.extend(
                profile
                    .get("monitor_samples")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .cloned(),
            );
            unsupported_monitors.extend(
                profile
                    .get("unsupported_monitors")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .cloned(),
            );
            top_script_methods.push(profile::top_script_methods_or_empty(
                profile.get("top_script_methods"),
            ));
            for timing in profile
                .get("step_timings")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
            {
                let mut timing = timing.clone();
                if let Some(timing_object) = timing.as_object_mut() {
                    timing_object
                        .entry("test_id".to_string())
                        .or_insert(test_id.clone());
                }
                per_test_timings.push(timing);
            }
        }
        per_test_timings.push(json!({
            "test_id": test_id,
            "label": "check_test_total",
            "duration_ms": payload.get("duration_ms").cloned().unwrap_or(Value::Null),
        }));
    }
    let percentiles = profile::profile_percentiles(&frame_samples);
    let mut slow_tests = payloads
        .iter()
        .filter_map(|payload| {
            Some(json!({
                "id": payload.get("id")?.clone(),
                "ok": payload.get("ok").cloned().unwrap_or(Value::Null),
                "duration_ms": payload.get("duration_ms").cloned().unwrap_or(Value::Null),
            }))
        })
        .collect::<Vec<_>>();
    slow_tests.sort_by(|left, right| {
        right
            .get("duration_ms")
            .and_then(Value::as_u64)
            .cmp(&left.get("duration_ms").and_then(Value::as_u64))
    });
    slow_tests.truncate(SLOW_SUMMARY_LIMIT);
    json!({
        "frame_time_ms": profile::profile_percentiles_json(&percentiles),
        "frame_time_percentiles_ms": profile::profile_percentiles_json(&percentiles),
        "monitor_counters": profile::monitor_counter_summaries(&monitor_samples),
        "monitor_sample_count": monitor_samples.len(),
        "unsupported_monitors": unsupported_monitors,
        "top_script_methods": profile::aggregate_top_script_methods(&top_script_methods, CHECK_PROFILE_TOP_SCRIPT_METHODS),
        "sample_limits": {
            "max_frame_samples": CHECK_PROFILE_MAX_FRAME_SAMPLES,
            "top_script_methods": CHECK_PROFILE_TOP_SCRIPT_METHODS,
        },
        "slow_tests": slow_tests,
        "per_test_timings": per_test_timings.iter().take(CHECK_PROFILE_MAX_FRAME_SAMPLES).cloned().collect::<Vec<_>>(),
        "per_test_timing_count": per_test_timings.len(),
        "artifacts": {
            "profile": artifacts.profile_path,
        },
    })
}

fn check_profile_artifact_json(
    config: &ProjectConfig,
    manifest_path: &Path,
    selection: &Value,
    execution_mode: CheckTestExecutionMode,
    artifacts: &CheckArtifacts,
    payloads: &[Value],
) -> Value {
    let mut frame_samples = Vec::new();
    let mut monitor_samples = Vec::new();
    let mut unsupported_monitors = Vec::new();
    let mut per_test_timings = Vec::new();
    let mut top_script_methods = Vec::new();
    for payload in payloads {
        let test_id = payload.get("id").cloned().unwrap_or(Value::Null);
        if let Some(profile) = payload.get("profile").and_then(Value::as_object) {
            frame_samples.extend(
                profile
                    .get("frame_times_ms")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .filter_map(Value::as_f64),
            );
            monitor_samples.extend(
                profile
                    .get("monitor_samples")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .cloned(),
            );
            unsupported_monitors.extend(
                profile
                    .get("unsupported_monitors")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .cloned(),
            );
            top_script_methods.push(profile::top_script_methods_or_empty(
                profile.get("top_script_methods"),
            ));
            per_test_timings.extend(
                profile
                    .get("step_timings")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .cloned(),
            );
        }
        per_test_timings.push(json!({
            "test_id": test_id,
            "label": "check_test_total",
            "duration_ms": payload.get("duration_ms").cloned().unwrap_or(Value::Null),
        }));
    }
    let percentiles = profile::profile_percentiles(&frame_samples);
    json!({
        "project_root": config.project_root,
        "manifest": manifest_path,
        "selection": selection,
        "execution_mode": match execution_mode {
            CheckTestExecutionMode::Isolated => "isolated",
            CheckTestExecutionMode::Batch => "batch",
        },
        "artifacts": artifacts.to_json(),
        "sample_limits": {
            "max_frame_samples": CHECK_PROFILE_MAX_FRAME_SAMPLES,
            "top_script_methods": CHECK_PROFILE_TOP_SCRIPT_METHODS,
        },
        "frame_time_percentiles_ms": profile::profile_percentiles_json(&percentiles),
        "monitor_counters": profile::monitor_counter_summaries(&monitor_samples),
        "unsupported_monitors": unsupported_monitors,
        "top_script_methods": profile::aggregate_top_script_methods(&top_script_methods, CHECK_PROFILE_TOP_SCRIPT_METHODS),
        "per_test_timings": per_test_timings,
        "frame_times_ms": profile::bounded_frame_values(&frame_samples, CHECK_PROFILE_MAX_FRAME_SAMPLES),
        "monitor_samples": monitor_samples.into_iter().take(CHECK_PROFILE_MAX_FRAME_SAMPLES).collect::<Vec<_>>(),
        "tests": payloads.iter().map(|payload| json!({
            "id": payload.get("id").cloned().unwrap_or(Value::Null),
            "ok": payload.get("ok").cloned().unwrap_or(Value::Null),
            "duration_ms": payload.get("duration_ms").cloned().unwrap_or(Value::Null),
        })).collect::<Vec<_>>(),
    })
}

fn write_check_profile_artifact(path: &Path, value: &Value) -> std::io::Result<()> {
    write_json(path, value)
}

fn append_check_progress_event(
    run_progress_path: &Path,
    test_progress_path: Option<&Path>,
    test_entry: &ManifestTestEntry,
    event: &str,
    started_at: &str,
    elapsed_ms: Option<u64>,
    ok: Option<bool>,
    failure_count: Option<usize>,
    artifacts: &CheckTestArtifacts,
) -> std::io::Result<()> {
    let mut payload = Map::from_iter([
        ("event".to_string(), json!(event)),
        ("id".to_string(), json!(test_entry.id)),
        ("kind".to_string(), json!(test_entry.kind)),
        ("target".to_string(), json!(test_entry.target)),
        ("started_at".to_string(), json!(started_at)),
        ("elapsed_ms".to_string(), json!(elapsed_ms)),
        ("ok".to_string(), json!(ok)),
        ("failure_count".to_string(), json!(failure_count)),
        ("artifacts".to_string(), artifacts.to_json()),
    ]);
    if event == "start" {
        payload.insert("elapsed_ms".to_string(), Value::Null);
        payload.insert("ok".to_string(), Value::Null);
        payload.insert("failure_count".to_string(), Value::Null);
    }
    let line = serde_json::to_string(&Value::Object(payload)).map_err(std::io::Error::other)?;
    append_jsonl(run_progress_path, &line)?;
    if let Some(test_progress_path) = test_progress_path {
        append_jsonl(test_progress_path, &line)?;
    }
    Ok(())
}

fn append_jsonl(path: &Path, line: &str) -> std::io::Result<()> {
    let mut file = OpenOptions::new().create(true).append(true).open(path)?;
    writeln!(file, "{line}")?;
    Ok(())
}

fn ensure_check_prerequisites(
    config: &ProjectConfig,
    command: &str,
) -> (Vec<FailureDetail>, Option<String>) {
    let mut failures = Vec::new();
    let mut version = None;
    if let Some(failure) = config.project_config_failure_for_command(command) {
        failures.push(failure);
    }
    if config.godot_binary.is_none() {
        failures.push(FailureDetail {
            kind: "missing_godot".to_string(),
            command: Some(command.to_string()),
            message: Some("Godot binary not found in PATH or GODOT_BIN".to_string()),
            expected: Some(json!(
                "discoverable Godot binary via --godot-bin, GODOT_BIN, or PATH"
            )),
            actual: Some(json!("missing")),
            ..FailureDetail::default()
        });
    } else if let Some(binary) = config.godot_binary.as_deref()
        && config.project_config_failure.is_none()
    {
        match godot_qa_process::probe_godot_version(binary, &config.godot_launch_prefix) {
            Ok(found) => version = Some(found),
            Err(failure) => failures.push(failure),
        }
    }
    if !config.project_root.exists() {
        failures.push(FailureDetail {
            kind: "missing_project_root".to_string(),
            command: Some(command.to_string()),
            path: Some(config.project_root.to_string_lossy().into_owned()),
            message: Some("project root does not exist".to_string()),
            expected: Some(json!("existing project root")),
            actual: Some(json!("missing")),
            ..FailureDetail::default()
        });
    }
    if config.project_root.exists() {
        if let Some(failure) = ensure_writable_artifact_dir(&config.artifacts_dir, command) {
            failures.push(failure);
        }
    }
    if !config.addon_dir.join("plugin.cfg").exists() {
        failures.push(FailureDetail {
            kind: "missing_addon".to_string(),
            command: Some(command.to_string()),
            path: Some(config.addon_dir.to_string_lossy().into_owned()),
            message: Some(
                "godot_qa addon must be installed in the target project for check".to_string(),
            ),
            expected: Some(json!("installed addon in target project")),
            actual: Some(json!("missing")),
            ..FailureDetail::default()
        });
    }
    (failures, version)
}

fn load_test_manifest(
    config: &ProjectConfig,
    command: &str,
) -> (PathBuf, Option<TestManifest>, Option<FailureDetail>) {
    let manifest_path = config.project_root.join(TEST_MANIFEST_RELATIVE_PATH);
    if !manifest_path.exists() {
        return (
            manifest_path.clone(),
            None,
            Some(manifest_failure(
                command,
                &manifest_path,
                "test_manifest_missing",
                "qa/tests/manifest.json is required for check --tests",
                Some(json!("existing manifest file")),
                Some(json!("missing")),
                None,
            )),
        );
    }

    let raw = match fs::read_to_string(&manifest_path) {
        Ok(raw) => raw,
        Err(error) => {
            return (
                manifest_path.clone(),
                None,
                Some(manifest_failure(
                    command,
                    &manifest_path,
                    "test_manifest_invalid",
                    &error.to_string(),
                    Some(json!("readable manifest file")),
                    Some(json!("OSError")),
                    None,
                )),
            );
        }
    };
    let payload: Value = match serde_json::from_str(&raw) {
        Ok(value) => value,
        Err(error) => {
            return (
                manifest_path.clone(),
                None,
                Some(manifest_failure(
                    command,
                    &manifest_path,
                    "test_manifest_invalid",
                    "manifest.json must be valid JSON",
                    Some(json!("JSON object")),
                    Some(json!(error.to_string())),
                    None,
                )),
            );
        }
    };
    let Some(payload_object) = payload.as_object() else {
        return (
            manifest_path.clone(),
            None,
            Some(manifest_failure(
                command,
                &manifest_path,
                "test_manifest_invalid",
                "manifest.json root must be an object",
                Some(json!("JSON object")),
                Some(json!(python_type_name(&payload))),
                None,
            )),
        );
    };
    if payload_object.get("version").and_then(Value::as_i64) != Some(1) {
        return (
            manifest_path.clone(),
            None,
            Some(manifest_failure(
                command,
                &manifest_path,
                "test_manifest_invalid",
                "manifest version must be 1",
                Some(json!(1)),
                payload_object.get("version").cloned(),
                None,
            )),
        );
    }
    let raw_tests = payload_object.get("tests");
    let Some(raw_tests) = raw_tests else {
        return (
            manifest_path.clone(),
            None,
            Some(manifest_failure(
                command,
                &manifest_path,
                "test_manifest_invalid",
                "manifest tests must be an array",
                Some(json!("array")),
                Some(Value::Null),
                None,
            )),
        );
    };
    let Some(raw_tests) = raw_tests.as_array() else {
        return (
            manifest_path.clone(),
            None,
            Some(manifest_failure(
                command,
                &manifest_path,
                "test_manifest_invalid",
                "manifest tests must be an array",
                Some(json!("array")),
                Some(json!(python_type_name(raw_tests))),
                None,
            )),
        );
    };
    let default_timeout = match parse_manifest_timeout(
        command,
        &manifest_path,
        payload_object.get("timeout_seconds"),
        payload_object.get("timeout"),
        None,
    ) {
        Ok(timeout) => timeout,
        Err(failure) => return (manifest_path.clone(), None, Some(failure)),
    };
    let default_slow_summary_threshold_ms =
        match parse_slow_summary_threshold_ms(payload_object.get("slow_summary_threshold_ms")) {
            Ok(threshold) => threshold.unwrap_or(DEFAULT_SLOW_SUMMARY_THRESHOLD_MS),
            Err(failure) => return (manifest_path.clone(), None, Some(failure)),
        };

    let mut seen_ids = HashSet::new();
    let mut normalized = Vec::new();
    for (index, raw_entry) in raw_tests.iter().enumerate() {
        let Some(raw_entry) = raw_entry.as_object() else {
            return (
                manifest_path.clone(),
                None,
                Some(manifest_failure(
                    command,
                    &manifest_path,
                    "test_manifest_invalid",
                    "manifest test entries must be objects",
                    Some(json!("JSON object")),
                    Some(json!(python_type_name(raw_entry))),
                    Some(index.to_string()),
                )),
            );
        };
        let Some(test_id) = raw_entry.get("id").and_then(Value::as_str).map(str::trim) else {
            return (
                manifest_path.clone(),
                None,
                Some(manifest_failure(
                    command,
                    &manifest_path,
                    "test_manifest_invalid",
                    "manifest test id must be a non-empty string",
                    Some(json!("non-empty string")),
                    raw_entry.get("id").cloned(),
                    None,
                )),
            );
        };
        if test_id.is_empty() {
            return (
                manifest_path.clone(),
                None,
                Some(manifest_failure(
                    command,
                    &manifest_path,
                    "test_manifest_invalid",
                    "manifest test id must be a non-empty string",
                    Some(json!("non-empty string")),
                    raw_entry.get("id").cloned(),
                    None,
                )),
            );
        }
        if !seen_ids.insert(test_id.to_string()) {
            return (
                manifest_path.clone(),
                None,
                Some(manifest_failure(
                    command,
                    &manifest_path,
                    "test_manifest_invalid",
                    "manifest test ids must be unique",
                    Some(json!("unique test id")),
                    Some(json!("duplicate")),
                    Some(test_id.to_string()),
                )),
            );
        }
        let kind = raw_entry
            .get("kind")
            .and_then(Value::as_str)
            .unwrap_or_default();
        if kind != "scene" && kind != "script" {
            return (
                manifest_path.clone(),
                None,
                Some(manifest_failure(
                    command,
                    &manifest_path,
                    "test_manifest_invalid",
                    "manifest test kind must be scene or script",
                    Some(json!(["scene", "script"])),
                    raw_entry.get("kind").cloned(),
                    Some(test_id.to_string()),
                )),
            );
        }
        let Some(target) = raw_entry.get("target").and_then(Value::as_str) else {
            return (
                manifest_path.clone(),
                None,
                Some(manifest_failure(
                    command,
                    &manifest_path,
                    "test_manifest_invalid",
                    "manifest test target must be a non-empty string",
                    Some(json!("non-empty string")),
                    raw_entry.get("target").cloned(),
                    Some(test_id.to_string()),
                )),
            );
        };
        if target.is_empty() {
            return (
                manifest_path.clone(),
                None,
                Some(manifest_failure(
                    command,
                    &manifest_path,
                    "test_manifest_invalid",
                    "manifest test target must be a non-empty string",
                    Some(json!("non-empty string")),
                    raw_entry.get("target").cloned(),
                    Some(test_id.to_string()),
                )),
            );
        }

        let startup_frames = raw_entry.get("startup_frames").cloned();
        let time_control = match raw_entry.get("time_control") {
            Some(time_control) => match validate_time_control_for_manifest(time_control) {
                Ok(settings) => Some(settings.to_normalized_value()),
                Err(mut failure) => {
                    failure.command = Some(command.to_string());
                    failure.path = Some(manifest_path.to_string_lossy().into_owned());
                    failure.target = Some(test_id.to_string());
                    return (manifest_path.clone(), None, Some(failure));
                }
            },
            None => None,
        };
        let frame_pacing = match manifest_frame_pacing(
            raw_entry.get("fixed_fps"),
            time_control.is_some(),
            &config.project_root,
            &manifest_path,
            test_id,
        ) {
            Ok(frame_pacing) => frame_pacing,
            Err(mut failure) => {
                failure.command = Some(command.to_string());
                return (manifest_path.clone(), None, Some(failure));
            }
        };
        let timeout = match parse_manifest_timeout(
            command,
            &manifest_path,
            raw_entry.get("timeout_seconds"),
            raw_entry.get("timeout"),
            Some(test_id.to_string()),
        ) {
            Ok(timeout) => timeout.or(default_timeout),
            Err(failure) => return (manifest_path.clone(), None, Some(failure)),
        };
        let slow_summary_threshold_ms =
            match parse_slow_summary_threshold_ms(raw_entry.get("slow_summary_threshold_ms")) {
                Ok(threshold) => threshold.unwrap_or(default_slow_summary_threshold_ms),
                Err(mut failure) => {
                    failure.target = Some(test_id.to_string());
                    return (manifest_path.clone(), None, Some(failure));
                }
            };
        let tags =
            match parse_manifest_tags(command, &manifest_path, raw_entry.get("tags"), test_id) {
                Ok(tags) => tags,
                Err(failure) => return (manifest_path.clone(), None, Some(failure)),
            };
        let normalized_entry = ManifestTestEntry {
            id: test_id.to_string(),
            kind: kind.to_string(),
            target: target.to_string(),
            tags,
            batchable: match raw_entry.get("batch") {
                Some(Value::Bool(batchable)) => *batchable,
                Some(raw_batch) => {
                    return (
                        manifest_path.clone(),
                        None,
                        Some(manifest_failure(
                            command,
                            &manifest_path,
                            "test_manifest_invalid",
                            "manifest test batch must be a boolean",
                            Some(json!("boolean")),
                            Some(raw_batch.clone()),
                            Some(test_id.to_string()),
                        )),
                    );
                }
                None => true,
            },
            startup_frames: startup_frames.as_ref().and_then(Value::as_i64),
            time_control,
            frame_pacing,
            timeout,
            slow_summary_threshold_ms,
        };
        normalized.push((normalized_entry, startup_frames));
    }

    let mut entries = Vec::new();
    for (mut entry, raw_startup_frames) in normalized {
        if let Some(failure) = validate_project_relative_target(
            command,
            &manifest_path,
            &config.project_root,
            &entry.id,
            &entry.target,
        ) {
            return (manifest_path.clone(), None, Some(failure));
        }
        if entry.kind == "scene" {
            let startup_frames = raw_startup_frames.unwrap_or_else(|| json!(CHECK_STARTUP_FRAMES));
            let Some(startup_frames) = startup_frames.as_i64() else {
                return (
                    manifest_path.clone(),
                    None,
                    Some(manifest_failure(
                        command,
                        &manifest_path,
                        "test_manifest_invalid",
                        "scene test startup_frames must be a non-negative integer",
                        Some(json!("non-negative integer")),
                        Some(startup_frames),
                        Some(entry.id.clone()),
                    )),
                );
            };
            if startup_frames < 0 {
                return (
                    manifest_path.clone(),
                    None,
                    Some(manifest_failure(
                        command,
                        &manifest_path,
                        "test_manifest_invalid",
                        "scene test startup_frames must be a non-negative integer",
                        Some(json!("non-negative integer")),
                        Some(json!(startup_frames)),
                        Some(entry.id.clone()),
                    )),
                );
            }
            entry.startup_frames = Some(startup_frames);
        } else {
            entry.startup_frames = None;
        }
        entries.push(entry);
    }

    let profiles = match parse_manifest_profiles(
        command,
        &manifest_path,
        payload_object.get("profiles"),
        &entries,
    ) {
        Ok(profiles) => profiles,
        Err(failure) => return (manifest_path.clone(), None, Some(failure)),
    };

    (
        manifest_path,
        Some(TestManifest {
            entries,
            profiles,
            slow_summary_threshold_ms: default_slow_summary_threshold_ms,
        }),
        None,
    )
}

fn parse_slow_summary_threshold_ms(raw: Option<&Value>) -> Result<Option<u64>, FailureDetail> {
    let Some(raw) = raw else {
        return Ok(None);
    };
    let Some(value) = raw.as_u64() else {
        return Err(FailureDetail {
            kind: "test_manifest_invalid".to_string(),
            command: Some("check".to_string()),
            message: Some("slow_summary_threshold_ms must be a non-negative integer".to_string()),
            expected: Some(json!("non-negative integer milliseconds")),
            actual: Some(raw.clone()),
            ..FailureDetail::default()
        });
    };
    Ok(Some(value))
}

fn parse_manifest_tags(
    command: &str,
    manifest_path: &Path,
    raw_tags: Option<&Value>,
    test_id: &str,
) -> Result<Vec<String>, FailureDetail> {
    let Some(raw_tags) = raw_tags else {
        return Ok(Vec::new());
    };
    let Some(raw_tags) = raw_tags.as_array() else {
        return Err(manifest_failure(
            command,
            manifest_path,
            "test_manifest_invalid",
            "manifest test tags must be an array of non-empty strings",
            Some(json!("array of non-empty strings")),
            Some(json!(python_type_name(raw_tags))),
            Some(test_id.to_string()),
        ));
    };
    parse_string_array(
        command,
        manifest_path,
        raw_tags,
        Some(test_id.to_string()),
        "manifest test tags must be an array of non-empty strings",
        true,
    )
}

fn parse_manifest_timeout(
    command: &str,
    manifest_path: &Path,
    raw_timeout_seconds: Option<&Value>,
    raw_timeout: Option<&Value>,
    target: Option<String>,
) -> Result<Option<Duration>, FailureDetail> {
    if raw_timeout_seconds.is_some() && raw_timeout.is_some() {
        return Err(manifest_failure(
            command,
            manifest_path,
            "test_manifest_invalid",
            "manifest timeout must use either timeout_seconds or timeout",
            Some(json!("one timeout field")),
            Some(json!("multiple timeout fields")),
            target,
        ));
    }
    if let Some(value) = raw_timeout_seconds {
        let Some(seconds) = value.as_f64() else {
            return Err(invalid_timeout_failure(
                command,
                manifest_path,
                value.clone(),
                target,
            ));
        };
        if !seconds.is_finite() || seconds <= 0.0 {
            return Err(invalid_timeout_failure(
                command,
                manifest_path,
                value.clone(),
                target,
            ));
        }
        return Ok(Some(Duration::from_secs_f64(seconds)));
    }
    if let Some(value) = raw_timeout {
        let Some(raw) = value.as_str().map(str::trim) else {
            return Err(invalid_timeout_failure(
                command,
                manifest_path,
                value.clone(),
                target,
            ));
        };
        let (number, multiplier) = if let Some(number) = raw.strip_suffix("ms") {
            (number, 0.001)
        } else if let Some(number) = raw.strip_suffix('s') {
            (number, 1.0)
        } else if let Some(number) = raw.strip_suffix('m') {
            (number, 60.0)
        } else {
            return Err(invalid_timeout_failure(
                command,
                manifest_path,
                value.clone(),
                target,
            ));
        };
        let Ok(amount) = number.trim().parse::<f64>() else {
            return Err(invalid_timeout_failure(
                command,
                manifest_path,
                value.clone(),
                target,
            ));
        };
        let seconds = amount * multiplier;
        if !seconds.is_finite() || seconds <= 0.0 {
            return Err(invalid_timeout_failure(
                command,
                manifest_path,
                value.clone(),
                target,
            ));
        }
        return Ok(Some(Duration::from_secs_f64(seconds)));
    }
    Ok(None)
}

fn invalid_timeout_failure(
    command: &str,
    manifest_path: &Path,
    actual: Value,
    target: Option<String>,
) -> FailureDetail {
    manifest_failure(
        command,
        manifest_path,
        "test_manifest_invalid",
        "manifest timeout must be a positive finite duration",
        Some(json!(
            "positive number of seconds or duration string ending in ms, s, or m"
        )),
        Some(actual),
        target,
    )
}

fn parse_manifest_profiles(
    command: &str,
    manifest_path: &Path,
    raw_profiles: Option<&Value>,
    entries: &[ManifestTestEntry],
) -> Result<HashMap<String, ManifestProfile>, FailureDetail> {
    let Some(raw_profiles) = raw_profiles else {
        return Ok(HashMap::new());
    };
    let Some(raw_profiles) = raw_profiles.as_object() else {
        return Err(manifest_failure(
            command,
            manifest_path,
            "test_manifest_invalid",
            "manifest profiles must be an object",
            Some(json!("object")),
            Some(json!(python_type_name(raw_profiles))),
            None,
        ));
    };

    let test_ids: HashSet<&str> = entries.iter().map(|entry| entry.id.as_str()).collect();
    let mut profiles = HashMap::new();
    for (raw_name, raw_profile) in raw_profiles {
        let profile_name = raw_name.trim();
        if profile_name.is_empty() || profile_name == "all" || !is_safe_profile_name(profile_name) {
            return Err(manifest_failure(
                command,
                manifest_path,
                "test_manifest_invalid",
                "manifest profile names must be non-empty safe names and must not be reserved",
                Some(json!(
                    "profile name using letters, numbers, dot, underscore, or hyphen"
                )),
                Some(json!(raw_name)),
                Some(raw_name.to_string()),
            ));
        }
        let Some(profile_object) = raw_profile.as_object() else {
            return Err(manifest_failure(
                command,
                manifest_path,
                "test_manifest_invalid",
                "manifest profile definitions must be objects",
                Some(json!("object with tags and/or tests arrays")),
                Some(json!(python_type_name(raw_profile))),
                Some(profile_name.to_string()),
            ));
        };
        let tags = match profile_object.get("tags") {
            Some(Value::Array(tags)) => parse_string_array(
                command,
                manifest_path,
                tags,
                Some(profile_name.to_string()),
                "manifest profile tags must be an array of non-empty strings",
                false,
            )?,
            Some(value) => {
                return Err(manifest_failure(
                    command,
                    manifest_path,
                    "test_manifest_invalid",
                    "manifest profile tags must be an array of non-empty strings",
                    Some(json!("array of non-empty strings")),
                    Some(json!(python_type_name(value))),
                    Some(profile_name.to_string()),
                ));
            }
            None => Vec::new(),
        };
        let tests = match profile_object.get("tests") {
            Some(Value::Array(tests)) => parse_string_array(
                command,
                manifest_path,
                tests,
                Some(profile_name.to_string()),
                "manifest profile tests must be an array of known test ids",
                false,
            )?,
            Some(value) => {
                return Err(manifest_failure(
                    command,
                    manifest_path,
                    "test_manifest_invalid",
                    "manifest profile tests must be an array of known test ids",
                    Some(json!("array of known test ids")),
                    Some(json!(python_type_name(value))),
                    Some(profile_name.to_string()),
                ));
            }
            None => Vec::new(),
        };
        for test in &tests {
            if !test_ids.contains(test.as_str()) {
                return Err(manifest_failure(
                    command,
                    manifest_path,
                    "test_manifest_invalid",
                    "manifest profile tests must reference known test ids",
                    Some(json!("known test id")),
                    Some(json!(test)),
                    Some(profile_name.to_string()),
                ));
            }
        }
        profiles.insert(profile_name.to_string(), ManifestProfile { tags, tests });
    }
    Ok(profiles)
}

fn parse_string_array(
    command: &str,
    manifest_path: &Path,
    values: &[Value],
    target: Option<String>,
    message: &str,
    reject_reserved_tags: bool,
) -> Result<Vec<String>, FailureDetail> {
    let mut seen = HashSet::new();
    let mut normalized = Vec::new();
    for value in values {
        let Some(raw_value) = value.as_str() else {
            return Err(manifest_failure(
                command,
                manifest_path,
                "test_manifest_invalid",
                message,
                Some(json!("array of non-empty strings")),
                Some(json!(python_type_name(value))),
                target.clone(),
            ));
        };
        let trimmed = raw_value.trim();
        if trimmed.is_empty() {
            return Err(manifest_failure(
                command,
                manifest_path,
                "test_manifest_invalid",
                message,
                Some(json!("non-empty string")),
                Some(json!(raw_value)),
                target.clone(),
            ));
        }
        if reject_reserved_tags && is_reserved_manifest_tag(trimmed) {
            return Err(manifest_failure(
                command,
                manifest_path,
                "test_manifest_invalid",
                "manifest test tags must not use reserved names",
                Some(json!("non-reserved tag name")),
                Some(json!(trimmed)),
                target.clone(),
            ));
        }
        if !seen.insert(trimmed.to_string()) {
            return Err(manifest_failure(
                command,
                manifest_path,
                "test_manifest_invalid",
                "manifest string arrays must not contain duplicate values",
                Some(json!("unique values")),
                Some(json!(trimmed)),
                target.clone(),
            ));
        }
        normalized.push(trimmed.to_string());
    }
    Ok(normalized)
}

fn is_reserved_manifest_tag(tag: &str) -> bool {
    matches!(tag, "all" | "full")
}

fn is_safe_profile_name(name: &str) -> bool {
    name.chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
}

fn validate_project_relative_target(
    command: &str,
    manifest_path: &Path,
    project_root: &Path,
    test_id: &str,
    target: &str,
) -> Option<FailureDetail> {
    if !target.starts_with("res://") {
        return Some(manifest_failure(
            command,
            manifest_path,
            "test_manifest_invalid",
            "manifest test target must be a res:// path",
            Some(json!("res:// path under the selected project root")),
            Some(json!(target)),
            Some(test_id.to_string()),
        ));
    }
    let target_path = project_root.join(target.trim_start_matches("res://"));
    let resolved_target = normalize_existing_or_join(&target_path);
    let resolved_root = normalize_existing_or_join(project_root);
    if !resolved_target.starts_with(&resolved_root) {
        return Some(manifest_failure(
            command,
            manifest_path,
            "test_manifest_invalid",
            "manifest test target must stay within the selected project root",
            Some(json!("path under the selected project root")),
            Some(json!("outside_project")),
            Some(test_id.to_string()),
        ));
    }
    if !resolved_target.exists() {
        return Some(manifest_failure(
            command,
            manifest_path,
            "test_manifest_invalid",
            "manifest test target must exist",
            Some(json!("existing file under the selected project root")),
            Some(json!("missing")),
            Some(test_id.to_string()),
        ));
    }
    if !resolved_target.is_file() {
        return Some(manifest_failure(
            command,
            manifest_path,
            "test_manifest_invalid",
            "manifest test target must resolve to a file",
            Some(json!("existing file under the selected project root")),
            Some(json!("not_file")),
            Some(test_id.to_string()),
        ));
    }
    None
}

fn manifest_failure(
    command: &str,
    manifest_path: &Path,
    failure_type: &str,
    message: &str,
    expected: Option<Value>,
    actual: Option<Value>,
    target: Option<String>,
) -> FailureDetail {
    FailureDetail {
        kind: failure_type.to_string(),
        command: Some(command.to_string()),
        path: Some(manifest_path.to_string_lossy().into_owned()),
        target,
        message: Some(message.to_string()),
        expected,
        actual,
        ..FailureDetail::default()
    }
}

fn prepare_check_artifacts(
    artifacts_root: &Path,
    project_root: &Path,
    command_line: &str,
) -> std::io::Result<CheckArtifacts> {
    let check_root = artifacts_root.join("check");
    fs::create_dir_all(&check_root)?;
    let base_run_id = unique_run_id();
    let mut candidate_run_id = base_run_id.clone();
    let mut suffix = 1;
    let root_dir = loop {
        let candidate = check_root.join(&candidate_run_id);
        match fs::create_dir(&candidate) {
            Ok(()) => break candidate,
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                candidate_run_id = format!("{base_run_id}-{suffix:02}");
                suffix += 1;
            }
            Err(error) => return Err(error),
        }
    };
    let import_dir = root_dir.join("import");
    let tests_dir = root_dir.join("tests");
    let batch_dir = root_dir.join("batch");
    fs::create_dir_all(&import_dir)?;
    fs::create_dir_all(&tests_dir)?;
    fs::create_dir_all(&batch_dir)?;
    let artifacts = CheckArtifacts {
        run_id: root_dir
            .file_name()
            .and_then(|value| value.to_str())
            .unwrap_or_default()
            .to_string(),
        root_dir: root_dir.clone(),
        meta_path: root_dir.join("meta.json"),
        result_path: root_dir.join("result.json"),
        progress_path: root_dir.join("progress.jsonl"),
        import_logs_path: import_dir.join("logs.txt"),
        tests_dir,
        batch_request_path: batch_dir.join("request.json"),
        batch_logs_path: batch_dir.join("logs.txt"),
        batch_summary_path: batch_dir.join("summary.json"),
        slow_summary_path: root_dir.join("slow-summary.json"),
        profile_path: root_dir.join("profile.json"),
    };
    write_json(
        &artifacts.meta_path,
        &json!({
            "tool": "godot-qa",
            "project_root": project_root,
            "command": command_line,
            "started_at": started_at_now(),
        }),
    )?;
    Ok(artifacts)
}

fn prepare_test_artifacts(
    check_artifacts: &CheckArtifacts,
    test_entry: &ManifestTestEntry,
) -> std::io::Result<CheckTestArtifacts> {
    let root_dir = check_artifacts.tests_dir.join(slug_value(&test_entry.id));
    fs::create_dir_all(&root_dir)?;
    let artifacts = CheckTestArtifacts {
        root_dir: root_dir.clone(),
        meta_path: root_dir.join("meta.json"),
        request_path: root_dir.join("request.json"),
        result_path: root_dir.join("result.json"),
        logs_path: root_dir.join("logs.txt"),
        progress_path: root_dir.join("progress.jsonl"),
    };
    let mut meta = json!({
        "id": test_entry.id,
        "kind": test_entry.kind,
        "target": test_entry.target,
        "tags": test_entry.tags,
    });
    if let Some(frame_pacing) = &test_entry.frame_pacing
        && let Some(meta) = meta.as_object_mut()
    {
        meta.insert("frame_pacing".to_string(), frame_pacing.to_json());
    }
    write_json(&artifacts.meta_path, &meta)?;
    Ok(artifacts)
}

fn test_check_request(
    test_entry: &ManifestTestEntry,
    artifacts: &CheckTestArtifacts,
    run_id: &str,
    command: &str,
    options: CheckTestRunOptions,
) -> Value {
    let mut payload = Map::from_iter([
        ("id".to_string(), json!(test_entry.id)),
        ("kind".to_string(), json!(test_entry.kind)),
        ("target".to_string(), json!(test_entry.target)),
        ("tags".to_string(), json!(test_entry.tags)),
    ]);
    if let Some(startup_frames) = test_entry.startup_frames {
        payload.insert("startup_frames".to_string(), json!(startup_frames));
    }
    if let Some(time_control) = &test_entry.time_control {
        payload.insert("time_control".to_string(), time_control.clone());
    }
    if let Some(frame_pacing) = &test_entry.frame_pacing {
        payload.insert("frame_pacing".to_string(), frame_pacing.to_json());
    }
    let mut request = Map::from_iter([
        ("check".to_string(), Value::Object(payload)),
        ("artifacts".to_string(), artifacts.to_json()),
        (
            "run".to_string(),
            json!({
                "id": run_id,
                "command": command,
            }),
        ),
    ]);
    if options.profile_enabled {
        request.insert(
            "profiling".to_string(),
            json!({
                "enabled": true,
                "max_frame_samples": CHECK_PROFILE_MAX_FRAME_SAMPLES,
            }),
        );
    }
    Value::Object(request)
}

fn load_runtime_test_result(
    test_entry: &ManifestTestEntry,
    artifacts: &CheckTestArtifacts,
    batch_evidence: Option<BatchEvidence<'_>>,
) -> Result<Value, FailureDetail> {
    let raw = fs::read_to_string(&artifacts.result_path).map_err(|_| FailureDetail {
        kind: "check_result_invalid".to_string(),
        command: Some("check".to_string()),
        path: Some(artifacts.result_path.to_string_lossy().into_owned()),
        target: Some(test_entry.id.clone()),
        message: Some("check result.json was not written".to_string()),
        expected: Some(json!("JSON result written by qa_runner")),
        actual: Some(json!("missing")),
        artifacts: failure_artifacts(artifacts, batch_evidence),
        ..FailureDetail::default()
    })?;
    let value: Value = serde_json::from_str(&raw).map_err(|error| FailureDetail {
        kind: "check_result_invalid".to_string(),
        command: Some("check".to_string()),
        path: Some(artifacts.result_path.to_string_lossy().into_owned()),
        target: Some(test_entry.id.clone()),
        message: Some("check result.json was not valid JSON".to_string()),
        expected: Some(json!("valid JSON object")),
        actual: Some(json!(error.to_string())),
        artifacts: failure_artifacts(artifacts, batch_evidence),
        ..FailureDetail::default()
    })?;
    if !value.is_object() {
        return Err(FailureDetail {
            kind: "check_result_invalid".to_string(),
            command: Some("check".to_string()),
            path: Some(artifacts.result_path.to_string_lossy().into_owned()),
            target: Some(test_entry.id.clone()),
            message: Some("check result.json root must be an object".to_string()),
            expected: Some(json!("JSON object")),
            actual: Some(json!(python_type_name(&value))),
            artifacts: failure_artifacts(artifacts, batch_evidence),
            ..FailureDetail::default()
        });
    }
    Ok(value)
}

#[cfg(test)]
fn validate_runtime_test_result(
    test_entry: &ManifestTestEntry,
    runtime_result: Value,
    artifacts: &CheckTestArtifacts,
    process_result: &godot_qa_process::GodotRunResult,
) -> (Value, Vec<FailureDetail>) {
    validate_runtime_test_result_with_evidence(
        test_entry,
        runtime_result,
        artifacts,
        process_result,
        None,
        CheckTestRunOptions::default(),
    )
}

fn validate_runtime_test_result_with_evidence(
    test_entry: &ManifestTestEntry,
    runtime_result: Value,
    artifacts: &CheckTestArtifacts,
    process_result: &godot_qa_process::GodotRunResult,
    batch_evidence: Option<BatchEvidence<'_>>,
    options: CheckTestRunOptions,
) -> (Value, Vec<FailureDetail>) {
    let mut hard_failures = Vec::new();
    let object = runtime_result.as_object().cloned().unwrap_or_default();
    for (field_name, expected) in [
        ("id", Value::String(test_entry.id.clone())),
        ("kind", Value::String(test_entry.kind.clone())),
        ("target", Value::String(test_entry.target.clone())),
    ] {
        let Some(value) = object.get(field_name) else {
            hard_failures.push(check_result_invalid_failure(
                artifacts,
                &test_entry.id,
                &format!("check result field '{field_name}' must be a string"),
                json!("string"),
                json!("NoneType"),
                batch_evidence,
            ));
            continue;
        };
        let Some(value_string) = value.as_str() else {
            hard_failures.push(check_result_invalid_failure(
                artifacts,
                &test_entry.id,
                &format!("check result field '{field_name}' must be a string"),
                json!("string"),
                json!(python_type_name(value)),
                batch_evidence,
            ));
            continue;
        };
        if value_string != expected.as_str().unwrap_or_default() {
            hard_failures.push(check_result_invalid_failure(
                artifacts,
                &test_entry.id,
                &format!("check result field '{field_name}' must match the requested test"),
                expected,
                json!(value_string),
                batch_evidence,
            ));
        }
    }
    let ok = match object.get("ok").and_then(Value::as_bool) {
        Some(ok) => ok,
        None => {
            hard_failures.push(check_result_invalid_failure(
                artifacts,
                &test_entry.id,
                "check result field 'ok' must be a boolean",
                json!("boolean"),
                json!(python_type_name(object.get("ok").unwrap_or(&Value::Null))),
                batch_evidence,
            ));
            false
        }
    };
    let duration_ms = match object.get("duration_ms").and_then(Value::as_i64) {
        Some(duration_ms) if duration_ms >= 0 => json!(duration_ms),
        _ => {
            hard_failures.push(check_result_invalid_failure(
                artifacts,
                &test_entry.id,
                "check result field 'duration_ms' must be a non-negative integer",
                json!("non-negative integer"),
                object.get("duration_ms").cloned().unwrap_or(Value::Null),
                batch_evidence,
            ));
            Value::Null
        }
    };
    let runtime_errors = object.get("runtime_errors").cloned().unwrap_or(Value::Null);
    if !runtime_errors.is_array()
        || runtime_errors
            .as_array()
            .is_some_and(|entries| entries.iter().any(|entry| !entry.is_object()))
    {
        hard_failures.push(check_result_invalid_failure(
            artifacts,
            &test_entry.id,
            "check result field 'runtime_errors' must be an array of objects",
            json!("JSON array of objects"),
            json!(python_type_name(&runtime_errors)),
            batch_evidence,
        ));
    }
    let failure_values = object.get("failures").cloned().unwrap_or(Value::Null);
    if !failure_values.is_array()
        || failure_values
            .as_array()
            .is_some_and(|entries| entries.iter().any(|entry| !entry.is_object()))
    {
        hard_failures.push(check_result_invalid_failure(
            artifacts,
            &test_entry.id,
            "check result field 'failures' must be an array of objects",
            json!("JSON array of objects"),
            json!(python_type_name(&failure_values)),
            batch_evidence,
        ));
    }
    let data = object.get("data").cloned();
    if data.as_ref().is_some_and(|value| !value.is_object()) {
        hard_failures.push(check_result_invalid_failure(
            artifacts,
            &test_entry.id,
            "check result field 'data' must be an object when present",
            json!("JSON object"),
            json!(python_type_name(data.as_ref().unwrap_or(&Value::Null))),
            batch_evidence,
        ));
    }
    let mut profile_payload = None;
    if options.profile_enabled {
        match validate_check_profile_payload(
            test_entry,
            artifacts,
            object.get("profile"),
            batch_evidence,
        ) {
            Ok(profile) => profile_payload = Some(profile),
            Err(failure) => hard_failures.push(failure),
        }
    }
    if !hard_failures.is_empty() {
        return (
            json!({
                "id": test_entry.id,
                "kind": test_entry.kind,
                "target": test_entry.target,
                "tags": test_entry.tags,
                "ok": false,
                "duration_ms": Value::Null,
                "runtime_errors": [],
                "artifacts": artifacts.to_json(),
                "failures": hard_failures.clone(),
            }),
            hard_failures,
        );
    }

    let mut failures = failure_values
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_object)
        .map(|failure| normalized_failure_from_value(&test_entry.id, artifacts, failure))
        .collect::<Vec<_>>();
    if process_result.returncode != 0 && failures.is_empty() {
        let mut failure_artifacts = value_as_object_map(artifacts.to_json()).unwrap_or_default();
        if process_result.timed_out {
            failure_artifacts.insert(
                "process".to_string(),
                json!({
                    "timed_out": process_result.timed_out,
                    "timeout_ms": process_result.timeout_ms,
                    "cleanup": process_result.cleanup.as_ref().map(|cleanup| json!({
                        "status": cleanup.status,
                        "failure_kind": cleanup.failure_kind,
                        "pid": cleanup.pid,
                        "process_group": cleanup.process_group,
                        "owned": cleanup.owned,
                        "message": cleanup.message,
                    })),
                    "infrastructure_failure_kind": process_result.infrastructure_failure_kind,
                }),
            );
        }
        failures.push(FailureDetail {
            kind: if process_result.timed_out {
                "owned_process_timeout".to_string()
            } else {
                "godot_run_failed".to_string()
            },
            command: Some(godot_qa_process::format_command(&process_result.command)),
            path: Some(artifacts.logs_path.to_string_lossy().into_owned()),
            target: Some(test_entry.id.clone()),
            expected: Some(json!("Godot check exits with status 0")),
            actual: Some(if process_result.timed_out {
                json!({
                    "timed_out": true,
                    "timeout_ms": process_result.timeout_ms,
                    "exit_code": process_result.returncode,
                })
            } else {
                json!(process_result.returncode)
            }),
            details: [("termination".to_string(), json!(process_result.termination))]
                .into_iter()
                .collect(),
            artifacts: Some(failure_artifacts),
            ..FailureDetail::default()
        });
    }

    let mut result = Map::from_iter([
        ("id".to_string(), json!(test_entry.id)),
        ("kind".to_string(), json!(test_entry.kind)),
        ("target".to_string(), json!(test_entry.target)),
        ("tags".to_string(), json!(test_entry.tags)),
        ("ok".to_string(), json!(ok && failures.is_empty())),
        ("duration_ms".to_string(), duration_ms),
        (
            "display_server".to_string(),
            object.get("display_server").cloned().unwrap_or(Value::Null),
        ),
        ("runtime_errors".to_string(), runtime_errors.clone()),
        ("artifacts".to_string(), artifacts.to_json()),
        (
            "failures".to_string(),
            serde_json::to_value(&failures).unwrap_or_else(|_| json!([])),
        ),
    ]);
    if let Some(data) = data {
        result.insert("data".to_string(), data);
    }
    if let Some(profile_payload) = profile_payload {
        result.insert("profile".to_string(), profile_payload);
    }
    if let Some(time_control) = object.get("time_control") {
        result.insert("time_control".to_string(), time_control.clone());
    }
    if let Some(frame_pacing) = &test_entry.frame_pacing {
        result.insert("frame_pacing".to_string(), frame_pacing.to_json());
    }
    (Value::Object(result), failures)
}

fn normalized_failure(
    test_id: &str,
    artifacts: &CheckTestArtifacts,
    failure: &FailureDetail,
) -> FailureDetail {
    let mut normalized = failure.clone();
    normalized.command = Some("check".to_string());
    normalized.target = Some(test_id.to_string());
    normalized.artifacts = value_as_object_map(artifacts.to_json());
    normalized
}

fn validate_check_profile_payload(
    test_entry: &ManifestTestEntry,
    artifacts: &CheckTestArtifacts,
    profile_value: Option<&Value>,
    batch_evidence: Option<BatchEvidence<'_>>,
) -> Result<Value, FailureDetail> {
    let Some(profile_value) = profile_value else {
        return Err(profile_result_invalid_failure(
            artifacts,
            &test_entry.id,
            "profile payload is required when check profiling is enabled",
            json!("result.json profile object"),
            json!("missing"),
            batch_evidence,
        ));
    };
    let Some(profile_object) = profile_value.as_object() else {
        return Err(profile_result_invalid_failure(
            artifacts,
            &test_entry.id,
            "profile payload must be an object",
            json!("result.json profile object"),
            json!(python_type_name(profile_value)),
            batch_evidence,
        ));
    };
    let frame_times_ms = profile::optional_number_array(profile_object.get("frame_times_ms"))
        .map_err(|actual| {
            profile_result_invalid_failure(
                artifacts,
                &test_entry.id,
                "profile.frame_times_ms must be an array of non-negative numbers",
                json!("array of non-negative numbers"),
                actual,
                batch_evidence,
            )
        })?;
    let monitor_samples = profile::optional_object_array(profile_object.get("monitor_samples"))
        .map_err(|actual| {
            profile_result_invalid_failure(
                artifacts,
                &test_entry.id,
                "profile.monitor_samples must be an array of objects",
                json!("array of objects"),
                actual,
                batch_evidence,
            )
        })?;
    let step_timings =
        profile::optional_object_array(profile_object.get("step_timings")).map_err(|actual| {
            profile_result_invalid_failure(
                artifacts,
                &test_entry.id,
                "profile.step_timings must be an array of objects",
                json!("array of objects"),
                actual,
                batch_evidence,
            )
        })?;
    Ok(json!({
        "frame_times_ms": frame_times_ms,
        "monitor_samples": monitor_samples,
        "step_timings": step_timings,
        "top_script_methods": profile::top_script_methods_or_empty(profile_object.get("top_script_methods")),
        "unsupported_monitors": profile_object.get("unsupported_monitors").cloned().unwrap_or_else(|| json!([])),
        "sample_limits": profile_object.get("sample_limits").cloned().unwrap_or_else(|| json!({
            "max_frame_samples": CHECK_PROFILE_MAX_FRAME_SAMPLES,
            "top_script_methods": CHECK_PROFILE_TOP_SCRIPT_METHODS,
        })),
    }))
}

fn profile_result_invalid_failure(
    artifacts: &CheckTestArtifacts,
    test_id: &str,
    message: &str,
    expected: Value,
    actual: Value,
    batch_evidence: Option<BatchEvidence<'_>>,
) -> FailureDetail {
    let mut artifact_map = failure_artifacts(artifacts, batch_evidence).unwrap_or_default();
    artifact_map.insert("profile".to_string(), json!(artifacts.result_path));
    FailureDetail {
        kind: "profile_result_invalid".to_string(),
        command: Some("check".to_string()),
        path: Some(artifacts.result_path.to_string_lossy().into_owned()),
        target: Some(test_id.to_string()),
        message: Some(message.to_string()),
        expected: Some(expected),
        actual: Some(actual),
        artifacts: Some(artifact_map),
        ..FailureDetail::default()
    }
}

fn normalized_failure_from_value(
    test_id: &str,
    artifacts: &CheckTestArtifacts,
    failure: &Map<String, Value>,
) -> FailureDetail {
    let mut merged_artifacts = value_as_object_map(artifacts.to_json()).unwrap_or_default();
    if let Some(extra_artifacts) = failure.get("artifacts").and_then(Value::as_object) {
        for (key, value) in extra_artifacts {
            merged_artifacts.insert(key.clone(), value.clone());
        }
    }
    FailureDetail {
        kind: failure
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or("runtime_error")
            .to_string(),
        command: Some("check".to_string()),
        path: failure
            .get("path")
            .and_then(Value::as_str)
            .map(str::to_string),
        target: failure
            .get("target")
            .and_then(Value::as_str)
            .map(str::to_string)
            .or_else(|| Some(test_id.to_string())),
        message: failure
            .get("message")
            .and_then(Value::as_str)
            .map(str::to_string),
        expected: failure.get("expected").cloned(),
        actual: failure.get("actual").cloned(),
        severity: failure
            .get("severity")
            .and_then(Value::as_str)
            .map(str::to_string),
        artifacts: Some(merged_artifacts),
        ..FailureDetail::default()
    }
}

fn check_test_timeout_failure(
    test_entry: &ManifestTestEntry,
    artifacts: &CheckTestArtifacts,
    process_result: &godot_qa_process::GodotRunResult,
    elapsed_ms: u64,
    configured_timeout: Option<Duration>,
    batch_evidence: Option<BatchEvidence<'_>>,
) -> FailureDetail {
    let mut artifact_map = failure_artifacts(artifacts, batch_evidence).unwrap_or_default();
    artifact_map.insert(
        "process".to_string(),
        json!({
            "timed_out": true,
            "timeout_ms": process_result.timeout_ms.or_else(|| configured_timeout.map(duration_millis_u64)),
            "elapsed_ms": elapsed_ms,
            "cleanup": process_result.cleanup.as_ref().map(|cleanup| json!({
                "status": cleanup.status,
                "failure_kind": cleanup.failure_kind,
                "pid": cleanup.pid,
                "process_group": cleanup.process_group,
                "owned": cleanup.owned,
                "message": cleanup.message,
            })),
            "infrastructure_failure_kind": process_result.infrastructure_failure_kind,
        }),
    );
    artifact_map.insert("progress".to_string(), json!(artifacts.progress_path));
    FailureDetail {
        kind: "check_test_timeout".to_string(),
        command: Some(godot_qa_process::format_command(&process_result.command)),
        path: Some(artifacts.logs_path.to_string_lossy().into_owned()),
        target: Some(test_entry.id.clone()),
        message: Some("Godot check test exceeded the configured timeout".to_string()),
        expected: Some(json!("Godot check exits with status 0 before timeout")),
        actual: Some(json!({
            "state": "timed_out",
            "elapsed_ms": elapsed_ms,
            "timeout_ms": process_result.timeout_ms.or_else(|| configured_timeout.map(duration_millis_u64)),
            "exit_code": process_result.returncode,
            "active_test": active_test_json(test_entry),
            "progress": artifacts.progress_path,
            "cleanup": process_result.cleanup.as_ref().map(|cleanup| json!({
                "status": cleanup.status,
                "failure_kind": cleanup.failure_kind,
                "pid": cleanup.pid,
                "process_group": cleanup.process_group,
                "owned": cleanup.owned,
                "message": cleanup.message,
            })),
        })),
        artifacts: Some(artifact_map),
        ..FailureDetail::default()
    }
}

fn active_test_json(test_entry: &ManifestTestEntry) -> Value {
    json!({
        "id": test_entry.id,
        "kind": test_entry.kind,
        "target": test_entry.target,
        "tags": test_entry.tags,
    })
}

fn duration_millis_u64(duration: Duration) -> u64 {
    duration.as_millis().try_into().unwrap_or(u64::MAX)
}

fn check_result_invalid_failure(
    artifacts: &CheckTestArtifacts,
    test_id: &str,
    message: &str,
    expected: Value,
    actual: Value,
    batch_evidence: Option<BatchEvidence<'_>>,
) -> FailureDetail {
    FailureDetail {
        kind: "check_result_invalid".to_string(),
        command: Some("check".to_string()),
        path: Some(artifacts.result_path.to_string_lossy().into_owned()),
        target: Some(test_id.to_string()),
        message: Some(message.to_string()),
        expected: Some(expected),
        actual: Some(actual),
        artifacts: failure_artifacts(artifacts, batch_evidence),
        ..FailureDetail::default()
    }
}

fn failure_artifacts(
    artifacts: &CheckTestArtifacts,
    batch_evidence: Option<BatchEvidence<'_>>,
) -> Option<Map<String, Value>> {
    let mut map = value_as_object_map(artifacts.to_json()).unwrap_or_default();
    if let Some(batch_evidence) = batch_evidence {
        map.insert("batch_logs".to_string(), json!(batch_evidence.logs_path));
        map.insert(
            "batch_summary".to_string(),
            json!(batch_evidence.summary_path),
        );
    }
    Some(map)
}

fn ensure_writable_artifact_dir(path: &Path, command: &str) -> Option<FailureDetail> {
    let error = if path.exists() && !path.is_dir() {
        Some(std::io::Error::other(format!(
            "{} is not a directory",
            path.display()
        )))
    } else if let Err(error) = fs::create_dir_all(path) {
        Some(error)
    } else {
        let probe_path = path.join(".godot-qa-write-test");
        match fs::write(&probe_path, b"") {
            Ok(_) => {
                let _ = fs::remove_file(probe_path);
                None
            }
            Err(error) => Some(error),
        }
    };
    error.map(|error| FailureDetail {
        kind: "artifact_dir_not_writable".to_string(),
        command: Some(command.to_string()),
        path: Some(path.to_string_lossy().into_owned()),
        message: Some(error.to_string()),
        expected: Some(json!("writable artifact directory")),
        actual: Some(json!(if path.exists() && !path.is_dir() {
            "NotADirectoryError"
        } else {
            match error.kind() {
                std::io::ErrorKind::PermissionDenied => "PermissionError",
                _ => "OSError",
            }
        })),
        ..FailureDetail::default()
    })
}

fn artifact_write_failed(
    command: &str,
    path: &Path,
    message: &str,
    data: Map<String, Value>,
) -> CommandResult {
    CommandResult {
        ok: false,
        command: command.to_string(),
        message: "project check failed".to_string(),
        data,
        failures: vec![FailureDetail {
            kind: "artifact_write_failed".to_string(),
            command: Some(command.to_string()),
            path: Some(path.to_string_lossy().into_owned()),
            message: Some(message.to_string()),
            expected: Some(json!(
                "artifact metadata written under the artifact directory"
            )),
            actual: Some(json!("OSError")),
            ..FailureDetail::default()
        }],
        exit_code: 1,
    }
}

fn write_json(path: &Path, payload: &Value) -> std::io::Result<()> {
    fs::write(
        path,
        serde_json::to_string_pretty(payload).unwrap_or_else(|_| "{}".to_string()),
    )
}

fn write_result_artifact(path: &Path, result: &CommandResult) -> std::io::Result<()> {
    fs::write(
        path,
        serde_json::to_string_pretty(result).unwrap_or_else(|_| "{}".to_string()),
    )
}

fn normalize_existing_or_join(path: &Path) -> PathBuf {
    path.canonicalize()
        .unwrap_or_else(|_| path.components().collect())
}

fn slug_value(value: &str) -> String {
    let cleaned = value
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() {
                character.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect::<String>();
    cleaned.trim_matches('-').to_string()
}

fn started_at_now() -> String {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| format!("{}Z", duration.as_secs()))
        .unwrap_or_else(|_| "0Z".to_string())
}

fn unique_run_id() -> String {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis().to_string())
        .unwrap_or_else(|_| "0".to_string())
}

fn value_as_object_map(value: Value) -> Option<Map<String, Value>> {
    value.as_object().cloned()
}

fn python_type_name(value: &Value) -> &'static str {
    match value {
        Value::Null => "NoneType",
        Value::Bool(_) => "bool",
        Value::Number(number) => {
            if number.is_i64() || number.is_u64() {
                "int"
            } else {
                "float"
            }
        }
        Value::String(_) => "str",
        Value::Array(_) => "list",
        Value::Object(_) => "dict",
    }
}

#[cfg(test)]
mod tests {
    use super::{
        CheckTestArtifacts, CheckTestExecutionMode, CheckTestRunOptions, CheckTestSelectionRequest,
        append_check_progress_event, check_profile_artifact_json, check_profile_summary_json,
        load_test_manifest, prepare_check_artifacts, prepare_test_artifacts,
        resolve_check_test_selection, run_check_tests, slow_check_summary_json, test_check_request,
        validate_runtime_test_result, validate_runtime_test_result_with_evidence, write_json,
    };
    use godot_qa_core::config::ProjectConfig;
    use serde_json::{Value, json};
    use std::env;
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn check_tests_accepts_empty_manifest_as_noop() {
        let project_root = unique_temp_dir("empty-manifest");
        fs::create_dir_all(project_root.join("addons/godot_qa")).unwrap();
        fs::create_dir_all(project_root.join("qa/tests")).unwrap();
        fs::write(project_root.join("project.godot"), "[application]\n").unwrap();
        fs::write(
            project_root.join("addons/godot_qa/plugin.cfg"),
            "[plugin]\n",
        )
        .unwrap();
        fs::write(
            project_root.join("qa/tests/manifest.json"),
            r#"{"version":1,"tests":[]}"#,
        )
        .unwrap();

        let config = ProjectConfig {
            project_root: project_root.clone(),
            repo_root: project_root.clone(),
            verbose: false,
            godot_binary: Some(
                godot_qa_test_support::fixture(
                    &project_root.join("fake-godot"),
                    &[("version", "4.5.2.test")],
                )
                .to_string_lossy()
                .into_owned(),
            ),
            qa_dir: project_root.join("qa"),
            artifacts_dir: project_root.join(".qa/artifacts"),
            scenarios_dir: project_root.join("qa/scenarios"),
            addon_dir: project_root.join("addons/godot_qa"),
            repo_addon_dir: project_root.join("addons/godot_qa"),
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
        };

        let result = run_check_tests(
            &config,
            "godot-qa check --tests --json",
            CheckTestSelectionRequest::default(),
            CheckTestExecutionMode::Isolated,
            CheckTestRunOptions::default(),
        );

        assert!(result.ok);
        assert_eq!(result.exit_code, 0);
        assert_eq!(result.data["checked_test_count"], 0);
        assert_eq!(result.data["selected_test_count"], 0);
        assert_eq!(result.data["skipped_test_count"], 0);
        assert_eq!(result.data["total_test_count"], 0);
        assert_eq!(result.data["selection"]["mode"], "all");
        assert_eq!(result.data["tests"].as_array().unwrap().len(), 0);
        assert_eq!(result.data["import"]["skipped"], true);

        fs::remove_dir_all(project_root).unwrap();
    }

    #[test]
    fn check_tests_reports_missing_manifest_without_scenarios_dir() {
        let project_root = unique_temp_dir("missing-manifest-no-scenarios");
        fs::create_dir_all(project_root.join("addons/godot_qa")).unwrap();
        fs::write(project_root.join("project.godot"), "[application]\n").unwrap();
        fs::write(
            project_root.join("addons/godot_qa/plugin.cfg"),
            "[plugin]\n",
        )
        .unwrap();

        let config = ProjectConfig {
            project_root: project_root.clone(),
            repo_root: project_root.clone(),
            verbose: false,
            godot_binary: Some(
                godot_qa_test_support::fixture(
                    &project_root.join("fake-godot"),
                    &[("version", "4.5.2.test")],
                )
                .to_string_lossy()
                .into_owned(),
            ),
            qa_dir: project_root.join("qa"),
            artifacts_dir: project_root.join(".qa/artifacts"),
            scenarios_dir: project_root.join("qa/scenarios"),
            addon_dir: project_root.join("addons/godot_qa"),
            repo_addon_dir: project_root.join("addons/godot_qa"),
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
        };

        let result = run_check_tests(
            &config,
            "godot-qa check --tests --json",
            CheckTestSelectionRequest::default(),
            CheckTestExecutionMode::Isolated,
            CheckTestRunOptions::default(),
        );

        assert!(!result.ok);
        assert_eq!(result.exit_code, 1);
        assert_eq!(result.failures[0].kind, "test_manifest_missing");
        assert!(
            result
                .failures
                .iter()
                .all(|failure| failure.kind != "missing_scenarios_dir")
        );

        fs::remove_dir_all(project_root).unwrap();
    }

    #[test]
    fn check_manifest_accepts_time_control() {
        let project_root = project_with_manifest(
            "manifest-time-control",
            r#"{
                "version": 1,
                "tests": [
                    {
                        "id": "accelerated_scene",
                        "kind": "scene",
                        "target": "res://scenes/main.tscn",
                        "startup_frames": 2,
                        "time_control": {
                            "time_scale": 4,
                            "physics_ticks_per_second": 120,
                            "max_physics_steps_per_frame": 16
                        }
                    },
                    {
                        "id": "normal_script",
                        "kind": "script",
                        "target": "res://tests/test_main.gd"
                    }
                ]
            }"#,
        );
        let config = test_config(&project_root);

        let (_, manifest, failure) = load_test_manifest(&config, "check");

        assert!(failure.is_none(), "{failure:?}");
        let entries = manifest.unwrap().entries;
        assert_eq!(entries.len(), 2);
        assert_eq!(
            entries[0].time_control,
            Some(json!({
                "time_scale": 4.0,
                "physics_ticks_per_second": 120,
                "max_physics_steps_per_frame": 16
            }))
        );
        assert_eq!(entries[1].time_control, None);

        let artifacts = test_artifacts(&project_root);
        let request = test_check_request(
            &entries[0],
            &artifacts,
            "run-1",
            "check",
            CheckTestRunOptions::default(),
        );
        assert_eq!(
            request["check"]["time_control"],
            json!({
                "time_scale": 4.0,
                "physics_ticks_per_second": 120,
                "max_physics_steps_per_frame": 16
            })
        );
        let normal_request = test_check_request(
            &entries[1],
            &artifacts,
            "run-1",
            "check",
            CheckTestRunOptions::default(),
        );
        assert!(normal_request["check"].get("time_control").is_none());
        assert_eq!(normal_request["check"]["frame_pacing"]["fixed_fps"], 60);

        let process_result = godot_qa_process::GodotRunResult {
            command: vec!["godot".to_string()],
            returncode: 0,
            termination: None,
            process_timing: None,
            stdout: String::new(),
            stderr: String::new(),
            display: godot_qa_process::resolve_project_display_launch_metadata(
                &project_root,
                &[],
                true,
            ),
            pid: None,
            process_group: None,
            cleanup: None,
            infrastructure_failure_kind: None,
            timed_out: false,
            timeout_ms: None,
            output_truncation: None,
        };
        let runtime_result = json!({
            "id": "accelerated_scene",
            "kind": "scene",
            "target": "res://scenes/main.tscn",
            "ok": true,
            "duration_ms": 7,
            "runtime_errors": [],
            "failures": [],
            "time_control": {
                "requested": {"time_scale": 4.0},
                "effective": {"time_scale": 4.0},
                "restored": true
            }
        });
        let (payload, failures) =
            validate_runtime_test_result(&entries[0], runtime_result, &artifacts, &process_result);
        assert!(failures.is_empty());
        assert_eq!(
            payload["time_control"],
            json!({
                "requested": {"time_scale": 4.0},
                "effective": {"time_scale": 4.0},
                "restored": true
            })
        );

        fs::remove_dir_all(project_root).unwrap();
    }

    #[test]
    fn check_manifest_accepts_fixed_fps_and_derives_project_default() {
        let project_root = project_with_manifest(
            "manifest-fixed-fps",
            r#"{
                "version": 1,
                "tests": [
                    {
                        "id": "custom_fps_scene",
                        "kind": "scene",
                        "target": "res://scenes/main.tscn",
                        "fixed_fps": 144
                    },
                    {
                        "id": "project_fps_script",
                        "kind": "script",
                        "target": "res://tests/test_main.gd"
                    }
                ]
            }"#,
        );
        fs::write(
            project_root.join("project.godot"),
            "[application]\nrun/max_fps=0\n[physics]\ncommon/physics_ticks_per_second=120\n",
        )
        .unwrap();
        let config = test_config(&project_root);

        let (_, manifest, failure) = load_test_manifest(&config, "check");

        assert!(failure.is_none(), "{failure:?}");
        let entries = manifest.unwrap().entries;
        assert_eq!(entries[0].frame_pacing.as_ref().unwrap().fixed_fps, 144);
        assert_eq!(entries[0].frame_pacing.as_ref().unwrap().source, "manifest");
        assert_eq!(entries[1].frame_pacing.as_ref().unwrap().fixed_fps, 120);
        assert_eq!(entries[1].frame_pacing.as_ref().unwrap().source, "project");
        assert_eq!(
            entries[1]
                .frame_pacing
                .as_ref()
                .unwrap()
                .project_setting
                .as_deref(),
            Some("physics/common/physics_ticks_per_second")
        );

        fs::remove_dir_all(project_root).unwrap();
    }

    #[test]
    fn check_tests_profile_request_contract() {
        let project_root = project_with_manifest(
            "profile-request-contract",
            r#"{
                "version": 1,
                "tests": [
                    {
                        "id": "sample_script_test",
                        "kind": "script",
                        "target": "res://tests/test_main.gd"
                    },
                    {
                        "id": "sample_scene_test",
                        "kind": "scene",
                        "target": "res://scenes/main.tscn"
                    }
                ]
            }"#,
        );
        let config = test_config(&project_root);
        let (_, manifest, failure) = load_test_manifest(&config, "check");
        assert!(failure.is_none(), "{failure:?}");
        let manifest = manifest.unwrap();
        let check_artifacts =
            prepare_check_artifacts(&config.artifacts_dir, &config.project_root, "check").unwrap();
        let script_artifacts =
            prepare_test_artifacts(&check_artifacts, &manifest.entries[0]).unwrap();
        let scene_artifacts =
            prepare_test_artifacts(&check_artifacts, &manifest.entries[1]).unwrap();
        let profiled_options = CheckTestRunOptions {
            timeout_override: None,
            profile_enabled: true,
        };

        let unprofiled_request = test_check_request(
            &manifest.entries[0],
            &script_artifacts,
            "run-1",
            "check",
            CheckTestRunOptions::default(),
        );
        assert!(unprofiled_request.get("profiling").is_none());

        let profiled_request = test_check_request(
            &manifest.entries[0],
            &script_artifacts,
            "run-1",
            "check",
            profiled_options,
        );
        assert_eq!(
            profiled_request["profiling"],
            json!({"enabled": true, "max_frame_samples": super::CHECK_PROFILE_MAX_FRAME_SAMPLES})
        );

        let scene_request = test_check_request(
            &manifest.entries[1],
            &scene_artifacts,
            "run-1",
            "check",
            profiled_options,
        );
        let batch_profiled = super::batch_check_request(
            &[
                (
                    &manifest.entries[0],
                    script_artifacts.clone(),
                    profiled_request,
                ),
                (&manifest.entries[1], scene_artifacts, scene_request),
            ],
            &check_artifacts,
            "check",
            profiled_options,
        );
        assert_eq!(
            batch_profiled["check"]["tests"][0]["profiling"],
            json!({"enabled": true, "max_frame_samples": super::CHECK_PROFILE_MAX_FRAME_SAMPLES})
        );
        assert_eq!(
            batch_profiled["check"]["tests"][1]["profiling"],
            json!({"enabled": true, "max_frame_samples": super::CHECK_PROFILE_MAX_FRAME_SAMPLES})
        );

        let batch_unprofiled = super::batch_check_request(
            &[(&manifest.entries[0], script_artifacts, unprofiled_request)],
            &check_artifacts,
            "check",
            CheckTestRunOptions::default(),
        );
        assert!(
            batch_unprofiled["check"]["tests"][0]
                .get("profiling")
                .is_none()
        );

        fs::remove_dir_all(project_root).unwrap();
    }

    #[test]
    fn check_tests_profile_result_contract() {
        let project_root = project_with_manifest(
            "profile-result-contract",
            r#"{"version":1,"tests":[{"id":"sample_script_test","kind":"script","target":"res://tests/test_main.gd"}]}"#,
        );
        let config = test_config(&project_root);
        let (_, manifest, failure) = load_test_manifest(&config, "check");
        assert!(failure.is_none(), "{failure:?}");
        let manifest = manifest.unwrap();
        let check_artifacts =
            prepare_check_artifacts(&config.artifacts_dir, &config.project_root, "check").unwrap();
        let artifacts = prepare_test_artifacts(&check_artifacts, &manifest.entries[0]).unwrap();
        let process_result = successful_process_result(&project_root);
        let runtime_result = json!({
            "id": "sample_script_test",
            "kind": "script",
            "target": "res://tests/test_main.gd",
            "ok": true,
            "duration_ms": 25,
            "runtime_errors": [],
            "failures": [],
            "profile": {
                "frame_times_ms": [16.0, 17.5, 14.0],
                "monitor_samples": [
                    {"name": "time/fps", "value": 60.0},
                    {"name": "time/fps", "value": 58.0},
                    {"name": "object/node_count", "value": 42.0}
                ],
                "unsupported_monitors": [{"name": "render/unsupported", "reason": "unavailable"}],
                "step_timings": [{"label": "run_qa_test", "duration_ms": 9}],
                "top_script_methods": {
                    "status": "available",
                    "instrumentation": "opt_in",
                    "method_count": 1,
                    "sample_count": 3,
                    "methods": [{
                        "label": "MainMenu._process",
                        "call_count": 3,
                        "total_ms": 1.5,
                        "self_ms": 1.2,
                        "avg_ms": 0.5,
                        "max_ms": 0.75,
                        "last_ms": 0.25
                    }]
                }
            }
        });
        let (payload, failures) = validate_runtime_test_result_with_evidence(
            &manifest.entries[0],
            runtime_result,
            &artifacts,
            &process_result,
            None,
            CheckTestRunOptions {
                timeout_override: None,
                profile_enabled: true,
            },
        );
        assert!(failures.is_empty(), "{failures:?}");
        let payloads = vec![payload];
        let summary = check_profile_summary_json(&payloads, &check_artifacts);
        assert_eq!(summary["frame_time_percentiles_ms"]["sample_count"], 3);
        assert_eq!(summary["monitor_counters"]["time/fps"]["sample_count"], 2);
        assert_eq!(summary["monitor_sample_count"], 3);
        assert_eq!(
            summary["unsupported_monitors"][0]["name"],
            "render/unsupported"
        );
        assert_eq!(
            summary["sample_limits"]["max_frame_samples"],
            super::CHECK_PROFILE_MAX_FRAME_SAMPLES
        );
        assert_eq!(
            summary["top_script_methods"]["methods"][0]["label"],
            "MainMenu._process"
        );
        assert_eq!(summary["slow_tests"][0]["id"], "sample_script_test");
        assert_eq!(summary["per_test_timing_count"], 2);
        assert!(
            summary["per_test_timings"].as_array().unwrap().len()
                <= super::CHECK_PROFILE_MAX_FRAME_SAMPLES
        );

        let artifact = check_profile_artifact_json(
            &config,
            &project_root.join("qa/tests/manifest.json"),
            &json!({"mode": "all", "selected_ids": ["sample_script_test"]}),
            CheckTestExecutionMode::Isolated,
            &check_artifacts,
            &payloads,
        );
        super::write_check_profile_artifact(&check_artifacts.profile_path, &artifact).unwrap();
        let written: Value =
            serde_json::from_str(&fs::read_to_string(&check_artifacts.profile_path).unwrap())
                .unwrap();
        assert_eq!(written["frame_time_percentiles_ms"]["p90"], 17.5);
        assert_eq!(
            written["monitor_counters"]["object/node_count"]["latest"],
            42.0
        );
        assert_eq!(
            written["unsupported_monitors"][0]["name"],
            "render/unsupported"
        );
        assert_eq!(
            written["top_script_methods"]["methods"][0]["label"],
            "MainMenu._process"
        );
        assert_eq!(written["per_test_timings"].as_array().unwrap().len(), 2);
        assert_eq!(written["tests"][0]["ok"], true);

        fs::remove_dir_all(project_root).unwrap();
    }

    #[test]
    fn check_tests_profile_malformed_payload_contract() {
        let project_root = project_with_manifest(
            "profile-malformed-payload-contract",
            r#"{"version":1,"tests":[{"id":"sample_script_test","kind":"script","target":"res://tests/test_main.gd"}]}"#,
        );
        let config = test_config(&project_root);
        let (_, manifest, failure) = load_test_manifest(&config, "check");
        assert!(failure.is_none(), "{failure:?}");
        let manifest = manifest.unwrap();
        let check_artifacts =
            prepare_check_artifacts(&config.artifacts_dir, &config.project_root, "check").unwrap();
        let artifacts = prepare_test_artifacts(&check_artifacts, &manifest.entries[0]).unwrap();
        let process_result = successful_process_result(&project_root);
        let runtime_result = json!({
            "id": "sample_script_test",
            "kind": "script",
            "target": "res://tests/test_main.gd",
            "ok": true,
            "duration_ms": 25,
            "runtime_errors": [],
            "failures": [],
            "profile": {"frame_times_ms": ["slow"]}
        });
        let (payload, failures) = validate_runtime_test_result_with_evidence(
            &manifest.entries[0],
            runtime_result,
            &artifacts,
            &process_result,
            None,
            CheckTestRunOptions {
                timeout_override: None,
                profile_enabled: true,
            },
        );
        assert_eq!(payload["ok"], false);
        assert_eq!(failures.len(), 1);
        assert_eq!(failures[0].kind, "profile_result_invalid");
        assert_eq!(
            failures[0].path,
            Some(artifacts.result_path.to_string_lossy().into_owned())
        );
        assert_eq!(
            failures[0].artifacts.as_ref().unwrap()["profile"],
            json!(artifacts.result_path)
        );

        fs::remove_dir_all(project_root).unwrap();
    }

    #[test]
    fn check_manifest_rejects_invalid_time_control() {
        let cases = [
            (
                "too-large",
                r#"{
                    "version": 1,
                    "tests": [{
                        "id": "bad_scale",
                        "kind": "scene",
                        "target": "res://scenes/main.tscn",
                        "time_control": {"time_scale": 65}
                    }]
                }"#,
                "bad_scale",
                "time_control.time_scale must be greater than 0.0 and <= 64",
            ),
            (
                "wrong-type",
                r#"{
                    "version": 1,
                    "tests": [{
                        "id": "bad_type",
                        "kind": "script",
                        "target": "res://tests/test_main.gd",
                        "time_control": {"time_scale": "fast"}
                    }]
                }"#,
                "bad_type",
                "time_control.time_scale must be a positive finite number",
            ),
        ];

        for (name, manifest, expected_target, expected_message) in cases {
            let project_root = project_with_manifest(name, manifest);
            let config = test_config(&project_root);

            let (_, manifest, failure) = load_test_manifest(&config, "check");

            assert!(manifest.is_none());
            let failure = failure.expect("invalid manifest failure");
            assert_eq!(failure.kind, "test_manifest_invalid");
            assert_eq!(failure.command.as_deref(), Some("check"));
            assert_eq!(failure.target.as_deref(), Some(expected_target));
            assert_eq!(failure.message.as_deref(), Some(expected_message));

            fs::remove_dir_all(project_root).unwrap();
        }
    }

    #[test]
    fn check_manifest_rejects_invalid_fixed_fps_and_time_control_conflict() {
        let cases = [
            (
                "bad-fixed-fps",
                r#"{
                    "version": 1,
                    "tests": [{
                        "id": "bad_fps",
                        "kind": "scene",
                        "target": "res://scenes/main.tscn",
                        "fixed_fps": 0
                    }]
                }"#,
                "bad_fps",
                "fixed_fps must be a positive integer",
            ),
            (
                "conflicting-fixed-fps",
                r#"{
                    "version": 1,
                    "tests": [{
                        "id": "ambiguous_pacing",
                        "kind": "scene",
                        "target": "res://scenes/main.tscn",
                        "fixed_fps": 60,
                        "time_control": {"time_scale": 1.0}
                    }]
                }"#,
                "ambiguous_pacing",
                "fixed_fps cannot be combined with time_control",
            ),
        ];

        for (name, manifest, expected_target, expected_message) in cases {
            let project_root = project_with_manifest(name, manifest);
            let config = test_config(&project_root);

            let (_, manifest, failure) = load_test_manifest(&config, "check");

            assert!(manifest.is_none());
            let failure = failure.expect("invalid manifest failure");
            assert_eq!(failure.kind, "test_manifest_invalid");
            assert_eq!(failure.command.as_deref(), Some("check"));
            assert_eq!(failure.target.as_deref(), Some(expected_target));
            assert_eq!(failure.message.as_deref(), Some(expected_message));

            fs::remove_dir_all(project_root).unwrap();
        }
    }

    #[test]
    fn check_tests_timeout_validation_contract() {
        let invalid_cases = [
            (
                "zero-number",
                r#"{"version":1,"timeout_seconds":0,"tests":[{"id":"script_test","kind":"script","target":"res://tests/test_main.gd"}]}"#,
            ),
            (
                "negative-number",
                r#"{"version":1,"tests":[{"id":"script_test","kind":"script","target":"res://tests/test_main.gd","timeout_seconds":-1}]}"#,
            ),
            (
                "non-number",
                r#"{"version":1,"tests":[{"id":"script_test","kind":"script","target":"res://tests/test_main.gd","timeout_seconds":"fast"}]}"#,
            ),
            (
                "unsupported-string",
                r#"{"version":1,"tests":[{"id":"script_test","kind":"script","target":"res://tests/test_main.gd","timeout":"5frames"}]}"#,
            ),
        ];
        for (name, manifest_json) in invalid_cases {
            let project_root = project_with_manifest(name, manifest_json);
            let config = test_config(&project_root);
            let (_, manifest, failure) = load_test_manifest(&config, "check");
            assert!(manifest.is_none(), "{name}");
            let failure = failure.expect("invalid timeout failure");
            assert_eq!(failure.kind, "test_manifest_invalid", "{name}");
            assert_eq!(
                failure.message.as_deref(),
                Some("manifest timeout must be a positive finite duration"),
                "{name}"
            );
            fs::remove_dir_all(project_root).unwrap();
        }
    }

    #[test]
    fn check_tests_timeout_contract() {
        let project_root = project_with_manifest(
            "check-timeout-contract",
            r#"{
                "version": 1,
                "timeout_seconds": 3,
                "tests": [
                    {
                        "id": "hung_script",
                        "kind": "script",
                        "target": "res://tests/test_main.gd",
                        "timeout": "2s"
                    }
                ]
            }"#,
        );
        let config = test_config(&project_root);
        let (_, manifest, failure) = load_test_manifest(&config, "check");
        assert!(failure.is_none(), "{failure:?}");
        let manifest = manifest.unwrap();
        assert_eq!(
            manifest.entries[0].timeout,
            Some(std::time::Duration::from_secs(2))
        );

        let result_payload = json!({
            "id": "hung_script", "kind": "script", "target": "res://tests/test_main.gd",
            "ok": true, "duration_ms": 1, "runtime_errors": [], "failures": []
        })
        .to_string();
        let fake_godot = godot_qa_test_support::fixture(
            &project_root.join("fake-godot-timeout"),
            &[
                ("version", "4.5.2.test"),
                ("request_prefix", "--qa-check-request="),
                ("write:result.json", &result_payload),
                ("sleep_ms", "5000"),
            ],
        );
        let mut config = test_config(&project_root);
        config.godot_binary = Some(fake_godot.to_string_lossy().into_owned());

        let result = run_check_tests(
            &config,
            "godot-qa check --tests --test hung_script --timeout-seconds 0.05 --json",
            CheckTestSelectionRequest {
                test_ids: vec!["hung_script".to_string()],
                tags: vec![],
                profile: None,
            },
            CheckTestExecutionMode::Isolated,
            CheckTestRunOptions {
                timeout_override: Some(std::time::Duration::from_millis(50)),
                profile_enabled: false,
            },
        );

        assert!(!result.ok);
        assert_eq!(result.exit_code, 1);
        assert_eq!(result.failures.len(), 1);
        let failure = &result.failures[0];
        assert_eq!(failure.kind, "check_test_timeout");
        assert_eq!(failure.target.as_deref(), Some("hung_script"));
        assert!(
            failure
                .command
                .as_deref()
                .unwrap()
                .contains("fake-godot-timeout")
        );
        assert_eq!(
            failure.expected,
            Some(json!("Godot check exits with status 0 before timeout"))
        );
        let actual = failure.actual.as_ref().unwrap();
        assert_eq!(actual["state"], "timed_out");
        assert!(actual["elapsed_ms"].as_u64().unwrap() >= 40);
        assert_eq!(actual["timeout_ms"], 50);
        assert_eq!(actual["active_test"]["id"], "hung_script");
        assert!(actual["progress"].as_str().is_some());
        assert!(actual["cleanup"]["owned"].as_bool().unwrap());
        assert_ne!(failure.kind, "assertion_failed");
        assert_eq!(result.data["tests"][0]["ok"], false);
        assert_eq!(
            result.data["tests"][0]["failures"][0]["type"],
            "check_test_timeout"
        );

        fs::remove_dir_all(project_root).unwrap();
    }

    #[test]
    fn check_manifest_tags_and_profiles_contract() {
        let valid_cases = [
            (
                "omitted-tags",
                r#"{
                    "version": 1,
                    "tests": [{
                        "id": "script_test",
                        "kind": "script",
                        "target": "res://tests/test_main.gd"
                    }]
                }"#,
                Vec::<(&str, Vec<&str>, Vec<&str>)>::new(),
            ),
            (
                "valid-tags",
                r#"{
                    "version": 1,
                    "tests": [{
                        "id": "script_test",
                        "kind": "script",
                        "target": "res://tests/test_main.gd",
                        "tags": [" smoke ", "ui"]
                    }]
                }"#,
                Vec::<(&str, Vec<&str>, Vec<&str>)>::new(),
            ),
            (
                "valid-profile-mixed",
                r#"{
                    "version": 1,
                    "tests": [
                        {
                            "id": "script_test",
                            "kind": "script",
                            "target": "res://tests/test_main.gd",
                            "tags": ["smoke"]
                        },
                        {
                            "id": "scene_test",
                            "kind": "scene",
                            "target": "res://scenes/main.tscn",
                            "tags": ["physics"]
                        }
                    ],
                    "profiles": {
                        "full": {
                            "tags": ["smoke"],
                            "tests": ["scene_test"]
                        }
                    }
                }"#,
                vec![("full", vec!["smoke"], vec!["scene_test"])],
            ),
            (
                "valid-empty-profile",
                r#"{
                    "version": 1,
                    "tests": [{
                        "id": "script_test",
                        "kind": "script",
                        "target": "res://tests/test_main.gd"
                    }],
                    "profiles": {
                        "smoke": {}
                    }
                }"#,
                vec![("smoke", vec![], vec![])],
            ),
        ];

        for (name, manifest_json, expected_profiles) in valid_cases {
            let project_root = project_with_manifest(name, manifest_json);
            let config = test_config(&project_root);

            let (_, manifest, failure) = load_test_manifest(&config, "check");

            assert!(failure.is_none(), "{name}: {failure:?}");
            let manifest = manifest.expect("valid manifest");
            assert!(!manifest.entries.is_empty());
            if name == "omitted-tags" {
                assert_eq!(manifest.entries[0].tags, Vec::<String>::new());
            }
            if name == "valid-tags" {
                assert_eq!(manifest.entries[0].tags, vec!["smoke", "ui"]);
            }
            for (profile_name, tags, tests) in expected_profiles {
                let profile = manifest.profiles.get(profile_name).unwrap();
                assert_eq!(profile.tags, tags);
                assert_eq!(profile.tests, tests);
            }

            fs::remove_dir_all(project_root).unwrap();
        }

        let invalid_cases = [
            (
                "empty-tag",
                r#"{
                    "version": 1,
                    "tests": [{
                        "id": "script_test",
                        "kind": "script",
                        "target": "res://tests/test_main.gd",
                        "tags": [" "]
                    }]
                }"#,
                "script_test",
                "manifest test tags must be an array of non-empty strings",
            ),
            (
                "non-string-tag",
                r#"{
                    "version": 1,
                    "tests": [{
                        "id": "script_test",
                        "kind": "script",
                        "target": "res://tests/test_main.gd",
                        "tags": [7]
                    }]
                }"#,
                "script_test",
                "manifest test tags must be an array of non-empty strings",
            ),
            (
                "duplicate-entry-tag",
                r#"{
                    "version": 1,
                    "tests": [{
                        "id": "script_test",
                        "kind": "script",
                        "target": "res://tests/test_main.gd",
                        "tags": ["smoke", " smoke "]
                    }]
                }"#,
                "script_test",
                "manifest string arrays must not contain duplicate values",
            ),
            (
                "reserved-entry-tag",
                r#"{
                    "version": 1,
                    "tests": [{
                        "id": "script_test",
                        "kind": "script",
                        "target": "res://tests/test_main.gd",
                        "tags": ["all"]
                    }]
                }"#,
                "script_test",
                "manifest test tags must not use reserved names",
            ),
            (
                "malformed-profile-root",
                r#"{
                    "version": 1,
                    "tests": [{
                        "id": "script_test",
                        "kind": "script",
                        "target": "res://tests/test_main.gd"
                    }],
                    "profiles": []
                }"#,
                "",
                "manifest profiles must be an object",
            ),
            (
                "unknown-profile-test",
                r#"{
                    "version": 1,
                    "tests": [{
                        "id": "script_test",
                        "kind": "script",
                        "target": "res://tests/test_main.gd"
                    }],
                    "profiles": {
                        "smoke": {"tests": ["missing_test"]}
                    }
                }"#,
                "smoke",
                "manifest profile tests must reference known test ids",
            ),
            (
                "duplicate-profile-value",
                r#"{
                    "version": 1,
                    "tests": [{
                        "id": "script_test",
                        "kind": "script",
                        "target": "res://tests/test_main.gd",
                        "tags": ["smoke"]
                    }],
                    "profiles": {
                        "smoke": {"tags": ["smoke", " smoke "]}
                    }
                }"#,
                "smoke",
                "manifest string arrays must not contain duplicate values",
            ),
        ];

        for (name, manifest_json, expected_target, expected_message) in invalid_cases {
            let project_root = project_with_manifest(name, manifest_json);
            let config = test_config(&project_root);

            let (_, manifest, failure) = load_test_manifest(&config, "check");

            assert!(manifest.is_none(), "{name}");
            let failure = failure.expect("invalid manifest failure");
            assert_eq!(failure.kind, "test_manifest_invalid", "{name}");
            assert_eq!(failure.command.as_deref(), Some("check"), "{name}");
            if expected_target.is_empty() {
                assert!(failure.target.is_none(), "{name}: {:?}", failure.target);
            } else {
                assert_eq!(failure.target.as_deref(), Some(expected_target), "{name}");
            }
            assert_eq!(failure.message.as_deref(), Some(expected_message), "{name}");
            assert!(failure.path.is_some(), "{name}");
            assert!(failure.expected.is_some(), "{name}");
            assert!(failure.actual.is_some(), "{name}");

            fs::remove_dir_all(project_root).unwrap();
        }
    }

    #[test]
    fn check_manifest_batch_opt_out_contract() {
        let project_root = project_with_manifest(
            "manifest-batch-opt-out",
            r#"{
                "version": 1,
                "tests": [
                    {
                        "id": "batch_default",
                        "kind": "script",
                        "target": "res://tests/test_main.gd",
                        "tags": ["smoke"]
                    },
                    {
                        "id": "batch_true",
                        "kind": "script",
                        "target": "res://tests/test_main.gd",
                        "batch": true,
                        "tags": ["smoke"]
                    },
                    {
                        "id": "batch_false",
                        "kind": "script",
                        "target": "res://tests/test_main.gd",
                        "batch": false,
                        "tags": ["slow"]
                    }
                ]
            }"#,
        );
        let config = test_config(&project_root);
        let (manifest_path, manifest, failure) = load_test_manifest(&config, "check");
        assert!(failure.is_none(), "{failure:?}");
        let manifest = manifest.unwrap();
        assert!(manifest.entries[0].batchable);
        assert!(manifest.entries[1].batchable);
        assert!(!manifest.entries[2].batchable);

        let isolated = run_check_tests(
            &config,
            "godot-qa check --tests --test batch_false --json",
            CheckTestSelectionRequest {
                test_ids: vec!["batch_false".to_string()],
                tags: vec![],
                profile: None,
            },
            CheckTestExecutionMode::Isolated,
            CheckTestRunOptions::default(),
        );
        assert_ne!(
            isolated
                .failures
                .first()
                .map(|failure| failure.kind.as_str()),
            Some("test_batch_opt_out")
        );

        let batch = run_check_tests(
            &config,
            "godot-qa check --tests --batch --test batch_false --json",
            CheckTestSelectionRequest {
                test_ids: vec!["batch_false".to_string()],
                tags: vec![],
                profile: None,
            },
            CheckTestExecutionMode::Batch,
            CheckTestRunOptions::default(),
        );
        assert!(!batch.ok);
        assert_eq!(batch.exit_code, 1);
        assert_eq!(batch.data["checked_test_count"], 0);
        assert_eq!(batch.data["selected_test_count"], 1);
        assert_eq!(batch.data["tests"], json!([]));
        assert_eq!(batch.failures.len(), 1);
        assert_eq!(batch.failures[0].kind, "test_batch_opt_out");
        assert_eq!(batch.failures[0].target.as_deref(), Some("batch_false"));

        let smoke = resolve_check_test_selection(
            "check",
            &manifest_path,
            &manifest,
            CheckTestSelectionRequest {
                test_ids: vec![],
                tags: vec!["smoke".to_string()],
                profile: None,
            },
        )
        .unwrap();
        assert!(
            super::selected_batch_opt_out_failures("check", &manifest, &smoke.selected_indices)
                .is_empty()
        );

        fs::remove_dir_all(project_root).unwrap();

        let malformed_project = project_with_manifest(
            "manifest-batch-malformed",
            r#"{
                "version": 1,
                "tests": [
                    {
                        "id": "bad_batch",
                        "kind": "script",
                        "target": "res://tests/test_main.gd",
                        "batch": "false"
                    }
                ]
            }"#,
        );
        let malformed_config = test_config(&malformed_project);
        let (_, manifest, failure) = load_test_manifest(&malformed_config, "check");
        assert!(manifest.is_none());
        let failure = failure.expect("malformed batch failure");
        assert_eq!(failure.kind, "test_manifest_invalid");
        assert_eq!(
            failure.message.as_deref(),
            Some("manifest test batch must be a boolean")
        );
        assert_eq!(failure.target.as_deref(), Some("bad_batch"));
        fs::remove_dir_all(malformed_project).unwrap();
    }

    #[test]
    fn check_tests_batch_request_contract() {
        let project_root = project_with_manifest(
            "batch-request",
            r#"{
                "version": 1,
                "tests": [
                    {
                        "id": "script_a",
                        "kind": "script",
                        "target": "res://tests/test_main.gd",
                        "tags": ["smoke"]
                    },
                    {
                        "id": "scene_b",
                        "kind": "scene",
                        "target": "res://scenes/main.tscn",
                        "startup_frames": 2,
                        "tags": ["smoke", "ui"]
                    }
                ]
            }"#,
        );
        let config = test_config(&project_root);
        let (_, manifest, failure) = load_test_manifest(&config, "check");
        assert!(failure.is_none(), "{failure:?}");
        let manifest = manifest.unwrap();
        let check_artifacts =
            prepare_check_artifacts(&config.artifacts_dir, &config.project_root, "check").unwrap();
        let script_artifacts =
            prepare_test_artifacts(&check_artifacts, &manifest.entries[0]).unwrap();
        let scene_artifacts =
            prepare_test_artifacts(&check_artifacts, &manifest.entries[1]).unwrap();
        let script_request = test_check_request(
            &manifest.entries[0],
            &script_artifacts,
            "run-1",
            "check",
            CheckTestRunOptions::default(),
        );
        let scene_request = test_check_request(
            &manifest.entries[1],
            &scene_artifacts,
            "run-1",
            "check",
            CheckTestRunOptions::default(),
        );
        let request = super::batch_check_request(
            &[
                (
                    &manifest.entries[0],
                    script_artifacts.clone(),
                    script_request,
                ),
                (&manifest.entries[1], scene_artifacts.clone(), scene_request),
            ],
            &check_artifacts,
            "check",
            CheckTestRunOptions::default(),
        );

        assert_eq!(request["check"]["batch"], true);
        assert_eq!(request["run"]["execution_mode"], "batch");
        assert_eq!(
            request["artifacts"]["summary"],
            json!(check_artifacts.batch_summary_path)
        );
        let tests = request["check"]["tests"].as_array().unwrap();
        assert_eq!(tests.len(), 2);
        assert_eq!(tests[0]["id"], "script_a");
        assert_eq!(
            tests[0]["artifacts"]["result"],
            json!(script_artifacts.result_path)
        );
        assert_eq!(tests[0]["request"], json!(script_artifacts.request_path));
        assert_eq!(tests[1]["id"], "scene_b");
        assert_eq!(tests[1]["check"]["startup_frames"], 2);
        assert_eq!(
            tests[1]["artifacts"]["logs"],
            json!(scene_artifacts.logs_path)
        );
        assert!(
            check_artifacts
                .batch_request_path
                .ends_with("batch/request.json")
        );
        assert!(check_artifacts.batch_logs_path.ends_with("batch/logs.txt"));
        assert!(
            check_artifacts
                .batch_summary_path
                .ends_with("batch/summary.json")
        );

        fs::remove_dir_all(project_root).unwrap();
    }

    #[test]
    fn check_tests_batch_runner_contract() {
        let project_root = project_with_manifest(
            "batch-runner-contract",
            r#"{
                "version": 1,
                "tests": [
                    {
                        "id": "script_a",
                        "kind": "script",
                        "target": "res://tests/test_main.gd",
                        "time_control": {"time_scale": 2.0},
                        "tags": ["smoke"]
                    },
                    {
                        "id": "scene_b",
                        "kind": "scene",
                        "target": "res://scenes/main.tscn",
                        "startup_frames": 2,
                        "tags": ["smoke", "ui"]
                    }
                ]
            }"#,
        );
        let config = test_config(&project_root);
        let (_, manifest, failure) = load_test_manifest(&config, "check");
        assert!(failure.is_none(), "{failure:?}");
        let manifest = manifest.unwrap();
        let check_artifacts =
            prepare_check_artifacts(&config.artifacts_dir, &config.project_root, "check").unwrap();
        let script_artifacts =
            prepare_test_artifacts(&check_artifacts, &manifest.entries[0]).unwrap();
        let scene_artifacts =
            prepare_test_artifacts(&check_artifacts, &manifest.entries[1]).unwrap();
        let request = super::batch_check_request(
            &[
                (
                    &manifest.entries[0],
                    script_artifacts.clone(),
                    test_check_request(
                        &manifest.entries[0],
                        &script_artifacts,
                        "run-1",
                        "check",
                        CheckTestRunOptions::default(),
                    ),
                ),
                (
                    &manifest.entries[1],
                    scene_artifacts.clone(),
                    test_check_request(
                        &manifest.entries[1],
                        &scene_artifacts,
                        "run-1",
                        "check",
                        CheckTestRunOptions::default(),
                    ),
                ),
            ],
            &check_artifacts,
            "check",
            CheckTestRunOptions::default(),
        );

        assert_eq!(request["check"]["batch"], true);
        assert_eq!(request["run"]["execution_mode"], "batch");
        let tests = request["check"]["tests"].as_array().unwrap();
        assert_eq!(tests.len(), 2);
        for test in tests {
            assert!(test["id"].is_string());
            assert!(test["kind"].is_string());
            assert!(test["target"].is_string());
            assert!(test["check"].is_object());
            assert!(test["artifacts"].is_object());
            assert!(test["artifacts"]["result"].is_string());
        }
        assert_eq!(tests[0]["check"]["time_control"]["time_scale"], 2.0);
        assert_eq!(tests[1]["check"]["startup_frames"], 2);
        assert_eq!(
            request["artifacts"]["summary"],
            json!(check_artifacts.batch_summary_path)
        );

        fs::remove_dir_all(project_root).unwrap();
    }

    #[test]
    fn check_tests_progress_evidence_contract() {
        let project_root = project_with_manifest(
            "progress-evidence",
            r#"{
                "version": 1,
                "tests": [
                    {
                        "id": "script_a",
                        "kind": "script",
                        "target": "res://tests/test_main.gd",
                        "tags": ["smoke"]
                    }
                ]
            }"#,
        );
        let config = test_config(&project_root);
        let (_, manifest, failure) = load_test_manifest(&config, "check");
        assert!(failure.is_none(), "{failure:?}");
        let manifest = manifest.unwrap();
        let check_artifacts =
            prepare_check_artifacts(&config.artifacts_dir, &config.project_root, "check").unwrap();
        let test_artifacts =
            prepare_test_artifacts(&check_artifacts, &manifest.entries[0]).unwrap();
        assert_eq!(
            check_artifacts.to_json()["progress"],
            json!(check_artifacts.progress_path)
        );
        assert_eq!(
            check_artifacts.batch_json()["progress"],
            json!(check_artifacts.progress_path)
        );
        assert_eq!(
            test_artifacts.to_json()["progress"],
            json!(test_artifacts.progress_path)
        );

        append_check_progress_event(
            &check_artifacts.progress_path,
            Some(&test_artifacts.progress_path),
            &manifest.entries[0],
            "start",
            "123Z",
            None,
            None,
            None,
            &test_artifacts,
        )
        .unwrap();
        append_check_progress_event(
            &check_artifacts.progress_path,
            Some(&test_artifacts.progress_path),
            &manifest.entries[0],
            "finish",
            "123Z",
            Some(7),
            Some(true),
            Some(0),
            &test_artifacts,
        )
        .unwrap();
        let events = fs::read_to_string(&test_artifacts.progress_path)
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str::<Value>(line).unwrap())
            .collect::<Vec<_>>();
        assert_eq!(events.len(), 2);
        assert_eq!(events[0]["event"], "start");
        assert_eq!(events[0]["id"], "script_a");
        assert_eq!(events[0]["kind"], "script");
        assert_eq!(events[0]["target"], "res://tests/test_main.gd");
        assert_eq!(events[0]["started_at"], "123Z");
        assert_eq!(events[0]["elapsed_ms"], Value::Null);
        assert_eq!(events[1]["event"], "finish");
        assert_eq!(events[1]["elapsed_ms"], 7);
        assert_eq!(events[1]["ok"], true);
        assert_eq!(events[1]["failure_count"], 0);
        assert_eq!(
            events[1]["artifacts"]["progress"],
            json!(test_artifacts.progress_path)
        );

        let request = super::batch_check_request(
            &[(
                &manifest.entries[0],
                test_artifacts.clone(),
                test_check_request(
                    &manifest.entries[0],
                    &test_artifacts,
                    "run-1",
                    "check",
                    CheckTestRunOptions::default(),
                ),
            )],
            &check_artifacts,
            "check",
            CheckTestRunOptions::default(),
        );
        assert_eq!(
            request["artifacts"]["progress"],
            json!(check_artifacts.progress_path)
        );
        assert_eq!(
            request["check"]["tests"][0]["artifacts"]["progress"],
            json!(test_artifacts.progress_path)
        );

        fs::remove_dir_all(project_root).unwrap();
    }

    #[test]
    fn slow_summary_contract_check_tests_are_bounded_sorted_artifact_backed_and_non_failing() {
        let root = unique_temp_dir("check-slow-summary-contract");
        fs::create_dir_all(&root).unwrap();
        let artifact_path = root.join("slow-summary.json");
        let payloads = vec![
            json!({"id":"fast","kind":"script","target":"res://fast.gd","ok":true,"duration_ms":5,"artifacts":{"result":"fast.json"}}),
            json!({"id":"slow_a","kind":"script","target":"res://slow_a.gd","ok":true,"duration_ms":75,"artifacts":{"result":"slow_a.json"}}),
            json!({"id":"slow_b","kind":"script","target":"res://slow_b.gd","ok":true,"duration_ms":125,"artifacts":{"result":"slow_b.json"}}),
        ];
        let summary = slow_check_summary_json(&payloads, 50, None, &artifact_path);
        write_json(&artifact_path, &summary).unwrap();

        assert_eq!(summary["slow_count"], json!(2));
        assert_eq!(summary["slow_tests"][0]["id"], json!("slow_b"));
        assert_eq!(summary["slow_tests"][1]["id"], json!("slow_a"));
        assert_eq!(summary["slow_tests"][0]["duration_ms"], json!(125));
        assert_eq!(summary["artifacts"]["slow_summary"], json!(artifact_path));
        assert!(artifact_path.is_file());

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn check_tests_batch_result_contract() {
        let project_root = project_with_manifest(
            "batch-result",
            r#"{
                "version": 1,
                "tests": [
                    {
                        "id": "script_bad",
                        "kind": "script",
                        "target": "res://tests/test_main.gd",
                        "tags": ["smoke"]
                    },
                    {
                        "id": "script_late",
                        "kind": "script",
                        "target": "res://tests/test_late.gd",
                        "tags": ["smoke"]
                    }
                ]
            }"#,
        );
        fs::write(project_root.join("tests/test_late.gd"), "extends Node\n").unwrap();
        let invalid_payload = json!({
            "id": "script_bad", "kind": "script", "target": "res://tests/test_main.gd",
            "duration_ms": 3, "runtime_errors": [], "failures": []
        })
        .to_string();
        let failed_payload = json!({
            "id": "script_late", "kind": "script", "target": "res://tests/test_late.gd",
            "ok": false, "duration_ms": 5,
            "runtime_errors": [{"message": "late runtime"}],
            "failures": [{"type": "assertion_failed", "message": "late failure"}]
        })
        .to_string();
        let fake_godot = godot_qa_test_support::fixture(
            &project_root.join("fake-godot-batch"),
            &[
                ("version", "4.5.2.test"),
                ("request_prefix", "--qa-check-request="),
                ("write:../tests/script-bad/result.json", &invalid_payload),
                ("write:../tests/script-late/result.json", &failed_payload),
                ("exit", "1"),
            ],
        );
        let mut config = test_config(&project_root);
        config.godot_binary = Some(fake_godot.to_string_lossy().into_owned());

        let result = run_check_tests(
            &config,
            "godot-qa check --tests --batch --tag smoke --json",
            CheckTestSelectionRequest {
                test_ids: vec![],
                tags: vec!["smoke".to_string()],
                profile: None,
            },
            CheckTestExecutionMode::Batch,
            CheckTestRunOptions::default(),
        );

        assert!(!result.ok);
        assert_eq!(result.exit_code, 1);
        assert_eq!(result.data["execution_mode"], "batch");
        assert!(result.data["artifacts"]["batch_summary"].is_string());
        assert_eq!(
            result.data["batch"]["artifacts"]["summary"],
            result.data["artifacts"]["batch_summary"]
        );
        let tests = result.data["tests"].as_array().unwrap();
        assert_eq!(tests.len(), 2);
        assert_eq!(tests[0]["id"], "script_bad");
        assert_eq!(tests[0]["ok"], false);
        assert_eq!(tests[0]["failures"][0]["type"], "check_result_invalid");
        assert!(tests[0]["failures"][0]["artifacts"]["batch_logs"].is_string());
        assert!(tests[0]["failures"][0]["artifacts"]["batch_summary"].is_string());
        assert_eq!(tests[1]["id"], "script_late");
        assert_eq!(tests[1]["kind"], "script");
        assert_eq!(tests[1]["target"], "res://tests/test_late.gd");
        assert_eq!(tests[1]["ok"], false);
        assert_eq!(tests[1]["duration_ms"], 5);
        assert_eq!(tests[1]["runtime_errors"][0]["message"], "late runtime");
        assert!(tests[1]["artifacts"]["result"].is_string());
        assert_eq!(tests[1]["failures"][0]["type"], "assertion_failed");
        assert_eq!(result.failures.len(), 2);
        assert_eq!(result.failures[0].target.as_deref(), Some("script_bad"));
        assert_eq!(result.failures[1].target.as_deref(), Some("script_late"));

        let summary_path =
            PathBuf::from(result.data["artifacts"]["batch_summary"].as_str().unwrap());
        let summary: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(summary_path).unwrap()).unwrap();
        assert_eq!(summary["execution_mode"], "batch");
        assert_eq!(summary["test_count"], 2);

        fs::remove_dir_all(project_root).unwrap();
    }

    #[test]
    fn check_tests_selection_resolution_contract() {
        let project_root = project_with_manifest(
            "selection-resolution",
            r#"{
                "version": 1,
                "tests": [
                    {
                        "id": "script_a",
                        "kind": "script",
                        "target": "res://tests/test_main.gd",
                        "tags": ["smoke", "script"]
                    },
                    {
                        "id": "scene_b",
                        "kind": "scene",
                        "target": "res://scenes/main.tscn",
                        "tags": ["smoke", "ui"]
                    },
                    {
                        "id": "script_c",
                        "kind": "script",
                        "target": "res://tests/test_other.gd",
                        "tags": ["physics"]
                    }
                ],
                "profiles": {
                    "smoke": {"tags": ["smoke"]},
                    "mixed": {"tags": ["physics"], "tests": ["script_a"]}
                }
            }"#,
        );
        fs::write(project_root.join("tests/test_other.gd"), "extends Node\n").unwrap();
        let config = test_config(&project_root);
        let (manifest_path, manifest, failure) = load_test_manifest(&config, "check");
        assert!(failure.is_none(), "{failure:?}");
        let manifest = manifest.unwrap();

        let all = resolve_check_test_selection(
            "check",
            &manifest_path,
            &manifest,
            CheckTestSelectionRequest::default(),
        )
        .unwrap();
        assert_eq!(all.selected_test_count(), 3);
        assert_eq!(all.skipped_test_count(), 0);
        assert_eq!(all.selection["mode"], "all");

        let explicit = resolve_check_test_selection(
            "check",
            &manifest_path,
            &manifest,
            CheckTestSelectionRequest {
                test_ids: vec!["script_c".to_string()],
                tags: vec![],
                profile: None,
            },
        )
        .unwrap();
        assert_eq!(selected_ids(&manifest, &explicit), vec!["script_c"]);
        assert_eq!(explicit.selected_test_count(), 1);
        assert_eq!(explicit.skipped_test_count(), 2);
        assert_eq!(explicit.total_test_count, 3);
        assert_eq!(explicit.selection["selected_ids"], json!(["script_c"]));

        let repeated_id = resolve_check_test_selection(
            "check",
            &manifest_path,
            &manifest,
            CheckTestSelectionRequest {
                test_ids: vec!["script_a".to_string(), " script_a ".to_string()],
                tags: vec![],
                profile: None,
            },
        )
        .unwrap();
        assert_eq!(selected_ids(&manifest, &repeated_id), vec!["script_a"]);

        let tag = resolve_check_test_selection(
            "check",
            &manifest_path,
            &manifest,
            CheckTestSelectionRequest {
                test_ids: vec![],
                tags: vec!["smoke".to_string()],
                profile: None,
            },
        )
        .unwrap();
        assert_eq!(selected_ids(&manifest, &tag), vec!["script_a", "scene_b"]);

        let tag_union = resolve_check_test_selection(
            "check",
            &manifest_path,
            &manifest,
            CheckTestSelectionRequest {
                test_ids: vec![],
                tags: vec!["ui".to_string(), "physics".to_string()],
                profile: None,
            },
        )
        .unwrap();
        assert_eq!(
            selected_ids(&manifest, &tag_union),
            vec!["scene_b", "script_c"]
        );

        let profile = resolve_check_test_selection(
            "check",
            &manifest_path,
            &manifest,
            CheckTestSelectionRequest {
                test_ids: vec![],
                tags: vec![],
                profile: Some("mixed".to_string()),
            },
        )
        .unwrap();
        assert_eq!(
            selected_ids(&manifest, &profile),
            vec!["script_a", "script_c"]
        );
        assert_eq!(profile.selection["profile"], "mixed");
        assert_eq!(profile.selection["profile_tags"], json!(["physics"]));
        assert_eq!(profile.selection["profile_tests"], json!(["script_a"]));

        let selected_failure_runtime = json!({
            "id": "script_c",
            "kind": "script",
            "target": "res://tests/test_other.gd",
            "ok": false,
            "duration_ms": 5,
            "runtime_errors": [],
            "failures": [{"type": "assertion_failed", "message": "selected failed"}]
        });
        let artifacts = test_artifacts(&project_root);
        let process_result = godot_qa_process::GodotRunResult {
            command: vec!["godot".to_string()],
            returncode: 0,
            termination: None,
            process_timing: None,
            stdout: String::new(),
            stderr: String::new(),
            display: godot_qa_process::resolve_project_display_launch_metadata(
                &project_root,
                &[],
                true,
            ),
            pid: None,
            process_group: None,
            cleanup: None,
            infrastructure_failure_kind: None,
            timed_out: false,
            timeout_ms: None,
            output_truncation: None,
        };
        let (payload, failures) = validate_runtime_test_result(
            &manifest.entries[2],
            selected_failure_runtime,
            &artifacts,
            &process_result,
        );
        assert!(!failures.is_empty());
        assert_eq!(payload["id"], "script_c");
        assert_eq!(payload["tags"], json!(["physics"]));
        assert_eq!(payload["ok"], false);

        for (name, request, expected_message) in [
            (
                "unknown-test",
                CheckTestSelectionRequest {
                    test_ids: vec!["missing".to_string()],
                    tags: vec![],
                    profile: None,
                },
                "check --tests --test value must match a manifest test id",
            ),
            (
                "unknown-tag",
                CheckTestSelectionRequest {
                    test_ids: vec![],
                    tags: vec!["missing".to_string()],
                    profile: None,
                },
                "check --tests --tag value must match at least one manifest test tag",
            ),
            (
                "duplicate-tag",
                CheckTestSelectionRequest {
                    test_ids: vec![],
                    tags: vec!["smoke".to_string(), " smoke ".to_string()],
                    profile: None,
                },
                "check --tests --tag values must not contain duplicates",
            ),
            (
                "unknown-profile",
                CheckTestSelectionRequest {
                    test_ids: vec![],
                    tags: vec![],
                    profile: Some("missing".to_string()),
                },
                "check --tests profile must exist in qa/tests/manifest.json",
            ),
        ] {
            let failure = resolve_check_test_selection("check", &manifest_path, &manifest, request)
                .expect_err(name);
            assert_eq!(failure.kind, "test_selection_invalid", "{name}");
            assert_eq!(failure.message.as_deref(), Some(expected_message), "{name}");
        }

        let result = run_check_tests(
            &config,
            "godot-qa check --tests --tag missing --json",
            CheckTestSelectionRequest {
                test_ids: vec![],
                tags: vec!["missing".to_string()],
                profile: None,
            },
            CheckTestExecutionMode::Isolated,
            CheckTestRunOptions::default(),
        );
        assert!(!result.ok);
        assert_eq!(result.exit_code, 1);
        assert_eq!(result.data["checked_test_count"], 0);
        assert_eq!(result.data["selected_test_count"], 0);
        assert_eq!(result.data["skipped_test_count"], 0);
        assert_eq!(result.data["total_test_count"], 3);
        assert_eq!(result.data["selection"]["mode"], "invalid");
        assert_eq!(result.data["tests"], json!([]));
        assert_eq!(result.failures[0].kind, "test_selection_invalid");

        let empty_profile_project = project_with_manifest(
            "selection-empty-profile",
            r#"{
                "version": 1,
                "tests": [{
                    "id": "script_test",
                    "kind": "script",
                    "target": "res://tests/test_main.gd",
                    "tags": ["ui"]
                }],
                "profiles": {
                    "smoke": {}
                }
            }"#,
        );
        let empty_profile_config = test_config(&empty_profile_project);
        let (empty_profile_manifest_path, empty_profile_manifest, empty_profile_failure) =
            load_test_manifest(&empty_profile_config, "check");
        assert!(empty_profile_failure.is_none(), "{empty_profile_failure:?}");
        let empty_profile_manifest = empty_profile_manifest.unwrap();
        let failure = resolve_check_test_selection(
            "check",
            &empty_profile_manifest_path,
            &empty_profile_manifest,
            CheckTestSelectionRequest {
                test_ids: vec![],
                tags: vec![],
                profile: Some("smoke".to_string()),
            },
        )
        .unwrap_err();
        assert_eq!(failure.kind, "test_selection_invalid");
        assert_eq!(failure.actual, Some(json!("empty_selection")));

        let empty_project = project_with_manifest(
            "selection-empty",
            r#"{
                "version": 1,
                "tests": [],
                "profiles": {}
            }"#,
        );
        let empty_config = test_config(&empty_project);
        let (empty_manifest_path, empty_manifest, empty_failure) =
            load_test_manifest(&empty_config, "check");
        assert!(empty_failure.is_none(), "{empty_failure:?}");
        let empty_manifest = empty_manifest.unwrap();
        let empty_selection = resolve_check_test_selection(
            "check",
            &empty_manifest_path,
            &empty_manifest,
            CheckTestSelectionRequest::default(),
        )
        .unwrap();
        assert_eq!(empty_selection.selected_test_count(), 0);
        assert_eq!(empty_selection.total_test_count, 0);
        assert_eq!(empty_selection.selection["mode"], "all");

        fs::remove_dir_all(project_root).unwrap();
        fs::remove_dir_all(empty_profile_project).unwrap();
        fs::remove_dir_all(empty_project).unwrap();
    }

    fn project_with_manifest(name: &str, manifest: &str) -> PathBuf {
        let project_root = unique_temp_dir(name);
        fs::create_dir_all(project_root.join("addons/godot_qa")).unwrap();
        fs::create_dir_all(project_root.join("qa/scenarios")).unwrap();
        fs::create_dir_all(project_root.join("qa/tests")).unwrap();
        fs::create_dir_all(project_root.join("scenes")).unwrap();
        fs::create_dir_all(project_root.join("tests")).unwrap();
        fs::write(project_root.join("project.godot"), "[application]\n").unwrap();
        fs::write(
            project_root.join("addons/godot_qa/plugin.cfg"),
            "[plugin]\n",
        )
        .unwrap();
        fs::write(project_root.join("scenes/main.tscn"), "[gd_scene]\n").unwrap();
        fs::write(project_root.join("tests/test_main.gd"), "extends Node\n").unwrap();
        fs::write(project_root.join("qa/tests/manifest.json"), manifest).unwrap();
        project_root
    }

    fn test_config(project_root: &Path) -> ProjectConfig {
        ProjectConfig {
            project_root: project_root.to_path_buf(),
            repo_root: project_root.to_path_buf(),
            verbose: false,
            godot_binary: Some(
                godot_qa_test_support::fixture(
                    &project_root.join("fake-godot"),
                    &[("version", "4.5.2.test")],
                )
                .to_string_lossy()
                .into_owned(),
            ),
            qa_dir: project_root.join("qa"),
            artifacts_dir: project_root.join(".qa/artifacts"),
            scenarios_dir: project_root.join("qa/scenarios"),
            addon_dir: project_root.join("addons/godot_qa"),
            repo_addon_dir: project_root.join("addons/godot_qa"),
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

    fn test_artifacts(project_root: &std::path::Path) -> CheckTestArtifacts {
        let root_dir = project_root.join(".qa/artifacts/check/run-1/tests/accelerated-scene");
        fs::create_dir_all(&root_dir).unwrap();
        CheckTestArtifacts {
            root_dir: root_dir.clone(),
            meta_path: root_dir.join("meta.json"),
            request_path: root_dir.join("request.json"),
            result_path: root_dir.join("result.json"),
            logs_path: root_dir.join("logs.txt"),
            progress_path: root_dir.join("progress.jsonl"),
        }
    }

    fn successful_process_result(project_root: &Path) -> godot_qa_process::GodotRunResult {
        godot_qa_process::GodotRunResult {
            command: vec!["godot".to_string()],
            returncode: 0,
            termination: None,
            process_timing: None,
            stdout: String::new(),
            stderr: String::new(),
            display: godot_qa_process::resolve_project_display_launch_metadata(
                project_root,
                &[],
                true,
            ),
            pid: None,
            process_group: None,
            cleanup: None,
            infrastructure_failure_kind: None,
            timed_out: false,
            timeout_ms: None,
            output_truncation: None,
        }
    }

    fn selected_ids(
        manifest: &super::TestManifest,
        selection: &super::ResolvedCheckTestSelection,
    ) -> Vec<String> {
        selection
            .selected_indices
            .iter()
            .map(|index| manifest.entries[*index].id.clone())
            .collect()
    }

    fn unique_temp_dir(name: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        env::temp_dir().join(format!("godot-qa-check-{name}-{unique}"))
    }
}
