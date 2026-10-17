use godot_qa_core::config::ProjectConfig;
use godot_qa_core::results::{CommandResult, FailureDetail};
use serde_json::{Map, Value, json};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use crate::artifacts::write_json;
use crate::run;
use crate::scenario_doc::is_safe_artifact_segment;

/// Hard cap on expanded sweep runs (seeds x repeat) so one invocation cannot
/// queue an unbounded number of cold Godot launches.
pub const MAX_SWEEP_RUNS: usize = 1_000;
/// Hard cap on `--repeat` so repeats stay a same-seed stability probe, not a
/// substitute for a proper seed range.
pub const MAX_SWEEP_REPEAT: u32 = 100;

#[derive(Debug, Clone)]
pub struct ScenarioSweepOptions {
    pub seeds: Vec<i64>,
    pub require_all_pass: bool,
    pub headless_override: Option<bool>,
    pub audio_override: Option<bool>,
    pub command_line: String,
    pub extra_project_args: Vec<String>,
}

impl Default for ScenarioSweepOptions {
    fn default() -> Self {
        Self {
            seeds: Vec::new(),
            require_all_pass: false,
            headless_override: None,
            audio_override: None,
            command_line: "godot-qa scenario sweep <name> --json".to_string(),
            extra_project_args: Vec::new(),
        }
    }
}

/// Expand the CLI seed selection into the ordered per-run seed sequence.
///
/// Exactly one selection mode must be used: an explicit `--seeds` list, or a
/// contiguous `--seed-start`/`--seed-count` range. Duplicates in the explicit
/// list are rejected (repeat a seed deliberately with `--repeat` instead), and
/// `--repeat` replays each selected seed K times back to back.
pub fn expand_sweep_seeds(
    seeds: &[i64],
    seed_start: Option<i64>,
    seed_count: Option<u32>,
    repeat: u32,
) -> Result<Vec<i64>, FailureDetail> {
    let command = "scenario sweep";
    if repeat == 0 || repeat > MAX_SWEEP_REPEAT {
        return Err(invalid_seed_params(
            command,
            "scenario sweep requires --repeat between 1 and 100",
            json!("integer repeat in [1, 100]"),
            json!(repeat),
        ));
    }
    let range_requested = seed_start.is_some() || seed_count.is_some();
    if seeds.is_empty() && !range_requested {
        return Err(invalid_seed_params(
            command,
            "scenario sweep requires --seeds or --seed-start/--seed-count",
            json!("--seeds 1,2,3 or --seed-start N --seed-count K"),
            json!({"seeds": seeds, "seed_start": seed_start, "seed_count": seed_count}),
        ));
    }
    if !seeds.is_empty() && range_requested {
        return Err(invalid_seed_params(
            command,
            "scenario sweep accepts either --seeds or --seed-start/--seed-count, not both",
            json!("one seed selection mode"),
            json!({"seeds": seeds, "seed_start": seed_start, "seed_count": seed_count}),
        ));
    }

    let selected = if !seeds.is_empty() {
        let mut seen = std::collections::HashSet::new();
        for seed in seeds {
            if !seen.insert(*seed) {
                return Err(invalid_seed_params(
                    command,
                    "scenario sweep --seeds must not contain duplicates (use --repeat to rerun a seed)",
                    json!("unique seed list"),
                    json!(seed),
                ));
            }
        }
        seeds.to_vec()
    } else {
        let (Some(start), Some(count)) = (seed_start, seed_count) else {
            return Err(invalid_seed_params(
                command,
                "scenario sweep requires both --seed-start and --seed-count",
                json!("--seed-start N --seed-count K"),
                json!({"seed_start": seed_start, "seed_count": seed_count}),
            ));
        };
        if count == 0 || count as usize > MAX_SWEEP_RUNS {
            return Err(invalid_seed_params(
                command,
                "scenario sweep requires --seed-count between 1 and 1000",
                json!("integer seed count in [1, 1000]"),
                json!(count),
            ));
        }
        if start.checked_add(count as i64 - 1).is_none() {
            return Err(invalid_seed_params(
                command,
                "scenario sweep seed range overflows a 64-bit seed",
                json!("seed range within i64"),
                json!({"seed_start": start, "seed_count": count}),
            ));
        }
        (0..count as i64).map(|offset| start + offset).collect()
    };

    let total = selected.len().saturating_mul(repeat as usize);
    if total > MAX_SWEEP_RUNS {
        return Err(invalid_seed_params(
            command,
            "scenario sweep expands to more than 1000 runs",
            json!("seeds x repeat <= 1000"),
            json!({"seeds": selected.len(), "repeat": repeat, "total": total}),
        ));
    }

    let mut expanded = Vec::with_capacity(total);
    for seed in selected {
        for _ in 0..repeat {
            expanded.push(seed);
        }
    }
    Ok(expanded)
}

