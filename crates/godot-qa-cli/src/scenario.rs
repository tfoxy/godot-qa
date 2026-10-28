#![allow(unused_imports)]

use crate::args::*;
use crate::*;
use godot_qa_core::config::{ProjectConfig, load_project_config};
use godot_qa_core::results::{CommandResult, FailureDetail};
use godot_qa_runtime as runtime;
use godot_qa_scenario::{
    artifact_query, artifact_summary,
    audio::{AudioAnalysisOptions, analyze_wav_file_with_options},
    bundle as scenario_bundle, bundle_multi as scenario_bundle_multi,
    bundle_schedule::BundleSchedulePolicy,
    bundle_selection::BundleSelection,
    check as scenario_check, contact_sheet as scenario_contact_sheet, edit as scenario_edit,
    init as scenario_init, lint as scenario_lint, live_gate, record as scenario_record, retention,
    review as scenario_review, run as scenario_run, scenario_doc,
    screenshots as scenario_screenshots, sweep as scenario_sweep, time_control, visual_audit,
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

pub(crate) fn scenario_list_result(cli: &Cli) -> CommandResult {
    let config = load_project_config(
        cli.project.as_deref().map(Path::new),
        cli.godot_bin.as_deref(),
        cli.artifact_dir.as_deref(),
        cli.verbose,
        None,
    );
    let command = "scenario list";
    let scenarios = scenario_doc::list_scenarios(&config.scenarios_dir);
    let mut failures = Vec::new();

    if !config.project_root.exists() {
        failures.push(missing_project_root_failure(command, &config.project_root));
    }

    let mut data = serde_json::Map::from_iter([("scenarios".to_string(), json!(scenarios))]);
    if config.verbose {
        let mut diagnostics = verbose_diagnostics(&config, cli)
            .as_object()
            .cloned()
            .unwrap_or_default();
        diagnostics.insert("scenario_files_scanned".to_string(), json!(scenarios.len()));
        data.insert(
            "diagnostics".to_string(),
            serde_json::Value::Object(diagnostics),
        );
    }

    let ok = failures.is_empty();
    CommandResult {
        ok,
        command: command.to_string(),
        message: if ok {
            format!("found {} scenario(s)", scenarios.len())
        } else {
            "unable to list scenarios".to_string()
        },
        data,
        failures,
        exit_code: if ok { 0 } else { 1 },
    }
}

pub(crate) fn scenario_lint_result(cli: &Cli, args: &ScenarioLintArgs) -> CommandResult {
    let config = load_project_config(
        cli.project.as_deref().map(Path::new),
        cli.godot_bin.as_deref(),
        cli.artifact_dir.as_deref(),
        cli.verbose,
        None,
    );
    scenario_lint::run_scenario_lint(
        &config,
        scenario_lint::LintOptions {
            scenario: args.scenario.clone(),
            strict: args.strict,
        },
    )
}

pub(crate) fn scenario_bundle_list_result(cli: &Cli) -> CommandResult {
    let config = load_project_config(
        cli.project.as_deref().map(Path::new),
        cli.godot_bin.as_deref(),
        cli.artifact_dir.as_deref(),
        cli.verbose,
        None,
    );
    let command = "scenario bundle list";
    let mut failures = Vec::new();
    let gates_dir = config.project_root.join("qa/gates");

    if !config.project_root.exists() {
        failures.push(missing_project_root_failure(command, &config.project_root));
    }
    if !gates_dir.is_dir() {
        failures.push(missing_gates_dir_failure(command, &gates_dir));
    }
    if !failures.is_empty() {
        return CommandResult {
            ok: false,
            command: command.to_string(),
            message: "unable to list scenario bundles".to_string(),
            data: serde_json::Map::new(),
            failures,
            exit_code: 1,
        };
    }

    let listed = scenario_bundle::list_bundles(&config);
    let bundles: Vec<serde_json::Value> = listed
        .bundles
        .iter()
        .map(|bundle| {
            json!({
                "name": bundle.name,
                "path": bundle.path,
                "purpose": bundle.purpose,
                "required_bundles": bundle.required_bundles,
                "required_scenarios": bundle.required_scenarios,
                "optional_scenarios": bundle.optional_scenarios,
                "recommended_manual_checks": bundle.recommended_manual_checks
            })
        })
        .collect();
    let failures: Vec<FailureDetail> = listed
        .failures
        .clone()
        .into_iter()
        .map(|mut failure| {
            if failure.command.is_none() {
                failure.command = Some(command.to_string());
            }
            failure
        })
        .collect();

    let data = serde_json::Map::from_iter([("bundles".to_string(), json!(bundles))]);
    let ok = failures.is_empty();

    CommandResult {
        ok,
        command: command.to_string(),
        message: if ok {
            format!("found {} bundle(s)", listed.bundles.len())
        } else {
            "unable to list scenario bundles".to_string()
        },
        data,
        failures,
        exit_code: if ok { 0 } else { 1 },
    }
}

pub(crate) fn live_gate_list_result(cli: &Cli) -> CommandResult {
    let config = load_project_config(
        cli.project.as_deref().map(Path::new),
        cli.godot_bin.as_deref(),
        cli.artifact_dir.as_deref(),
        cli.verbose,
        None,
    );
    let command = "live-gate list";
    let mut failures = Vec::new();
    let gates_dir = live_gate::live_gates_dir(&config);

    if !config.project_root.exists() {
        failures.push(missing_project_root_failure(command, &config.project_root));
    }
    if !gates_dir.is_dir() {
        failures.push(missing_live_gates_dir_failure(command, &gates_dir));
    }
    if !failures.is_empty() {
        return CommandResult {
            ok: false,
            command: command.to_string(),
            message: "unable to list live gates".to_string(),
            data: serde_json::Map::new(),
            failures,
            exit_code: 1,
        };
    }

    let listed = live_gate::list_live_gates(&config);
    let gates: Vec<serde_json::Value> = listed
        .gates
        .iter()
        .map(|gate| {
            json!({
                "name": gate.name,
                "path": gate.path,
                "scene": gate.scene,
                "overlay": gate.overlay_summary(),
                "required_anchor_count": gate.required_anchors.len(),
                "optional_anchor_count": gate.optional_anchors.len(),
                "methods": gate.methods,
                "probes": gate.probes
            })
        })
        .collect();
    let failures: Vec<FailureDetail> = listed
        .failures
        .clone()
        .into_iter()
        .map(|mut failure| {
            if failure.command.is_none() {
                failure.command = Some(command.to_string());
            }
            failure
        })
        .collect();

    let data = serde_json::Map::from_iter([("gates".to_string(), json!(gates))]);
    let ok = failures.is_empty();

    CommandResult {
        ok,
        command: command.to_string(),
        message: if ok {
            format!("found {} live gate(s)", listed.gates.len())
        } else {
            "unable to list live gates".to_string()
        },
        data,
        failures,
        exit_code: if ok { 0 } else { 1 },
    }
}

pub(crate) fn live_gate_run_result(cli: &Cli, args: &LiveGateRunArgs) -> CommandResult {
    let config = load_project_config(
        cli.project.as_deref().map(Path::new),
        cli.godot_bin.as_deref(),
        cli.artifact_dir.as_deref(),
        cli.verbose,
        godot_qa_process::resolve_godot_binary(cli.godot_bin.as_deref()),
    );
    let show_overlay = if args.show_overlay {
        Some(true)
    } else if args.hide_overlay {
        Some(false)
    } else {
        None
    };
    live_gate::run_live_gate(
        &config,
        &args.name,
        live_gate::LiveGateRunOptions {
            strict: args.strict,
            attach: args.attach,
            scene: args.scene.clone(),
            show_overlay,
            viewport: args.viewport.clone(),
            headless: args.headless || config.launch_mode_headless_override().unwrap_or(false),
        },
    )
}

pub(crate) fn scenario_preflight_result(
    cli: &Cli,
    command: &str,
    scenario_name: &str,
) -> Option<CommandResult> {
    let config = load_project_config(
        cli.project.as_deref().map(Path::new),
        cli.godot_bin.as_deref(),
        cli.artifact_dir.as_deref(),
        cli.verbose,
        godot_qa_process::resolve_godot_binary(cli.godot_bin.as_deref()),
    );
    let failed_message = format!("{command} failed");
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
    if !config.scenarios_dir.is_dir() {
        failures.push(missing_scenarios_dir_failure(
            command,
            &config.scenarios_dir,
        ));
    }
    if !config.addon_dir.join("plugin.cfg").exists() {
        failures.push(FailureDetail {
            kind: "missing_addon".to_string(),
            command: Some(command.to_string()),
            path: Some(config.addon_dir.to_string_lossy().into_owned()),
            message: Some("godot_qa addon must be installed in the target project".to_string()),
            expected: Some(json!("installed addon in target project")),
            actual: Some(json!("missing")),
            ..FailureDetail::default()
        });
    }
    if !failures.is_empty() {
        return Some(CommandResult {
            ok: false,
            command: command.to_string(),
            message: failed_message,
            data: serde_json::Map::new(),
            failures,
            exit_code: 1,
        });
    }

    let matches = scenario_doc::find_scenarios_by_name(&config.scenarios_dir, scenario_name);
    if matches.is_empty() {
        return Some(CommandResult {
            ok: false,
            command: command.to_string(),
            message: failed_message,
            data: serde_json::Map::new(),
            failures: vec![FailureDetail {
                kind: "missing_scenario".to_string(),
                command: Some(command.to_string()),
                path: Some(config.scenarios_dir.to_string_lossy().into_owned()),
                message: Some(format!("scenario '{scenario_name}' was not found")),
                expected: Some(json!("existing scenario file by declared name or stem")),
                actual: Some(json!(scenario_name)),
                ..FailureDetail::default()
            }],
            exit_code: 1,
        });
    }
    if matches.len() > 1 {
        return Some(CommandResult {
            ok: false,
            command: command.to_string(),
            message: failed_message,
            data: serde_json::Map::new(),
            failures: vec![FailureDetail {
                kind: "scenario_name_ambiguous".to_string(),
                command: Some(command.to_string()),
                path: Some(config.scenarios_dir.to_string_lossy().into_owned()),
                message: Some(format!(
                    "scenario name '{scenario_name}' matched multiple files"
                )),
                expected: Some(json!("unique scenario file by declared name or stem")),
                actual: Some(json!(matches)),
                ..FailureDetail::default()
            }],
            exit_code: 1,
        });
    }

    let scenario_path = matches.first().cloned().unwrap();
    if let Err(error) = scenario_doc::load_and_normalize_scenario_document(&scenario_path) {
        return Some(CommandResult {
            ok: false,
            command: command.to_string(),
            message: failed_message,
            data: serde_json::Map::new(),
            failures: vec![FailureDetail {
                kind: error.failure.kind,
                command: Some(command.to_string()),
                path: error.failure.path,
                message: error.failure.message,
                expected: error.failure.expected,
                actual: error.failure.actual,
                ..FailureDetail::default()
            }],
            exit_code: 1,
        });
    }

    None
}

pub(crate) fn scenario_run_preflight_result(
    cli: &Cli,
    args: &ScenarioRunArgs,
) -> Option<CommandResult> {
    if args.file.is_some() {
        return None;
    }
    scenario_preflight_result(cli, "scenario run", scenario_run_name(args))
}

pub(crate) fn scenario_run_result(cli: &Cli, args: &ScenarioRunArgs) -> CommandResult {
    if let Some(result) = scenario_run_preflight_result(cli, args) {
        return result;
    }
    let config = load_project_config(
        cli.project.as_deref().map(Path::new),
        cli.godot_bin.as_deref(),
        cli.artifact_dir.as_deref(),
        cli.verbose,
        godot_qa_process::resolve_godot_binary(cli.godot_bin.as_deref()),
    );
    let scenario_path = match scenario_run_path(&config, args) {
        Ok(path) => path,
        Err(failure) => {
            return CommandResult {
                ok: false,
                command: "scenario run".to_string(),
                message: "scenario run failed".to_string(),
                data: serde_json::Map::new(),
                failures: vec![failure],
                exit_code: 1,
            };
        }
    };
    let scenario = match scenario_doc::load_and_normalize_scenario_document(&scenario_path) {
        Ok(scenario) => scenario,
        Err(error) => {
            return CommandResult {
                ok: false,
                command: "scenario run".to_string(),
                message: "scenario run failed".to_string(),
                data: serde_json::Map::new(),
                failures: vec![FailureDetail {
                    kind: error.failure.kind,
                    command: Some("scenario run".to_string()),
                    path: error.failure.path,
                    message: error.failure.message,
                    expected: error.failure.expected,
                    actual: error.failure.actual,
                    ..FailureDetail::default()
                }],
                exit_code: 1,
            };
        }
    };
    if args.target == SessionTargetArg::Android {
        return append_config_warnings(
            apply_scenario_run_retention(
                scenario_run::run_scenario_once_android(
                    &config,
                    &scenario_path,
                    &scenario,
                    &scenario_run_command_string(cli, args),
                    scenario_run::ScenarioRunAndroidOptions {
                        preset: args.preset.as_deref(),
                        device: args.device.as_deref(),
                        package: args.package_id.as_deref(),
                        no_apk_cache: args.no_apk_cache,
                        audio: args.audio,
                        no_audio: args.no_audio,
                        headless: args.headless,
                        windowed: args.windowed,
                        visual_risk: args.visual_risk,
                        visual_risk_strict: args.visual_risk_strict,
                    },
                ),
                &config,
                &scenario,
            ),
            &config,
        );
    }
    // Desktop must not silently ignore the Android deployment flags.
    if let Some(failure) = scenario_run::desktop_rejects_android_scenario_flags(
        args.preset.as_deref(),
        args.device.as_deref(),
        args.package_id.as_deref(),
        args.no_apk_cache,
    ) {
        return CommandResult {
            ok: false,
            command: "scenario run".to_string(),
            message: "scenario run failed".to_string(),
            data: serde_json::Map::new(),
            failures: vec![failure],
            exit_code: 1,
        };
    }
    append_config_warnings(
        apply_scenario_run_retention(
            scenario_run::run_scenario_once(
                &config,
                &scenario_path,
                &scenario,
                &scenario_run_command_string(cli, args),
                scenario_run_headless_override(args)
                    .or_else(|| config.launch_mode_headless_override()),
                scenario_run_audio_override(args).or_else(|| config.audio_mode_override()),
                scenario_run_visual_risk_options(args),
            ),
            &config,
            &scenario,
        ),
        &config,
    )
}

/// Enforce `artifacts.keep_runs` after one `scenario run` and record what it removed under
/// `data.retention`. A no-op — no `retention` key at all — unless the project opted in.
///
/// This deliberately lives at the CLI command layer instead of inside `run_scenario_once` or
/// `finalize_runtime_run`: those are shared with warm bundle children that execute in
/// parallel threads, and pruning from there would have sibling runs of the same scenario
/// deleting each other's freshly written run roots. A prune error is reported as an
/// `artifact_prune_failed` warning and never changes `ok`, `failures`, or the exit code.
fn apply_scenario_run_retention(
    mut result: CommandResult,
    config: &ProjectConfig,
    scenario: &Value,
) -> CommandResult {
    let Some(keep) = config.artifacts_keep_runs else {
        return result;
    };
    let Some(scenario_name) = scenario.get("name").and_then(Value::as_str) else {
        return result;
    };
    let report = retention::prune_scenario_runs(&config.artifacts_dir, scenario_name, keep);
    result.data.insert(
        "retention".to_string(),
        json!({
            "keep_runs": keep,
            "pruned": {"scenario_runs": report.removed.len()},
            "errors": report.errors,
        }),
    );
    if !report.errors.is_empty() {
        let warning = FailureDetail {
            kind: "artifact_prune_failed".to_string(),
            command: Some("scenario run".to_string()),
            path: Some(config.artifacts_dir.to_string_lossy().into_owned()),
            message: Some(format!(
                "{} artifact directories could not be pruned",
                report.errors.len()
            )),
            expected: Some(json!(
                "stale run directories removed under artifacts.keep_runs"
            )),
            actual: Some(json!(report.errors)),
            severity: Some("warning".to_string()),
            ..FailureDetail::default()
        };
        let entry = result
            .data
            .entry("warnings".to_string())
            .or_insert_with(|| Value::Array(Vec::new()));
        if let Some(array) = entry.as_array_mut() {
            array.push(json!(warning));
        }
    }
    result
}

pub(crate) fn scenario_sweep_result(cli: &Cli, args: &ScenarioSweepArgs) -> CommandResult {
    let seeds = match scenario_sweep::expand_sweep_seeds(
        &args.seeds,
        args.seed_start,
        args.seed_count,
        args.repeat,
    ) {
        Ok(seeds) => seeds,
        Err(failure) => {
            return CommandResult {
                ok: false,
                command: "scenario sweep".to_string(),
                message: "scenario sweep failed".to_string(),
                data: serde_json::Map::new(),
                failures: vec![failure],
                exit_code: 2,
            };
        }
    };
    if args.file.is_none() {
        if let Some(result) =
            scenario_preflight_result(cli, "scenario sweep", scenario_sweep_name(args))
        {
            return result;
        }
    }
    let config = load_project_config(
        cli.project.as_deref().map(Path::new),
        cli.godot_bin.as_deref(),
        cli.artifact_dir.as_deref(),
        cli.verbose,
        godot_qa_process::resolve_godot_binary(cli.godot_bin.as_deref()),
    );
    let scenario_path = match scenario_sweep_path(&config, args) {
        Ok(path) => path,
        Err(failure) => {
            return CommandResult {
                ok: false,
                command: "scenario sweep".to_string(),
                message: "scenario sweep failed".to_string(),
                data: serde_json::Map::new(),
                failures: vec![failure],
                exit_code: 1,
            };
        }
    };
    let scenario = match scenario_doc::load_and_normalize_scenario_document(&scenario_path) {
        Ok(scenario) => scenario,
        Err(error) => {
            return CommandResult {
                ok: false,
                command: "scenario sweep".to_string(),
                message: "scenario sweep failed".to_string(),
                data: serde_json::Map::new(),
                failures: vec![FailureDetail {
                    kind: error.failure.kind,
                    command: Some("scenario sweep".to_string()),
                    path: error.failure.path,
                    message: error.failure.message,
                    expected: error.failure.expected,
                    actual: error.failure.actual,
                    ..FailureDetail::default()
                }],
                exit_code: 1,
            };
        }
    };
    scenario_sweep::run_scenario_sweep(
        &config,
        &scenario_path,
        &scenario,
        &scenario_sweep::ScenarioSweepOptions {
            seeds,
            require_all_pass: args.require_all_pass,
            headless_override: scenario_sweep_headless_override(args)
                .or_else(|| config.launch_mode_headless_override()),
            audio_override: scenario_sweep_audio_override(args)
                .or_else(|| config.audio_mode_override()),
            command_line: scenario_sweep_command_string(cli, args),
            extra_project_args: args.project_args.clone(),
        },
    )
}

pub(crate) fn scenario_sweep_name(args: &ScenarioSweepArgs) -> &str {
    args.name.as_deref().unwrap_or_default()
}

pub(crate) fn scenario_sweep_path(
    config: &ProjectConfig,
    args: &ScenarioSweepArgs,
) -> Result<PathBuf, FailureDetail> {
    if let Some(file) = args.file.as_deref() {
        return external_harness_path(config, "scenario sweep", file);
    }
    Ok(
        scenario_doc::find_scenario_by_name(&config.scenarios_dir, scenario_sweep_name(args))
            .unwrap_or_else(|| config.scenarios_dir.join(scenario_sweep_name(args))),
    )
}

pub(crate) fn scenario_sweep_headless_override(args: &ScenarioSweepArgs) -> Option<bool> {
    if args.headless {
        Some(true)
    } else if args.windowed {
        Some(false)
    } else {
        None
    }
}

pub(crate) fn scenario_profile_result(cli: &Cli, args: &ScenarioProfileArgs) -> CommandResult {
    if let Some(result) = scenario_preflight_result(cli, "scenario profile", &args.name) {
        return result;
    }
    let config = load_project_config(
        cli.project.as_deref().map(Path::new),
        cli.godot_bin.as_deref(),
        cli.artifact_dir.as_deref(),
        cli.verbose,
        godot_qa_process::resolve_godot_binary(cli.godot_bin.as_deref()),
    );
    let scenario_path = scenario_doc::find_scenario_by_name(&config.scenarios_dir, &args.name)
        .unwrap_or_else(|| config.scenarios_dir.join(&args.name));
    let mut scenario = match scenario_doc::load_and_normalize_scenario_document(&scenario_path) {
        Ok(scenario) => scenario,
        Err(error) => {
            return CommandResult {
                ok: false,
                command: "scenario profile".to_string(),
                message: "scenario profile failed".to_string(),
                data: serde_json::Map::new(),
                failures: vec![FailureDetail {
                    kind: error.failure.kind,
                    command: Some("scenario profile".to_string()),
                    path: error.failure.path,
                    message: error.failure.message,
                    expected: error.failure.expected,
                    actual: error.failure.actual,
                    ..FailureDetail::default()
                }],
                exit_code: 1,
            };
        }
    };
    if let Some(spec_path) = &args.soak {
        let spec = match godot_qa_core::soak::load_spec(Path::new(spec_path), "scenario profile") {
            Ok(spec) => spec,
            Err(failure) => {
                return CommandResult {
                    ok: false,
                    command: "scenario profile".into(),
                    message: "scenario profile failed".into(),
                    data: Map::new(),
                    failures: vec![failure],
                    exit_code: 1,
                };
            }
        };
        let realtime = scenario
            .get("time_control")
            .and_then(|v| v.get("time_scale"))
            .and_then(Value::as_f64)
            == Some(1.0)
            && scenario
                .get("determinism")
                .and_then(|v| v.get("fixed_fps"))
                .is_none();
        if !realtime {
            return invalid_params_result(
                "scenario profile",
                FailureDetail {
                    kind: "soak_realtime_required".into(),
                    command: Some("scenario profile".into()),
                    message: Some(
                        "soak requires scenario time_control.time_scale: 1.0 and no fixed_fps"
                            .into(),
                    ),
                    expected: Some(json!({"time_control":{"time_scale":1.0}})),
                    actual: Some(json!(scenario.get("time_control"))),
                    ..FailureDetail::default()
                },
            );
        }
        if let Some(object) = scenario.as_object_mut() {
            object.insert("soak".into(), spec);
        }
    }
    append_config_warnings(
        scenario_run::run_scenario_profile_once(
            &config,
            &scenario_path,
            &scenario,
            &scenario_profile_command_string(cli, args),
            scenario_profile_headless_override(args)
                .or_else(|| config.launch_mode_headless_override()),
            scenario_profile_audio_override(args).or_else(|| config.audio_mode_override()),
        ),
        &config,
    )
}

pub(crate) fn scenario_bundle_run_result(cli: &Cli, args: &ScenarioBundleRunArgs) -> CommandResult {
    if args.retry_infrastructure_startup == 0 || args.retry_infrastructure_startup > 10 {
        return invalid_bundle_retry_policy_result(
            "retry-startup",
            "integer from 1 through 10",
            json!(args.retry_infrastructure_startup),
        );
    }
    if args.retry_backoff_ms > 60_000 {
        return invalid_bundle_retry_policy_result(
            "retry-startup-backoff-ms",
            "integer from 0 through 60000",
            json!(args.retry_backoff_ms),
        );
    }
    if let Some(rejection) = invalid_bundle_names_result(args) {
        return rejection;
    }
    let config = load_project_config(
        cli.project.as_deref().map(Path::new),
        cli.godot_bin.as_deref(),
        cli.artifact_dir.as_deref(),
        cli.verbose,
        godot_qa_process::resolve_godot_binary(cli.godot_bin.as_deref()),
    );
    // --cold and --no-warm-strict always win over a project config default,
    // so a project that defaults to warm/warm_strict still has a one-off
    // escape hatch: --cold alone forces a plain cold run outright (strict is
    // moot without warm), and --no-warm-strict alone disables strict for this
    // run even if config defaults it on. Validation only fires when strict
    // was requested (by flag or config) and warm was never requested at all
    // (by flag or config) — not merely turned off by one of those escape
    // hatches, which are deliberate, not a misconfiguration.
    let warm_requested = args.warm || config.scenario_warm_default;
    let warm_strict_requested = args.warm_strict || config.scenario_warm_strict_default;
    if !args.cold && !args.no_warm_strict && warm_strict_requested && !warm_requested {
        return invalid_warm_strict_without_warm_result();
    }
    let warm = !args.cold && warm_requested;
    let warm_strict = warm && !args.no_warm_strict && warm_strict_requested;
    // Warm now uses a POOL of independent live sessions (one Godot process each),
    // so warm scales with --jobs exactly like cold: the pool is sized to the
    // resolved parallelism and distributes headless-warm scenarios across K
    // concurrent sessions. --jobs 1 collapses the pool to a single serial session.
    // The flag wins over the qa/config.yaml scenario.jobs project default.
    let (jobs, jobs_origin) = match args.jobs {
        Some(jobs) => (jobs, "flag"),
        None => match config.scenario_jobs_default {
            Some(jobs) => (jobs, "config"),
            None => (0, "default"),
        },
    };
    // The dispatch order resolves exactly like --jobs: the flag wins, then the
    // qa/config.yaml scenario.schedule project default, then the built-in
    // longest-first. The origin rides along so a run can say which of the three it
    // took without the reader having to reconstruct it.
    let (schedule, schedule_origin) = match args.schedule {
        Some(BundleScheduleArg::Manifest) => (BundleSchedulePolicy::Manifest, "flag"),
        Some(BundleScheduleArg::LongestFirst) => (BundleSchedulePolicy::LongestFirst, "flag"),
        None => match config.scenario_schedule_default {
            Some(schedule) => (BundleSchedulePolicy::from(schedule), "config"),
            None => (BundleSchedulePolicy::default(), "default"),
        },
    };
    let options = scenario_bundle::ScenarioBundleRunOptions {
        strict: args.strict,
        warm_strict,
        headless_override: scenario_bundle_run_headless_override(args)
            .or_else(|| config.launch_mode_headless_override()),
        audio_override: scenario_bundle_run_audio_override(args)
            .or_else(|| config.audio_mode_override()),
        command_line: scenario_bundle_run_command_string(cli, args),
        retry_policy: scenario_bundle::BundleRetryPolicy {
            max_infrastructure_startup_attempts: args.retry_infrastructure_startup,
            backoff_ms: args.retry_backoff_ms,
        },
        jobs,
        jobs_origin: jobs_origin.to_string(),
        selection: BundleSelection {
            only: args.only.clone(),
            failed_only: args.failed_only,
        },
        schedule,
        schedule_origin: schedule_origin.to_string(),
        ..Default::default()
    };
    // One name keeps today's dispatch verbatim, so a single-bundle invocation's envelope is
    // byte-identical to what it was before multi-bundle runs existed. Several names go to a
    // driver that owns the run-level resources: warm shares one pool across them, cold just
    // loops. Both preflight every name before anything runs.
    let multi = args.names.len() > 1;
    let name = &args.names[0];
    if warm {
        if args.equivalence_check {
            return append_config_warnings(
                scenario_run::run_scenario_bundle_warm_equivalence(&config, name, &options),
                &config,
            );
        }
        if multi {
            return append_config_warnings(
                scenario_run::run_scenario_bundles_warm(&config, &args.names, &options),
                &config,
            );
        }
        return append_config_warnings(
            scenario_run::run_scenario_bundle_warm(&config, name, &options),
            &config,
        );
    }
    if multi {
        return append_config_warnings(
            scenario_bundle_multi::run_scenario_bundles(&config, &args.names, &options),
            &config,
        );
    }
    append_config_warnings(
        scenario_bundle::run_scenario_bundle(&config, name, &options),
        &config,
    )
}

/// Reject a bundle-name list that cannot mean what it says, before the project config is
/// even loaded. Exit code 2 keeps "the invocation was wrong" distinct from "a gate failed".
///
/// A repeated name is always a mistake: the second run of the same bundle would overwrite
/// the first's `latest` link and produce two `data.bundles[]` entries with one name, and no
/// caller wants that. The per-bundle flags are rejected with several names because their
/// meaning is per bundle and v1 has nowhere to say which bundle a name belongs to:
/// `--only`/`--failed-only` select children of ONE bundle, and `--equivalence-check` runs
/// each scenario twice against a pool it must own alone.
fn invalid_bundle_names_result(args: &ScenarioBundleRunArgs) -> Option<CommandResult> {
    let mut seen: Vec<&str> = Vec::new();
    let mut duplicates: Vec<&str> = Vec::new();
    for name in &args.names {
        if seen.contains(&name.as_str()) {
            if !duplicates.contains(&name.as_str()) {
                duplicates.push(name);
            }
        } else {
            seen.push(name);
        }
    }
    if !duplicates.is_empty() {
        return Some(invalid_bundle_arguments_result(
            format!(
                "{} named more than once",
                if duplicates.len() == 1 {
                    "a bundle is"
                } else {
                    "several bundles are"
                }
            ),
            json!("each bundle named at most once"),
            json!(duplicates),
        ));
    }
    if args.names.len() < 2 {
        return None;
    }
    for (flag, requested) in [
        ("--equivalence-check", args.equivalence_check),
        ("--only", !args.only.is_empty()),
        ("--failed-only", args.failed_only),
    ] {
        if requested {
            return Some(invalid_bundle_arguments_result(
                format!("{flag} applies to a single bundle"),
                json!(format!("{flag} with exactly one bundle name")),
                json!({"flag": flag, "bundles": args.names}),
            ));
        }
    }
    None
}

/// The exit-2 shape for an invocation whose arguments cannot be honoured, alongside
/// `invalid_retry_policy` and `warm_strict_requires_warm`. Nothing has been loaded, launched
/// or written when this is returned.
fn invalid_bundle_arguments_result(
    message: String,
    expected: Value,
    actual: Value,
) -> CommandResult {
    CommandResult {
        ok: false,
        command: "scenario bundle run".to_string(),
        message: "scenario bundle run failed".to_string(),
        data: serde_json::Map::new(),
        failures: vec![FailureDetail {
            kind: "invalid_arguments".to_string(),
            command: Some("scenario bundle run".to_string()),
            message: Some(message),
            expected: Some(expected),
            actual: Some(actual),
            ..FailureDetail::default()
        }],
        exit_code: 2,
    }
}

pub(crate) fn invalid_warm_strict_without_warm_result() -> CommandResult {
    CommandResult {
        ok: false,
        command: "scenario bundle run".to_string(),
        message: "scenario bundle run failed".to_string(),
        data: serde_json::Map::new(),
        failures: vec![FailureDetail {
            kind: "warm_strict_requires_warm".to_string(),
            command: Some("scenario bundle run".to_string()),
            message: Some(
                "--warm-strict requires --warm (or a `scenario.warm: true` project config default)"
                    .to_string(),
            ),
            expected: Some(json!("--warm or scenario.warm: true in qa/config.yaml")),
            actual: Some(json!("warm_strict without warm")),
            ..FailureDetail::default()
        }],
        exit_code: 2,
    }
}

pub(crate) fn invalid_bundle_retry_policy_result(
    field: &str,
    expected: &str,
    actual: Value,
) -> CommandResult {
    CommandResult {
        ok: false,
        command: "scenario bundle run".to_string(),
        message: "scenario bundle run failed".to_string(),
        data: serde_json::Map::new(),
        failures: vec![FailureDetail {
            kind: "invalid_retry_policy".to_string(),
            command: Some("scenario bundle run".to_string()),
            message: Some(format!(
                "--{field} is outside the supported retry policy bounds"
            )),
            expected: Some(json!(expected)),
            actual: Some(actual),
            ..FailureDetail::default()
        }],
        exit_code: 2,
    }
}

pub(crate) fn scenario_bundle_run_headless_override(args: &ScenarioBundleRunArgs) -> Option<bool> {
    if args.headless {
        Some(true)
    } else if args.windowed {
        Some(false)
    } else {
        None
    }
}

pub(crate) fn scenario_bundle_run_audio_override(args: &ScenarioBundleRunArgs) -> Option<bool> {
    audio_override(args.audio, args.no_audio)
}

pub(crate) fn scenario_run_headless_override(args: &ScenarioRunArgs) -> Option<bool> {
    if args.headless {
        Some(true)
    } else if args.windowed {
        Some(false)
    } else {
        None
    }
}

pub(crate) fn scenario_run_audio_override(args: &ScenarioRunArgs) -> Option<bool> {
    audio_override(args.audio, args.no_audio)
}

pub(crate) fn scenario_sweep_audio_override(args: &ScenarioSweepArgs) -> Option<bool> {
    audio_override(args.audio, args.no_audio)
}

pub(crate) fn scenario_profile_headless_override(args: &ScenarioProfileArgs) -> Option<bool> {
    if args.headless {
        Some(true)
    } else if args.windowed {
        Some(false)
    } else {
        None
    }
}

pub(crate) fn scenario_profile_audio_override(args: &ScenarioProfileArgs) -> Option<bool> {
    audio_override(args.audio, args.no_audio)
}

/// Resolve the `--audio` / `--no-audio` mutually-exclusive pair to an override:
/// Some(true) for `--audio`, Some(false) for `--no-audio`, None otherwise (the
/// audio flavor is then chosen by the project config, then the scenario).
pub(crate) fn audio_override(audio: bool, no_audio: bool) -> Option<bool> {
    if audio {
        Some(true)
    } else if no_audio {
        Some(false)
    } else {
        None
    }
}

pub(crate) fn scenario_init_result(cli: &Cli, args: &ScenarioInitArgs) -> CommandResult {
    let config = load_project_config(
        cli.project.as_deref().map(Path::new),
        cli.godot_bin.as_deref(),
        cli.artifact_dir.as_deref(),
        cli.verbose,
        godot_qa_process::resolve_godot_binary(cli.godot_bin.as_deref()),
    );
    scenario_init::run_scenario_init(
        &config,
        &args.name,
        &args.scene,
        &args.viewport,
        args.out.as_deref(),
    )
}

pub(crate) fn review_list_templates_result(cli: &Cli) -> CommandResult {
    let _config = load_project_config(
        cli.project.as_deref().map(Path::new),
        cli.godot_bin.as_deref(),
        cli.artifact_dir.as_deref(),
        cli.verbose,
        godot_qa_process::resolve_godot_binary(cli.godot_bin.as_deref()),
    );
    scenario_review::run_review_list_templates()
}

pub(crate) fn review_init_result(cli: &Cli, args: &ReviewInitArgs) -> CommandResult {
    let config = load_project_config(
        cli.project.as_deref().map(Path::new),
        cli.godot_bin.as_deref(),
        cli.artifact_dir.as_deref(),
        cli.verbose,
        godot_qa_process::resolve_godot_binary(cli.godot_bin.as_deref()),
    );
    let mut targets = args.targets.clone();
    if let Some(trace_target) = args.trace_target.as_ref() {
        targets.push(trace_target.clone());
    }
    scenario_review::run_review_init(
        &config,
        &scenario_review::ReviewInitOptions {
            template_id: args.template.clone(),
            name: args.name.clone(),
            scene: args.scene.clone(),
            targets,
            project_args: args.project_args.clone(),
            frames: args.frames,
            screenshot_every: args.screenshot_every,
            audio_seconds: args.audio_seconds,
            event_provider: args.event_provider.clone(),
            out: args.out.clone(),
        },
    )
}

pub(crate) fn scenario_record_result(cli: &Cli, args: &ScenarioRecordArgs) -> CommandResult {
    let name = match args.resolved_name() {
        Ok(name) => name,
        Err(message) => {
            return CommandResult {
                ok: false,
                command: "scenario record".to_string(),
                message: "scenario recording failed".to_string(),
                data: serde_json::Map::new(),
                failures: vec![FailureDetail {
                    kind: "invalid_params".to_string(),
                    command: Some("scenario record".to_string()),
                    message: Some(message.to_string()),
                    ..FailureDetail::default()
                }],
                exit_code: 2,
            };
        }
    };
    let resolved_binary = godot_qa_process::resolve_godot_binary(cli.godot_bin.as_deref());
    let config = load_project_config(
        cli.project.as_deref().map(Path::new),
        cli.godot_bin.as_deref(),
        cli.artifact_dir.as_deref(),
        cli.verbose,
        resolved_binary,
    );
    scenario_record::run_scenario_record(
        &config,
        name,
        &args.scene,
        &args.viewport,
        args.out.as_deref(),
        scenario_record::RecordOptions {
            input_script: args.input_script.as_deref(),
            duration_frames: args.duration_frames,
            headless: args.headless,
            joy_motion: scenario_record::JoyMotionRecordOptions {
                step: args.joy_motion_step,
                deadzone: args.joy_motion_deadzone,
                frame_gap: args.joy_motion_frame_gap,
            },
        },
        &scenario_record_command_string(cli, args),
    )
}

pub(crate) fn screenshot_result(cli: &Cli, args: &ScreenshotArgs) -> CommandResult {
    let config = load_project_config(
        cli.project.as_deref().map(Path::new),
        cli.godot_bin.as_deref(),
        cli.artifact_dir.as_deref(),
        cli.verbose,
        godot_qa_process::resolve_godot_binary(cli.godot_bin.as_deref()),
    );
    match &args.command {
        ScreenshotCommands::AssertStartupPresented(args) => {
            crate::presented_startup::assert_startup_presented(&config, args)
        }
        ScreenshotCommands::Capture(args) => {
            if let Some(result) =
                run_setup_script_before(&config, "screenshot capture", args.setup_script.as_deref())
            {
                return result;
            }
            attach_setup_script_metadata(
                scenario_screenshots::capture_screenshot(
                    &config,
                    &args.out,
                    match screenshot_state_guards(
                        &args.require_visible,
                        &args.require_hidden,
                        args.require_state.as_deref(),
                        "screenshot capture",
                    ) {
                        Ok(guards) => guards,
                        Err(result) => return result,
                    },
                ),
                args.setup_script.as_deref(),
            )
        }
        ScreenshotCommands::Compare(args) => scenario_screenshots::compare_screenshots(
            &config,
            args.actual.as_deref(),
            args.baseline.as_deref(),
            args.diff.as_deref(),
            args.threshold,
            args.live,
        ),
        ScreenshotCommands::Sequence(args) => {
            if let Some(result) = run_setup_script_before(
                &config,
                "screenshot sequence",
                args.setup_script.as_deref(),
            ) {
                return result;
            }
            let scenario_resolution =
                match resolve_screenshot_sequence_scenario(&config, args.scenario.as_deref()) {
                    Ok(resolved) => resolved,
                    Err(result) => return result,
                };
            let stop_after_state = match parse_stop_after_state(
                "screenshot sequence",
                args.stop_after_state.as_deref(),
            ) {
                Ok(value) => value,
                Err(failure) => return invalid_params_result("screenshot sequence", failure),
            };
            let result = scenario_screenshots::capture_sequence(
                &config,
                scenario_screenshots::ScreenshotSequenceOptions {
                    capture_mode: args.capture_mode.clone(),
                    max_bytes: args.max_bytes,
                    max_seconds: args.max_seconds,
                    frames: args.frames,
                    every: args.every,
                    seconds: args.seconds,
                    fps: args.fps,
                    out: args.out.clone(),
                    scenario: scenario_resolution.scenario,
                    scenario_path: scenario_resolution.path,
                    scenario_scene: scenario_resolution.scene,
                    state_guards: match screenshot_state_guards(
                        &args.require_visible,
                        &args.require_hidden,
                        args.require_state.as_deref(),
                        "screenshot sequence",
                    ) {
                        Ok(guards) => guards,
                        Err(result) => return result,
                    },
                    stop_after_state,
                    allow_low_information: args.allow_low_information,
                    visual_risk: args.visual_risk || args.visual_risk_strict,
                    visual_risk_strict: args.visual_risk_strict,
                    contact_sheet: args.contact_sheet,
                    contact_sheet_columns: args.contact_sheet_columns,
                    contact_sheet_thumb_width: args.contact_sheet_thumb_width,
                    contact_sheet_out: args.contact_sheet_out.clone(),
                },
            );
            attach_setup_script_metadata(result, args.setup_script.as_deref())
        }
        ScreenshotCommands::ContactSheet(args) => scenario_contact_sheet::build_contact_sheet(
            &config,
            scenario_contact_sheet::ScreenshotContactSheetOptions {
                sequence_dir: args.sequence_dir.clone(),
                columns: args.columns,
                thumb_width: args.thumb_width,
                out: args.out.clone(),
            },
        ),
    }
}

fn screenshot_state_guards(
    require_visible: &[String],
    require_hidden: &[String],
    require_state: Option<&str>,
    command: &str,
) -> Result<scenario_screenshots::ScreenshotStateGuards, CommandResult> {
    let parsed_state = match require_state
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        Some(raw) => match serde_json::from_str::<Value>(raw) {
            Ok(value) if value.is_object() => Some(value),
            Ok(value) => {
                return Err(screenshot_state_guard_parse_failure(
                    command,
                    raw,
                    json!("JSON object"),
                    value,
                ));
            }
            Err(error) => {
                return Err(screenshot_state_guard_parse_failure(
                    command,
                    raw,
                    json!("valid state_matches JSON object"),
                    json!(error.to_string()),
                ));
            }
        },
        None => None,
    };
    Ok(scenario_screenshots::ScreenshotStateGuards {
        require_visible: require_visible
            .iter()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
            .collect(),
        require_hidden: require_hidden
            .iter()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
            .collect(),
        require_state: parsed_state,
    })
}

