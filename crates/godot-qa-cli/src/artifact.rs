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
    record as scenario_record, retention as scenario_retention, review as scenario_review,
    run as scenario_run, scenario_doc, screenshots as scenario_screenshots, time_control,
    visual_audit,
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

pub(crate) fn artifact_query_result(args: &ArtifactQueryArgs) -> CommandResult {
    let command = "artifact query";
    let target = args
        .target
        .clone()
        .or_else(|| args.qa_id.as_ref().map(|qa_id| format!("qa:{qa_id}")));
    let request = artifact_query::QueryRequest {
        target,
        node_type: args.node_type.clone(),
        visible: args.visible,
    };
    match artifact_query::query_run_snapshot(
        Path::new(&args.run_root),
        &args.snapshot,
        request,
        command,
    ) {
        Ok(result) => CommandResult {
            ok: true,
            command: command.to_string(),
            message: "artifact query completed".to_string(),
            data: result.data(Some(Path::new(&args.run_root))),
            failures: Vec::new(),
            exit_code: 0,
        },
        Err(failure) => CommandResult {
            ok: false,
            command: command.to_string(),
            message: "artifact query failed".to_string(),
            data: serde_json::Map::new(),
            failures: vec![failure],
            exit_code: 1,
        },
    }
}

pub(crate) fn artifact_summarize_result(args: &ArtifactSummarizeArgs) -> CommandResult {
    let command = "artifact.summarize";
    let run_root = absolute_path(Path::new(&args.run_root));
    let summary_path = (!args.no_write).then(|| run_root.join("artifact_summary.json"));
    let markdown_path = args
        .markdown_out
        .as_ref()
        .map(|path| absolute_path(Path::new(path)));
    let options = artifact_summary::ArtifactSummaryOptions {
        type_filters: args.artifact_types.clone(),
        warnings_only: args.warnings_only,
        expected_types: args.expected_types.clone(),
        write_summary: false,
        summary_filename: "artifact_summary.json".to_string(),
        markdown_run_label: None,
    };

    let mut summary = match artifact_summary::summarize_artifact_root(&run_root, &options) {
        Ok(summary) => summary,
        Err(error) => {
            return artifact_summarize_failure_result(
                command,
                "artifact summary failed",
                run_root,
                summary_path,
                markdown_path,
                FailureDetail {
                    kind: "artifact_summary.read_failed".to_string(),
                    command: Some(command.to_string()),
                    message: Some("failed to read artifact run root".to_string()),
                    path: Some(args.run_root.clone()),
                    expected: Some(json!("readable artifact run root")),
                    actual: Some(json!(error.to_string())),
                    ..FailureDetail::default()
                },
            );
        }
    };
    summary.summary_path = summary_path
        .as_ref()
        .map(|path| path.to_string_lossy().into_owned());

    if !run_root.is_dir() {
        return artifact_summarize_failure_result(
            command,
            "artifact summary failed",
            run_root.clone(),
            summary_path,
            markdown_path,
            FailureDetail {
                kind: "artifact_summary.run_root_missing".to_string(),
                command: Some(command.to_string()),
                message: Some("artifact run root does not exist or is not a directory".to_string()),
                path: Some(run_root.to_string_lossy().into_owned()),
                expected: Some(json!("existing artifact run root directory")),
                actual: Some(json!("missing")),
                ..FailureDetail::default()
            },
        );
    }

    if let Some(path) = &summary_path {
        let bytes = match serde_json::to_vec_pretty(&summary) {
            Ok(bytes) => bytes,
            Err(error) => {
                return artifact_summarize_failure_result(
                    command,
                    "artifact summary failed",
                    run_root,
                    summary_path.clone(),
                    markdown_path.clone(),
                    FailureDetail {
                        kind: "artifact_summary.write_failed".to_string(),
                        command: Some(command.to_string()),
                        message: Some("failed to serialize artifact summary".to_string()),
                        path: Some(path.to_string_lossy().into_owned()),
                        expected: Some(json!("serializable artifact summary")),
                        actual: Some(json!(error.to_string())),
                        ..FailureDetail::default()
                    },
                );
            }
        };
        if let Err(error) = std::fs::write(path, bytes) {
            return artifact_summarize_failure_result(
                command,
                "artifact summary failed",
                run_root,
                summary_path.clone(),
                markdown_path.clone(),
                FailureDetail {
                    kind: "artifact_summary.write_failed".to_string(),
                    command: Some(command.to_string()),
                    message: Some("failed to write artifact summary JSON".to_string()),
                    path: Some(path.to_string_lossy().into_owned()),
                    expected: Some(json!("writable artifact_summary.json path")),
                    actual: Some(json!(error.to_string())),
                    ..FailureDetail::default()
                },
            );
        }
    }

    if let Some(path) = &markdown_path {
        if let Some(parent) = path.parent()
            && let Err(error) = std::fs::create_dir_all(parent)
        {
            return artifact_summarize_failure_result(
                command,
                "artifact summary failed",
                run_root,
                summary_path.clone(),
                markdown_path.clone(),
                FailureDetail {
                    kind: "artifact_summary.markdown_write_failed".to_string(),
                    command: Some(command.to_string()),
                    message: Some("failed to create Markdown output directory".to_string()),
                    path: Some(parent.to_string_lossy().into_owned()),
                    expected: Some(json!("writable Markdown output directory")),
                    actual: Some(json!(error.to_string())),
                    ..FailureDetail::default()
                },
            );
        }
        let markdown = artifact_summary::render_markdown_summary(&summary);
        if let Err(error) = std::fs::write(path, markdown) {
            return artifact_summarize_failure_result(
                command,
                "artifact summary failed",
                run_root,
                summary_path.clone(),
                markdown_path.clone(),
                FailureDetail {
                    kind: "artifact_summary.markdown_write_failed".to_string(),
                    command: Some(command.to_string()),
                    message: Some("failed to write artifact summary Markdown".to_string()),
                    path: Some(path.to_string_lossy().into_owned()),
                    expected: Some(json!("writable Markdown output path")),
                    actual: Some(json!(error.to_string())),
                    ..FailureDetail::default()
                },
            );
        }
    }

    CommandResult {
        ok: true,
        command: command.to_string(),
        message: "artifact summary completed".to_string(),
        data: artifact_summarize_data(
            &run_root,
            summary_path.as_deref(),
            markdown_path.as_deref(),
            &summary,
        ),
        failures: Vec::new(),
        exit_code: 0,
    }
}