/// Build the per-seed child scenario document: the sweep seed is injected as
/// the LAST `--seed=<s>` project arg (any `--seed` already declared by the
/// scenario or the `--project-arg` passthrough is stripped first, so the sweep
/// seed always wins), and `determinism.seed` is overwritten when the scenario
/// declares one. Precedence: sweep seed > scenario/CLI `--seed` project arg
/// and > the scenario's own `determinism.seed`.
pub fn scenario_with_sweep_seed(
    scenario: &Value,
    seed: i64,
    extra_project_args: &[String],
) -> Value {
    let mut child = scenario.clone();
    let Some(object) = child.as_object_mut() else {
        return child;
    };
    let mut args: Vec<String> = object
        .get("project_args")
        .and_then(Value::as_array)
        .map(|values| {
            values
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default();
    args.extend(extra_project_args.iter().cloned());
    let mut args = strip_seed_args(&args);
    args.push(format!("--seed={seed}"));
    object.insert("project_args".to_string(), json!(args));
    if let Some(determinism) = object.get_mut("determinism").and_then(Value::as_object_mut) {
        if determinism.contains_key("seed") {
            determinism.insert("seed".to_string(), json!(seed));
        }
    }
    child
}

fn strip_seed_args(args: &[String]) -> Vec<String> {
    let mut stripped = Vec::with_capacity(args.len());
    let mut skip_next = false;
    for arg in args {
        if skip_next {
            skip_next = false;
            continue;
        }
        if arg == "--seed" {
            skip_next = true;
            continue;
        }
        if arg.starts_with("--seed=") {
            continue;
        }
        stripped.push(arg.clone());
    }
    stripped
}

pub fn run_scenario_sweep(
    config: &ProjectConfig,
    scenario_path: &Path,
    scenario: &Value,
    options: &ScenarioSweepOptions,
) -> CommandResult {
    // v1 sweeps run every seed cold: the warm bundle session serves one fixed
    // launch configuration, while a sweep changes the launch project_args per
    // seed, so a fresh process per seed is the faithful production path.
    run_scenario_sweep_with_runner(
        config,
        scenario_path,
        scenario,
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
    )
}

pub fn run_scenario_sweep_with_runner<F>(
    config: &ProjectConfig,
    scenario_path: &Path,
    scenario: &Value,
    options: &ScenarioSweepOptions,
    runner: F,
) -> CommandResult
where
    F: Fn(&ProjectConfig, &Path, &Value, &str, Option<bool>, Option<bool>) -> CommandResult,
{
    let command = "scenario sweep";
    let scenario_name = scenario
        .get("name")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    if scenario_name.is_empty() || !is_safe_artifact_segment(&scenario_name) {
        return sweep_failure_result(
            command,
            vec![FailureDetail {
                kind: "scenario_validation_failed".to_string(),
                command: Some(command.to_string()),
                path: Some(scenario_path.to_string_lossy().into_owned()),
                message: Some(
                    "scenario field 'name' must be a safe artifact path segment".to_string(),
                ),
                expected: Some(json!("letters, numbers, dot, underscore, or hyphen")),
                actual: Some(json!(scenario_name)),
                ..FailureDetail::default()
            }],
        );
    }
    if options.seeds.is_empty() {
        return sweep_failure_result(
            command,
            vec![invalid_seed_params(
                command,
                "scenario sweep requires at least one seed",
                json!("non-empty seed sequence"),
                json!(options.seeds),
            )],
        );
    }

    // The sweep is ONE aggregate artifact identity with per-seed children:
    // `.qa/artifacts/<scenario>/sweep-<run-id>/seed-<s>/...`. Single-run
    // artifact behavior is untouched — each child run is a plain
    // `scenario run` whose artifacts root is redirected into its seed dir.
    let (sweep_run_id, sweep_root) =
        match allocate_sweep_root(&config.artifacts_dir.join(&scenario_name)) {
            Ok(allocated) => allocated,
            Err(error) => {
                return sweep_failure_result(
                    command,
                    vec![FailureDetail {
                        kind: "artifact_write_failed".to_string(),
                        command: Some(command.to_string()),
                        path: Some(config.artifacts_dir.to_string_lossy().into_owned()),
                        message: Some(error.to_string()),
                        expected: Some(json!(
                            "sweep artifact root created under the artifact directory"
                        )),
                        actual: Some(json!("OSError")),
                        ..FailureDetail::default()
                    }],
                );
            }
        };

    let mut per_seed = Vec::new();
    let mut passed = 0_usize;
    for seed in &options.seeds {
        let child_scenario = scenario_with_sweep_seed(scenario, *seed, &options.extra_project_args);
        let mut child_config = config.clone();
        child_config.artifacts_dir = sweep_root.join(format!("seed-{seed}"));
        let started = Instant::now();
        let child = runner(
            &child_config,
            scenario_path,
            &child_scenario,
            &options.command_line,
            options.headless_override,
            options.audio_override,
        );
        let duration_ms = child
            .data
            .get("duration_ms")
            .and_then(Value::as_i64)
            .unwrap_or_else(|| started.elapsed().as_millis() as i64);
        if child.ok {
            passed += 1;
        }
        per_seed.push(json!({
            "seed": seed,
            "ok": child.ok,
            "failed_asserts": child.failures.len(),
            "failure_kinds": child
                .failures
                .iter()
                .map(|failure| failure.kind.clone())
                .collect::<Vec<_>>(),
            "duration_ms": duration_ms,
            "artifact_dir": child
                .data
                .get("artifacts")
                .and_then(|artifacts| artifacts.get("run_root"))
                .cloned()
                .unwrap_or(Value::Null),
            // Per-seed record_state evidence so win-rates/balance metrics across a
            // seed range are a single read of sweep.json. Bounded runtime-side;
            // null when the child run recorded nothing. Aggregation stays out of
            // the tool (agent-side reads of sweep.json).
            "state_records": child
                .data
                .get("state_records")
                .cloned()
                .unwrap_or(Value::Null),
        }));
    }

    let total = options.seeds.len();
    let failed = total - passed;
    let pass_rate = if total == 0 {
        0.0
    } else {
        passed as f64 / total as f64
    };
    let summary = json!({
        "total": total,
        "passed": passed,
        "failed": failed,
        "pass_rate": pass_rate,
    });
    let aggregate = json!({
        "scenario": scenario_name,
        "scenario_path": scenario_path,
        "command": options.command_line,
        "sweep_run_id": sweep_run_id,
        "seeds": options.seeds,
        "require_all_pass": options.require_all_pass,
        "per_seed": per_seed,
        "summary": summary,
    });
    let sweep_path = sweep_root.join("sweep.json");
    if let Err(error) = write_json(&sweep_path, &aggregate) {
        return sweep_failure_result(
            command,
            vec![FailureDetail {
                kind: "artifact_write_failed".to_string(),
                command: Some(command.to_string()),
                path: Some(sweep_path.to_string_lossy().into_owned()),
                message: Some(error.to_string()),
                expected: Some(json!(
                    "sweep aggregate written under the sweep artifact root"
                )),
                actual: Some(json!("OSError")),
                ..FailureDetail::default()
            }],
        );
    }

    let mut failures = Vec::new();
    if options.require_all_pass && failed > 0 {
        failures.push(FailureDetail {
            kind: "sweep_seeds_failed".to_string(),
            command: Some(command.to_string()),
            path: Some(sweep_path.to_string_lossy().into_owned()),
            message: Some(format!(
                "{failed} of {total} sweep seed run(s) failed and --require-all-pass was set"
            )),
            expected: Some(json!("every seed run passing")),
            actual: Some(json!(per_seed_failures(per_seed_entries(&aggregate)))),
            ..FailureDetail::default()
        });
    }

    let ok = failures.is_empty();
    let data = Map::from_iter([
        ("scenario".to_string(), json!(scenario_name)),
        (
            "scenario_path".to_string(),
            json!(scenario_path.to_string_lossy().into_owned()),
        ),
        ("sweep_run_id".to_string(), json!(sweep_run_id)),
        ("seeds".to_string(), json!(options.seeds)),
        (
            "per_seed".to_string(),
            aggregate
                .get("per_seed")
                .cloned()
                .unwrap_or_else(|| json!([])),
        ),
        ("summary".to_string(), summary),
        (
            "artifacts".to_string(),
            json!({
                "sweep_root": sweep_root,
                "sweep": sweep_path,
            }),
        ),
    ]);
    CommandResult {
        ok,
        command: command.to_string(),
        message: format!(
            "scenario sweep {}: {passed}/{total} seed run(s) passed (pass_rate {pass_rate:.2})",
            if ok { "completed" } else { "failed" }
        ),
        data,
        failures,
        exit_code: if ok { 0 } else { 1 },
    }
}

fn per_seed_entries(aggregate: &Value) -> &[Value] {
    aggregate
        .get("per_seed")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default()
}

fn per_seed_failures(per_seed: &[Value]) -> Vec<Value> {
    per_seed
        .iter()
        .filter(|entry| entry.get("ok").and_then(Value::as_bool) != Some(true))
        .cloned()
        .collect()
}

fn allocate_sweep_root(scenario_root: &Path) -> io::Result<(String, PathBuf)> {
    fs::create_dir_all(scenario_root)?;
    let base_run_id = format!(
        "sweep-{}Z",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_secs())
            .unwrap_or(0)
    );
    let mut candidate_run_id = base_run_id.clone();
    let mut suffix = 1;
    loop {
        let root_dir = scenario_root.join(&candidate_run_id);
        match fs::create_dir(&root_dir) {
            Ok(()) => return Ok((candidate_run_id, root_dir)),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                candidate_run_id = format!("{base_run_id}-{suffix:02}");
                suffix += 1;
            }
            Err(error) => return Err(error),
        }
    }
}

