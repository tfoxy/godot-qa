//! Running several bundles in one invocation.
//!
//! One `scenario bundle run a b c` is not three invocations glued together: the point is
//! that the expensive process-level resources — a warm session pool above all — are created
//! once and shared, and that the caller gets ONE verdict, one timing block and one flat
//! failure list instead of three envelopes to reconcile.
//!
//! This module owns the parts of that which do not depend on how the children are actually
//! executed: resolving every requested name before anything runs, and folding a `Vec` of
//! per-bundle results into the multi-bundle envelope. The cold driver here loops over
//! [`crate::bundle::run_scenario_bundle`]; a warm driver reuses the same two helpers with a
//! pool it owns.

use godot_qa_core::config::ProjectConfig;
use godot_qa_core::results::{CommandResult, FailureDetail};
use serde_json::{Value, json};
use std::time::Instant;

use crate::bundle::{self, BundleManifest, ScenarioBundleRunOptions};

const COMMAND: &str = "scenario bundle run";

/// Resolve every requested bundle name against the project's manifests, in ONE listing pass.
///
/// Every problem is reported at once and nothing runs: with three gates and two typos, an
/// operator should learn about both typos from one invocation rather than fixing them one
/// failed run at a time. A partial run is the worse alternative — it would write artifacts
/// and burn a warm pool for bundles that were only going to be discarded.
///
/// The returned manifests are in the order the names were given, so a caller can zip them
/// against `names` and against the per-bundle results.
pub fn preflight_bundle_names(
    config: &ProjectConfig,
    names: &[String],
) -> Result<Vec<BundleManifest>, CommandResult> {
    let listed = bundle::list_bundles(config);
    // A manifest that does not load or a globally duplicated name poisons the whole
    // listing, so it is reported before any per-name lookup: "bundle 'x' was not found"
    // would be a misleading second-order symptom of a manifest that failed to parse.
    if !listed.failures.is_empty() {
        return Err(preflight_failure_result(
            listed
                .failures
                .into_iter()
                .map(|mut failure| {
                    failure.command = Some(COMMAND.to_string());
                    failure
                })
                .collect(),
        ));
    }

    let mut manifests = Vec::with_capacity(names.len());
    let mut failures = Vec::new();
    for name in names {
        let matches: Vec<&BundleManifest> =
            listed.bundles.iter().filter(|b| &b.name == name).collect();
        match matches.as_slice() {
            [single] => manifests.push((*single).clone()),
            [] => failures.push(missing_bundle_failure(config, name)),
            many => failures.push(duplicate_bundle_failure(name, many)),
        }
    }
    if !failures.is_empty() {
        return Err(preflight_failure_result(failures));
    }
    Ok(manifests)
}

/// The run-level facts a multi-bundle invocation owns rather than any single bundle: the
/// shared warm pool, the wall clock around the whole invocation, and the retention pass that
/// can only run once every bundle is done.
///
/// Every field is optional because the cold driver has none of them; a warm driver fills in
/// `warm`, `warm_pool` and the two pool spans.
#[derive(Debug, Clone, Default)]
pub struct MultiBundleEnvelope {
    /// The invocation's warm status — one decision for all the bundles, not one per bundle.
    pub warm: Option<Value>,
    /// Pool counters for the single shared pool. Deliberately hoisted out of the per-bundle
    /// data (see [`aggregate_bundle_results`]).
    pub warm_pool: Option<Value>,
    /// Pool-level warnings, applied once rather than repeated under every bundle.
    pub pool_warnings: Vec<Value>,
    /// The one retention pass, which can only happen after the last bundle and the pool
    /// shutdown are done.
    pub retention: Option<Value>,
    pub wall_ms: u64,
    pub pool_seed_ms: Option<u64>,
    pub pool_shutdown_ms: Option<u64>,
}