pub(crate) fn artifact_summarize_failure_result(
    command: &str,
    message: &str,
    run_root: PathBuf,
    summary_path: Option<PathBuf>,
    markdown_path: Option<PathBuf>,
    failure: FailureDetail,
) -> CommandResult {
    let data = Map::from_iter([
        ("run_root".to_string(), json!(run_root)),
        (
            "summary_path".to_string(),
            path_option_json(summary_path.as_deref()),
        ),
        (
            "markdown_path".to_string(),
            path_option_json(markdown_path.as_deref()),
        ),
    ]);
    CommandResult {
        ok: false,
        command: command.to_string(),
        message: message.to_string(),
        data,
        failures: vec![failure],
        exit_code: 1,
    }
}

pub(crate) fn artifact_summarize_data(
    run_root: &Path,
    summary_path: Option<&Path>,
    markdown_path: Option<&Path>,
    summary: &artifact_summary::ArtifactSummary,
) -> Map<String, Value> {
    Map::from_iter([
        ("run_root".to_string(), json!(run_root)),
        ("summary_path".to_string(), path_option_json(summary_path)),
        ("markdown_path".to_string(), path_option_json(markdown_path)),
        ("counts".to_string(), json!(summary.counts)),
        (
            "important_findings".to_string(),
            json!(artifact_summary::collect_important_findings(summary)),
        ),
        ("summary".to_string(), json!(summary)),
    ])
}