fn screenshot_state_guard_parse_failure(
    command: &str,
    raw: &str,
    expected: Value,
    actual: Value,
) -> CommandResult {
    CommandResult {
        ok: false,
        command: command.to_string(),
        message: format!("{command} failed"),
        data: Map::from_iter([("require_state".to_string(), json!(raw))]),
        failures: vec![FailureDetail {
            kind: "screenshot_state_guard_args_invalid".to_string(),
            command: Some(command.to_string()),
            message: Some("--require-state must be a valid JSON object".to_string()),
            expected: Some(expected),
            actual: Some(actual),
            ..FailureDetail::default()
        }],
        exit_code: 2,
    }
}

pub(crate) struct ScreenshotSequenceScenarioResolution {
    pub(crate) scenario: Option<String>,
    pub(crate) path: Option<String>,
    pub(crate) scene: Option<String>,
}

pub(crate) fn resolve_screenshot_sequence_scenario(
    config: &ProjectConfig,
    scenario_name: Option<&str>,
) -> Result<ScreenshotSequenceScenarioResolution, CommandResult> {
    let Some(raw_name) = scenario_name else {
        return Ok(ScreenshotSequenceScenarioResolution {
            scenario: None,
            path: None,
            scene: None,
        });
    };
    let name = raw_name.trim();
    if name.is_empty() {
        return Err(CommandResult {
            ok: false,
            command: "screenshot sequence".to_string(),
            message: "screenshot sequence failed".to_string(),
            data: Map::new(),
            failures: vec![FailureDetail {
                kind: "missing_scenario".to_string(),
                command: Some("screenshot sequence".to_string()),
                path: Some(config.scenarios_dir.to_string_lossy().into_owned()),
                message: Some(
                    "screenshot sequence --scenario requires a scenario name".to_string(),
                ),
                expected: Some(json!("existing scenario file by declared name")),
                actual: Some(json!(raw_name)),
                ..FailureDetail::default()
            }],
            exit_code: 1,
        });
    }

    let Some(scenario_path) = scenario_doc::find_scenario_by_name(&config.scenarios_dir, name)
    else {
        return Err(CommandResult {
            ok: false,
            command: "screenshot sequence".to_string(),
            message: "screenshot sequence failed".to_string(),
            data: Map::new(),
            failures: vec![FailureDetail {
                kind: "missing_scenario".to_string(),
                command: Some("screenshot sequence".to_string()),
                path: Some(config.scenarios_dir.to_string_lossy().into_owned()),
                message: Some(format!("scenario '{name}' was not found")),
                expected: Some(json!("existing scenario file by declared name")),
                actual: Some(json!(name)),
                ..FailureDetail::default()
            }],
            exit_code: 1,
        });
    };

    let matches = scenario_doc::find_scenarios_by_name(&config.scenarios_dir, name);
    if matches.len() > 1 {
        return Err(CommandResult {
            ok: false,
            command: "screenshot sequence".to_string(),
            message: "screenshot sequence failed".to_string(),
            data: Map::new(),
            failures: vec![FailureDetail {
                kind: "scenario_name_ambiguous".to_string(),
                command: Some("screenshot sequence".to_string()),
                path: Some(config.scenarios_dir.to_string_lossy().into_owned()),
                message: Some(format!("scenario name '{name}' matched multiple files")),
                expected: Some(json!("unique scenario file by declared name")),
                actual: Some(json!(matches)),
                ..FailureDetail::default()
            }],
            exit_code: 1,
        });
    }

    let scenario = match scenario_doc::load_and_normalize_scenario_document(&scenario_path) {
        Ok(scenario) => scenario,
        Err(error) => {
            return Err(CommandResult {
                ok: false,
                command: "screenshot sequence".to_string(),
                message: "screenshot sequence failed".to_string(),
                data: Map::new(),
                failures: vec![FailureDetail {
                    kind: error.failure.kind,
                    command: Some("screenshot sequence".to_string()),
                    path: error.failure.path,
                    message: error.failure.message,
                    expected: error.failure.expected,
                    actual: error.failure.actual,
                    ..FailureDetail::default()
                }],
                exit_code: 1,
            });
        }
    };

    Ok(ScreenshotSequenceScenarioResolution {
        scenario: scenario
            .get("name")
            .and_then(Value::as_str)
            .map(str::to_string)
            .or_else(|| Some(name.to_string())),
        path: Some(scenario_path.to_string_lossy().into_owned()),
        scene: scenario
            .get("scene")
            .and_then(Value::as_str)
            .map(str::to_string),
    })
}