/// Fold per-bundle results into one multi-bundle `CommandResult`.
///
/// `results` is positional against `names`. Each bundle keeps its own `data` verbatim under
/// `data.bundles[]` with one exception: `warm_pool` is stripped, because in multi-bundle mode
/// the pool belongs to the invocation and no bundle's summary may claim counters for sessions
/// it did not start or stop. The envelope carries the real ones once.
///
/// Failures are concatenated into one flat top-level list with `bundle` stamped on each, so
/// the usual "read `failures[0]`" reflex still identifies what broke. An existing `bundle`
/// key always wins, matching how the bundle runner stamps `scenario` and `run_root`.
pub fn aggregate_bundle_results(
    names: &[String],
    results: Vec<CommandResult>,
    envelope: MultiBundleEnvelope,
) -> CommandResult {
    let total = results.len();
    let mut bundles = Vec::with_capacity(total);
    let mut failures: Vec<FailureDetail> = Vec::new();
    let mut failed = 0usize;

    for (index, result) in results.into_iter().enumerate() {
        let name = names
            .get(index)
            .cloned()
            .or_else(|| {
                result
                    .data
                    .get("bundle")
                    .and_then(Value::as_str)
                    .map(str::to_string)
            })
            .unwrap_or_default();
        if !result.ok {
            failed += 1;
        }
        for mut failure in result.failures.iter().cloned() {
            failure
                .details
                .entry("bundle")
                .or_insert_with(|| json!(name));
            failures.push(failure);
        }

        let mut entry = serde_json::Map::new();
        entry.insert("name".to_string(), json!(name));
        entry.insert("ok".to_string(), json!(result.ok));
        entry.insert("failures".to_string(), json!(result.failures));
        for (key, value) in result.data {
            if key == "warm_pool" {
                continue;
            }
            entry.entry(key).or_insert(value);
        }
        bundles.push(Value::Object(entry));
    }

    let passed = total - failed;
    let ok = failed == 0;
    let mut data = serde_json::Map::new();
    data.insert("bundles".to_string(), Value::Array(bundles));
    data.insert(
        "summary".to_string(),
        json!({"total": total, "passed": passed, "failed": failed}),
    );
    if let Some(warm) = envelope.warm {
        data.insert("warm".to_string(), warm);
    }
    if let Some(warm_pool) = envelope.warm_pool {
        data.insert("warm_pool".to_string(), warm_pool);
    }
    data.insert(
        "timing".to_string(),
        json!({
            "wall_ms": envelope.wall_ms,
            "pool_seed_ms": envelope.pool_seed_ms,
            "pool_shutdown_ms": envelope.pool_shutdown_ms,
        }),
    );
    if let Some(retention) = envelope.retention {
        data.insert("retention".to_string(), retention);
    }
    if !envelope.pool_warnings.is_empty() {
        data.insert("warnings".to_string(), json!(envelope.pool_warnings));
    }

    CommandResult {
        ok,
        command: COMMAND.to_string(),
        message: if ok {
            format!("scenario bundle run completed ({total} bundles)")
        } else {
            format!("scenario bundle run failed ({failed} of {total} bundles failed)")
        },
        data,
        failures,
        exit_code: if ok { 0 } else { 1 },
    }
}

/// Run several bundles cold, one after another, under one envelope.
///
/// This is the fallback path: no shared pool, so the only thing the invocation owns is the
/// wall clock. Preflight still runs first, so a typo in the third name costs nothing.
pub fn run_scenario_bundles(
    config: &ProjectConfig,
    names: &[String],
    options: &ScenarioBundleRunOptions,
) -> CommandResult {
    let started = Instant::now();
    if let Err(result) = preflight_bundle_names(config, names) {
        return result;
    }
    let results: Vec<CommandResult> = names
        .iter()
        .map(|name| bundle::run_scenario_bundle(config, name, options))
        .collect();
    let envelope = MultiBundleEnvelope {
        wall_ms: u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
        ..MultiBundleEnvelope::default()
    };
    aggregate_bundle_results(names, results, envelope)
}

fn preflight_failure_result(failures: Vec<FailureDetail>) -> CommandResult {
    CommandResult {
        ok: false,
        command: COMMAND.to_string(),
        message: "scenario bundle run failed".to_string(),
        data: serde_json::Map::new(),
        failures,
        exit_code: 1,
    }
}

fn missing_bundle_failure(config: &ProjectConfig, name: &str) -> FailureDetail {
    FailureDetail {
        kind: "missing_bundle".to_string(),
        command: Some(COMMAND.to_string()),
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
    }
}

