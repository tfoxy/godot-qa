use godot_qa_core::config::ProjectConfig;
use godot_qa_core::results::{CommandResult, FailureDetail};
use serde_json::{Value, json};
use std::collections::{BTreeSet, HashMap, HashSet, VecDeque};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, mpsc};
use std::thread;
use std::time::{Duration, Instant};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::bundle_schedule::{self, BundleSchedulePolicy, SchedulePlan};
use crate::bundle_selection::{self, BundleSelection};
use crate::scenario_doc::is_safe_artifact_segment;
use crate::{retention, run, scenario_doc};

#[derive(Debug, Clone)]
pub struct ScenarioBundleRunOptions {
    pub strict: bool,
    pub warm_strict: bool,
    pub headless_override: Option<bool>,
    pub audio_override: Option<bool>,
    pub command_line: String,
    pub retry_policy: BundleRetryPolicy,
    pub jobs: i64,
    /// Where the effective jobs value came from: "flag", "config"
    /// (qa/config.yaml scenario.jobs), or "default" (neither set).
    pub jobs_origin: String,
    /// Which scenarios of the bundle this run is restricted to. The default is empty,
    /// meaning the whole bundle.
    pub selection: BundleSelection,
    /// The order scenarios are dispatched in.
    pub schedule: BundleSchedulePolicy,
    /// Where the effective schedule came from: "flag", "config"
    /// (qa/config.yaml scenario.schedule), or "default" (neither set).
    pub schedule_origin: String,
    /// Whether the run prunes pooled live-session directories it abandoned. On by
    /// default; a caller that wants to inspect those sessions can turn it off.
    pub prune_pool_sessions: bool,
}