pub(crate) fn scenario_run_command_string(cli: &Cli, args: &ScenarioRunArgs) -> String {
    let mut command = if let Some(file) = args.file.as_deref() {
        scenario_file_command_string(
            cli,
            "run",
            file,
            args.headless,
            args.windowed,
            args.audio,
            args.no_audio,
        )
    } else {
        scenario_command_string(
            cli,
            "run",
            scenario_run_name(args),
            args.headless,
            args.windowed,
            args.audio,
            args.no_audio,
        )
    };
    if args.visual_risk {
        command.push_str(" --visual-risk");
    }
    if args.visual_risk_strict {
        command.push_str(" --visual-risk-strict");
    }
    command
}

pub(crate) fn scenario_run_name(args: &ScenarioRunArgs) -> &str {
    args.name.as_deref().unwrap_or_default()
}

pub(crate) fn scenario_run_path(
    config: &ProjectConfig,
    args: &ScenarioRunArgs,
) -> Result<PathBuf, FailureDetail> {
    if let Some(file) = args.file.as_deref() {
        return external_harness_path(config, "scenario run", file);
    }
    Ok(
        scenario_doc::find_scenario_by_name(&config.scenarios_dir, scenario_run_name(args))
            .unwrap_or_else(|| config.scenarios_dir.join(scenario_run_name(args))),
    )
}