/// `godot-qa artifact prune`: enforce `artifacts.keep_runs` across the whole artifacts root on
/// demand instead of only as a side effect of running something.
///
/// Retention normally prunes just the scenarios a run touched, so a project that turned it on
/// late — or that ran for months without it — keeps paying for history no command will ever
/// revisit. This sweeps every scenario directory, every bundle, and the abandoned pooled
/// sessions under `live/` in one pass, using exactly the rules `scenario run` and
/// `scenario bundle run` already apply, and refuses to guess: without `--keep-runs` and
/// without `artifacts.keep_runs` there is no retention policy to enforce.
pub(crate) fn artifact_prune_result(cli: &Cli, args: &ArtifactPruneArgs) -> CommandResult {
    let command = "artifact prune";
    let config = load_project_config(
        cli.project.as_deref().map(Path::new),
        cli.godot_bin.as_deref(),
        cli.artifact_dir.as_deref(),
        cli.verbose,
        None,
    );
    if let Some(failure) = config.project_config_failure_for_command(command) {
        return artifact_prune_rejected(command, &config, 2, failure);
    }

    // `--keep-runs 0` would mean "delete every run", which is exactly what the config parser
    // refuses for `artifacts.keep_runs`; the flag must not be a way around that.
    if args.keep_runs == Some(0) {
        return artifact_prune_rejected(
            command,
            &config,
            2,
            FailureDetail {
                kind: "invalid_arguments".to_string(),
                command: Some(command.to_string()),
                message: Some("--keep-runs must be a positive integer".to_string()),
                expected: Some(json!("--keep-runs positive integer")),
                actual: Some(json!("0")),
                details: Map::from_iter([(
                    "remediation".to_string(),
                    json!("pass --keep-runs 1 or higher to keep at least the newest run"),
                )]),
                ..FailureDetail::default()
            },
        );
    }
    let Some(keep_runs) = args.keep_runs.or(config.artifacts_keep_runs) else {
        return artifact_prune_rejected(
            command,
            &config,
            2,
            FailureDetail {
                kind: "retention_not_configured".to_string(),
                command: Some(command.to_string()),
                message: Some(
                    "no retention policy: artifacts.keep_runs is unset and --keep-runs was not \
                     passed"
                        .to_string(),
                ),
                path: Some(
                    config
                        .qa_dir
                        .join("config.yaml")
                        .to_string_lossy()
                        .into_owned(),
                ),
                expected: Some(json!(
                    "artifacts.keep_runs in qa/config.yaml or --keep-runs <n>"
                )),
                actual: Some(json!("unset")),
                details: Map::from_iter([(
                    "remediation".to_string(),
                    json!("set artifacts.keep_runs in qa/config.yaml or pass --keep-runs"),
                )]),
                ..FailureDetail::default()
            },
        );
    };

    let plan = match scenario_retention::plan_artifact_prune(&config.artifacts_dir, keep_runs) {
        Ok(plan) => plan,
        // Exit 1, not 2: the invocation was well formed, the artifacts root is what is wrong.
        Err(error) => {
            return artifact_prune_rejected(
                command,
                &config,
                1,
                FailureDetail {
                    kind: "artifact_prune_failed".to_string(),
                    command: Some(command.to_string()),
                    message: Some("failed to list the artifacts directory".to_string()),
                    path: Some(config.artifacts_dir.to_string_lossy().into_owned()),
                    expected: Some(json!("readable artifacts directory")),
                    actual: Some(json!(error)),
                    details: Map::from_iter([(
                        "remediation".to_string(),
                        json!(
                            "check that the path exists and is a readable directory, or point \
                               --artifact-dir at the right one"
                        ),
                    )]),
                    ..FailureDetail::default()
                },
            );
        }
    };

    let (removed, errors, bytes_reclaimed) = apply_artifact_prune_plan(&plan, args.dry_run);
    let failures: Vec<FailureDetail> = errors
        .iter()
        .map(|error| FailureDetail {
            kind: "artifact_prune_failed".to_string(),
            command: Some(command.to_string()),
            message: Some("failed to remove an artifact directory".to_string()),
            path: error
                .get("path")
                .and_then(Value::as_str)
                .map(str::to_string),
            expected: Some(json!("removable artifact directory")),
            actual: error.get("error").cloned(),
            details: Map::from_iter([(
                "remediation".to_string(),
                json!("remove the directory by hand or fix its permissions, then rerun"),
            )]),
            ..FailureDetail::default()
        })
        .collect();

    let counts = json!({
        "scenario_runs": removed.scenario_runs.len(),
        "bundle_runs": removed.bundle_runs.len(),
        "sessions": removed.sessions.len(),
    });
    let paths: Vec<Value> = removed.paths().map(|path| json!(path)).collect();
    let mut retention = Map::from_iter([
        ("keep_runs".to_string(), json!(keep_runs)),
        ("dry_run".to_string(), json!(args.dry_run)),
        ("bytes_reclaimed".to_string(), json!(bytes_reclaimed)),
        ("errors".to_string(), json!(errors)),
    ]);
    let (counts_key, paths_key) = if args.dry_run {
        ("would_remove", "would_remove_paths")
    } else {
        ("pruned", "removed_paths")
    };
    retention.insert(counts_key.to_string(), counts);
    retention.insert(paths_key.to_string(), json!(paths));

    if !cli.json {
        print_artifact_prune_human_summary(&removed, &errors, bytes_reclaimed, args.dry_run);
    }

    let ok = failures.is_empty();
    CommandResult {
        ok,
        command: command.to_string(),
        message: match (ok, args.dry_run) {
            (true, true) => "artifact prune dry run completed".to_string(),
            (true, false) => "artifact prune completed".to_string(),
            (false, _) => "artifact prune failed".to_string(),
        },
        data: Map::from_iter([
            ("project_root".to_string(), json!(config.project_root)),
            ("artifacts_dir".to_string(), json!(config.artifacts_dir)),
            ("retention".to_string(), Value::Object(retention)),
        ]),
        failures,
        exit_code: i32::from(!ok),
    }
}