impl Default for ScenarioBundleRunOptions {
    fn default() -> Self {
        Self {
            strict: false,
            warm_strict: false,
            headless_override: None,
            audio_override: None,
            command_line: "godot-qa scenario bundle run <name> --json".to_string(),
            retry_policy: BundleRetryPolicy::default(),
            jobs: 0,
            jobs_origin: "default".to_string(),
            selection: BundleSelection::default(),
            schedule: BundleSchedulePolicy::default(),
            schedule_origin: "default".to_string(),
            prune_pool_sessions: true,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct BundleParallelism {
    pub raw_jobs: i64,
    pub resolved_jobs: usize,
    pub source: String,
    pub available_parallelism: usize,
    pub reserved_count: usize,
    pub runnable_count: usize,
    pub complete_queued: bool,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct BundleRetryPolicy {
    pub max_infrastructure_startup_attempts: usize,
    pub backoff_ms: u64,
}

impl Default for BundleRetryPolicy {
    fn default() -> Self {
        Self {
            max_infrastructure_startup_attempts: 1,
            backoff_ms: 0,
        }
    }
}

pub fn resolve_bundle_parallelism(raw_jobs: i64, runnable_count: usize) -> BundleParallelism {
    let available = thread::available_parallelism().map(usize::from).ok();
    resolve_bundle_parallelism_with_available(raw_jobs, available, runnable_count)
}

pub fn resolve_bundle_parallelism_with_available(
    raw_jobs: i64,
    available_parallelism: Option<usize>,
    runnable_count: usize,
) -> BundleParallelism {
    let available = available_parallelism
        .filter(|value| *value > 0)
        .unwrap_or(1);
    let fallback = available_parallelism.is_none();
    let reserved_count = if raw_jobs < 0 {
        raw_jobs.unsigned_abs().try_into().unwrap_or(usize::MAX)
    } else {
        0
    };
    let requested_jobs = if raw_jobs > 0 {
        raw_jobs.try_into().unwrap_or(usize::MAX)
    } else if raw_jobs == 0 {
        available
    } else {
        available.saturating_sub(reserved_count).max(1)
    };
    let source = match (raw_jobs.cmp(&0), fallback) {
        (std::cmp::Ordering::Greater, _) => "explicit",
        (std::cmp::Ordering::Equal, false) => "available_parallelism",
        (std::cmp::Ordering::Equal, true) => "available_parallelism_fallback",
        (std::cmp::Ordering::Less, false) => "reserved_parallelism",
        (std::cmp::Ordering::Less, true) => "reserved_parallelism_fallback",
    }
    .to_string();

    BundleParallelism {
        raw_jobs,
        resolved_jobs: requested_jobs.min(runnable_count),
        source,
        available_parallelism: available,
        reserved_count,
        runnable_count,
        complete_queued: true,
    }
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct BundleRunSummary {
    pub index: usize,
    pub scenario: String,
    pub scenario_path: String,
    pub required: bool,
    pub source_bundle: String,
    pub bundle_chain: Vec<String>,
    pub source_kind: String,
    pub run_root: Option<String>,
    pub godot_args: Option<Value>,
    pub project_args: Option<Value>,
    pub warm: Option<Value>,
    pub time_control: Option<Value>,
    pub launch_display: Option<Value>,
    pub status: String,
    pub ok: bool,
    pub result_path: Option<String>,
    pub logs_path: Option<String>,
    pub screenshot_paths: Vec<String>,
    pub diagnostic_paths: Vec<String>,
    pub artifacts: Option<Value>,
    pub snapshot_artifacts: Option<Value>,
    pub failures: Vec<FailureDetail>,
    pub attempts: Vec<BundleRunAttemptSummary>,
    pub final_attempt: usize,
    pub category: Option<String>,
    pub retry_policy: BundleRetryPolicy,
    pub retry_guidance: Option<String>,
    pub cleanup_status: Option<Value>,
    pub display_lifecycle: Option<Value>,
    pub attempt_artifacts: Vec<Value>,
    /// The child's own engine-reported wall clock (its `result.json`
    /// `data.duration_ms`), or None when the run produced no result to read it
    /// from. This is engine time, so it is always shorter than the CLI wall clock
    /// below.
    pub duration_ms: Option<f64>,
    /// The child's engine phase breakdown (`data.timing`: `scene_load_ms`,
    /// `first_frame_ms`, `steps_ms`, `asserts_ms`, `teardown_ms`,
    /// `engine_total_ms`), verbatim, or None when the engine reported none.
    pub timing: Option<Value>,
    /// CLI wall clock around this child in epoch milliseconds: taken before the
    /// first attempt's runner call and after the last one returns, so a bounded
    /// startup retry is inside the span rather than beside it.
    pub started_at_ms: u64,
    pub finished_at_ms: u64,
    /// Which worker lane ran the child (always `0` on the serial path), so a slow
    /// bundle can be read as a schedule rather than a list.
    pub lane: usize,
    /// How long the child sat in the bundle work queue before a lane picked it
    /// up, measured from the start of child execution.
    pub queue_wait_ms: f64,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct BundleArtifactSummary {
    pub children: Vec<BundleArtifactChildEntry>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct BundleArtifactChildEntry {
    pub scenario: String,
    pub required: bool,
    pub status: String,
    pub source_bundle: String,
    pub bundle_chain: Vec<String>,
    pub run_root: Option<String>,
    pub result_path: Option<String>,
    pub logs_path: Option<String>,
    pub screenshot_paths: Vec<String>,
    pub diagnostic_paths: Vec<String>,
    pub failure_artifact_paths: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub warning: Option<FailureDetail>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct BundleRunAttemptSummary {
    pub attempt: usize,
    pub run_root: Option<String>,
    pub result_path: Option<String>,
    pub logs_path: Option<String>,
    pub screenshot_paths: Vec<String>,
    pub diagnostic_paths: Vec<String>,
    pub godot_args: Option<Value>,
    pub project_args: Option<Value>,
    pub time_control: Option<Value>,
    pub launch_display: Option<Value>,
    pub display_lifecycle: Option<Value>,
    pub cleanup_status: Option<Value>,
    pub status: String,
    pub ok: bool,
    pub artifacts: Option<Value>,
    pub snapshot_artifacts: Option<Value>,
    pub failures: Vec<FailureDetail>,
    pub category: Option<String>,
    pub retryable: bool,
    pub retry_decision: String,
    pub retry_guidance: Option<String>,
    /// Same meaning as on `BundleRunSummary`, scoped to this one attempt: a
    /// retried child has one span per attempt, so the cost of a failed startup is
    /// attributable instead of hidden inside the run's total.
    pub duration_ms: Option<f64>,
    pub timing: Option<Value>,
    pub started_at_ms: u64,
    pub finished_at_ms: u64,
}

#[derive(Debug, Clone)]
struct BundleChildTask {
    index: usize,
    scenario_name: String,
    required: bool,
    source_bundle: String,
    bundle_chain: Vec<String>,
    source_kind: BundleReferenceSource,
    scenario_path: PathBuf,
    scenario: Value,
}

/// What a bundle RUNNER wants folded into the bundle result after every child has
/// run but BEFORE any summary is computed or written: extra bundle-level warnings
/// and extra `data` entries.
///
/// This exists because a runner can own process-level resources the bundle knows
/// nothing about — the warm session pool is the live case. Tearing that pool down
/// here (rather than after `run_scenario_bundle_with_runner` returns) is what makes
/// its cost and its failures part of `summary.json` and the CLI JSON alike, instead
/// of happening invisibly once the bundle already reported success.
#[derive(Debug, Default)]
pub struct BundlePostRun {
    /// Appended to the bundle's own warnings. Bundle `--strict` never promotes
    /// warnings to failures, so a hook cannot change a bundle's verdict.
    pub warnings: Vec<FailureDetail>,
    /// Merged into the bundle result's `data`; an existing key always wins, so a
    /// hook can never rewrite a bundle contract field.
    pub data: serde_json::Map<String, Value>,
}

#[derive(Debug, Default)]
struct BundlePreparedChildren {
    warnings: Vec<FailureDetail>,
    failures: Vec<IndexedFailure>,
    tasks: Vec<BundleChildTask>,
}

#[derive(Debug)]
struct IndexedFailure {
    index: usize,
    order: usize,
    failure: FailureDetail,
}

/// The two dispatch-shaping decisions a bundle run made, carried into the artifact
/// writer so `meta.json` and `summary.json` record exactly what the command JSON
/// reports. Both are always present, so a consumer never has to infer "unfiltered"
/// or "manifest order" from a missing key.
#[derive(Debug, Clone)]
struct BundleRunAnnotations {
    /// `data.selection` from [`bundle_selection::ResolvedBundleSelection`].
    selection: Value,
    /// `data.scheduling` from [`SchedulePlan::to_json`].
    scheduling: Value,
}

/// Count the runnable scenario tasks a bundle expands to, WITHOUT running any of
/// them (no Godot launched). Mirrors the resolution `run_scenario_bundle_with_runner`
/// does before dispatch (`list_bundles` -> `expand_bundle` -> `prepare_bundle_child_tasks`),
/// returning `prepared.tasks.len()` — the same value fed to `resolve_bundle_parallelism`.
///
/// Returns `None` if the bundle cannot be resolved (listing failure, unknown or
/// duplicate name, expansion failure) so callers fall through to the normal path,
/// which surfaces the real failure rather than masking it here.
pub fn resolve_bundle_runnable_count(
    config: &ProjectConfig,
    name: &str,
    options: &ScenarioBundleRunOptions,
) -> Option<usize> {
    let command = "scenario bundle run";
    let listed = list_bundles(config);
    if !listed.failures.is_empty() {
        return None;
    }
    let matches: Vec<&BundleManifest> = listed.bundles.iter().filter(|b| b.name == name).collect();
    if matches.len() != 1 {
        return None;
    }
    let expanded = expand_bundle(matches[0], &listed.bundles, command).ok()?;
    // Selection narrows what "runnable" means, so the warm skip decision has to see the
    // FILTERED count: `--only one_scenario` on a 100-scenario gate runs one child and must
    // take the same below-parallelism skip a one-scenario bundle would. An unresolvable
    // selection returns `None` for the same reason an unresolvable bundle does — the normal
    // path is where that failure is reported, not here.
    let (selection, _) = bundle_selection::resolve_bundle_selection(
        &config.artifacts_dir,
        name,
        &expanded_member_names(&expanded),
        &options.selection,
    )
    .ok()?;
    let prepared = prepare_bundle_child_tasks(
        config,
        expanded,
        options,
        command,
        selection.selected.as_ref(),
    );
    Some(prepared.tasks.len())
}

/// The bundle's expansion as a plain name list, in expansion order — the member list
/// [`bundle_selection::resolve_bundle_selection`] matches `--only` names and previous-run
/// failures against, and whose indices it returns.
fn expanded_member_names(expanded: &[ExpandedBundleChild]) -> Vec<String> {
    expanded
        .iter()
        .map(|child| child.scenario.clone())
        .collect()
}

pub fn run_scenario_bundle(
    config: &ProjectConfig,
    name: &str,
    options: &ScenarioBundleRunOptions,
) -> CommandResult {
    run_scenario_bundle_with_runner(
        config,
        name,
        options,
        |config, scenario_path, scenario, command_line, headless_override, audio_override| {
            run::run_scenario_once(
                config,
                scenario_path,
                scenario,
                command_line,
                headless_override,
                audio_override,
                run::VisualRiskExecutionOptions::default(),
            )
        },
        BundlePostRun::default,
    )
}

pub fn run_scenario_bundle_with_runner<F, P>(
    config: &ProjectConfig,
    name: &str,
    options: &ScenarioBundleRunOptions,
    runner: F,
    post_run: P,
) -> CommandResult
where
    F: Fn(&ProjectConfig, &Path, &Value, &str, Option<bool>, Option<bool>) -> CommandResult + Sync,
    P: FnOnce() -> BundlePostRun,
{
    let command = "scenario bundle run";
    // The bundle wall clock starts here, before discovery: a bundle that spends
    // seconds resolving 101 manifests has to be able to say so.
    let bundle_started = Instant::now();
    let bundle_started_at_ms = epoch_millis();
    let listed = list_bundles(config);
    if !listed.failures.is_empty() {
        // A runner can already own process-level resources before the bundle is even
        // resolved — the warm pool is seeded before this call — so every abandoning
        // return runs the post-run hook. Nothing it reports can be published from these
        // shapes, but a leaked pool would leave Godot processes running after the CLI
        // exits, which is far worse than losing the (empty) evidence.
        let _ = post_run();
        return CommandResult {
            ok: false,
            command: command.to_string(),
            message: "scenario bundle run failed".to_string(),
            data: serde_json::Map::new(),
            failures: listed
                .failures
                .into_iter()
                .map(|mut failure| {
                    failure.command = Some(command.to_string());
                    failure
                })
                .collect(),
            exit_code: 1,
        };
    }
    let matches: Vec<&BundleManifest> = listed.bundles.iter().filter(|b| b.name == name).collect();
    if matches.is_empty() {
        let _ = post_run();
        if let Some(mut failure) = invalid_manifest_failure_for_name(config, name) {
            failure.command = Some(command.to_string());
            return CommandResult {
                ok: false,
                command: command.to_string(),
                message: "scenario bundle run failed".to_string(),
                data: serde_json::Map::new(),
                failures: vec![failure],
                exit_code: 1,
            };
        }
        return CommandResult {
            ok: false,
            command: command.to_string(),
            message: "scenario bundle run failed".to_string(),
            data: serde_json::Map::new(),
            failures: vec![FailureDetail {
                kind: "missing_bundle".to_string(),
                command: Some(command.to_string()),
                path: Some(
                    config
                        .project_root
                        .join("qa/gates")
                        .to_string_lossy()
                        .into_owned(),
                ),
                message: Some(format!("bundle '{name}' was not found")),
                expected: Some(json!("existing bundle by unique name")),
                actual: Some(json!(name)),
                ..FailureDetail::default()
            }],
            exit_code: 1,
        };
    }
    if matches.len() > 1 {
        let _ = post_run();
        return CommandResult {
            ok: false,
            command: command.to_string(),
            message: "scenario bundle run failed".to_string(),
            data: serde_json::Map::new(),
            failures: vec![FailureDetail {
                kind: "duplicate_bundle_name".to_string(),
                command: Some(command.to_string()),
                path: Some(matches[0].path.to_string_lossy().into_owned()),
                message: Some(format!("bundle name '{name}' is duplicated")),
                expected: Some(json!("unique bundle names")),
                actual: Some(json!(
                    matches.iter().map(|b| b.path.clone()).collect::<Vec<_>>()
                )),
                ..FailureDetail::default()
            }],
            exit_code: 1,
        };
    }
    let bundle = matches[0];
    let expanded = match expand_bundle(bundle, &listed.bundles, command) {
        Ok(expanded) => expanded,
        Err(failure) => {
            let _ = post_run();
            return CommandResult {
                ok: false,
                command: command.to_string(),
                message: "scenario bundle run failed".to_string(),
                data: serde_json::Map::new(),
                failures: vec![failure],
                exit_code: 1,
            };
        }
    };
    // Which of the expanded children actually run. Resolved here, before anything is
    // prepared or written, so a bad `--only` name or a missing `--failed-only` baseline
    // costs nothing and leaves no run directory behind.
    let (selection, selection_warnings) = match bundle_selection::resolve_bundle_selection(
        &config.artifacts_dir,
        &bundle.name,
        &expanded_member_names(&expanded),
        &options.selection,
    ) {
        Ok(resolved) => resolved,
        Err(mut failure) => {
            let _ = post_run();
            failure.command = Some(command.to_string());
            return CommandResult {
                ok: false,
                command: command.to_string(),
                message: "scenario bundle run failed".to_string(),
                data: serde_json::Map::new(),
                failures: vec![failure],
                // A selection that cannot be resolved is a bad invocation, not a failed
                // gate: exit 2 keeps "the bundle ran and something failed" (1) distinct
                // from "the bundle never ran".
                exit_code: 2,
            };
        }
    };
    let prepared = prepare_bundle_child_tasks(
        config,
        expanded,
        options,
        command,
        selection.selected.as_ref(),
    );
    let parallelism = resolve_bundle_parallelism(options.jobs, prepared.tasks.len());
    // Dispatch order is decided after parallelism, because a serial run has nothing to
    // gain from reordering and must stay in plain expansion order.
    let (tasks, schedule_plan) = schedule_bundle_tasks(
        config,
        &bundle.name,
        prepared.tasks,
        options,
        parallelism.resolved_jobs,
    );
    let prepare_ms = millis_since(bundle_started);
    let execute_started = Instant::now();
    let mut child_summaries =
        execute_bundle_child_tasks(config, tasks, options, parallelism.resolved_jobs, &runner);
    let execute_ms = millis_since(execute_started);
    child_summaries.sort_by_key(|summary| summary.index);

    // Every child is done; run the runner's post-run hook BEFORE the summary is
    // computed or written, so anything it reports (and anything it tears down) is
    // inside the artifacts rather than after them.
    let post_run = post_run();
    // The warm runner already measured its pool lifecycle; lift those two spans
    // into the bundle timing rather than plumbing a second channel for them.
    let (pool_seed_ms, pool_shutdown_ms) = bundle_pool_timing(&post_run.data);

    let failures = ordered_bundle_failures(prepared.failures, &child_summaries);
    let failure_clusters = cluster_failures(&failures, child_summaries.len());
    let mut warnings = prepared.warnings;
    // Selection warnings (today only `selection_empty`) describe the run itself, so they
    // have to be in `warnings` BEFORE the artifacts are written: a run that executed
    // nothing must say so in its own summary.json, not only in the command JSON.
    warnings.extend(
        selection_warnings
            .iter()
            .filter_map(|warning| serde_json::from_value::<FailureDetail>(warning.clone()).ok()),
    );
    warnings.extend(post_run.warnings);
    let artifact_index = bundle_artifact_summary(&child_summaries, &warnings);
    let timing = BundleTiming {
        started: bundle_started,
        started_at_ms: bundle_started_at_ms,
        prepare_ms,
        execute_ms,
        // Nothing has been written yet, and summary.json is written from inside
        // the artifact phase — so the snapshot it carries necessarily stops here.
        artifacts_ms: 0.0,
        pool_seed_ms,
        pool_shutdown_ms,
    };
    let annotations = BundleRunAnnotations {
        selection: selection.json.clone(),
        scheduling: schedule_plan.to_json(),
    };
    let artifacts_started = Instant::now();
    let bundle_artifacts = prepare_bundle_artifacts(
        config,
        bundle,
        options,
        &parallelism,
        &warnings,
        &failures,
        &child_summaries,
        &artifact_index,
        &timing.to_json(&child_summaries, parallelism.resolved_jobs),
        &failure_clusters,
        &annotations,
    );
    let timing = BundleTiming {
        artifacts_ms: millis_since(artifacts_started),
        ..timing
    };
    // Retention is housekeeping that runs after the bundle's artifacts are already on disk.
    // It does nothing unless the project set `artifacts.keep_runs`, and a prune failure is
    // reported as a warning: it must never change the bundle verdict or the exit code.
    let retention = apply_bundle_retention(
        config,
        &bundle.name,
        &child_summaries,
        options.prune_pool_sessions,
    );
    warnings.extend(bundle_retention_warning(config, retention.as_ref()));
    let mut data = serde_json::Map::new();
    if let Some(retention) = retention {
        data.insert("retention".to_string(), retention);
    }
    data.insert("bundle".to_string(), json!(bundle.name));
    data.insert("path".to_string(), json!(bundle.path));
    data.insert(
        "required_bundles".to_string(),
        json!(bundle.required_bundles),
    );
    data.insert(
        "required_scenarios".to_string(),
        json!(bundle.required_scenarios),
    );
    data.insert(
        "optional_scenarios".to_string(),
        json!(bundle.optional_scenarios),
    );
    data.insert(
        "recommended_manual_checks".to_string(),
        json!(bundle.recommended_manual_checks),
    );
    data.insert("runs".to_string(), json!(child_summaries));
    data.insert(
        "timing".to_string(),
        timing.to_json(&child_summaries, parallelism.resolved_jobs),
    );
    data.insert("failure_clusters".to_string(), json!(failure_clusters));
    data.insert("parallelism".to_string(), json!(parallelism));
    data.insert("jobs_origin".to_string(), json!(options.jobs_origin));
    data.insert("selection".to_string(), annotations.selection.clone());
    data.insert("scheduling".to_string(), annotations.scheduling.clone());
    data.insert("artifact_index".to_string(), json!(artifact_index));
    data.insert("artifact_summary".to_string(), json!(artifact_index));
    data.insert("artifacts".to_string(), json!(bundle_artifacts));
    if !warnings.is_empty() {
        data.insert("warnings".to_string(), json!(warnings));
    }
    // Additive only: a bundle contract field the runner also produced always wins.
    for (key, value) in post_run.data {
        data.entry(key).or_insert(value);
    }
    let ok = failures.is_empty();
    // Only a FILTERED run says so. An unfiltered bundle keeps its historical message
    // byte-for-byte, so nothing downstream that matches on it has to change.
    let filtered = if selection.selected.is_some() {
        format!(
            " (filtered: {} of {})",
            selection.selected_count, selection.member_count
        )
    } else {
        String::new()
    };
    CommandResult {
        ok,
        command: command.to_string(),
        message: if ok {
            format!("scenario bundle run completed{filtered}")
        } else {
            format!("scenario bundle run failed{filtered}")
        },
        data,
        failures,
        exit_code: if ok { 0 } else { 1 },
    }
}

/// Order the prepared child tasks for dispatch and describe the decision.
///
/// Reordering only ever pays off when several children run at once, so the read of the
/// bundle's duration history is skipped entirely unless the `longest-first` policy is in
/// effect AND more than one worker will run. That keeps the serial path free of any I/O it
/// cannot benefit from and keeps `--jobs 1` in plain expansion order, which is what makes a
/// serial bundle run reproducible against its manifest.
///
/// `applied` is true only when the order was actually computed from history. A bundle with
/// no usable history — the first-ever run — is reported as not applied and keeps expansion
/// order, which is also exactly what `order_for_dispatch` would have produced.
///
/// This never touches `data.runs[]`: child summaries are sorted back into expansion index
/// order after execution, so dispatch order is invisible in the result.
fn schedule_bundle_tasks(
    config: &ProjectConfig,
    bundle_name: &str,
    tasks: Vec<BundleChildTask>,
    options: &ScenarioBundleRunOptions,
    resolved_jobs: usize,
) -> (Vec<BundleChildTask>, SchedulePlan) {
    let policy = options.schedule;
    let unapplied = SchedulePlan {
        policy,
        origin: options.schedule_origin.clone(),
        applied: false,
        history_runs: 0,
        predicted: 0,
        unknown: 0,
    };
    if policy != BundleSchedulePolicy::LongestFirst || resolved_jobs <= 1 {
        return (tasks, unapplied);
    }
    let history = bundle_schedule::read_duration_history(
        &config.artifacts_dir,
        bundle_name,
        bundle_schedule::HISTORY_RUNS,
    );
    if history.is_empty() {
        return (tasks, unapplied);
    }
    let history_runs = history.history_runs();
    let (ordered, predicted, unknown) =
        bundle_schedule::order_for_dispatch(tasks, |task| task.scenario_name.as_str(), &history);
    (
        ordered,
        SchedulePlan {
            policy,
            origin: options.schedule_origin.clone(),
            applied: true,
            history_runs,
            predicted,
            unknown,
        },
    )
}

/// Turn a bundle's expansion into runnable child tasks.
///
/// `selected` restricts the run to those expansion indices; `None` runs everything. The
/// filter is applied at the TOP of the loop, before a child is looked up or loaded, so a
/// deselected scenario contributes nothing at all: no `missing_scenario` failure, no
/// preparation warning, no strict violation. Narrowing a run must never change the verdict
/// of a scenario it did not run. The `index` handed to each task stays the EXPANSION index,
/// which keeps `data.runs[]` ordering and every failure's identity stable whether or not
/// the run was filtered.
fn prepare_bundle_child_tasks(
    config: &ProjectConfig,
    expanded: Vec<ExpandedBundleChild>,
    options: &ScenarioBundleRunOptions,
    command: &str,
    selected: Option<&BTreeSet<usize>>,
) -> BundlePreparedChildren {
    let mut prepared = BundlePreparedChildren::default();
    // One discovery pass for the whole bundle: per-name lookup would re-walk and
    // re-read the entire scenarios tree once per child.
    let scenario_index = scenario_doc::ScenarioIndex::load(&config.scenarios_dir);
    for (index, child_entry) in expanded.into_iter().enumerate() {
        if let Some(selected) = selected
            && !selected.contains(&index)
        {
            continue;
        }
        let scenario_name = child_entry.scenario.clone();
        let required = child_entry.required;
        let paths = scenario_index.paths_for(&scenario_name);
        if paths.is_empty() {
            let failure = FailureDetail {
                kind: "missing_scenario".to_string(),
                command: Some(command.to_string()),
                path: Some(child_entry.manifest_path.to_string_lossy().into_owned()),
                message: Some(format!("scenario '{scenario_name}' was not found")),
                expected: Some(json!("existing scenario file by declared name or stem")),
                actual: Some(json!(scenario_name)),
                details: serde_json::Map::from_iter([
                    (
                        "source_bundle".to_string(),
                        json!(child_entry.source_bundle),
                    ),
                    ("bundle_chain".to_string(), json!(child_entry.bundle_chain)),
                    (
                        "source_kind".to_string(),
                        json!(child_entry.source_kind.as_str()),
                    ),
                    ("required".to_string(), json!(required)),
                ]),
                ..FailureDetail::default()
            };
            if required || options.strict {
                prepared.failures.push(IndexedFailure {
                    index,
                    order: 0,
                    failure,
                });
            } else {
                prepared.warnings.push(failure);
            }
            continue;
        }

        let scenario_path = paths[0].clone();
        let scenario = match scenario_doc::load_and_normalize_scenario_document(&scenario_path) {
            Ok(v) => v,
            Err(error) => {
                prepared.failures.push(IndexedFailure {
                    index,
                    order: 0,
                    failure: FailureDetail {
                        kind: error.failure.kind,
                        command: Some(command.to_string()),
                        path: error.failure.path,
                        message: error.failure.message,
                        expected: error.failure.expected,
                        actual: error.failure.actual,
                        ..FailureDetail::default()
                    },
                });
                continue;
            }
        };

        prepared.tasks.push(BundleChildTask {
            index,
            scenario_name,
            required,
            source_bundle: child_entry.source_bundle,
            bundle_chain: child_entry.bundle_chain,
            source_kind: child_entry.source_kind,
            scenario_path,
            scenario,
        });
    }
    prepared
}

fn execute_bundle_child_tasks<F>(
    config: &ProjectConfig,
    tasks: Vec<BundleChildTask>,
    options: &ScenarioBundleRunOptions,
    worker_count: usize,
    runner: &F,
) -> Vec<BundleRunSummary>
where
    F: Fn(&ProjectConfig, &Path, &Value, &str, Option<bool>, Option<bool>) -> CommandResult + Sync,
{
    if tasks.is_empty() {
        return Vec::new();
    }
    // Both paths time the queue from the same origin, so `queue_wait_ms` means the
    // same thing serially (how much of the bundle ran before this child) as it does
    // in parallel (how long this child waited for a free lane).
    let execute_started = Instant::now();
    if worker_count <= 1 {
        return tasks
            .into_iter()
            .map(|task| {
                let queue_wait_ms = millis_since(execute_started);
                execute_bundle_child_task(config, task, options, runner, 0, queue_wait_ms)
            })
            .collect();
    }

    let queue = Arc::new(Mutex::new(VecDeque::from(tasks)));
    let (sender, receiver) = mpsc::channel();
    thread::scope(|scope| {
        for lane in 0..worker_count {
            let queue = Arc::clone(&queue);
            let sender = sender.clone();
            scope.spawn(move || {
                loop {
                    let task = {
                        // Recover from poisoning instead of cascading the panic to every
                        // worker: the queue is a plain VecDeque of pending child tasks, so
                        // a panicked worker still leaves it in a consistent, drainable state.
                        let mut queue = queue
                            .lock()
                            .unwrap_or_else(|poisoned| poisoned.into_inner());
                        queue.pop_front()
                    };
                    let Some(task) = task else {
                        break;
                    };
                    let queue_wait_ms = millis_since(execute_started);
                    let summary = execute_bundle_child_task(
                        config,
                        task,
                        options,
                        runner,
                        lane,
                        queue_wait_ms,
                    );
                    if sender.send(summary).is_err() {
                        break;
                    }
                }
            });
        }
        drop(sender);
        receiver.into_iter().collect()
    })
}

fn execute_bundle_child_task<F>(
    config: &ProjectConfig,
    task: BundleChildTask,
    options: &ScenarioBundleRunOptions,
    runner: &F,
    lane: usize,
    queue_wait_ms: f64,
) -> BundleRunSummary
where
    F: Fn(&ProjectConfig, &Path, &Value, &str, Option<bool>, Option<bool>) -> CommandResult,
{
    let max_attempts = options
        .retry_policy
        .max_infrastructure_startup_attempts
        .max(1);
    let mut attempts = Vec::new();
    let mut final_child = None;
    let started_at_ms = epoch_millis();
    for attempt_number in 1..=max_attempts {
        let attempt_started_at_ms = epoch_millis();
        let child = runner(
            config,
            &task.scenario_path,
            &task.scenario,
            options.command_line.as_str(),
            options.headless_override,
            options.audio_override,
        );
        let mut attempt = bundle_attempt_summary(
            attempt_number,
            &child,
            attempt_started_at_ms,
            epoch_millis(),
        );
        let retryable = !child.ok && is_retryable_startup_infrastructure_failure(&child);
        if retryable && attempt_number < max_attempts {
            attempt.retry_decision = "retrying".to_string();
            attempts.push(attempt);
            if options.retry_policy.backoff_ms > 0 {
                thread::sleep(Duration::from_millis(options.retry_policy.backoff_ms));
            }
            continue;
        }
        attempt.retry_decision = if retryable && attempt_number >= max_attempts {
            "exhausted".to_string()
        } else {
            "not_retryable".to_string()
        };
        attempts.push(attempt);
        final_child = Some(child);
        break;
    }
    let finished_at_ms = epoch_millis();
    let child = final_child.unwrap_or_else(|| CommandResult {
        ok: false,
        command: "scenario run".to_string(),
        message: "scenario bundle child attempt loop did not produce a result".to_string(),
        data: serde_json::Map::new(),
        failures: vec![FailureDetail {
            kind: "bundle_child_attempt_missing".to_string(),
            command: Some("scenario bundle run".to_string()),
            path: Some(task.scenario_path.to_string_lossy().into_owned()),
            message: Some("bundle child attempt loop did not produce a result".to_string()),
            expected: Some(json!("at least one child attempt")),
            actual: Some(json!(0)),
            ..FailureDetail::default()
        }],
        exit_code: 1,
    });
    let artifacts = child.data.get("artifacts").cloned();
    let run_root = artifact_string(&artifacts, "run_root");
    let result_path = artifact_string(&artifacts, "result_path");
    let logs_path = artifact_string(&artifacts, "logs_path");
    let screenshot_paths = child_screenshot_paths(&child);
    let diagnostic_paths = child_diagnostic_paths(&child);
    let final_attempt = attempts.last().map(|attempt| attempt.attempt).unwrap_or(1);
    let category = child_failure_category(&child);
    let retry_guidance = child_retry_guidance(&child);
    let cleanup_status = child_cleanup_status(&child);
    let display_lifecycle = child_display_lifecycle(&child);
    let attempt_artifacts = attempts
        .iter()
        .filter_map(|attempt| attempt.artifacts.clone())
        .collect();
    BundleRunSummary {
        index: task.index,
        scenario: task.scenario_name,
        scenario_path: task.scenario_path.to_string_lossy().into_owned(),
        required: task.required,
        source_bundle: task.source_bundle,
        bundle_chain: task.bundle_chain,
        source_kind: task.source_kind.as_str().to_string(),
        run_root,
        godot_args: child.data.get("godot_args").cloned(),
        project_args: Some(child_project_args(&child)),
        warm: child.data.get("warm").cloned(),
        time_control: child.data.get("time_control").cloned(),
        launch_display: child.data.get("launch_display").cloned(),
        status: if child.ok { "passed" } else { "failed" }.to_string(),
        ok: child.ok,
        result_path,
        logs_path,
        screenshot_paths,
        diagnostic_paths,
        artifacts,
        snapshot_artifacts: child.data.get("snapshot_artifacts").cloned(),
        failures: child.failures.clone(),
        attempts,
        final_attempt,
        category,
        retry_policy: options.retry_policy.clone(),
        retry_guidance,
        cleanup_status,
        display_lifecycle,
        attempt_artifacts,
        duration_ms: child.data.get("duration_ms").and_then(Value::as_f64),
        timing: child.data.get("timing").cloned(),
        started_at_ms,
        finished_at_ms,
        lane,
        queue_wait_ms,
    }
}

/// The bundle-level `failures[]`, ordered by (child index, order within the child).
///
/// Every failure raised by a CHILD is stamped with the `scenario` it came from and
/// that run's `run_root`, because the flat failure list is otherwise unattributable:
/// on a 101-scenario gate the same `type` from six scenarios reads as six identical
/// entries. Preparation failures (`missing_scenario` and friends) are left alone —
/// they already carry their own bundle-side context and have no run to point at. An
/// existing key always wins, so a failure that already named its scenario keeps what
/// it said.
fn ordered_bundle_failures(
    prepared_failures: Vec<IndexedFailure>,
    runs: &[BundleRunSummary],
) -> Vec<FailureDetail> {
    let mut indexed = Vec::new();
    for failure in prepared_failures {
        indexed.push((failure.index, failure.order, failure.failure));
    }
    for run in runs {
        if !run.ok {
            for (offset, mut failure) in run.failures.iter().cloned().enumerate() {
                failure
                    .details
                    .entry("scenario")
                    .or_insert_with(|| json!(run.scenario));
                if let Some(run_root) = &run.run_root {
                    failure
                        .details
                        .entry("run_root")
                        .or_insert_with(|| json!(run_root));
                }
                indexed.push((run.index, offset + 1, failure));
            }
        }
    }
    indexed.sort_by_key(|(index, order, _)| (*index, *order));
    indexed.into_iter().map(|(_, _, failure)| failure).collect()
}

/// How many distinct scenarios a repeated failure must span before it is worth
/// reporting as one shared cause instead of N independent ones.
const FAILURE_CLUSTER_MIN_SCENARIOS: usize = 3;
/// How many scenario names a cluster lists. The count is exact; the list is a
/// sample, because a 101-scenario gate can put every scenario in one cluster.
const FAILURE_CLUSTER_MAX_LISTED_SCENARIOS: usize = 10;
/// How many runs `timing.slowest[]` names. Enough to see the tail that dominates
/// a bundle without turning the timing block into a second copy of `runs`.
const BUNDLE_SLOWEST_RUNS: usize = 10;

fn epoch_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|elapsed| u64::try_from(elapsed.as_millis()).ok())
        .unwrap_or(0)
}

/// Wall-clock milliseconds since `started`, rounded to microseconds.
///
/// The rounding is not cosmetic. Every span here is published twice — in the
/// command JSON and in `summary.json` — and consumers compare the two. A
/// full-precision f64 does not survive that JSON round-trip bit-for-bit, so
/// `0.054419999999999996` written to disk reads back as `0.05442` and the two
/// copies stop being equal. Microsecond resolution is far finer than anything
/// these spans measure and round-trips exactly.
fn millis_since(started: Instant) -> f64 {
    round_to(started.elapsed().as_secs_f64() * 1000.0, 1000.0)
}

fn round_to(value: f64, scale: f64) -> f64 {
    (value * scale).round() / scale
}

/// The wall-clock accounting for one bundle run, published as `data.timing` in the
/// command JSON and as `timing` in `summary.json`.
///
/// It exists because "which scenario is slow" was previously answerable only by
/// reading every child `result.json`: the bundle had no clock of its own and the
/// per-scenario engine timing was never aggregated.
///
/// The two consumers see slightly different numbers on purpose. `summary.json` is
/// written from INSIDE the artifact phase, so the snapshot it carries has
/// `artifacts_ms: 0` and a `wall_ms` that stops before the summary write itself;
/// the command JSON is built afterwards and carries the final numbers.
#[derive(Debug, Clone, Copy)]
struct BundleTiming {
    started: Instant,
    started_at_ms: u64,
    prepare_ms: f64,
    execute_ms: f64,
    artifacts_ms: f64,
    pool_seed_ms: Option<f64>,
    pool_shutdown_ms: Option<f64>,
}

impl BundleTiming {
    fn to_json(self, runs: &[BundleRunSummary], resolved_jobs: usize) -> Value {
        let sum_child_duration_ms = round_to(
            runs.iter().filter_map(|run| run.duration_ms).sum::<f64>(),
            1000.0,
        );
        // Lane occupancy, not a score: how much of the worker time the bundle held
        // was actually spent inside Godot. Advisory and deliberately unclamped, and
        // null rather than a division by zero when nothing ran.
        let parallel_efficiency = if self.execute_ms > 0.0 && resolved_jobs > 0 {
            Some(round_to(
                sum_child_duration_ms / (self.execute_ms * resolved_jobs as f64),
                1_000_000.0,
            ))
        } else {
            None
        };
        json!({
            "started_at_ms": self.started_at_ms,
            "finished_at_ms": epoch_millis(),
            "wall_ms": millis_since(self.started),
            "prepare_ms": self.prepare_ms,
            "execute_ms": self.execute_ms,
            "artifacts_ms": self.artifacts_ms,
            "sum_child_duration_ms": sum_child_duration_ms,
            "parallel_efficiency": parallel_efficiency,
            "slowest": slowest_runs(runs),
            "pool_seed_ms": self.pool_seed_ms,
            "pool_shutdown_ms": self.pool_shutdown_ms,
        })
    }
}

/// The pool lifecycle spans a warm runner already measured, lifted out of its
/// post-run `data.warm_pool` so bundle timing has one home. Both are None on a
/// cold bundle, which has no pool at all.
fn bundle_pool_timing(
    post_run_data: &serde_json::Map<String, Value>,
) -> (Option<f64>, Option<f64>) {
    let pool = post_run_data.get("warm_pool");
    (
        pool.and_then(|pool| pool.get("seed_boot_ms"))
            .and_then(Value::as_f64),
        pool.and_then(|pool| pool.get("shutdown_ms"))
            .and_then(Value::as_f64),
    )
}

/// The slowest children by engine `duration_ms`, worst first, with the two engine
/// phases that usually explain them (`scene_load_ms`, `steps_ms`) and the run root
/// to read the rest from. Runs whose engine reported no duration are omitted
/// rather than sorted as zero, so the list never implies a measurement that does
/// not exist.
fn slowest_runs(runs: &[BundleRunSummary]) -> Vec<Value> {
    let mut rows: Vec<(f64, Value)> = runs
        .iter()
        .filter_map(|run| {
            let duration_ms = run.duration_ms?;
            Some((
                duration_ms,
                json!({
                    "scenario": run.scenario,
                    "duration_ms": duration_ms,
                    "scene_load_ms": child_timing_number(run.timing.as_ref(), "scene_load_ms"),
                    "steps_ms": child_timing_number(run.timing.as_ref(), "steps_ms"),
                    "run_root": run.run_root,
                }),
            ))
        })
        .collect();
    // A stable sort keeps equal durations in bundle expansion order, so repeated
    // runs of the same bundle produce the same list.
    rows.sort_by(|left, right| {
        right
            .0
            .partial_cmp(&left.0)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    rows.truncate(BUNDLE_SLOWEST_RUNS);
    rows.into_iter().map(|(_, row)| row).collect()
}

fn child_timing_number(timing: Option<&Value>, key: &str) -> Value {
    timing
        .and_then(|timing| timing.get(key))
        .filter(|value| value.is_number())
        .cloned()
        .unwrap_or(Value::Null)
}

#[derive(Debug)]
struct FailureCluster {
    kind: String,
    key: String,
    failure_count: usize,
    scenarios: Vec<String>,
}

/// Collapse repeated CHILD failures into one root-cause line each.
///
/// One missing provider registration once produced 306 identical
/// `provider_not_registered` failures across a single gate, and nothing in the
/// output said they were one bug. Failures themselves are never suppressed,
/// reordered, or rewritten by this: the clusters are an additive index published
/// alongside them.
///
/// Failures group by (`type`, `provider` when the failure names one, else
/// `target`), and a group is only reported once it spans at least
/// `FAILURE_CLUSTER_MIN_SCENARIOS` distinct scenarios — below that, the
/// per-scenario failures are already the clearest reading. Preparation failures
/// carry no `scenario` and never cluster.
fn cluster_failures(failures: &[FailureDetail], total_scenarios: usize) -> Vec<Value> {
    let mut groups: Vec<FailureCluster> = Vec::new();
    for failure in failures {
        let Some(scenario) = failure.details.get("scenario").and_then(Value::as_str) else {
            continue;
        };
        let key = failure
            .details
            .get("provider")
            .and_then(Value::as_str)
            .or(failure.target.as_deref())
            .unwrap_or_default()
            .to_string();
        let index = match groups
            .iter()
            .position(|group| group.kind == failure.kind && group.key == key)
        {
            Some(index) => index,
            None => {
                groups.push(FailureCluster {
                    kind: failure.kind.clone(),
                    key,
                    failure_count: 0,
                    scenarios: Vec::new(),
                });
                groups.len() - 1
            }
        };
        let group = &mut groups[index];
        group.failure_count += 1;
        if !group.scenarios.iter().any(|name| name == scenario) {
            group.scenarios.push(scenario.to_string());
        }
    }

    let mut clusters: Vec<FailureCluster> = groups
        .into_iter()
        .filter(|group| group.scenarios.len() >= FAILURE_CLUSTER_MIN_SCENARIOS)
        .collect();
    clusters.sort_by(|left, right| {
        right
            .scenarios
            .len()
            .cmp(&left.scenarios.len())
            .then_with(|| right.failure_count.cmp(&left.failure_count))
            .then_with(|| left.kind.cmp(&right.kind))
            .then_with(|| left.key.cmp(&right.key))
    });
    clusters
        .iter()
        .map(|cluster| {
            json!({
                "kind": cluster.kind,
                "key": cluster.key,
                "scenario_count": cluster.scenarios.len(),
                "failure_count": cluster.failure_count,
                "scenarios": cluster
                    .scenarios
                    .iter()
                    .take(FAILURE_CLUSTER_MAX_LISTED_SCENARIOS)
                    .collect::<Vec<_>>(),
                "hint": failure_cluster_hint(cluster, total_scenarios),
            })
        })
        .collect()
}

fn failure_cluster_hint(cluster: &FailureCluster, total_scenarios: usize) -> String {
    let scenario_count = cluster.scenarios.len();
    if cluster.kind == "provider_not_registered" {
        return format!(
            "provider '{}' is not registered in {scenario_count} of {total_scenarios} scenarios \
             — a QA surface failed to attach; fix that registration before reading per-scenario \
             failures",
            cluster.key
        );
    }
    format!(
        "{} repeats across {scenario_count} scenarios; a shared cause is likely",
        cluster.kind
    )
}

fn bundle_attempt_summary(
    attempt: usize,
    child: &CommandResult,
    started_at_ms: u64,
    finished_at_ms: u64,
) -> BundleRunAttemptSummary {
    let artifacts = child.data.get("artifacts").cloned();
    let run_root = artifact_string(&artifacts, "run_root");
    let result_path = artifact_string(&artifacts, "result_path");
    let logs_path = artifact_string(&artifacts, "logs_path");
    BundleRunAttemptSummary {
        attempt,
        run_root,
        result_path,
        logs_path,
        screenshot_paths: child_screenshot_paths(child),
        diagnostic_paths: child_diagnostic_paths(child),
        godot_args: child.data.get("godot_args").cloned(),
        project_args: Some(child_project_args(child)),
        time_control: child.data.get("time_control").cloned(),
        launch_display: child.data.get("launch_display").cloned(),
        display_lifecycle: child_display_lifecycle(child),
        cleanup_status: child_cleanup_status(child),
        status: if child.ok { "passed" } else { "failed" }.to_string(),
        ok: child.ok,
        artifacts,
        snapshot_artifacts: child.data.get("snapshot_artifacts").cloned(),
        failures: child.failures.clone(),
        category: child_failure_category(child),
        retryable: is_retryable_startup_infrastructure_failure(child),
        retry_decision: "not_retryable".to_string(),
        retry_guidance: child_retry_guidance(child),
        duration_ms: child.data.get("duration_ms").and_then(Value::as_f64),
        timing: child.data.get("timing").cloned(),
        started_at_ms,
        finished_at_ms,
    }
}

/// Enforce `artifacts.keep_runs` for a finished bundle run and describe what it removed.
///
/// Returns `None` when the project has not opted in, in which case nothing on disk is
/// touched and the command JSON carries no `retention` key at all. The run directories this
/// bundle just wrote are the newest ids under each scenario and under the bundle, so they
/// are always inside the retained window.
///
/// `prune_pool_sessions` is what a multi-bundle invocation turns off: the pooled sessions
/// belong to the whole invocation rather than to any one bundle, so the driver prunes them
/// once after the shared pool is down (see `run::run_scenario_bundles_warm`). The reported
/// `pruned.sessions` is then `0` here, which is the truth about what this bundle removed.
fn apply_bundle_retention(
    config: &ProjectConfig,
    bundle_name: &str,
    child_summaries: &[BundleRunSummary],
    prune_pool_sessions: bool,
) -> Option<Value> {
    let keep = config.artifacts_keep_runs?;
    let mut scenarios: Vec<&str> = child_summaries
        .iter()
        .map(|summary| summary.scenario.as_str())
        .collect();
    scenarios.sort_unstable();
    scenarios.dedup();

    let mut scenario_report = retention::PruneReport::default();
    for scenario in scenarios {
        scenario_report.merge(retention::prune_scenario_runs(
            &config.artifacts_dir,
            scenario,
            keep,
        ));
    }
    let bundle_report = retention::prune_bundle_runs(&config.artifacts_dir, bundle_name, keep);
    // Pooled warm sessions are only removed once no retained bundle run can still reference
    // them, so the cutoff is the oldest bundle run left on disk after pruning, less
    // retention::POOL_SESSION_RETENTION_MARGIN_SECS.
    let session_report =
        match retention::oldest_retained_bundle_run_epoch_secs(&config.artifacts_dir, bundle_name)
            .filter(|_| prune_pool_sessions)
        {
            Some(oldest_retained_epoch_secs) => retention::prune_pool_sessions(
                &config.artifacts_dir,
                oldest_retained_epoch_secs
                    .saturating_sub(retention::POOL_SESSION_RETENTION_MARGIN_SECS),
            ),
            None => retention::PruneReport::default(),
        };

    let mut errors = Vec::new();
    errors.extend(scenario_report.errors.iter().cloned());
    errors.extend(bundle_report.errors.iter().cloned());
    errors.extend(session_report.errors.iter().cloned());
    Some(json!({
        "keep_runs": keep,
        "pruned": {
            "scenario_runs": scenario_report.removed.len(),
            "bundle_runs": bundle_report.removed.len(),
            "sessions": session_report.removed.len(),
        },
        "errors": errors,
    }))
}

/// Prune pooled live-session directories ONCE for an invocation that ran several bundles
/// over one shared warm pool, and describe what it removed.
///
/// The per-bundle prune is turned off in that mode (`prune_pool_sessions: false`) because no
/// single bundle owns those sessions: a cutoff derived from bundle A's oldest retained run
/// could delete the sessions bundle B's retained run still points at. The cutoff here is the
/// OLDEST retained run across every bundle in the invocation, so a session survives as long
/// as any of them could still reference it.
///
/// Returns `None` when the project has not set `artifacts.keep_runs`, exactly like
/// [`apply_bundle_retention`]. `pruned` carries only `sessions`: the per-bundle prunes
/// already removed the scenario and bundle run directories and reported those counts
/// themselves.
pub(crate) fn apply_multi_bundle_session_retention(
    config: &ProjectConfig,
    bundle_names: &[String],
) -> Option<Value> {
    let keep = config.artifacts_keep_runs?;
    let oldest_retained = bundle_names
        .iter()
        .filter_map(|name| {
            retention::oldest_retained_bundle_run_epoch_secs(&config.artifacts_dir, name)
        })
        .min();
    // No retained bundle run anywhere means no cutoff can be justified, so nothing is
    // removed — the same rule the per-bundle prune applies.
    let report = match oldest_retained {
        Some(oldest_retained_epoch_secs) => retention::prune_pool_sessions(
            &config.artifacts_dir,
            oldest_retained_epoch_secs
                .saturating_sub(retention::POOL_SESSION_RETENTION_MARGIN_SECS),
        ),
        None => retention::PruneReport::default(),
    };
    Some(json!({
        "keep_runs": keep,
        "pruned": {"sessions": report.removed.len()},
        "errors": report.errors,
    }))
}

/// One `artifact_prune_failed` warning when retention could not delete something. Housekeeping
/// only: it never becomes a failure and never changes the exit code.
pub(crate) fn bundle_retention_warning(
    config: &ProjectConfig,
    retention: Option<&Value>,
) -> Option<FailureDetail> {
    let errors = retention?.get("errors")?.as_array()?;
    if errors.is_empty() {
        return None;
    }
    Some(FailureDetail {
        kind: "artifact_prune_failed".to_string(),
        command: Some("scenario bundle run".to_string()),
        path: Some(config.artifacts_dir.to_string_lossy().into_owned()),
        message: Some(format!(
            "{} artifact directories could not be pruned",
            errors.len()
        )),
        expected: Some(json!(
            "stale run directories removed under artifacts.keep_runs"
        )),
        actual: Some(json!(errors)),
        severity: Some("warning".to_string()),
        ..FailureDetail::default()
    })
}

fn prepare_bundle_artifacts(
    config: &ProjectConfig,
    bundle: &BundleManifest,
    options: &ScenarioBundleRunOptions,
    parallelism: &BundleParallelism,
    warnings: &[FailureDetail],
    failures: &[FailureDetail],
    runs: &[BundleRunSummary],
    artifact_index: &BundleArtifactSummary,
    timing: &Value,
    failure_clusters: &[Value],
    annotations: &BundleRunAnnotations,
) -> serde_json::Map<String, Value> {
    let run_id = format!(
        "{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0)
    );
    let run_parent = config.artifacts_dir.join("bundles").join(&bundle.name);
    let root = run_parent.join(&run_id);
    let mut out = serde_json::Map::new();
    if fs::create_dir_all(&root).is_ok() {
        let redacted_runs = redacted_bundle_runs_for_metadata(runs);
        // `meta.json` is bundle identity + invocation only. `runs` and `artifact_index` are
        // deliberately absent: they are byte-for-byte duplicates of what `summary.json` and
        // `artifact-index.json` already hold, and on a 100-scenario gate that duplication
        // cost megabytes per run.
        let meta = json!({"bundle": bundle.name, "bundle_path": bundle.path, "command": options.command_line, "parallelism": parallelism, "selection": annotations.selection, "required_bundles": bundle.required_bundles, "required_scenarios": bundle.required_scenarios, "optional_scenarios": bundle.optional_scenarios, "recommended_manual_checks": bundle.recommended_manual_checks});
        let summary = json!({
            "bundle": bundle.name,
            "bundle_path": bundle.path,
            "purpose": bundle.purpose,
            "parallelism": parallelism,
            "selection": annotations.selection,
            "scheduling": annotations.scheduling,
            "required_bundles": bundle.required_bundles,
            "required_scenarios": bundle.required_scenarios,
            "optional_scenarios": bundle.optional_scenarios,
            "recommended_manual_checks": bundle.recommended_manual_checks,
            "warnings": warnings,
            "failures": failures,
            "failure_clusters": failure_clusters,
            "timing": timing,
            "runs": redacted_runs,
            "artifact_index": artifact_index
        });
        let result = json!({"ok": failures.is_empty(), "summary": "summary.json"});
        let _ = fs::write(
            root.join("meta.json"),
            serde_json::to_vec_pretty(&meta).unwrap_or_default(),
        );
        let _ = fs::write(
            root.join("summary.json"),
            serde_json::to_vec_pretty(&summary).unwrap_or_default(),
        );
        let _ = fs::write(
            root.join("artifact-index.json"),
            serde_json::to_vec_pretty(artifact_index).unwrap_or_default(),
        );
        let _ = fs::write(
            root.join("result.json"),
            serde_json::to_vec_pretty(&result).unwrap_or_default(),
        );
        out.insert("run_root".to_string(), json!(root));
        out.insert("meta_path".to_string(), json!(root.join("meta.json")));
        out.insert("summary_path".to_string(), json!(root.join("summary.json")));
        out.insert(
            "artifact_index_path".to_string(),
            json!(root.join("artifact-index.json")),
        );
        out.insert("result_path".to_string(), json!(root.join("result.json")));
        // A stable path to the run that just finished, rotated after the four writes so
        // the link never points at a half-written directory. It is a convenience alias,
        // not evidence: a platform without symlinks simply has no `latest_path`, and a
        // rotation failure is silent for the same reason — housekeeping must never change
        // a bundle's verdict.
        if retention::update_bundle_latest_link(&run_parent, &run_id).is_ok() {
            out.insert("latest_path".to_string(), json!(run_parent.join("latest")));
        }
    }
    out
}

fn redacted_bundle_runs_for_metadata(runs: &[BundleRunSummary]) -> Value {
    let mut value = serde_json::to_value(runs).unwrap_or_else(|_| json!([]));
    redact_project_args_fields(&mut value);
    value
}

fn redact_project_args_fields(value: &mut Value) {
    match value {
        Value::Object(map) => {
            if let Some(project_args) = map.get_mut("project_args") {
                *project_args = run::redact_project_args_for_metadata(project_args);
            }
            for value in map.values_mut() {
                redact_project_args_fields(value);
            }
        }
        Value::Array(values) => {
            for value in values {
                redact_project_args_fields(value);
            }
        }
        _ => {}
    }
}

// Implementation chunks, included into this same `crate::bundle` module (see the
// module-size rule in AGENTS.md). Paths resolve relative to this file.
include!("bundle/manifest.rs");
include!("bundle/artifact_paths.rs");

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn late_display_text_after_runtime_progress_cannot_retry_gameplay() {
        let progress = std::env::temp_dir().join(format!(
            "godot-qa-runtime-progress-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let failure = FailureDetail {
            kind: "scenario_result_missing".into(),
            details: serde_json::from_value(json!({
                "category": "display_infrastructure",
                "retryable": true,
                "infrastructure_failure_kind": "display_launch_failed",
                "stderr": "gameplay failed; cannot open display during teardown"
            }))
            .unwrap(),
            ..FailureDetail::default()
        };
        let mut child = CommandResult {
            ok: false,
            command: "scenario run".into(),
            message: "missing result".into(),
            data: serde_json::from_value(json!({
                "artifacts": {"progress": progress.to_string_lossy()}
            }))
            .unwrap(),
            failures: vec![failure],
            exit_code: 1,
        };
        assert!(is_retryable_startup_infrastructure_failure(&child));
        std::fs::write(
            &progress,
            b"{\"event\":\"runtime_started\",\"run_id\":\"run-1\",\"nonce\":\"fresh\"}\n",
        )
        .unwrap();
        assert!(!is_retryable_startup_infrastructure_failure(&child));
        child
            .data
            .insert("logs".into(), json!("cannot open display"));
        assert!(!is_retryable_startup_infrastructure_failure(&child));
        let _ = std::fs::remove_file(progress);
    }

    fn child_failure(kind: &str, scenario: &str, details: &[(&str, Value)]) -> FailureDetail {
        let mut failure = FailureDetail {
            kind: kind.to_string(),
            ..FailureDetail::default()
        };
        failure
            .details
            .insert("scenario".to_string(), json!(scenario));
        for (key, value) in details {
            failure.details.insert((*key).to_string(), value.clone());
        }
        failure
    }

    fn run_summary(
        scenario: &str,
        duration_ms: Option<f64>,
        scene_load_ms: f64,
    ) -> BundleRunSummary {
        BundleRunSummary {
            index: 0,
            scenario: scenario.to_string(),
            scenario_path: format!("qa/scenarios/{scenario}.json"),
            required: true,
            source_bundle: "gate".to_string(),
            bundle_chain: vec!["gate".to_string()],
            source_kind: "direct".to_string(),
            run_root: Some(format!("/artifacts/{scenario}/run")),
            godot_args: None,
            project_args: None,
            warm: None,
            time_control: None,
            launch_display: None,
            status: "passed".to_string(),
            ok: true,
            result_path: None,
            logs_path: None,
            screenshot_paths: Vec::new(),
            diagnostic_paths: Vec::new(),
            artifacts: None,
            snapshot_artifacts: None,
            failures: Vec::new(),
            attempts: Vec::new(),
            final_attempt: 1,
            category: None,
            retry_policy: BundleRetryPolicy::default(),
            retry_guidance: None,
            cleanup_status: None,
            display_lifecycle: None,
            attempt_artifacts: Vec::new(),
            duration_ms,
            timing: Some(json!({"scene_load_ms": scene_load_ms, "steps_ms": 7})),
            started_at_ms: 0,
            finished_at_ms: 0,
            lane: 0,
            queue_wait_ms: 0.0,
        }
    }

    #[test]
    fn failure_clusters_collapse_one_shared_provider_cause() {
        let failures: Vec<FailureDetail> = ["a", "b", "c"]
            .into_iter()
            .map(|scenario| {
                child_failure(
                    "provider_not_registered",
                    scenario,
                    &[("provider", json!("layout_probe"))],
                )
            })
            .collect();

        let clusters = cluster_failures(&failures, 10);

        assert_eq!(clusters.len(), 1);
        assert_eq!(clusters[0]["kind"], "provider_not_registered");
        assert_eq!(clusters[0]["key"], "layout_probe");
        assert_eq!(clusters[0]["scenario_count"], 3);
        assert_eq!(clusters[0]["failure_count"], 3);
        assert_eq!(clusters[0]["scenarios"], json!(["a", "b", "c"]));
        assert!(
            clusters[0]["hint"]
                .as_str()
                .unwrap()
                .starts_with("provider 'layout_probe' is not registered in 3 of 10 scenarios")
        );
    }

    #[test]
    fn failure_clusters_count_scenarios_not_failures() {
        // Five failures from one scenario are one scenario, not five: the cluster
        // reports both numbers so a repeat within a run cannot fake a spread.
        let mut failures = vec![child_failure("scenario_assertion_failed", "a", &[]); 5];
        failures.push(child_failure("scenario_assertion_failed", "b", &[]));
        failures.push(child_failure("scenario_assertion_failed", "c", &[]));

        let clusters = cluster_failures(&failures, 3);

        assert_eq!(clusters.len(), 1);
        assert_eq!(clusters[0]["scenario_count"], 3);
        assert_eq!(clusters[0]["failure_count"], 7);
        assert_eq!(
            clusters[0]["hint"],
            "scenario_assertion_failed repeats across 3 scenarios; a shared cause is likely"
        );
    }

    #[test]
    fn failure_clusters_skip_small_repeats_and_preparation_failures() {
        let mut failures = vec![
            child_failure("scenario_assertion_failed", "a", &[]),
            child_failure("scenario_assertion_failed", "b", &[]),
        ];
        // A preparation failure carries no scenario, so it never clusters however
        // often it repeats.
        for _ in 0..3 {
            failures.push(FailureDetail {
                kind: "missing_scenario".to_string(),
                ..FailureDetail::default()
            });
        }

        assert_eq!(cluster_failures(&failures, 5), Vec::<Value>::new());
    }

    #[test]
    fn failure_clusters_separate_different_keys_of_one_kind() {
        let mut failures: Vec<FailureDetail> = ["a", "b", "c"]
            .into_iter()
            .map(|scenario| {
                child_failure(
                    "provider_not_registered",
                    scenario,
                    &[("provider", json!("layout_probe"))],
                )
            })
            .collect();
        failures.extend(["d", "e", "f", "g"].into_iter().map(|scenario| {
            child_failure(
                "provider_not_registered",
                scenario,
                &[("provider", json!("audio_probe"))],
            )
        }));

        let clusters = cluster_failures(&failures, 7);

        // Two causes, biggest first.
        assert_eq!(clusters.len(), 2);
        assert_eq!(clusters[0]["key"], "audio_probe");
        assert_eq!(clusters[0]["scenario_count"], 4);
        assert_eq!(clusters[1]["key"], "layout_probe");
        assert_eq!(clusters[1]["scenario_count"], 3);
    }

    #[test]
    fn slowest_runs_are_capped_and_sorted_descending() {
        let runs: Vec<BundleRunSummary> = (0..12)
            .map(|index| {
                run_summary(
                    &format!("scenario_{index}"),
                    Some(f64::from(index) * 10.0),
                    f64::from(index),
                )
            })
            .collect();

        let slowest = slowest_runs(&runs);

        assert_eq!(slowest.len(), BUNDLE_SLOWEST_RUNS);
        assert_eq!(slowest[0]["scenario"], "scenario_11");
        assert_eq!(slowest[0]["duration_ms"], 110.0);
        assert_eq!(slowest[0]["scene_load_ms"], 11.0);
        assert_eq!(slowest[0]["steps_ms"], 7);
        assert_eq!(slowest[0]["run_root"], "/artifacts/scenario_11/run");
        let durations: Vec<f64> = slowest
            .iter()
            .map(|entry| entry["duration_ms"].as_f64().unwrap())
            .collect();
        assert!(durations.windows(2).all(|pair| pair[0] >= pair[1]));
    }

    #[test]
    fn slowest_runs_omit_children_without_a_measured_duration() {
        let runs = vec![
            run_summary("measured", Some(5.0), 1.0),
            run_summary("unmeasured", None, 1.0),
        ];

        let slowest = slowest_runs(&runs);

        assert_eq!(slowest.len(), 1);
        assert_eq!(slowest[0]["scenario"], "measured");
    }
}