pub(crate) fn external_harness_path(
    config: &ProjectConfig,
    command: &str,
    raw_path: &str,
) -> Result<PathBuf, FailureDetail> {
    let trimmed = raw_path.trim();
    if trimmed.is_empty() {
        return Err(FailureDetail {
            kind: "invalid_path".to_string(),
            command: Some(command.to_string()),
            message: Some("external harness path must be non-empty".to_string()),
            expected: Some(json!("non-empty file path")),
            actual: Some(json!(raw_path)),
            ..FailureDetail::default()
        });
    }
    let candidate = PathBuf::from(trimmed);
    let path = if candidate.is_absolute() {
        candidate
    } else {
        config.project_root.join(candidate)
    };
    let supported = matches!(
        path.extension().and_then(|ext| ext.to_str()),
        Some("json" | "yaml" | "yml")
    );
    if !supported {
        return Err(FailureDetail {
            kind: "unsupported_file_type".to_string(),
            command: Some(command.to_string()),
            path: Some(path.to_string_lossy().into_owned()),
            message: Some("external harness files must be JSON or YAML".to_string()),
            expected: Some(json!(["json", "yaml", "yml"])),
            actual: Some(json!(
                path.extension()
                    .and_then(|ext| ext.to_str())
                    .unwrap_or_default()
            )),
            ..FailureDetail::default()
        });
    }
    if path.is_dir() {
        return Err(FailureDetail {
            kind: "path_is_directory".to_string(),
            command: Some(command.to_string()),
            path: Some(path.to_string_lossy().into_owned()),
            message: Some("external harness path must point to a file".to_string()),
            expected: Some(json!("file")),
            actual: Some(json!("directory")),
            ..FailureDetail::default()
        });
    }
    if !path.is_file() {
        return Err(FailureDetail {
            kind: "file_not_found".to_string(),
            command: Some(command.to_string()),
            path: Some(path.to_string_lossy().into_owned()),
            message: Some("external harness file was not found".to_string()),
            expected: Some(json!("existing file")),
            actual: Some(json!("missing")),
            ..FailureDetail::default()
        });
    }
    Ok(path)
}