fn invalid_seed_params(
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

fn sweep_failure_result(command: &str, failures: Vec<FailureDetail>) -> CommandResult {
    CommandResult {
        ok: false,
        command: command.to_string(),
        message: "scenario sweep failed".to_string(),
        data: Map::new(),
        failures,
        exit_code: 1,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::sync::Mutex;

    #[test]
    fn expand_sweep_seeds_uses_explicit_list() {
        let seeds = expand_sweep_seeds(&[3, 1, 2], None, None, 1).unwrap();
        assert_eq!(seeds, vec![3, 1, 2]);
    }

    #[test]
    fn expand_sweep_seeds_expands_range_and_repeat() {
        let seeds = expand_sweep_seeds(&[], Some(10), Some(3), 2).unwrap();
        assert_eq!(seeds, vec![10, 10, 11, 11, 12, 12]);
    }

    #[test]
    fn expand_sweep_seeds_rejects_empty_selection() {
        let failure = expand_sweep_seeds(&[], None, None, 1).unwrap_err();
        assert_eq!(failure.kind, "invalid_params");
        assert_eq!(
            failure.expected,
            Some(json!("--seeds 1,2,3 or --seed-start N --seed-count K"))
        );
    }

    #[test]
    fn expand_sweep_seeds_rejects_duplicates_and_mixed_modes() {
        let duplicate = expand_sweep_seeds(&[1, 2, 1], None, None, 1).unwrap_err();
        assert_eq!(duplicate.kind, "invalid_params");
        assert_eq!(duplicate.actual, Some(json!(1)));

        let mixed = expand_sweep_seeds(&[1], Some(2), Some(3), 1).unwrap_err();
        assert_eq!(mixed.kind, "invalid_params");
        assert_eq!(mixed.expected, Some(json!("one seed selection mode")));
    }

    #[test]
    fn expand_sweep_seeds_bounds_repeat_count_and_total() {
        assert_eq!(
            expand_sweep_seeds(&[1], None, None, 0).unwrap_err().kind,
            "invalid_params"
        );
        assert_eq!(
            expand_sweep_seeds(&[], Some(0), Some(0), 1)
                .unwrap_err()
                .kind,
            "invalid_params"
        );
        assert_eq!(
            expand_sweep_seeds(&[], Some(0), Some(2_000), 1)
                .unwrap_err()
                .kind,
            "invalid_params"
        );
        let total = expand_sweep_seeds(&[], Some(0), Some(600), 2).unwrap_err();
        assert_eq!(total.kind, "invalid_params");
        assert_eq!(total.expected, Some(json!("seeds x repeat <= 1000")));
        assert_eq!(
            expand_sweep_seeds(&[], Some(i64::MAX), Some(2), 1)
                .unwrap_err()
                .kind,
            "invalid_params"
        );
    }

    #[test]
    fn scenario_with_sweep_seed_overrides_project_args_and_determinism() {
        let scenario = json!({
            "name": "demo",
            "scene": "res://main.tscn",
            "project_args": ["--seed=7", "--qa-fixture=boss", "--seed", "9"],
            "determinism": {"seed": 7, "fixed_fps": 60},
            "steps": [{"wait_frames": 1}]
        });

        let child = scenario_with_sweep_seed(&scenario, 42, &["--qa-difficulty=hard".to_string()]);

        assert_eq!(
            child.get("project_args"),
            Some(&json!([
                "--qa-fixture=boss",
                "--qa-difficulty=hard",
                "--seed=42"
            ]))
        );
        assert_eq!(
            child.get("determinism"),
            Some(&json!({"seed": 42, "fixed_fps": 60}))
        );
        // The source document is untouched.
        assert_eq!(
            scenario.get("project_args"),
            Some(&json!(["--seed=7", "--qa-fixture=boss", "--seed", "9"]))
        );
    }

    #[test]
    fn scenario_with_sweep_seed_leaves_absent_determinism_seed_alone() {
        let scenario = json!({
            "name": "demo",
            "scene": "res://main.tscn",
            "determinism": {"fixed_fps": 60}
        });
        let child = scenario_with_sweep_seed(&scenario, 5, &[]);
        assert_eq!(child.get("determinism"), Some(&json!({"fixed_fps": 60})));
        assert_eq!(child.get("project_args"), Some(&json!(["--seed=5"])));
    }

    #[test]
    fn run_scenario_sweep_aggregates_without_aborting_on_failures() {
        let root = temp_root("sweep-aggregate");
        let config = test_config(&root);
        let scenario = json!({
            "name": "sweep_demo",
            "scene": "res://main.tscn",
            "steps": [{"wait_frames": 1}]
        });
        let scenario_path = root.join("qa/scenarios/sweep_demo.json");
        let observed: Mutex<Vec<(i64, PathBuf)>> = Mutex::new(Vec::new());

        let result = run_scenario_sweep_with_runner(
            &config,
            &scenario_path,
            &scenario,
            &ScenarioSweepOptions {
                seeds: vec![1, 2, 3],
                ..ScenarioSweepOptions::default()
            },
            |child_config, _path, child_scenario, _command_line, _headless, _audio| {
                let seed = child_scenario
                    .get("project_args")
                    .and_then(Value::as_array)
                    .and_then(|args| args.last())
                    .and_then(Value::as_str)
                    .and_then(|arg| arg.strip_prefix("--seed="))
                    .and_then(|value| value.parse::<i64>().ok())
                    .expect("child scenario carries the injected seed");
                observed
                    .lock()
                    .unwrap()
                    .push((seed, child_config.artifacts_dir.clone()));
                let ok = seed != 2;
                CommandResult {
                    ok,
                    command: "scenario run".to_string(),
                    message: String::new(),
                    data: Map::from_iter([
                        ("duration_ms".to_string(), json!(17)),
                        (
                            "artifacts".to_string(),
                            json!({"run_root": child_config.artifacts_dir.join("run")}),
                        ),
                    ]),
                    failures: if ok {
                        Vec::new()
                    } else {
                        vec![FailureDetail {
                            kind: "assertion_failed".to_string(),
                            ..FailureDetail::default()
                        }]
                    },
                    exit_code: if ok { 0 } else { 1 },
                }
            },
        );

        // A failing seed never aborts the sweep, and without
        // --require-all-pass the sweep itself still passes.
        assert!(result.ok, "{:?}", result.failures);
        assert_eq!(result.exit_code, 0);
        assert_eq!(result.data["summary"]["total"], json!(3));
        assert_eq!(result.data["summary"]["passed"], json!(2));
        assert_eq!(result.data["summary"]["failed"], json!(1));
        assert_eq!(result.data["per_seed"][1]["ok"], json!(false));
        assert_eq!(result.data["per_seed"][1]["failed_asserts"], json!(1));
        assert_eq!(result.data["per_seed"][0]["duration_ms"], json!(17));

        let observed = observed.lock().unwrap();
        assert_eq!(
            observed.iter().map(|(seed, _)| *seed).collect::<Vec<_>>(),
            vec![1, 2, 3]
        );
        let sweep_root = PathBuf::from(
            result.data["artifacts"]["sweep_root"]
                .as_str()
                .expect("sweep root path"),
        );
        assert!(
            sweep_root.starts_with(config.artifacts_dir.join("sweep_demo")),
            "{sweep_root:?}"
        );
        for (seed, artifacts_dir) in observed.iter() {
            assert_eq!(*artifacts_dir, sweep_root.join(format!("seed-{seed}")));
        }
        let aggregate: Value = serde_json::from_str(
            &fs::read_to_string(sweep_root.join("sweep.json")).expect("sweep.json"),
        )
        .expect("sweep.json parses");
        assert_eq!(aggregate["summary"]["pass_rate"].as_f64(), Some(2.0 / 3.0));
        assert_eq!(aggregate["seeds"], json!([1, 2, 3]));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn run_scenario_sweep_require_all_pass_fails_after_collecting_all_seeds() {
        let root = temp_root("sweep-require-all");
        let config = test_config(&root);
        let scenario = json!({
            "name": "sweep_demo",
            "scene": "res://main.tscn"
        });
        let scenario_path = root.join("qa/scenarios/sweep_demo.json");

        let result = run_scenario_sweep_with_runner(
            &config,
            &scenario_path,
            &scenario,
            &ScenarioSweepOptions {
                seeds: vec![4, 5],
                require_all_pass: true,
                ..ScenarioSweepOptions::default()
            },
            |_config, _path, _scenario, _command_line, _headless, _audio| CommandResult {
                ok: false,
                command: "scenario run".to_string(),
                message: String::new(),
                data: Map::new(),
                failures: vec![FailureDetail {
                    kind: "assertion_failed".to_string(),
                    ..FailureDetail::default()
                }],
                exit_code: 1,
            },
        );

        assert!(!result.ok);
        assert_eq!(result.exit_code, 1);
        assert_eq!(result.failures.len(), 1);
        assert_eq!(result.failures[0].kind, "sweep_seeds_failed");
        // Both seeds still ran and were aggregated before the failure verdict.
        assert_eq!(result.data["summary"]["total"], json!(2));
        assert_eq!(result.data["summary"]["failed"], json!(2));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn run_scenario_sweep_repeat_reuses_seed_dir_per_run() {
        let root = temp_root("sweep-repeat");
        let config = test_config(&root);
        let scenario = json!({"name": "sweep_demo", "scene": "res://main.tscn"});
        let scenario_path = root.join("qa/scenarios/sweep_demo.json");

        let result = run_scenario_sweep_with_runner(
            &config,
            &scenario_path,
            &scenario,
            &ScenarioSweepOptions {
                seeds: expand_sweep_seeds(&[9], None, None, 2).unwrap(),
                ..ScenarioSweepOptions::default()
            },
            |child_config, _path, _scenario, _command_line, _headless, _audio| CommandResult {
                ok: true,
                command: "scenario run".to_string(),
                message: String::new(),
                data: Map::from_iter([(
                    "artifacts".to_string(),
                    json!({"run_root": child_config.artifacts_dir.join("run")}),
                )]),
                failures: Vec::new(),
                exit_code: 0,
            },
        );

        assert!(result.ok, "{:?}", result.failures);
        assert_eq!(result.data["seeds"], json!([9, 9]));
        assert_eq!(result.data["per_seed"][0]["seed"], json!(9));
        assert_eq!(result.data["per_seed"][1]["seed"], json!(9));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn run_scenario_sweep_passes_through_state_records_per_seed() {
        let root = temp_root("sweep-state-records");
        let config = test_config(&root);
        let scenario = json!({"name": "sweep_demo", "scene": "res://main.tscn"});
        let scenario_path = root.join("qa/scenarios/sweep_demo.json");

        let result = run_scenario_sweep_with_runner(
            &config,
            &scenario_path,
            &scenario,
            &ScenarioSweepOptions {
                seeds: vec![1, 2],
                ..ScenarioSweepOptions::default()
            },
            // Seed 1 records a match summary; seed 2 records nothing, so the
            // sweep must surface seed 1's evidence and leave seed 2's null.
            |child_config, _path, _scenario, _command_line, _headless, _audio| {
                let mut data = Map::from_iter([(
                    "artifacts".to_string(),
                    json!({"run_root": child_config.artifacts_dir.join("run")}),
                )]);
                if child_config.artifacts_dir.ends_with("seed-1") {
                    data.insert(
                        "state_records".to_string(),
                        json!({
                            "match_summary": {
                                "name": "match_summary",
                                "step_index": 1,
                                "source": "provider",
                                "provider": "match_state",
                                "value": {"winner": "team_a"}
                            }
                        }),
                    );
                }
                CommandResult {
                    ok: true,
                    command: "scenario run".to_string(),
                    message: String::new(),
                    data,
                    failures: Vec::new(),
                    exit_code: 0,
                }
            },
        );

        assert!(result.ok, "{:?}", result.failures);
        assert_eq!(
            result.data["per_seed"][0]["state_records"]["match_summary"]["value"]["winner"],
            json!("team_a")
        );
        assert_eq!(result.data["per_seed"][1]["state_records"], json!(null));
        let _ = fs::remove_dir_all(root);
    }

    fn temp_root(label: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("godot-qa-sweep-{label}-{unique}"));
        let _ = fs::create_dir_all(&root);
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
            project_config_failure: None,
            artifacts_keep_runs: None,
            scenario_schedule_default: None,
            log_ignore_patterns: Vec::new(),
            config_warnings: Vec::new(),
        }
    }
}
