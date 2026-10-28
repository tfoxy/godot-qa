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

const DEFAULT_SCRIPT_CHECK_TIMEOUT_SECONDS: f64 = 8.0;

pub(crate) fn check_tests_result(cli: &Cli, args: &CheckArgs) -> CommandResult {
    let config = load_project_config(
        cli.project.as_deref().map(Path::new),
        cli.godot_bin.as_deref(),
        cli.artifact_dir.as_deref(),
        cli.verbose,
        godot_qa_process::resolve_godot_binary(cli.godot_bin.as_deref()),
    );
    let execution_mode = if args.batch {
        scenario_check::CheckTestExecutionMode::Batch
    } else {
        scenario_check::CheckTestExecutionMode::Isolated
    };
    let timeout_override = match args.timeout_seconds {
        Some(seconds) if seconds.is_finite() && seconds > 0.0 => {
            Some(std::time::Duration::from_secs_f64(seconds))
        }
        Some(seconds) => {
            return CommandResult {
                ok: false,
                command: "check".to_string(),
                message: "project check failed".to_string(),
                data: Map::from_iter([
                    ("project_root".to_string(), json!(config.project_root)),
                    ("mode".to_string(), json!("tests")),
                ]),
                failures: vec![FailureDetail {
                    kind: "test_manifest_invalid".to_string(),
                    command: Some("check".to_string()),
                    message: Some(
                        "check --tests --timeout-seconds must be a positive finite number"
                            .to_string(),
                    ),
                    expected: Some(json!("positive finite seconds")),
                    actual: Some(json!(seconds)),
                    ..FailureDetail::default()
                }],
                exit_code: 1,
            };
        }
        None => None,
    };
    let parsed_profile = match check_tests_profile_args(args) {
        Ok(parsed_profile) => parsed_profile,
        Err(failure) => {
            return CommandResult {
                ok: false,
                command: "check".to_string(),
                message: "project check failed".to_string(),
                data: Map::from_iter([
                    ("project_root".to_string(), json!(config.project_root)),
                    ("mode".to_string(), json!("tests")),
                ]),
                failures: vec![failure],
                exit_code: 1,
            };
        }
    };
    let selection = scenario_check::CheckTestSelectionRequest {
        test_ids: args.test.clone(),
        tags: args.tag.clone(),
        profile: parsed_profile.manifest_profile.clone(),
    };
    append_config_warnings(
        scenario_check::run_check_tests(
            &config,
            &check_tests_command_string(cli, args),
            selection,
            execution_mode,
            scenario_check::CheckTestRunOptions {
                timeout_override,
                profile_enabled: parsed_profile.profile_enabled,
            },
        ),
        &config,
    )
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ParsedCheckTestsProfileArgs {
    pub(crate) profile_enabled: bool,
    pub(crate) manifest_profile: Option<String>,
}

pub(crate) fn check_tests_profile_args(
    args: &CheckArgs,
) -> Result<ParsedCheckTestsProfileArgs, FailureDetail> {
    let profiling_flags = args
        .profile
        .iter()
        .filter(|profile| profile.is_empty())
        .count();
    let manifest_profiles: Vec<String> = args
        .profile
        .iter()
        .filter(|profile| !profile.is_empty())
        .cloned()
        .collect();

    if profiling_flags > 1 {
        return Err(check_tests_profile_arg_failure(
            "duplicate_profile_flag",
            "check --tests accepts the bare --profile profiling flag at most once",
            json!("one bare --profile flag"),
            json!(profiling_flags),
        ));
    }
    if manifest_profiles.len() > 1 {
        return Err(check_tests_profile_arg_failure(
            "duplicate_profile_selector",
            "check --tests accepts at most one manifest profile selector",
            json!("one --profile <manifest-profile> selector"),
            json!(manifest_profiles),
        ));
    }
    let manifest_profile = manifest_profiles.into_iter().next();
    if manifest_profile.is_some() && (!args.test.is_empty() || !args.tag.is_empty()) {
        return Err(check_tests_profile_arg_failure(
            "selection_conflict",
            "check --tests manifest profile selection cannot be combined with --test or --tag",
            json!("one selection mode: --test, --tag, or --profile <manifest-profile>"),
            json!({
                "tests": args.test,
                "tags": args.tag,
                "profile": manifest_profile,
            }),
        ));
    }

    Ok(ParsedCheckTestsProfileArgs {
        profile_enabled: profiling_flags == 1,
        manifest_profile,
    })
}

pub(crate) fn check_tests_profile_arg_failure(
    kind: &str,
    message: &str,
    expected: Value,
    actual: Value,
) -> FailureDetail {
    FailureDetail {
        kind: kind.to_string(),
        command: Some("check".to_string()),
        message: Some(message.to_string()),
        expected: Some(expected),
        actual: Some(actual),
        ..FailureDetail::default()
    }
}

pub(crate) fn check_tests_command_string(cli: &Cli, args: &CheckArgs) -> String {
    let mut parts = vec![
        "godot-qa".to_string(),
        "check".to_string(),
        "--tests".to_string(),
    ];
    if args.batch {
        parts.push("--batch".to_string());
    }
    for test_id in &args.test {
        parts.push("--test".to_string());
        parts.push(shell_escape(test_id));
    }
    for tag in &args.tag {
        parts.push("--tag".to_string());
        parts.push(shell_escape(tag));
    }
    if args.profile.iter().any(|profile| profile.is_empty()) {
        parts.push("--profile".to_string());
    }
    for profile in args.profile.iter().filter(|profile| !profile.is_empty()) {
        parts.push("--profile".to_string());
        parts.push(shell_escape(profile));
    }
    if let Some(timeout_seconds) = args.timeout_seconds {
        parts.push("--timeout-seconds".to_string());
        parts.push(timeout_seconds.to_string());
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

#[cfg(test)]
pub(crate) fn check_script_command_string(cli: &Cli, args: &CheckArgs) -> String {
    let mut parts = vec!["godot-qa".to_string(), "check".to_string()];
    if args.all_scripts {
        parts.push("--all-scripts".to_string());
    } else if let Some(script) = &args.script {
        parts.push("--script".to_string());
        parts.push(shell_escape(script));
    }
    if let Some(timeout_seconds) = args.timeout_seconds {
        parts.push("--timeout-seconds".to_string());
        parts.push(timeout_seconds.to_string());
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

pub(crate) struct CheckArtifacts {
    root_dir: std::path::PathBuf,
    meta_path: std::path::PathBuf,
    result_path: std::path::PathBuf,
    import_logs_path: std::path::PathBuf,
    scenes_dir: std::path::PathBuf,
    scripts_dir: std::path::PathBuf,
}

impl CheckArtifacts {
    fn to_json(&self) -> serde_json::Value {
        json!({
            "run_root": self.root_dir,
            "root": self.root_dir,
            "meta": self.meta_path,
            "result": self.result_path,
        })
    }
}

#[derive(Debug)]
pub(crate) struct CheckSceneArtifacts {
    root_dir: std::path::PathBuf,
    meta_path: std::path::PathBuf,
    request_path: std::path::PathBuf,
    result_path: std::path::PathBuf,
    logs_path: std::path::PathBuf,
}

impl CheckSceneArtifacts {
    fn to_json(&self) -> serde_json::Value {
        json!({
            "run_root": self.root_dir,
            "root": self.root_dir,
            "meta": self.meta_path,
            "request": self.request_path,
            "result": self.result_path,
            "logs": self.logs_path,
        })
    }
}

pub(crate) fn check_result(cli: &Cli) -> CommandResult {
    let check_args = match &cli.command {
        Commands::Check(args) => args,
        _ => unreachable!(),
    };
    let resolved_binary = godot_qa_process::resolve_godot_binary(cli.godot_bin.as_deref());
    let config = load_project_config(
        cli.project.as_deref().map(Path::new),
        cli.godot_bin.as_deref(),
        cli.artifact_dir.as_deref(),
        cli.verbose,
        resolved_binary,
    );
    let command = "check";
    let mode = if check_args.all_scripts {
        "all_scripts"
    } else if check_args.script.is_some() {
        "script"
    } else if check_args.all_scenes {
        "all_scenes"
    } else {
        "scene"
    };
    let mut base_data = serde_json::Map::from_iter([
        ("project_root".to_string(), json!(config.project_root)),
        ("mode".to_string(), json!(mode)),
    ]);
    let (mut failures, version) = ensure_check_prerequisites(command, &config);
    if let Some(version) = version {
        base_data.insert(
            "godot".to_string(),
            json!({"binary": config.godot_binary, "version": version}),
        );
    }
    if !failures.is_empty() {
        return CommandResult {
            ok: false,
            command: command.to_string(),
            message: "project check failed".to_string(),
            data: base_data,
            failures,
            exit_code: 1,
        };
    }
    let script_mode = check_args.all_scripts || check_args.script.is_some();
    let script_timeout =
        match check_timeout_for_mode(command, check_args.timeout_seconds, script_mode) {
            Ok(timeout) => timeout,
            Err(failure) => {
                return CommandResult {
                    ok: false,
                    command: command.to_string(),
                    message: "project check failed".to_string(),
                    data: base_data,
                    failures: vec![failure],
                    exit_code: 1,
                };
            }
        };
    if script_mode {
        return append_config_warnings(
            check_scripts_result(
                &config,
                check_args,
                command,
                mode,
                base_data,
                script_timeout.unwrap_or_else(default_script_check_timeout),
            ),
            &config,
        );
    }

    let scenes = if check_args.all_scenes {
        match discover_project_scenes(&config.project_root) {
            Ok(scenes) => scenes,
            Err(error) => {
                return CommandResult {
                    ok: false,
                    command: command.to_string(),
                    message: "project check failed".to_string(),
                    data: base_data,
                    failures: vec![FailureDetail {
                        kind: "scene_enumeration_failed".to_string(),
                        command: Some(command.to_string()),
                        path: Some(config.project_root.to_string_lossy().into_owned()),
                        message: Some(error.to_string()),
                        expected: Some(json!("enumerable project scene files")),
                        actual: Some(json!("OSError")),
                        ..FailureDetail::default()
                    }],
                    exit_code: 1,
                };
            }
        }
    } else {
        let scene =
            match resolve_check_scene(command, &config.project_root, check_args.scene.as_deref()) {
                Ok(scene) => scene,
                Err(failure) => {
                    let mut data = base_data;
                    data.insert(
                        "scene".to_string(),
                        json!(check_args.scene.clone().unwrap_or_default()),
                    );
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
        if let Some(failure) = validate_requested_scene(command, &config.project_root, &scene) {
            let mut data = base_data;
            data.insert("scene".to_string(), json!(scene));
            return CommandResult {
                ok: false,
                command: command.to_string(),
                message: "project check failed".to_string(),
                data,
                failures: vec![failure],
                exit_code: 1,
            };
        }
        vec![scene]
    };

    let artifacts = prepare_check_artifacts(&config, command);
    let import_summary = json!({"ok": false, "artifacts": {"logs": artifacts.import_logs_path}});
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
                data: serde_json::Map::from_iter([
                    ("project_root".to_string(), json!(config.project_root)),
                    ("mode".to_string(), json!(mode)),
                    ("artifacts".to_string(), artifacts.to_json()),
                    ("import".to_string(), import_summary),
                    ("checked_scene_count".to_string(), json!(0)),
                    ("scenes".to_string(), json!([])),
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
                data: serde_json::Map::from_iter([
                    ("project_root".to_string(), json!(config.project_root)),
                    ("mode".to_string(), json!(mode)),
                    ("artifacts".to_string(), artifacts.to_json()),
                    ("import".to_string(), import_summary),
                    ("checked_scene_count".to_string(), json!(0)),
                    ("scenes".to_string(), json!([])),
                ]),
                failures: vec![failure],
                exit_code: 1,
            };
            let _ = write_result_artifact(&artifacts.result_path, &result);
            return result;
        }
    };

    let mut scene_payloads = Vec::new();
    for scene in &scenes {
        let (scene_data, scene_failures) = run_scene_check(&config, scene, &artifacts, command);
        scene_payloads.push(scene_data);
        failures.extend(scene_failures);
    }

    let mut data = serde_json::Map::from_iter([
        ("project_root".to_string(), json!(config.project_root)),
        ("mode".to_string(), json!(mode)),
        ("artifacts".to_string(), artifacts.to_json()),
        (
            "import".to_string(),
            json!({"ok": true, "artifacts": {"logs": artifacts.import_logs_path}}),
        ),
        (
            "checked_scene_count".to_string(),
            json!(scene_payloads.len()),
        ),
        ("scenes".to_string(), json!(scene_payloads)),
    ]);
    if !check_args.all_scenes {
        if let Some(scene) = scenes.first() {
            data.insert("scene".to_string(), json!(scene));
        }
        let copied_fields = data
            .get("scenes")
            .and_then(|value| value.as_array())
            .and_then(|items| items.first())
            .map(|scene_entry| {
                ["duration_ms", "startup_frames", "runtime_errors"]
                    .iter()
                    .filter_map(|key| {
                        scene_entry
                            .get(*key)
                            .cloned()
                            .map(|value| ((*key).to_string(), value))
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        for (key, value) in copied_fields {
            data.insert(key, value);
        }
    }
    let exit_code = if failures.is_empty() { 0 } else { 1 };
    let result = CommandResult {
        ok: failures.is_empty(),
        command: command.to_string(),
        message: if failures.is_empty() {
            "project check passed".to_string()
        } else {
            "project check failed".to_string()
        },
        data,
        failures,
        exit_code,
    };
    let _ = import_run;
    let _ = write_result_artifact(&artifacts.result_path, &result);
    append_config_warnings(result, &config)
}

pub(crate) fn default_script_check_timeout() -> Duration {
    Duration::from_secs_f64(DEFAULT_SCRIPT_CHECK_TIMEOUT_SECONDS)
}

pub(crate) fn check_timeout_for_mode(
    command: &str,
    timeout_seconds: Option<f64>,
    script_mode: bool,
) -> Result<Option<Duration>, FailureDetail> {
    match (timeout_seconds, script_mode) {
        (Some(seconds), true) if seconds.is_finite() && seconds > 0.0 => {
            Ok(Some(Duration::from_secs_f64(seconds)))
        }
        (Some(seconds), true) => Err(FailureDetail {
            kind: "invalid_params".to_string(),
            command: Some(command.to_string()),
            message: Some(
                "check --timeout-seconds must be a positive finite number".to_string(),
            ),
            expected: Some(json!("positive finite seconds")),
            actual: Some(json!(seconds)),
            ..FailureDetail::default()
        }),
        (Some(_), false) => Err(FailureDetail {
            kind: "invalid_params".to_string(),
            command: Some(command.to_string()),
            message: Some(
                "check --timeout-seconds is only supported with --tests, --script, or --all-scripts"
                    .to_string(),
            ),
            expected: Some(json!("check --tests, check --script, or check --all-scripts")),
            actual: Some(json!("scene check mode")),
            ..FailureDetail::default()
        }),
        (None, true) => Ok(Some(default_script_check_timeout())),
        (None, false) => Ok(None),
    }
}

pub(crate) fn check_scripts_result(
    config: &ProjectConfig,
    check_args: &CheckArgs,
    command: &str,
    mode: &str,
    mut base_data: serde_json::Map<String, serde_json::Value>,
    timeout: Duration,
) -> CommandResult {
    let scripts = if check_args.all_scripts {
        match discover_project_scripts(&config.project_root) {
            Ok(scripts) => scripts,
            Err(error) => {
                return CommandResult {
                    ok: false,
                    command: command.to_string(),
                    message: "project check failed".to_string(),
                    data: base_data,
                    failures: vec![FailureDetail {
                        kind: "script_enumeration_failed".to_string(),
                        command: Some(command.to_string()),
                        path: Some(config.project_root.to_string_lossy().into_owned()),
                        message: Some(error.to_string()),
                        expected: Some(json!("enumerable project GDScript files")),
                        actual: Some(json!("OSError")),
                        ..FailureDetail::default()
                    }],
                    exit_code: 1,
                };
            }
        }
    } else {
        let script = match resolve_check_script(command, check_args.script.as_deref()) {
            Ok(script) => script,
            Err(failure) => {
                base_data.insert(
                    "target".to_string(),
                    json!(check_args.script.clone().unwrap_or_default()),
                );
                return CommandResult {
                    ok: false,
                    command: command.to_string(),
                    message: "project check failed".to_string(),
                    data: base_data,
                    failures: vec![failure],
                    exit_code: 1,
                };
            }
        };
        if let Some(failure) = validate_requested_script(command, &config.project_root, &script) {
            base_data.insert("target".to_string(), json!(script));
            return CommandResult {
                ok: false,
                command: command.to_string(),
                message: "project check failed".to_string(),
                data: base_data,
                failures: vec![failure],
                exit_code: 1,
            };
        }
        vec![script]
    };

    let artifacts = prepare_check_artifacts(config, command);
    let mut script_payloads = Vec::new();
    let mut failures = Vec::new();
    for script in &scripts {
        let (script_data, script_failures) =
            run_script_compile_check(config, script, &artifacts, command, timeout);
        script_payloads.push(script_data);
        failures.extend(script_failures);
    }

    let mut data = serde_json::Map::from_iter([
        ("project_root".to_string(), json!(config.project_root)),
        ("mode".to_string(), json!(mode)),
        ("artifacts".to_string(), artifacts.to_json()),
        (
            "checked_script_count".to_string(),
            json!(script_payloads.len()),
        ),
        ("scripts".to_string(), json!(script_payloads)),
        ("timeout_seconds".to_string(), json!(timeout.as_secs_f64())),
    ]);
    if mode == "script" {
        if let Some(script) = scripts.first() {
            data.insert("target".to_string(), json!(script));
        }
        let copied_fields = data
            .get("scripts")
            .and_then(|value| value.as_array())
            .and_then(|items| items.first())
            .map(|script_entry| {
                ["duration_ms", "runtime_errors", "status"]
                    .iter()
                    .filter_map(|key| {
                        script_entry
                            .get(*key)
                            .cloned()
                            .map(|value| ((*key).to_string(), value))
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        for (key, value) in copied_fields {
            data.insert(key, value);
        }
    }
    let exit_code = if failures.is_empty() { 0 } else { 1 };
    let result = CommandResult {
        ok: failures.is_empty(),
        command: command.to_string(),
        message: if failures.is_empty() {
            "project check passed".to_string()
        } else {
            "project check failed".to_string()
        },
        data,
        failures,
        exit_code,
    };
    let _ = write_result_artifact(&artifacts.result_path, &result);
    result
}

pub(crate) fn prepare_check_artifacts(config: &ProjectConfig, command: &str) -> CheckArtifacts {
    let root_dir = config.artifacts_dir.join("check").join(unique_run_id());
    let _ = std::fs::create_dir_all(root_dir.join("import"));
    let _ = std::fs::create_dir_all(root_dir.join("scenes"));
    let _ = std::fs::create_dir_all(root_dir.join("scripts"));
    let artifacts = CheckArtifacts {
        root_dir: root_dir.clone(),
        meta_path: root_dir.join("meta.json"),
        result_path: root_dir.join("result.json"),
        import_logs_path: root_dir.join("import").join("logs.txt"),
        scenes_dir: root_dir.join("scenes"),
        scripts_dir: root_dir.join("scripts"),
    };
    let _ = std::fs::write(
        &artifacts.meta_path,
        serde_json::to_string_pretty(
            &json!({"tool": "godot-qa", "project_root": config.project_root, "command": command}),
        )
        .unwrap_or_else(|_| "{}".to_string()),
    );
    artifacts
}

pub(crate) fn ensure_check_prerequisites(
    command: &str,
    config: &ProjectConfig,
) -> (Vec<FailureDetail>, Option<String>) {
    let mut failures = Vec::new();
    let mut version = None;
    if let Some(failure) = config.project_config_failure_for_command(command) {
        failures.push(failure);
    }
    if config.godot_binary.is_none() {
        failures.push(missing_godot_failure(command));
    } else if let Some(binary) = config.godot_binary.as_deref()
        && config.project_config_failure.is_none()
    {
        match godot_qa_process::probe_godot_version(binary, &config.godot_launch_prefix) {
            Ok(found) => version = Some(found),
            Err(failure) => failures.push(failure),
        }
    }
    if !config.project_root.exists() {
        failures.push(missing_project_root_failure(command, &config.project_root));
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

pub(crate) fn resolve_check_scene(
    command: &str,
    project_root: &Path,
    explicit_scene: Option<&str>,
) -> Result<String, FailureDetail> {
    if let Some(scene) = explicit_scene
        .map(str::trim)
        .filter(|scene| !scene.is_empty())
    {
        return Ok(scene.to_string());
    }
    let project_path = project_root.join("project.godot");
    let contents = std::fs::read_to_string(&project_path).map_err(|error| FailureDetail {
        kind: "missing_main_scene".to_string(),
        command: Some(command.to_string()),
        path: Some(project_path.to_string_lossy().into_owned()),
        message: Some(error.to_string()),
        expected: Some(json!(
            "run/main_scene in project.godot or explicit --scene <scene>"
        )),
        actual: Some(json!("project.godot unreadable")),
        ..FailureDetail::default()
    })?;
    parse_project_main_scene(&contents).ok_or_else(|| FailureDetail {
        kind: "missing_main_scene".to_string(),
        command: Some(command.to_string()),
        path: Some(project_path.to_string_lossy().into_owned()),
        message: Some("project.godot does not define application run/main_scene".to_string()),
        expected: Some(json!(
            "run/main_scene in project.godot or explicit --scene <scene>"
        )),
        actual: Some(json!("missing")),
        ..FailureDetail::default()
    })
}

pub(crate) fn parse_project_main_scene(contents: &str) -> Option<String> {
    let mut in_application = false;
    for raw_line in contents.lines() {
        let line = raw_line.trim();
        if line.is_empty() || line.starts_with(';') {
            continue;
        }
        if line.starts_with('[') && line.ends_with(']') {
            in_application = line == "[application]";
            continue;
        }
        if !in_application {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        if key.trim() != "run/main_scene" {
            continue;
        }
        let parsed = parse_project_setting_string(value.trim());
        if parsed.is_empty() {
            return None;
        }
        return Some(parsed);
    }
    None
}

pub(crate) fn parse_project_setting_string(value: &str) -> String {
    if value.len() >= 2 && value.starts_with('"') && value.ends_with('"') {
        return value[1..value.len() - 1]
            .replace("\\\"", "\"")
            .replace("\\\\", "\\");
    }
    value.to_string()
}

pub(crate) fn validate_requested_scene(
    command: &str,
    project_root: &Path,
    scene: &str,
) -> Option<FailureDetail> {
    if !scene.starts_with("res://") {
        return Some(scene_missing_failure(
            command,
            scene,
            &project_root.join(scene),
            None,
            "missing",
        ));
    }
    let scene_path = project_root.join(scene.trim_start_matches("res://"));
    let resolved = normalize_existing_or_join(&scene_path);
    let project_resolved = normalize_existing_or_join(project_root);
    if !resolved.starts_with(&project_resolved) {
        return Some(scene_missing_failure(
            command,
            scene,
            &resolved,
            Some("scene path must stay within the selected project root"),
            "outside_project",
        ));
    }
    if !resolved.exists() {
        return Some(scene_missing_failure(
            command, scene, &resolved, None, "missing",
        ));
    }
    if !resolved.is_file() {
        return Some(scene_missing_failure(
            command,
            scene,
            &resolved,
            Some("scene target must resolve to a file"),
            "not_file",
        ));
    }
    None
}

pub(crate) fn scene_missing_failure(
    command: &str,
    scene: &str,
    path: &Path,
    message: Option<&str>,
    actual: &str,
) -> FailureDetail {
    FailureDetail {
        kind: "scene_missing".to_string(),
        command: Some(command.to_string()),
        path: Some(path.to_string_lossy().into_owned()),
        target: Some(scene.to_string()),
        message: message.map(str::to_string),
        expected: Some(json!("existing scene file under the project root")),
        actual: Some(json!(actual)),
        ..FailureDetail::default()
    }
}

pub(crate) fn resolve_check_script(
    command: &str,
    explicit_script: Option<&str>,
) -> Result<String, FailureDetail> {
    explicit_script
        .map(str::trim)
        .filter(|script| !script.is_empty())
        .map(str::to_string)
        .ok_or_else(|| FailureDetail {
            kind: "script_missing".to_string(),
            command: Some(command.to_string()),
            target: Some(String::new()),
            message: Some("check --script requires a non-empty res:// script path".to_string()),
            expected: Some(json!("res:// path to a .gd script")),
            actual: Some(json!("missing")),
            ..FailureDetail::default()
        })
}

pub(crate) fn validate_requested_script(
    command: &str,
    project_root: &Path,
    script: &str,
) -> Option<FailureDetail> {
    if !script.starts_with("res://") {
        return Some(script_missing_failure(
            command,
            script,
            &project_root.join(script),
            None,
            "missing",
        ));
    }
    if !script.ends_with(".gd") {
        return Some(script_missing_failure(
            command,
            script,
            &project_root.join(script.trim_start_matches("res://")),
            Some("script target must use the .gd extension"),
            "not_gd_script",
        ));
    }
    let resource_path = script.trim_start_matches("res://");
    if Path::new(resource_path)
        .components()
        .any(|component| matches!(component, std::path::Component::ParentDir))
    {
        return Some(script_missing_failure(
            command,
            script,
            &project_root.join(resource_path),
            Some("script path must stay within the selected project root"),
            "outside_project",
        ));
    }
    let script_path = project_root.join(resource_path);
    let resolved = normalize_existing_or_join(&script_path);
    let project_resolved = normalize_existing_or_join(project_root);
    if !resolved.starts_with(&project_resolved) {
        return Some(script_missing_failure(
            command,
            script,
            &resolved,
            Some("script path must stay within the selected project root"),
            "outside_project",
        ));
    }
    if !resolved.exists() {
        return Some(script_missing_failure(
            command, script, &resolved, None, "missing",
        ));
    }
    if !resolved.is_file() {
        return Some(script_missing_failure(
            command,
            script,
            &resolved,
            Some("script target must resolve to a file"),
            "not_file",
        ));
    }
    None
}

pub(crate) fn script_missing_failure(
    command: &str,
    script: &str,
    path: &Path,
    message: Option<&str>,
    actual: &str,
) -> FailureDetail {
    FailureDetail {
        kind: "script_missing".to_string(),
        command: Some(command.to_string()),
        path: Some(path.to_string_lossy().into_owned()),
        target: Some(script.to_string()),
        message: message.map(str::to_string),
        expected: Some(json!("existing .gd script file under the project root")),
        actual: Some(json!(actual)),
        ..FailureDetail::default()
    }
}

pub(crate) fn normalize_existing_or_join(path: &Path) -> std::path::PathBuf {
    path.canonicalize()
        .unwrap_or_else(|_| path.components().collect())
}

pub(crate) fn discover_project_scenes(project_root: &Path) -> io::Result<Vec<String>> {
    let mut scenes = Vec::new();
    visit_scene_files(project_root, project_root, &mut scenes)?;
    scenes.sort();
    Ok(scenes)
}

pub(crate) fn discover_project_scripts(project_root: &Path) -> io::Result<Vec<String>> {
    let mut scripts = Vec::new();
    visit_script_files(project_root, project_root, &mut scripts)?;
    scripts.sort();
    Ok(scripts)
}

pub(crate) fn visit_scene_files(
    root: &Path,
    current: &Path,
    scenes: &mut Vec<String>,
) -> io::Result<()> {
    for entry in std::fs::read_dir(current)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            visit_scene_files(root, &path, scenes)?;
            continue;
        }
        let Some(extension) = path.extension().and_then(|value| value.to_str()) else {
            continue;
        };
        if !SCENE_EXTENSIONS.contains(&extension) {
            continue;
        }
        let relative = path
            .strip_prefix(root)
            .unwrap_or(&path)
            .to_string_lossy()
            .replace('\\', "/");
        let scene = format!("res://{relative}");
        if SCENE_EXCLUDED_PREFIXES
            .iter()
            .any(|prefix| scene.starts_with(prefix))
            || SCENE_EXCLUDED_PATHS.contains(&scene.as_str())
        {
            continue;
        }
        scenes.push(scene);
    }
    Ok(())
}

pub(crate) fn visit_script_files(
    root: &Path,
    current: &Path,
    scripts: &mut Vec<String>,
) -> io::Result<()> {
    for entry in std::fs::read_dir(current)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            let relative = path
                .strip_prefix(root)
                .unwrap_or(&path)
                .to_string_lossy()
                .replace('\\', "/");
            let resource_prefix = format!("res://{relative}/");
            if SCENE_EXCLUDED_PREFIXES
                .iter()
                .any(|prefix| resource_prefix.starts_with(prefix))
            {
                continue;
            }
            visit_script_files(root, &path, scripts)?;
            continue;
        }
        let Some(extension) = path.extension().and_then(|value| value.to_str()) else {
            continue;
        };
        if !SCRIPT_EXTENSIONS.contains(&extension) {
            continue;
        }
        let relative = path
            .strip_prefix(root)
            .unwrap_or(&path)
            .to_string_lossy()
            .replace('\\', "/");
        let script = format!("res://{relative}");
        if SCENE_EXCLUDED_PREFIXES
            .iter()
            .any(|prefix| script.starts_with(prefix))
        {
            continue;
        }
        scripts.push(script);
    }
    Ok(())
}

pub(crate) fn run_scene_check(
    config: &ProjectConfig,
    scene: &str,
    check_artifacts: &CheckArtifacts,
    command: &str,
) -> (serde_json::Value, Vec<FailureDetail>) {
    let scene_artifacts = prepare_scene_artifacts(check_artifacts, scene);
    let request = json!({
        "check": {"scene": scene, "startup_frames": STARTUP_FRAMES},
        "artifacts": scene_artifacts.to_json(),
        "run": {"id": check_artifacts.root_dir.file_name().and_then(|value| value.to_str()).unwrap_or("0"), "command": command}
    });
    let _ = std::fs::write(
        &scene_artifacts.request_path,
        serde_json::to_string_pretty(&request).unwrap_or_else(|_| "{}".to_string()),
    );
    let process_result = match godot_qa_process::launch_godot_check(
        config.godot_binary.as_deref().unwrap_or_default(),
        &config.godot_launch_prefix,
        &config.godot_args,
        &config.godot_project_args,
        &config.project_root,
        &scene_artifacts.request_path,
        &scene_artifacts.logs_path,
    ) {
        Ok(run) => run,
        Err(failure) => {
            return (
                json!({
                    "scene": scene,
                    "ok": false,
                    "duration_ms": serde_json::Value::Null,
                    "startup_frames": STARTUP_FRAMES,
                    "runtime_errors": [],
                    "artifacts": scene_artifacts.to_json(),
                    "failures": [normalized_failure(scene, &scene_artifacts, &failure)],
                }),
                vec![failure],
            );
        }
    };
    let runtime_result = match load_runtime_result(scene, &scene_artifacts) {
        Ok(result) => result,
        Err(failure) => {
            return (
                json!({
                    "scene": scene,
                    "ok": false,
                    "duration_ms": serde_json::Value::Null,
                    "startup_frames": STARTUP_FRAMES,
                    "runtime_errors": [],
                    "artifacts": scene_artifacts.to_json(),
                    "failures": [failure.clone()],
                }),
                vec![failure],
            );
        }
    };
    validate_runtime_result(scene, runtime_result, &scene_artifacts, &process_result)
}

pub(crate) fn run_script_compile_check(
    config: &ProjectConfig,
    script: &str,
    check_artifacts: &CheckArtifacts,
    command: &str,
    timeout: Duration,
) -> (serde_json::Value, Vec<FailureDetail>) {
    let script_artifacts = prepare_script_artifacts(check_artifacts, script);
    let request = json!({
        "check": {"id": script, "kind": "script_compile", "target": script},
        "artifacts": script_artifacts.to_json(),
        "run": {"id": check_artifacts.root_dir.file_name().and_then(|value| value.to_str()).unwrap_or("0"), "command": command}
    });
    let _ = std::fs::write(
        &script_artifacts.request_path,
        serde_json::to_string_pretty(&request).unwrap_or_else(|_| "{}".to_string()),
    );
    let process_result = match godot_qa_process::launch_godot_check_with_timeout(
        config.godot_binary.as_deref().unwrap_or_default(),
        &config.godot_launch_prefix,
        &config.godot_args,
        &config.godot_project_args,
        &config.project_root,
        &script_artifacts.request_path,
        &script_artifacts.logs_path,
        Some(timeout),
    ) {
        Ok(run) => run,
        Err(failure) => {
            return (
                json!({
                    "target": script,
                    "ok": false,
                    "status": "failed",
                    "duration_ms": serde_json::Value::Null,
                    "runtime_errors": [],
                    "artifacts": script_artifacts.to_json(),
                    "failures": [normalized_failure(script, &script_artifacts, &failure)],
                }),
                vec![failure],
            );
        }
    };
    if process_result.timed_out {
        let failure = check_script_timeout_failure(script, &script_artifacts, &process_result);
        return (
            json!({
                "target": script,
                "ok": false,
                "status": "timeout",
                "duration_ms": serde_json::Value::Null,
                "runtime_errors": [],
                "artifacts": script_artifacts.to_json(),
                "failures": [failure.clone()],
            }),
            vec![failure],
        );
    }
    let runtime_result = match load_runtime_result(script, &script_artifacts) {
        Ok(result) => result,
        Err(failure) => {
            return (
                json!({
                    "target": script,
                    "ok": false,
                    "status": "failed",
                    "duration_ms": serde_json::Value::Null,
                    "runtime_errors": [],
                    "artifacts": script_artifacts.to_json(),
                    "failures": [failure.clone()],
                }),
                vec![failure],
            );
        }
    };
    validate_script_compile_result(script, runtime_result, &script_artifacts, &process_result)
}

pub(crate) fn prepare_scene_artifacts(
    check_artifacts: &CheckArtifacts,
    scene: &str,
) -> CheckSceneArtifacts {
    let root_dir = check_artifacts.scenes_dir.join(slug_value(scene));
    let _ = std::fs::create_dir_all(&root_dir);
    let artifacts = CheckSceneArtifacts {
        root_dir: root_dir.clone(),
        meta_path: root_dir.join("meta.json"),
        request_path: root_dir.join("request.json"),
        result_path: root_dir.join("result.json"),
        logs_path: root_dir.join("logs.txt"),
    };
    let _ = std::fs::write(
        &artifacts.meta_path,
        serde_json::to_string_pretty(&json!({"scene": scene, "startup_frames": STARTUP_FRAMES}))
            .unwrap_or_else(|_| "{}".to_string()),
    );
    artifacts
}

pub(crate) fn prepare_script_artifacts(
    check_artifacts: &CheckArtifacts,
    script: &str,
) -> CheckSceneArtifacts {
    let root_dir = check_artifacts.scripts_dir.join(slug_value(script));
    let _ = std::fs::create_dir_all(&root_dir);
    let artifacts = CheckSceneArtifacts {
        root_dir: root_dir.clone(),
        meta_path: root_dir.join("meta.json"),
        request_path: root_dir.join("request.json"),
        result_path: root_dir.join("result.json"),
        logs_path: root_dir.join("logs.txt"),
    };
    let _ = std::fs::write(
        &artifacts.meta_path,
        serde_json::to_string_pretty(&json!({"script": script, "kind": "script_compile"}))
            .unwrap_or_else(|_| "{}".to_string()),
    );
    artifacts
}

pub(crate) fn slug_value(value: &str) -> String {
    let cleaned = value
        .trim_start_matches("res://")
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

pub(crate) fn load_runtime_result(
    scene: &str,
    artifacts: &CheckSceneArtifacts,
) -> Result<serde_json::Value, FailureDetail> {
    let contents = std::fs::read_to_string(&artifacts.result_path).map_err(|_| FailureDetail {
        kind: "check_result_invalid".to_string(),
        command: Some("check".to_string()),
        path: Some(artifacts.result_path.to_string_lossy().into_owned()),
        target: Some(scene.to_string()),
        message: Some("check result.json was not written".to_string()),
        expected: Some(json!("JSON result written by qa_runner")),
        actual: Some(json!("missing")),
        artifacts: value_as_object_map(artifacts.to_json()),
        ..FailureDetail::default()
    })?;
    let value: serde_json::Value =
        serde_json::from_str(&contents).map_err(|error| FailureDetail {
            kind: "check_result_invalid".to_string(),
            command: Some("check".to_string()),
            path: Some(artifacts.result_path.to_string_lossy().into_owned()),
            target: Some(scene.to_string()),
            message: Some("check result.json was not valid JSON".to_string()),
            expected: Some(json!("valid JSON object")),
            actual: Some(json!(error.to_string())),
            artifacts: value_as_object_map(artifacts.to_json()),
            ..FailureDetail::default()
        })?;
    if !value.is_object() {
        return Err(FailureDetail {
            kind: "check_result_invalid".to_string(),
            command: Some("check".to_string()),
            path: Some(artifacts.result_path.to_string_lossy().into_owned()),
            target: Some(scene.to_string()),
            message: Some("check result.json root must be an object".to_string()),
            expected: Some(json!("JSON object")),
            actual: Some(json!(type_name(&value))),
            artifacts: value_as_object_map(artifacts.to_json()),
            ..FailureDetail::default()
        });
    }
    Ok(value)
}

pub(crate) fn validate_runtime_result(
    scene: &str,
    runtime_result: serde_json::Value,
    artifacts: &CheckSceneArtifacts,
    process_result: &godot_qa_process::GodotRunResult,
) -> (serde_json::Value, Vec<FailureDetail>) {
    let object = runtime_result.as_object().unwrap();
    let mut failures = Vec::new();
    let scene_value = object
        .get("scene")
        .and_then(|value| value.as_str())
        .unwrap_or_default();
    if scene_value != scene {
        failures.push(FailureDetail {
            kind: "check_result_invalid".to_string(),
            command: Some("check".to_string()),
            path: Some(artifacts.result_path.to_string_lossy().into_owned()),
            target: Some(scene.to_string()),
            message: Some("check result field 'scene' must match the requested scene".to_string()),
            expected: Some(json!(scene)),
            actual: Some(json!(scene_value)),
            artifacts: value_as_object_map(artifacts.to_json()),
            ..FailureDetail::default()
        });
    }
    let ok = object
        .get("ok")
        .and_then(|value| value.as_bool())
        .unwrap_or(false);
    let duration_ms = object
        .get("duration_ms")
        .cloned()
        .unwrap_or_else(|| serde_json::Value::Null);
    let startup_frames = object
        .get("startup_frames")
        .cloned()
        .unwrap_or_else(|| serde_json::Value::Null);
    let runtime_errors = object
        .get("runtime_errors")
        .cloned()
        .unwrap_or_else(|| json!([]));
    let failure_values = object.get("failures").cloned().unwrap_or_else(|| json!([]));
    if !runtime_errors.is_array()
        || runtime_errors
            .as_array()
            .is_some_and(|entries| entries.iter().any(|entry| !entry.is_object()))
    {
        failures.push(FailureDetail {
            kind: "check_result_invalid".to_string(),
            command: Some("check".to_string()),
            path: Some(artifacts.result_path.to_string_lossy().into_owned()),
            target: Some(scene.to_string()),
            message: Some(
                "check result field 'runtime_errors' must be an array of objects".to_string(),
            ),
            expected: Some(json!("JSON array of objects")),
            actual: Some(json!(type_name(&runtime_errors))),
            artifacts: value_as_object_map(artifacts.to_json()),
            ..FailureDetail::default()
        });
    }
    if !failure_values.is_array()
        || failure_values
            .as_array()
            .is_some_and(|entries| entries.iter().any(|entry| !entry.is_object()))
    {
        failures.push(FailureDetail {
            kind: "check_result_invalid".to_string(),
            command: Some("check".to_string()),
            path: Some(artifacts.result_path.to_string_lossy().into_owned()),
            target: Some(scene.to_string()),
            message: Some("check result field 'failures' must be an array of objects".to_string()),
            expected: Some(json!("JSON array of objects")),
            actual: Some(json!(type_name(&failure_values))),
            artifacts: value_as_object_map(artifacts.to_json()),
            ..FailureDetail::default()
        });
    }
    let mut normalized_failures = failure_values
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|value| value.as_object())
        .map(|failure| normalized_failure_from_value(scene, artifacts, failure))
        .collect::<Vec<_>>();
    if runtime_errors
        .as_array()
        .is_some_and(|entries| !entries.is_empty())
        && !normalized_failures
            .iter()
            .any(|failure| failure.kind == "runtime_errors_present")
    {
        normalized_failures.push(FailureDetail {
            kind: "runtime_errors_present".to_string(),
            command: Some("check".to_string()),
            target: Some(scene.to_string()),
            message: Some("runtime errors were recorded during startup".to_string()),
            expected: Some(json!([])),
            actual: Some(runtime_errors.clone()),
            artifacts: value_as_object_map(artifacts.to_json()),
            ..FailureDetail::default()
        });
    }
    if let Some(diagnostic) = script_diagnostic_excerpt(process_result, &artifacts.logs_path) {
        normalized_failures.push(FailureDetail {
            kind: "script_diagnostics_present".to_string(),
            command: Some("check".to_string()),
            path: Some(artifacts.logs_path.to_string_lossy().into_owned()),
            target: Some(scene.to_string()),
            message: Some(
                "script parse or compilation diagnostics were recorded during scene check"
                    .to_string(),
            ),
            expected: Some(json!("clean script load during check --scene")),
            actual: Some(json!(diagnostic)),
            artifacts: value_as_object_map(artifacts.to_json()),
            ..FailureDetail::default()
        });
    }
    if process_result.returncode != 0 && normalized_failures.is_empty() {
        normalized_failures.push(FailureDetail {
            kind: "godot_run_failed".to_string(),
            command: Some(godot_qa_process::format_command(&process_result.command)),
            path: Some(artifacts.logs_path.to_string_lossy().into_owned()),
            target: Some(scene.to_string()),
            expected: Some(json!("Godot check exits with status 0")),
            actual: Some(json!(process_result.returncode)),
            details: [("termination".to_string(), json!(process_result.termination))]
                .into_iter()
                .collect(),
            artifacts: value_as_object_map(artifacts.to_json()),
            ..FailureDetail::default()
        });
    }
    failures.extend(normalized_failures.iter().cloned());
    (
        json!({
            "scene": scene,
            "ok": ok && failures.is_empty(),
            "duration_ms": duration_ms,
            "startup_frames": startup_frames,
            "runtime_errors": runtime_errors,
            "artifacts": artifacts.to_json(),
            "failures": normalized_failures,
        }),
        failures,
    )
}

pub(crate) fn validate_script_compile_result(
    script: &str,
    runtime_result: serde_json::Value,
    artifacts: &CheckSceneArtifacts,
    process_result: &godot_qa_process::GodotRunResult,
) -> (serde_json::Value, Vec<FailureDetail>) {
    let object = runtime_result.as_object().unwrap();
    let mut failures = Vec::new();
    let target_value = object
        .get("target")
        .and_then(|value| value.as_str())
        .unwrap_or_default();
    if target_value != script {
        failures.push(FailureDetail {
            kind: "check_result_invalid".to_string(),
            command: Some("check".to_string()),
            path: Some(artifacts.result_path.to_string_lossy().into_owned()),
            target: Some(script.to_string()),
            message: Some(
                "check result field 'target' must match the requested script".to_string(),
            ),
            expected: Some(json!(script)),
            actual: Some(json!(target_value)),
            artifacts: value_as_object_map(artifacts.to_json()),
            ..FailureDetail::default()
        });
    }
    let kind_value = object
        .get("kind")
        .and_then(|value| value.as_str())
        .unwrap_or_default();
    if kind_value != "script_compile" {
        failures.push(FailureDetail {
            kind: "check_result_invalid".to_string(),
            command: Some("check".to_string()),
            path: Some(artifacts.result_path.to_string_lossy().into_owned()),
            target: Some(script.to_string()),
            message: Some("check result field 'kind' must be script_compile".to_string()),
            expected: Some(json!("script_compile")),
            actual: Some(json!(kind_value)),
            artifacts: value_as_object_map(artifacts.to_json()),
            ..FailureDetail::default()
        });
    }
    let ok = object
        .get("ok")
        .and_then(|value| value.as_bool())
        .unwrap_or(false);
    let duration_ms = object
        .get("duration_ms")
        .cloned()
        .unwrap_or_else(|| serde_json::Value::Null);
    let runtime_errors = object
        .get("runtime_errors")
        .cloned()
        .unwrap_or_else(|| json!([]));
    let failure_values = object.get("failures").cloned().unwrap_or_else(|| json!([]));
    if !runtime_errors.is_array()
        || runtime_errors
            .as_array()
            .is_some_and(|entries| entries.iter().any(|entry| !entry.is_object()))
    {
        failures.push(FailureDetail {
            kind: "check_result_invalid".to_string(),
            command: Some("check".to_string()),
            path: Some(artifacts.result_path.to_string_lossy().into_owned()),
            target: Some(script.to_string()),
            message: Some(
                "check result field 'runtime_errors' must be an array of objects".to_string(),
            ),
            expected: Some(json!("JSON array of objects")),
            actual: Some(json!(type_name(&runtime_errors))),
            artifacts: value_as_object_map(artifacts.to_json()),
            ..FailureDetail::default()
        });
    }
    if !failure_values.is_array()
        || failure_values
            .as_array()
            .is_some_and(|entries| entries.iter().any(|entry| !entry.is_object()))
    {
        failures.push(FailureDetail {
            kind: "check_result_invalid".to_string(),
            command: Some("check".to_string()),
            path: Some(artifacts.result_path.to_string_lossy().into_owned()),
            target: Some(script.to_string()),
            message: Some("check result field 'failures' must be an array of objects".to_string()),
            expected: Some(json!("JSON array of objects")),
            actual: Some(json!(type_name(&failure_values))),
            artifacts: value_as_object_map(artifacts.to_json()),
            ..FailureDetail::default()
        });
    }
    let mut normalized_failures = failure_values
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|value| value.as_object())
        .map(|failure| normalized_failure_from_value(script, artifacts, failure))
        .collect::<Vec<_>>();
    if runtime_errors
        .as_array()
        .is_some_and(|entries| !entries.is_empty())
        && !normalized_failures
            .iter()
            .any(|failure| failure.kind == "runtime_errors_present")
    {
        normalized_failures.push(FailureDetail {
            kind: "runtime_errors_present".to_string(),
            command: Some("check".to_string()),
            target: Some(script.to_string()),
            message: Some("runtime errors were recorded during script validation".to_string()),
            expected: Some(json!([])),
            actual: Some(runtime_errors.clone()),
            artifacts: value_as_object_map(artifacts.to_json()),
            ..FailureDetail::default()
        });
    }
    if let Some(diagnostic) = script_diagnostic_excerpt(process_result, &artifacts.logs_path) {
        normalized_failures.push(FailureDetail {
            kind: "script_diagnostics_present".to_string(),
            command: Some("check".to_string()),
            path: Some(artifacts.logs_path.to_string_lossy().into_owned()),
            target: Some(script.to_string()),
            message: Some(
                "script parse or compilation diagnostics were recorded during script check"
                    .to_string(),
            ),
            expected: Some(json!("clean script load during check --script")),
            actual: Some(json!(diagnostic)),
            artifacts: value_as_object_map(artifacts.to_json()),
            ..FailureDetail::default()
        });
    }
    if process_result.returncode != 0 && normalized_failures.is_empty() {
        normalized_failures.push(FailureDetail {
            kind: "godot_run_failed".to_string(),
            command: Some(godot_qa_process::format_command(&process_result.command)),
            path: Some(artifacts.logs_path.to_string_lossy().into_owned()),
            target: Some(script.to_string()),
            expected: Some(json!("Godot script check exits with status 0")),
            actual: Some(json!(process_result.returncode)),
            details: [("termination".to_string(), json!(process_result.termination))]
                .into_iter()
                .collect(),
            artifacts: value_as_object_map(artifacts.to_json()),
            ..FailureDetail::default()
        });
    }
    failures.extend(normalized_failures.iter().cloned());
    let status = if ok && failures.is_empty() {
        "passed"
    } else {
        "failed"
    };
    (
        json!({
            "target": script,
            "ok": ok && failures.is_empty(),
            "status": status,
            "duration_ms": duration_ms,
            "runtime_errors": runtime_errors,
            "artifacts": artifacts.to_json(),
            "failures": normalized_failures,
        }),
        failures,
    )
}

pub(crate) fn check_script_timeout_failure(
    script: &str,
    artifacts: &CheckSceneArtifacts,
    process_result: &godot_qa_process::GodotRunResult,
) -> FailureDetail {
    FailureDetail {
        kind: "check_script_timeout".to_string(),
        command: Some(godot_qa_process::format_command(&process_result.command)),
        path: Some(artifacts.logs_path.to_string_lossy().into_owned()),
        target: Some(script.to_string()),
        message: Some("Godot script validation exceeded the configured timeout".to_string()),
        expected: Some(json!("loadable/compilable GDScript before timeout")),
        actual: Some(json!({
            "returncode": process_result.returncode,
            "timeout_ms": process_result.timeout_ms,
            "cleanup": process_result.cleanup.as_ref().map(|cleanup| json!({
                "status": cleanup.status.clone(),
                "failure_kind": cleanup.failure_kind.clone(),
                "pid": cleanup.pid,
                "process_group": cleanup.process_group,
                "owned": cleanup.owned,
                "message": cleanup.message.clone(),
            })),
        })),
        artifacts: value_as_object_map(artifacts.to_json()),
        ..FailureDetail::default()
    }
}

pub(crate) fn script_diagnostic_excerpt(
    process_result: &godot_qa_process::GodotRunResult,
    logs_path: &Path,
) -> Option<String> {
    let logs = std::fs::read_to_string(logs_path).unwrap_or_default();
    let combined = format!(
        "{}\n{}\n{}",
        process_result.stdout, process_result.stderr, logs
    );
    let lines: Vec<&str> = combined.lines().collect();
    let mut excerpt = Vec::new();
    for (index, line) in lines.iter().enumerate() {
        if !is_script_diagnostic_line(line) {
            continue;
        }
        let start = index.saturating_sub(2);
        let end = (index + 3).min(lines.len());
        for context_line in &lines[start..end] {
            let trimmed = context_line.trim();
            if !trimmed.is_empty() && !excerpt.iter().any(|entry| entry == trimmed) {
                excerpt.push(trimmed.to_string());
            }
            if excerpt.len() >= 12 {
                break;
            }
        }
        if excerpt.len() >= 12 {
            break;
        }
    }
    if excerpt.is_empty() {
        None
    } else {
        Some(excerpt.join("\n"))
    }
}

pub(crate) fn is_script_diagnostic_line(line: &str) -> bool {
    let lower = line.to_ascii_lowercase();
    lower.contains("failed to load script")
        || lower.contains("parse error")
        || lower.contains("parser error")
        || lower.contains("parse error:")
        || lower.contains("compilation failed")
}

pub(crate) fn normalized_failure(
    scene: &str,
    artifacts: &CheckSceneArtifacts,
    failure: &FailureDetail,
) -> serde_json::Value {
    let mut value = serde_json::to_value(failure).unwrap_or_else(|_| json!({}));
    if value.get("target").is_none() {
        value["target"] = json!(scene);
    }
    value["artifacts"] = artifacts.to_json();
    value
}

pub(crate) fn normalized_failure_from_value(
    scene: &str,
    artifacts: &CheckSceneArtifacts,
    failure: &serde_json::Map<String, serde_json::Value>,
) -> FailureDetail {
    let mut merged_artifacts = value_as_object_map(artifacts.to_json()).unwrap_or_default();
    if let Some(extra) = failure.get("artifacts").and_then(|value| value.as_object()) {
        for (key, value) in extra {
            merged_artifacts.insert(key.clone(), value.clone());
        }
    }
    FailureDetail {
        kind: failure
            .get("type")
            .and_then(|value| value.as_str())
            .unwrap_or("runtime_error")
            .to_string(),
        command: Some("check".to_string()),
        path: failure
            .get("path")
            .and_then(|value| value.as_str())
            .map(str::to_string),
        target: failure
            .get("target")
            .and_then(|value| value.as_str())
            .map(str::to_string)
            .or_else(|| Some(scene.to_string())),
        message: failure
            .get("message")
            .and_then(|value| value.as_str())
            .map(str::to_string),
        expected: failure.get("expected").cloned(),
        actual: failure.get("actual").cloned(),
        severity: failure
            .get("severity")
            .and_then(|value| value.as_str())
            .map(str::to_string),
        artifacts: Some(merged_artifacts),
        ..FailureDetail::default()
    }
}

pub(crate) fn value_as_object_map(
    value: serde_json::Value,
) -> Option<serde_json::Map<String, serde_json::Value>> {
    value.as_object().cloned()
}

pub(crate) fn type_name(value: &serde_json::Value) -> &'static str {
    match value {
        serde_json::Value::Null => "null",
        serde_json::Value::Bool(_) => "bool",
        serde_json::Value::Number(_) => "number",
        serde_json::Value::String(_) => "str",
        serde_json::Value::Array(_) => "array",
        serde_json::Value::Object(_) => "object",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_project(name: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("godot-qa-check-{name}-{unique}"));
        fs::create_dir_all(&root).unwrap();
        root
    }

    #[test]
    fn check_uses_project_main_scene_when_scene_is_omitted() {
        let contents = r#"
; comment
[application]
config/name="Sample"
run/main_scene="res://ui/main_menu.tscn"

[rendering]
run/main_scene="res://wrong.tscn"
"#;

        assert_eq!(
            parse_project_main_scene(contents).as_deref(),
            Some("res://ui/main_menu.tscn")
        );
    }

    #[test]
    fn check_project_main_scene_parser_unescapes_godot_strings() {
        let contents = r#"
[application]
run/main_scene="res://ui/menu\"quoted.tscn"
"#;

        assert_eq!(
            parse_project_main_scene(contents).as_deref(),
            Some("res://ui/menu\"quoted.tscn")
        );
    }

    #[test]
    fn check_script_validation_rejects_non_project_or_non_gd_targets() {
        let root = temp_project("script-validation");
        fs::create_dir_all(root.join("src")).unwrap();
        fs::write(root.join("src/main.gd"), "extends Node\n").unwrap();

        assert!(validate_requested_script("check", &root, "res://src/main.gd").is_none());

        let relative = validate_requested_script("check", &root, "src/main.gd").unwrap();
        assert_eq!(relative.kind, "script_missing");
        assert_eq!(relative.actual, Some(json!("missing")));

        let wrong_extension = validate_requested_script("check", &root, "res://src/main.txt")
            .expect("non-gd target should fail");
        assert_eq!(wrong_extension.actual, Some(json!("not_gd_script")));

        let outside = validate_requested_script("check", &root, "res://../outside.gd")
            .expect("outside target should fail");
        assert_eq!(outside.actual, Some(json!("outside_project")));
    }

    #[test]
    fn discover_project_scripts_excludes_addons_and_artifacts() {
        let root = temp_project("script-discovery");
        for path in [
            "src/main.gd",
            "ui/menu.gd",
            "addons/plugin/plugin.gd",
            ".godot/generated.gd",
            ".qa/artifacts/check/generated.gd",
            "qa/artifacts/legacy/generated.gd",
        ] {
            let path = root.join(path);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, "extends Node\n").unwrap();
        }

        let scripts = discover_project_scripts(&root).unwrap();
        assert_eq!(
            scripts,
            vec![
                "res://src/main.gd".to_string(),
                "res://ui/menu.gd".to_string()
            ]
        );
    }

    #[test]
    fn script_compile_result_shape_is_target_based() {
        let root = temp_project("script-result-shape");
        let config = ProjectConfig {
            project_root: root.clone(),
            repo_root: root.clone(),
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
        };
        let check_artifacts = prepare_check_artifacts(&config, "check");
        let script_artifacts = prepare_script_artifacts(&check_artifacts, "res://src/main.gd");
        fs::write(
            &script_artifacts.result_path,
            r#"{"ok":true,"id":"res://src/main.gd","kind":"script_compile","target":"res://src/main.gd","duration_ms":3,"runtime_errors":[],"failures":[]}"#,
        )
        .unwrap();
        fs::write(&script_artifacts.logs_path, "").unwrap();
        let runtime_result = load_runtime_result("res://src/main.gd", &script_artifacts).unwrap();
        let process_result = godot_qa_process::GodotRunResult {
            command: vec!["godot".to_string()],
            returncode: 0,
            termination: None,
            process_timing: None,
            stdout: String::new(),
            stderr: String::new(),
            display: godot_qa_process::resolve_project_display_launch_metadata(&root, &[], true),
            pid: None,
            process_group: None,
            cleanup: None,
            infrastructure_failure_kind: None,
            timed_out: false,
            timeout_ms: None,
            output_truncation: None,
        };

        let (payload, failures) = validate_script_compile_result(
            "res://src/main.gd",
            runtime_result,
            &script_artifacts,
            &process_result,
        );
        assert!(failures.is_empty());
        assert_eq!(payload["target"], json!("res://src/main.gd"));
        assert_eq!(payload["status"], json!("passed"));
        assert_eq!(payload["ok"], json!(true));
    }
}