/// Echo the `--audio` / `--no-audio` pair into a reproduced command string,
/// mirroring the `--headless` / `--windowed` echo (only the flag that was set).
fn append_audio_flags(parts: &mut Vec<String>, audio: bool, no_audio: bool) {
    if audio {
        parts.push("--audio".to_string());
    }
    if no_audio {
        parts.push("--no-audio".to_string());
    }
}

pub(crate) fn scenario_file_command_string(
    cli: &Cli,
    subcommand: &str,
    file: &str,
    headless: bool,
    windowed: bool,
    audio: bool,
    no_audio: bool,
) -> String {
    let mut parts = vec![
        "godot-qa".to_string(),
        "scenario".to_string(),
        subcommand.to_string(),
    ];
    if let Some(project) = &cli.project {
        parts.push("--project".to_string());
        parts.push(shell_escape(project));
    }
    if let Some(godot_bin) = &cli.godot_bin {
        parts.push("--godot-bin".to_string());
        parts.push(shell_escape(godot_bin));
    }
    if let Some(artifact_dir) = &cli.artifact_dir {
        parts.push("--artifact-dir".to_string());
        parts.push(shell_escape(artifact_dir));
    }
    parts.push("--file".to_string());
    parts.push(shell_escape(file));
    if headless {
        parts.push("--headless".to_string());
    }
    if windowed {
        parts.push("--windowed".to_string());
    }
    append_audio_flags(&mut parts, audio, no_audio);
    if cli.json {
        parts.push("--json".to_string());
    }
    parts.join(" ")
}