fn duplicate_bundle_failure(name: &str, matches: &[&BundleManifest]) -> FailureDetail {
    FailureDetail {
        kind: "duplicate_bundle_name".to_string(),
        command: Some(COMMAND.to_string()),
        path: matches
            .first()
            .map(|manifest| manifest.path.to_string_lossy().into_owned()),
        message: Some(format!("bundle name '{name}' is duplicated")),
        expected: Some(json!("unique bundle names")),
        actual: Some(json!(
            matches
                .iter()
                .map(|manifest| manifest.path.clone())
                .collect::<Vec<_>>()
        )),
        ..FailureDetail::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_root(label: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!(
            "godot-qa-multi-{label}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("clock")
                .as_nanos()
        ));
        fs::create_dir_all(root.join("qa/gates")).expect("create temp root");
        root
    }

    fn test_config(root: &Path) -> ProjectConfig {
        ProjectConfig {
            project_root: root.to_path_buf(),
            repo_root: root.to_path_buf(),
            verbose: false,
            godot_binary: None,
            qa_dir: root.join("qa"),
            artifacts_dir: root.join(".qa/artifacts"),
            scenarios_dir: root.join("qa/scenarios"),
            addon_dir: root.join("addons/godot_qa"),
            repo_addon_dir: root.join("addons/godot_qa"),
            godot_launch_prefix: Vec::new(),
            godot_windowed_launch_prefix: Vec::new(),
            godot_args: Vec::new(),
            godot_project_args: Vec::new(),
            godot_launch_mode: godot_qa_core::config::GodotLaunchMode::Auto,
            godot_audio_mode: godot_qa_core::config::GodotAudioMode::Auto,
            scenario_warm_default: false,
            scenario_warm_strict_default: false,
            scenario_jobs_default: None,
            scenario_schedule_default: None,
            log_ignore_patterns: Vec::new(),
            artifacts_keep_runs: None,
            project_config_failure: None,
            config_warnings: Vec::new(),
        }
    }

    fn write_gate(root: &Path, file_stem: &str, name: &str) {
        fs::write(
            root.join("qa/gates").join(format!("{file_stem}.yaml")),
            format!("name: {name}\nrequired_scenarios:\n  - some_scenario\n"),
        )
        .expect("write gate manifest");
    }

    fn names(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| value.to_string()).collect()
    }

    /// A stand-in for one bundle's result, with just the fields the aggregator reads.
    fn bundle_result(name: &str, ok: bool, failure_kinds: &[&str]) -> CommandResult {
        let mut data = serde_json::Map::new();
        data.insert("bundle".to_string(), json!(name));
        data.insert("runs".to_string(), json!([{"scenario": "child"}]));
        // Every bundle result carries pool counters when the runner was warm; in multi mode
        // they are the invocation's, not this bundle's.
        data.insert(
            "warm_pool".to_string(),
            json!({"sessions_started": 4, "reused": 3}),
        );
        CommandResult {
            ok,
            command: COMMAND.to_string(),
            message: if ok {
                "scenario bundle run completed".to_string()
            } else {
                "scenario bundle run failed".to_string()
            },
            data,
            failures: failure_kinds
                .iter()
                .map(|kind| FailureDetail {
                    kind: (*kind).to_string(),
                    ..FailureDetail::default()
                })
                .collect(),
            exit_code: if ok { 0 } else { 1 },
        }
    }

    #[test]
    fn aggregate_counts_bundles_strips_pool_counters_and_stamps_failures() {
        let envelope = MultiBundleEnvelope {
            warm: Some(json!({"enabled": true, "mode": "warm"})),
            warm_pool: Some(json!({"sessions_started": 4, "reused": 3})),
            pool_warnings: vec![json!({"type": "warm_session_log_noise"})],
            retention: Some(json!({"pruned": {"sessions": 2}})),
            wall_ms: 8700,
            pool_seed_ms: Some(1200),
            pool_shutdown_ms: Some(90),
        };

        let result = aggregate_bundle_results(
            &names(&["gate_a", "gate_b", "gate_c"]),
            vec![
                bundle_result("gate_a", true, &[]),
                bundle_result("gate_b", false, &["assertion_failed", "godot_log_error"]),
                bundle_result("gate_c", true, &[]),
            ],
            envelope,
        );

        assert!(!result.ok);
        assert_eq!(result.exit_code, 1);
        assert_eq!(
            result.message,
            "scenario bundle run failed (1 of 3 bundles failed)"
        );
        assert_eq!(
            result.data["summary"],
            json!({"total": 3, "passed": 2, "failed": 1})
        );

        let bundles = result.data["bundles"].as_array().expect("bundles array");
        assert_eq!(bundles.len(), 3);
        assert_eq!(bundles[0]["name"], "gate_a");
        assert_eq!(bundles[0]["ok"], true);
        assert_eq!(bundles[1]["name"], "gate_b");
        assert_eq!(bundles[1]["ok"], false);
        // Each bundle keeps its own data...
        assert_eq!(bundles[1]["runs"], json!([{"scenario": "child"}]));
        assert_eq!(
            bundles[1]["failures"].as_array().expect("failures").len(),
            2
        );
        // ...except the pool counters, which belong to the invocation.
        for entry in bundles {
            assert!(
                entry.get("warm_pool").is_none(),
                "warm_pool must be hoisted out of per-bundle data"
            );
        }

        // One flat failure list, each entry naming the bundle it came from.
        assert_eq!(result.failures.len(), 2);
        for failure in &result.failures {
            assert_eq!(failure.details["bundle"], "gate_b");
        }
        // `details` is flattened, so this reads as `failures[0].bundle` in the JSON.
        assert_eq!(
            serde_json::to_value(&result.failures[0]).expect("serialize")["bundle"],
            "gate_b"
        );

        assert_eq!(
            result.data["warm"],
            json!({"enabled": true, "mode": "warm"})
        );
        assert_eq!(
            result.data["warm_pool"],
            json!({"sessions_started": 4, "reused": 3})
        );
        assert_eq!(
            result.data["timing"],
            json!({"wall_ms": 8700, "pool_seed_ms": 1200, "pool_shutdown_ms": 90})
        );
        assert_eq!(result.data["retention"], json!({"pruned": {"sessions": 2}}));
        assert_eq!(
            result.data["warnings"],
            json!([{"type": "warm_session_log_noise"}])
        );
    }

    #[test]
    fn aggregate_reports_an_all_green_invocation() {
        let result = aggregate_bundle_results(
            &names(&["gate_a", "gate_b"]),
            vec![
                bundle_result("gate_a", true, &[]),
                bundle_result("gate_b", true, &[]),
            ],
            MultiBundleEnvelope {
                wall_ms: 2900,
                ..MultiBundleEnvelope::default()
            },
        );

        assert!(result.ok);
        assert_eq!(result.exit_code, 0);
        assert_eq!(result.message, "scenario bundle run completed (2 bundles)");
        assert_eq!(
            result.data["summary"],
            json!({"total": 2, "passed": 2, "failed": 0})
        );
        assert!(result.failures.is_empty());
        // The cold path has no pool, so those keys are simply absent rather than null.
        assert!(result.data.get("warm").is_none());
        assert!(result.data.get("warm_pool").is_none());
        assert!(result.data.get("retention").is_none());
        assert!(result.data.get("warnings").is_none());
        assert_eq!(
            result.data["timing"],
            json!({"wall_ms": 2900, "pool_seed_ms": Value::Null, "pool_shutdown_ms": Value::Null})
        );
    }

    #[test]
    fn preflight_accepts_every_known_name_in_order() {
        let root = temp_root("preflight-ok");
        write_gate(&root, "a", "gate_a");
        write_gate(&root, "b", "gate_b");
        let config = test_config(&root);

        let manifests =
            preflight_bundle_names(&config, &names(&["gate_b", "gate_a"])).expect("both resolve");

        // Positional against `names`, not alphabetical, so a caller can zip them.
        assert_eq!(
            manifests
                .iter()
                .map(|manifest| manifest.name.clone())
                .collect::<Vec<_>>(),
            names(&["gate_b", "gate_a"])
        );
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn preflight_reports_every_missing_name_at_once() {
        let root = temp_root("preflight-missing");
        write_gate(&root, "a", "gate_a");
        let config = test_config(&root);

        let result = preflight_bundle_names(&config, &names(&["gate_a", "typo", "also_typo"]))
            .expect_err("missing names must fail");

        assert!(!result.ok);
        assert_eq!(result.exit_code, 1);
        assert_eq!(result.message, "scenario bundle run failed");
        // Both typos in one result: an operator fixes them in one pass, and nothing ran.
        assert_eq!(result.failures.len(), 2);
        assert!(
            result
                .failures
                .iter()
                .all(|failure| failure.kind == "missing_bundle")
        );
        assert_eq!(result.failures[0].actual, Some(json!("typo")));
        assert_eq!(result.failures[1].actual, Some(json!("also_typo")));
        assert_eq!(
            result.failures[0].command.as_deref(),
            Some("scenario bundle run")
        );
        assert!(result.data.is_empty());
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn preflight_rejects_a_duplicated_bundle_name() {
        let root = temp_root("preflight-duplicate");
        // Two manifests declaring the same bundle name.
        write_gate(&root, "first", "gate_a");
        write_gate(&root, "second", "gate_a");
        let config = test_config(&root);

        let result =
            preflight_bundle_names(&config, &names(&["gate_a"])).expect_err("duplicate must fail");

        assert!(!result.ok);
        assert_eq!(result.exit_code, 1);
        assert!(
            result
                .failures
                .iter()
                .any(|failure| failure.kind == "duplicate_bundle_name"),
            "got {:?}",
            result
                .failures
                .iter()
                .map(|failure| failure.kind.clone())
                .collect::<Vec<_>>()
        );
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn run_scenario_bundles_runs_nothing_when_preflight_fails() {
        let root = temp_root("run-preflight");
        write_gate(&root, "a", "gate_a");
        let config = test_config(&root);

        let result = run_scenario_bundles(
            &config,
            &names(&["gate_a", "missing_gate"]),
            &ScenarioBundleRunOptions::default(),
        );

        assert!(!result.ok);
        assert_eq!(result.failures.len(), 1);
        assert_eq!(result.failures[0].kind, "missing_bundle");
        // Nothing ran, so there is no envelope and no artifacts directory.
        assert!(result.data.get("bundles").is_none());
        assert!(!root.join(".qa/artifacts").exists());
        let _ = fs::remove_dir_all(&root);
    }
}