/// Execute (or, for `--dry-run`, merely price) a prune plan.
///
/// Sizes are measured before deletion — afterwards there is nothing left to measure — and a
/// directory only contributes to `bytes_reclaimed` once it is actually gone, so a partial
/// failure cannot overstate what was freed.
fn apply_artifact_prune_plan(
    plan: &scenario_retention::ArtifactPrunePlan,
    dry_run: bool,
) -> (scenario_retention::ArtifactPrunePlan, Vec<Value>, u64) {
    let mut done = scenario_retention::ArtifactPrunePlan::default();
    let mut errors: Vec<Value> = Vec::new();
    let mut bytes = 0u64;
    for (planned, completed) in [
        (&plan.scenario_runs, &mut done.scenario_runs),
        (&plan.bundle_runs, &mut done.bundle_runs),
        (&plan.sessions, &mut done.sessions),
    ] {
        for path in planned {
            let size = scenario_retention::dir_size_bytes(path);
            if !dry_run && let Err(error) = std::fs::remove_dir_all(path) {
                errors.push(json!({
                    "path": path.to_string_lossy(),
                    "error": error.to_string(),
                }));
                continue;
            }
            bytes = bytes.saturating_add(size);
            completed.push(path.clone());
        }
    }
    (done, errors, bytes)
}

/// Human output: one line per category plus the byte total, so a person reading a terminal
/// learns what happened without reaching for `--json`.
fn print_artifact_prune_human_summary(
    removed: &scenario_retention::ArtifactPrunePlan,
    errors: &[Value],
    bytes_reclaimed: u64,
    dry_run: bool,
) {
    let verb = if dry_run { "to remove" } else { "removed" };
    let mut stdout = io::stdout().lock();
    let _ = writeln!(
        stdout,
        "scenario runs {verb}: {}",
        removed.scenario_runs.len()
    );
    let _ = writeln!(stdout, "bundle runs {verb}: {}", removed.bundle_runs.len());
    let _ = writeln!(stdout, "sessions {verb}: {}", removed.sessions.len());
    let _ = writeln!(stdout, "bytes reclaimed: {bytes_reclaimed}");
    for error in errors {
        let _ = writeln!(
            stdout,
            "error: {}: {}",
            error.get("path").and_then(Value::as_str).unwrap_or("?"),
            error.get("error").and_then(Value::as_str).unwrap_or("?")
        );
    }
}

/// An `artifact prune` that never got as far as deleting anything. `exit_code` separates the
/// two reasons that can happen: 2 when the invocation or the project configuration is wrong,
/// 1 when the artifacts root itself could not be read.
fn artifact_prune_rejected(
    command: &str,
    config: &ProjectConfig,
    exit_code: i32,
    failure: FailureDetail,
) -> CommandResult {
    CommandResult {
        ok: false,
        command: command.to_string(),
        message: "artifact prune failed".to_string(),
        data: Map::from_iter([
            ("project_root".to_string(), json!(config.project_root)),
            ("artifacts_dir".to_string(), json!(config.artifacts_dir)),
        ]),
        failures: vec![failure],
        exit_code,
    }
}

pub(crate) fn path_option_json(path: Option<&Path>) -> Value {
    path.map(|path| json!(path)).unwrap_or(Value::Null)
}

pub(crate) fn absolute_path(path: &Path) -> PathBuf {
    if path.is_absolute() {
        return path.to_path_buf();
    }
    std::env::current_dir()
        .unwrap_or_else(|_| PathBuf::from("."))
        .join(path)
}