pub(crate) fn scenario_run_visual_risk_options(
    args: &ScenarioRunArgs,
) -> scenario_run::VisualRiskExecutionOptions {
    scenario_run::VisualRiskExecutionOptions {
        enabled: args.visual_risk || args.visual_risk_strict,
        strict: args.visual_risk_strict,
    }
}

pub(crate) fn scenario_profile_command_string(cli: &Cli, args: &ScenarioProfileArgs) -> String {
    scenario_command_string(
        cli,
        "profile",
        &args.name,
        args.headless,
        args.windowed,
        args.audio,
        args.no_audio,
    )
}

pub(crate) fn scenario_sweep_command_string(cli: &Cli, args: &ScenarioSweepArgs) -> String {
    let base = if let Some(file) = args.file.as_deref() {
        scenario_file_command_string(
            cli,
            "sweep",
            file,
            args.headless,
            args.windowed,
            args.audio,
            args.no_audio,
        )
    } else {
        scenario_command_string(
            cli,
            "sweep",
            scenario_sweep_name(args),
            args.headless,
            args.windowed,
            args.audio,
            args.no_audio,
        )
    };
    let mut parts = vec![base];
    if !args.seeds.is_empty() {
        parts.push("--seeds".to_string());
        parts.push(
            args.seeds
                .iter()
                .map(i64::to_string)
                .collect::<Vec<_>>()
                .join(","),
        );
    }
    if let Some(seed_start) = args.seed_start {
        parts.push("--seed-start".to_string());
        parts.push(seed_start.to_string());
    }
    if let Some(seed_count) = args.seed_count {
        parts.push("--seed-count".to_string());
        parts.push(seed_count.to_string());
    }
    if args.repeat != 1 {
        parts.push("--repeat".to_string());
        parts.push(args.repeat.to_string());
    }
    if args.require_all_pass {
        parts.push("--require-all-pass".to_string());
    }
    for project_arg in &args.project_args {
        parts.push("--project-arg".to_string());
        parts.push(shell_escape(project_arg));
    }
    parts.join(" ")
}

pub(crate) fn visual_preflight_command_string(cli: &Cli, args: &VisualPreflightArgs) -> String {
    let mut parts = vec![
        "godot-qa".to_string(),
        "visual".to_string(),
        "preflight".to_string(),
    ];
    if args.headless {
        parts.push("--headless".to_string());
    }
    if args.windowed {
        parts.push("--windowed".to_string());
    }
    if let Some(project) = &cli.project {
        parts.push("--project".to_string());
        parts.push(shell_escape(project));
    }
    if let Some(godot_bin) = &cli.godot_bin {
        parts.push("--godot-bin".to_string());
        parts.push(shell_escape(godot_bin));
    }
    if let Some(artifact_dir) = &cli.artifact_dir {
        parts.push("--artifact-dir".to_string());
        parts.push(shell_escape(artifact_dir));
    }
    if cli.json {
        parts.push("--json".to_string());
    }
    parts.join(" ")
}

pub(crate) fn scenario_command_string(
    cli: &Cli,
    subcommand: &str,
    name: &str,
    headless: bool,
    windowed: bool,
    audio: bool,
    no_audio: bool,
) -> String {
    let mut parts = vec![
        "godot-qa".to_string(),
        "scenario".to_string(),
        subcommand.to_string(),
        shell_escape(name),
    ];
    if headless {
        parts.push("--headless".to_string());
    }
    if windowed {
        parts.push("--windowed".to_string());
    }
    append_audio_flags(&mut parts, audio, no_audio);
    if let Some(project) = &cli.project {
        parts.push("--project".to_string());
        parts.push(shell_escape(project));
    }
    if let Some(godot_bin) = &cli.godot_bin {
        parts.push("--godot-bin".to_string());
        parts.push(shell_escape(godot_bin));
    }
    if let Some(artifact_dir) = &cli.artifact_dir {
        parts.push("--artifact-dir".to_string());
        parts.push(shell_escape(artifact_dir));
    }
    if cli.json {
        parts.push("--json".to_string());
    }
    parts.join(" ")
}

pub(crate) fn scenario_bundle_run_command_string(
    cli: &Cli,
    args: &ScenarioBundleRunArgs,
) -> String {
    let mut parts = vec![
        "godot-qa".to_string(),
        "scenario".to_string(),
        "bundle".to_string(),
        "run".to_string(),
    ];
    // Every name, so each bundle's `meta.json.command` reproduces the WHOLE invocation —
    // including the sibling bundles it shared a warm pool with, which is what explains its
    // timing and its (absent) pool counters.
    parts.extend(args.names.iter().map(|name| shell_escape(name)));
    if args.strict {
        parts.push("--strict".to_string());
    }
    if args.headless {
        parts.push("--headless".to_string());
    }
    if args.windowed {
        parts.push("--windowed".to_string());
    }
    append_audio_flags(&mut parts, args.audio, args.no_audio);
    if let Some(jobs) = args.jobs {
        parts.push("--jobs".to_string());
        parts.push(jobs.to_string());
    }
    if args.retry_infrastructure_startup != 1 {
        parts.push("--retry-startup".to_string());
        parts.push(args.retry_infrastructure_startup.to_string());
    }
    if args.retry_backoff_ms != 0 {
        parts.push("--retry-startup-backoff-ms".to_string());
        parts.push(args.retry_backoff_ms.to_string());
    }
    if args.warm {
        parts.push("--warm".to_string());
    }
    if args.equivalence_check {
        parts.push("--equivalence-check".to_string());
    }
    if args.warm_strict {
        parts.push("--warm-strict".to_string());
    }
    if args.cold {
        parts.push("--cold".to_string());
    }
    if args.no_warm_strict {
        parts.push("--no-warm-strict".to_string());
    }
    for only in &args.only {
        parts.push("--only".to_string());
        parts.push(shell_escape(only));
    }
    if args.failed_only {
        parts.push("--failed-only".to_string());
    }
    if let Some(schedule) = args.schedule {
        parts.push("--schedule".to_string());
        parts.push(
            match schedule {
                BundleScheduleArg::Manifest => "manifest",
                BundleScheduleArg::LongestFirst => "longest-first",
            }
            .to_string(),
        );
    }
    if let Some(project) = &cli.project {
        parts.push("--project".to_string());
        parts.push(shell_escape(project));
    }
    if let Some(godot_bin) = &cli.godot_bin {
        parts.push("--godot-bin".to_string());
        parts.push(shell_escape(godot_bin));
    }
    if let Some(artifact_dir) = &cli.artifact_dir {
        parts.push("--artifact-dir".to_string());
        parts.push(shell_escape(artifact_dir));
    }
    if cli.json {
        parts.push("--json".to_string());
    }
    parts.join(" ")
}

pub(crate) fn scenario_record_command_string(cli: &Cli, args: &ScenarioRecordArgs) -> String {
    let name = args.resolved_name().unwrap_or("<missing-name>");
    let mut parts = vec![
        "godot-qa".to_string(),
        "scenario".to_string(),
        "record".to_string(),
        shell_escape(name),
        "--scene".to_string(),
        shell_escape(&args.scene),
        "--viewport".to_string(),
        shell_escape(&args.viewport),
    ];
    if let Some(out) = &args.out {
        parts.push("--out".to_string());
        parts.push(shell_escape(out));
    }
    if let Some(input_script) = &args.input_script {
        parts.push("--input-script".to_string());
        parts.push(shell_escape(input_script));
    }
    if let Some(duration_frames) = args.duration_frames {
        parts.push("--duration-frames".to_string());
        parts.push(duration_frames.to_string());
    }
    if args.headless {
        parts.push("--headless".to_string());
    }
    let joy_defaults = scenario_record::JoyMotionRecordOptions::default();
    if args.joy_motion_step != joy_defaults.step {
        parts.push("--joy-motion-step".to_string());
        parts.push(args.joy_motion_step.to_string());
    }
    if args.joy_motion_deadzone != joy_defaults.deadzone {
        parts.push("--joy-motion-deadzone".to_string());
        parts.push(args.joy_motion_deadzone.to_string());
    }
    if args.joy_motion_frame_gap != joy_defaults.frame_gap {
        parts.push("--joy-motion-frame-gap".to_string());
        parts.push(args.joy_motion_frame_gap.to_string());
    }
    if let Some(project) = &cli.project {
        parts.push("--project".to_string());
        parts.push(shell_escape(project));
    }
    if let Some(godot_bin) = &cli.godot_bin {
        parts.push("--godot-bin".to_string());
        parts.push(shell_escape(godot_bin));
    }
    if let Some(artifact_dir) = &cli.artifact_dir {
        parts.push("--artifact-dir".to_string());
        parts.push(shell_escape(artifact_dir));
    }
    if cli.json {
        parts.push("--json".to_string());
    }
    if cli.verbose {
        parts.push("--verbose".to_string());
    }
    parts.join(" ")
}
