#![allow(
    clippy::collapsible_if,
    clippy::large_enum_variant,
    clippy::result_large_err,
    clippy::unnecessary_lazy_evaluations
)]

mod args;
mod artifact;
mod artifact_export;
mod artifact_verdict;
mod asset_commands;
mod audio;
mod check_command;
mod collision_report;
mod diagnostics_command;
mod doctor_command;
mod edit_runtime;
mod editor_profile;
mod import_command;
mod install_skills;
mod live_params;
mod physics_overlay;
#[cfg(target_os = "linux")]
mod presented_startup;
#[cfg(not(target_os = "linux"))]
#[path = "presented_startup_unsupported.rs"]
mod presented_startup;
mod qa_state;
mod qa_state_command;
mod runtime_router;
mod scenario;
mod session_commands;
mod session_script;
mod surface_audit;
mod visual;
mod watch;

#[cfg(test)]
mod cli_tests;
#[cfg(test)]
mod live_params_tests;
#[cfg(test)]
mod session_command_tests;
#[cfg(test)]
mod session_script_tests;
#[cfg(test)]
mod watch_tests;

#[cfg(test)]
pub(crate) use godot_qa_runtime as runtime;

pub(crate) use args::*;
pub(crate) use artifact::*;
pub(crate) use artifact_export::*;
pub(crate) use artifact_verdict::*;
pub(crate) use audio::*;
pub(crate) use check_command::*;
pub(crate) use diagnostics_command::*;
pub(crate) use doctor_command::*;
pub(crate) use edit_runtime::*;
pub(crate) use editor_profile::*;
pub(crate) use import_command::*;
pub(crate) use live_params::*;
pub(crate) use qa_state_command::*;
pub(crate) use runtime_router::*;
pub(crate) use scenario::*;
pub(crate) use session_commands::*;
pub(crate) use session_script::*;
pub(crate) use visual::*;
pub(crate) use watch::*;

use clap::{Parser, error::ErrorKind};
use godot_qa_core::config::ProjectConfig;
use godot_qa_core::results::{CommandResult, FailureDetail};
use serde_json::json;
use std::fmt::Write as _;
use std::io::{self, Write};
use std::path::Path;

pub(crate) const STARTUP_FRAMES: i64 = 5;
pub(crate) const SESSION_SCRIPT_RETRY_ATTEMPTS: usize = 3;
pub(crate) const SESSION_SCRIPT_RETRY_BACKOFF_MS: u64 = 100;
pub(crate) const SESSION_SCRIPT_WAIT_BUSY_FPS: f64 = 30.0;
pub(crate) const SESSION_SCRIPT_WAIT_BUSY_GRACE_MS: u64 = 500;
pub(crate) const SESSION_SCRIPT_WAIT_BUSY_UNKNOWN_BACKOFF_MS: u64 = 1_000;
pub(crate) const SESSION_SCRIPT_WAIT_BUSY_MAX_BACKOFF_MS: u64 = 120_000;
pub(crate) const SCENE_EXTENSIONS: [&str; 2] = ["tscn", "scn"];
pub(crate) const SCRIPT_EXTENSIONS: [&str; 1] = ["gd"];
pub(crate) const SCENE_EXCLUDED_PREFIXES: [&str; 4] = [
    "res://addons/",
    "res://.godot/",
    "res://.qa/",
    "res://qa/artifacts/",
];
pub(crate) const SCENE_EXCLUDED_PATHS: [&str; 1] = ["res://ui/fixtures/runtime_error_probe.tscn"];

fn main() {
    match run(std::env::args_os()) {
        Ok(code) => std::process::exit(code),
        Err(code) => std::process::exit(code),
    }
}

fn run<I, T>(args: I) -> Result<i32, i32>
where
    I: IntoIterator<Item = T>,
    T: Into<std::ffi::OsString> + Clone,
{
    let cli = match Cli::try_parse_from(args) {
        Ok(cli) => cli,
        Err(error) => {
            let _ = error.print();
            return match error.kind() {
                ErrorKind::DisplayHelp | ErrorKind::DisplayVersion => Ok(0),
                _ => Err(2),
            };
        }
    };

    let result = match cli.command {
        Commands::Doctor(ref args) => doctor_result(&cli, args),
        Commands::InstallSkills => install_skills::install_skills_result(&cli),
        Commands::Import => import_result(&cli),
        Commands::Asset(ref args) => asset_commands::asset_result(&cli, args),
        Commands::Check(ref args) if args.tests => check_tests_result(&cli, args),
        Commands::Check(_) => check_result(&cli),
        Commands::Diagnostics(ref args) => match args.command {
            DiagnosticsCommands::Editor(ref args) => editor_diagnostics_result(&cli, args),
        },
        Commands::Profile(ref args) => match args.command {
            ProfileCommands::Editor(ref args) => match args.command {
                EditorProfileCommands::Export(ref args) => editor_profile_export_result(&cli, args),
            },
        },
        Commands::Scenario(ref args) => match args.command {
            ScenarioCommands::List => scenario_list_result(&cli),
            ScenarioCommands::Bundle(ref args) => match args.command {
                ScenarioBundleCommands::List => scenario_bundle_list_result(&cli),
                ScenarioBundleCommands::Run(ref args) => scenario_bundle_run_result(&cli, args),
            },
            ScenarioCommands::Lint(ref args) => scenario_lint_result(&cli, args),
            ScenarioCommands::Run(ref args) => scenario_run_result(&cli, args),
            ScenarioCommands::Sweep(ref args) => scenario_sweep_result(&cli, args),
            ScenarioCommands::Profile(ref args) => scenario_profile_result(&cli, args),
            ScenarioCommands::Init(ref args) => scenario_init_result(&cli, args),
            ScenarioCommands::Record(ref args) => scenario_record_result(&cli, args),
        },
        Commands::Review(ref args) => match args.command {
            ReviewCommands::ListTemplates => review_list_templates_result(&cli),
            ReviewCommands::Init(ref args) => review_init_result(&cli, args),
        },
        Commands::LiveGate(ref args) => match args.command {
            LiveGateCommands::List => live_gate_list_result(&cli),
            LiveGateCommands::Run(ref args) => live_gate_run_result(&cli, args),
        },
        Commands::QaState(ref args) => match args.command {
            QaStateCommands::Validate(ref args) => qa_state_validate_result(&cli, args),
        },
        Commands::Artifact(ref args) => match args.command {
            ArtifactCommands::Query(ref args) => artifact_query_result(args),
            ArtifactCommands::Summarize(ref args) => artifact_summarize_result(args),
            ArtifactCommands::Prune(ref args) => artifact_prune_result(&cli, args),
            ArtifactCommands::BeginRun(ref args) => artifact_begin_run_result(args),
            ArtifactCommands::ValidateResult(ref args) => artifact_validate_result_result(args),
            ArtifactCommands::Export(ref args) => artifact_export_result(args),
        },
        Commands::Layout(ref args) => match args.command {
            LayoutCommands::Audit(ref args) => layout_audit_result(&cli, args),
            LayoutCommands::AuditCurrent(ref args) => layout_audit_current_result(&cli, args),
        },
        Commands::Assert(ref args) => assert_result(&cli, args),
        Commands::Screenshot(ref args) => screenshot_result(&cli, args),
        Commands::PresentedHelper(ref args) => presented_startup::helper_result(&cli, args),
        Commands::Visual(ref args) => match args.command {
            VisualCommands::Preflight(ref args) => visual_preflight_result(&cli, args),
            VisualCommands::Audit(ref args) => visual_audit_result(&cli, args),
            VisualCommands::Isolate(ref args) => visual_isolate_result(&cli, args),
        },
        Commands::Audio(ref args) => match args.command {
            AudioCommands::Analyze(ref args) => audio_analyze_result(args),
            AudioCommands::Capture(_) | AudioCommands::Export(_) | AudioCommands::Render(_) => {
                runtime_result(&cli).expect("runtime command must be handled")
            }
        },
        Commands::EditLive(ref args) => edit_live_result(&cli, args),
        Commands::ApplyPatch(ref args) => apply_patch_result(&cli, args),
        Commands::Mcp => return run_mcp_server(&cli),
        Commands::DebugHost(ref args) => return run_debug_host(args),
        Commands::Session(ref args) if matches!(args.command, SessionCommands::Watch(_)) => {
            let SessionCommands::Watch(ref watch_args) = args.command else {
                unreachable!("watch command was matched")
            };
            return run_session_watch(&cli, watch_args);
        }
        Commands::Session(_)
        | Commands::Input(_)
        | Commands::Snapshot(_)
        | Commands::Tree(_)
        | Commands::Ui
        | Commands::Focus(_)
        | Commands::Node(_)
        | Commands::Physics(_)
        | Commands::Surface(_)
        | Commands::Trace(_)
        | Commands::Event(_)
        | Commands::Telemetry(_)
        | Commands::Errors
        | Commands::Logs(_) => runtime_result(&cli).expect("runtime command must be handled"),
    };

    print_result(&result, cli.json).map_err(|_| 1)?;
    Ok(result.exit_code)
}

fn run_mcp_server(cli: &Cli) -> Result<i32, i32> {
    let defaults = godot_qa_scenario::mcp::McpServerDefaults {
        project: cli.project.clone(),
        godot_bin: cli.godot_bin.clone(),
        artifact_dir: cli.artifact_dir.clone(),
        verbose: cli.verbose,
    };
    let executable = std::env::current_exe().map_err(|_| 1)?;
    Ok(godot_qa_scenario::mcp::run_stdio_server(
        defaults,
        &executable,
    ))
}

/// Run the persistent debug-host sidecar. It binds its listeners, writes its record,
/// and serves until terminated (Ctrl+C or the `session stop` SIGTERM).
fn run_debug_host(args: &DebugHostArgs) -> Result<i32, i32> {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};

    let _project_lease = args
        .project_root
        .as_deref()
        .map(|project| {
            godot_qa_process::project_access::LaunchLease::acquire(
                project,
                godot_qa_process::project_access::LaunchClass::SourceWriter,
            )
        })
        .transpose()
        .map_err(|error| {
            eprintln!("godot-qa: {}", error.message.unwrap_or(error.kind));
            1
        })?;
    let shutdown = Arc::new(AtomicBool::new(false));
    let handler_flag = Arc::clone(&shutdown);
    let _ = ctrlc::set_handler(move || handler_flag.store(true, Ordering::SeqCst));

    let cfg = godot_qa_debug::DebugHostConfig {
        record_path: args.record.clone(),
        log_path: args.log.clone(),
        game_accept_timeout: std::time::Duration::from_secs(30),
        project_root: args.project_root.clone(),
        initial_scene: args.scene.clone(),
    };
    let host = godot_qa_debug::DebugHost::bind(&cfg).map_err(|error| {
        eprintln!("godot-qa: debug host failed to bind: {error}");
        1
    })?;
    host.run(shutdown).map_err(|error| {
        eprintln!("godot-qa: debug host exited with error: {error}");
        1
    })?;
    Ok(0)
}

fn shell_escape(value: &str) -> String {
    if value.is_empty() {
        return "''".to_string();
    }
    if value.bytes().all(|byte| {
        matches!(
            byte,
            b'a'..=b'z'
                | b'A'..=b'Z'
                | b'0'..=b'9'
                | b'_'
                | b'/'
                | b'.'
                | b'-'
        )
    }) {
        return value.to_string();
    }
    let mut escaped = String::from("'");
    for ch in value.chars() {
        if ch == '\'' {
            let _ = escaped.write_str("'\"'\"'");
        } else {
            escaped.push(ch);
        }
    }
    escaped.push('\'');
    escaped
}

fn verbose_diagnostics(config: &ProjectConfig, cli: &Cli) -> serde_json::Value {
    json!({
        "cwd": std::env::current_dir().ok().map(|path| path.to_string_lossy().into_owned()).unwrap_or_else(|| ".".to_string()),
        "requested": {
            "project": cli.project,
            "artifact_dir": cli.artifact_dir,
            "godot_bin": cli.godot_bin,
        },
        "paths": {
            "project_root_exists": config.project_root.exists(),
            "scenarios_dir_exists": config.scenarios_dir.exists(),
            "project_addon_exists": config.addon_dir.join("plugin.cfg").exists(),
            "repo_addon_exists": config.repo_addon_dir.join("plugin.cfg").exists(),
        }
    })
}

fn addon_status(config: &ProjectConfig) -> &'static str {
    if config.addon_dir.join("plugin.cfg").exists() {
        "installed"
    } else if config.repo_addon_dir.join("plugin.cfg").exists() {
        "installable"
    } else {
        "missing"
    }
}

/// Append the project config's `unsupported_config_file` warnings to a command result's
/// `data.warnings[]`, creating the array if absent. Godot-launching commands call this at
/// the same choke points that surface `project_config_failure`, so a consumer who ships an
/// ignored `qa/config.json`/`.yml` sees the nudge in the command's JSON output. This never
/// touches `ok`, `exit_code`, `failures`, or verdicts.
pub(crate) fn append_config_warnings(
    mut result: CommandResult,
    config: &ProjectConfig,
) -> CommandResult {
    if config.config_warnings.is_empty() {
        return result;
    }
    let entry = result
        .data
        .entry("warnings".to_string())
        .or_insert_with(|| serde_json::Value::Array(Vec::new()));
    if let Some(array) = entry.as_array_mut() {
        array.extend(config.config_warnings.iter().cloned());
    }
    result
}

fn missing_godot_failure(command: &str) -> FailureDetail {
    FailureDetail {
        kind: "missing_godot".to_string(),
        command: Some(command.to_string()),
        message: Some("Godot binary not found in PATH or GODOT_BIN".to_string()),
        expected: Some(json!(
            "discoverable Godot binary via --godot-bin, GODOT_BIN, or PATH"
        )),
        actual: Some(json!("missing")),
        ..FailureDetail::default()
    }
}

fn missing_project_root_failure(command: &str, path: &Path) -> FailureDetail {
    FailureDetail {
        kind: "missing_project_root".to_string(),
        command: Some(command.to_string()),
        path: Some(path.to_string_lossy().into_owned()),
        message: Some("project root does not exist".to_string()),
        expected: Some(json!("existing project root")),
        actual: Some(json!("missing")),
        ..FailureDetail::default()
    }
}

fn missing_scenarios_dir_failure(command: &str, path: &Path) -> FailureDetail {
    FailureDetail {
        kind: "missing_scenarios_dir".to_string(),
        command: Some(command.to_string()),
        path: Some(path.to_string_lossy().into_owned()),
        expected: Some(json!("existing qa/scenarios directory")),
        actual: Some(json!(if path.exists() {
            "not_directory"
        } else {
            "missing"
        })),
        ..FailureDetail::default()
    }
}

fn missing_gates_dir_failure(command: &str, path: &Path) -> FailureDetail {
    FailureDetail {
        kind: "missing_gates_dir".to_string(),
        command: Some(command.to_string()),
        path: Some(path.to_string_lossy().into_owned()),
        expected: Some(json!("existing qa/gates directory")),
        actual: Some(json!(if path.exists() {
            "not_directory"
        } else {
            "missing"
        })),
        ..FailureDetail::default()
    }
}

fn missing_live_gates_dir_failure(command: &str, path: &Path) -> FailureDetail {
    FailureDetail {
        kind: "missing_live_gates_dir".to_string(),
        command: Some(command.to_string()),
        path: Some(path.to_string_lossy().into_owned()),
        message: Some(format!(
            "live gates directory '{}' was not found",
            path.display()
        )),
        expected: Some(json!("qa/live_gates directory")),
        actual: Some(json!("missing")),
        ..FailureDetail::default()
    }
}

fn missing_addon_failure(command: &str, path: &Path) -> FailureDetail {
    FailureDetail {
        kind: "missing_addon".to_string(),
        command: Some(command.to_string()),
        path: Some(path.to_string_lossy().into_owned()),
        message: Some("godot_qa addon is not installed in the target project and no source addon is available".to_string()),
        expected: Some(json!("installed addon in target project or source addon in repo")),
        actual: Some(json!("missing")),
        ..FailureDetail::default()
    }
}

fn ensure_writable_artifact_dir(path: &Path, command: &str) -> Option<FailureDetail> {
    let error = if path.exists() && !path.is_dir() {
        Some(std::io::Error::other(format!(
            "{} is not a directory",
            path.display()
        )))
    } else if let Err(error) = std::fs::create_dir_all(path) {
        Some(error)
    } else {
        let probe_path = path.join(".godot-qa-write-test");
        match std::fs::write(&probe_path, b"") {
            Ok(_) => {
                let _ = std::fs::remove_file(probe_path);
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
        actual: Some(json!(error_kind_name(&error, path))),
        ..FailureDetail::default()
    })
}

fn error_kind_name(error: &std::io::Error, path: &Path) -> &'static str {
    if !path.is_dir() && path.exists() {
        "NotADirectoryError"
    } else {
        match error.kind() {
            std::io::ErrorKind::PermissionDenied => "PermissionError",
            _ => "OSError",
        }
    }
}

#[allow(dead_code)]
fn command_path(command: &Commands) -> &'static str {
    match command {
        Commands::Doctor(_) => "doctor",
        Commands::InstallSkills => "install-skills",
        Commands::Import => "import",
        Commands::Asset(args) => match args.command {
            AssetCommands::Try { .. } => "asset try",
            AssetCommands::Status { .. } => "asset status",
            AssetCommands::Keep { .. } => "asset keep",
            AssetCommands::Discard { .. } => "asset discard",
        },
        Commands::Check(_) => "check",
        Commands::Diagnostics(args) => match args.command {
            DiagnosticsCommands::Editor(_) => "diagnostics editor",
        },
        Commands::Profile(args) => match args.command {
            ProfileCommands::Editor(ref args) => match args.command {
                EditorProfileCommands::Export(_) => "profile editor export",
            },
        },
        Commands::Scenario(args) => match args.command {
            ScenarioCommands::List => "scenario list",
            ScenarioCommands::Bundle(ref args) => match args.command {
                ScenarioBundleCommands::List => "scenario bundle list",
                ScenarioBundleCommands::Run(_) => "scenario bundle run",
            },
            ScenarioCommands::Lint(_) => "scenario lint",
            ScenarioCommands::Run(_) => "scenario run",
            ScenarioCommands::Sweep(_) => "scenario sweep",
            ScenarioCommands::Profile(_) => "scenario profile",
            ScenarioCommands::Init(_) => "scenario init",
            ScenarioCommands::Record(_) => "scenario record",
        },
        Commands::Review(args) => match args.command {
            ReviewCommands::ListTemplates => "review list-templates",
            ReviewCommands::Init(_) => "review init",
        },
        Commands::LiveGate(args) => match args.command {
            LiveGateCommands::List => "live-gate list",
            LiveGateCommands::Run(_) => "live-gate run",
        },
        Commands::QaState(args) => match args.command {
            QaStateCommands::Validate(_) => "qa-state validate",
        },
        Commands::Artifact(args) => match args.command {
            ArtifactCommands::Query(_) => "artifact query",
            ArtifactCommands::Summarize(_) => "artifact summarize",
            ArtifactCommands::Prune(_) => "artifact prune",
            ArtifactCommands::BeginRun(_) => "artifact begin-run",
            ArtifactCommands::ValidateResult(_) => "artifact validate-result",
            ArtifactCommands::Export(_) => "artifact export",
        },
        Commands::Session(args) => match args.command {
            SessionCommands::Start(_) => "session start",
            SessionCommands::Status => "session status",
            SessionCommands::List => "session list",
            SessionCommands::RunScript(_) => "session run-script",
            SessionCommands::Wait(_) => "session wait",
            SessionCommands::TimeScale(_) => "session time-scale",
            SessionCommands::Reload(_) => "session reload",
            SessionCommands::Watch(_) => "session watch",
            SessionCommands::Stop => "session stop",
            SessionCommands::Pause => "session pause",
            SessionCommands::Resume => "session resume",
            SessionCommands::Step(_) => "session step",
            SessionCommands::Profile(_) => "session profile",
        },
        Commands::Input(args) => match args.command {
            InputCommands::Tap(_) => "input tap",
            InputCommands::Hold(_) => "input hold",
            InputCommands::Release(_) => "input release",
            InputCommands::Click(_) => "input click",
            InputCommands::Mouse(ref mouse) => match mouse.command {
                InputMouseCommands::Click(_) => "input mouse click",
                InputMouseCommands::Wheel(_) => "input mouse wheel",
            },
            InputCommands::Touch(ref touch) => match touch.command {
                InputTouchCommands::Down(_) => "input touch down",
                InputTouchCommands::Move(_) => "input touch move",
                InputTouchCommands::Up(_) => "input touch up",
                InputTouchCommands::Tap(_) => "input touch tap",
            },
            InputCommands::Drag(_) => "input drag",
            InputCommands::Key(_) => "input key",
            InputCommands::Type(_) => "input type",
            InputCommands::Joy(ref joy) => match joy.command {
                InputJoyCommands::Button(_) => "input joy button",
                InputJoyCommands::Motion(_) => "input joy motion",
            },
        },
        Commands::Snapshot(_) => "snapshot",
        Commands::Tree(_) => "tree",
        Commands::Ui => "ui",
        Commands::Focus(args) => match args.command {
            Some(FocusCommands::Graph) => "focus graph",
            None => "focus",
        },
        Commands::Node(_) => "node inspect",
        Commands::Physics(args) => match args.command {
            PhysicsCommands::Inspect(_) => "physics inspect",
            PhysicsCommands::Overlay(_) => "physics overlay",
            PhysicsCommands::CollisionReport(_) => "physics collision-report",
        },
        Commands::Surface(args) => match args.command {
            SurfaceCommands::Audit(_) => "surface audit",
        },
        Commands::Trace(_) => "trace record",
        Commands::Errors => "errors",
        Commands::Layout(args) => match args.command {
            LayoutCommands::AuditCurrent(_) => "layout audit-current",
            LayoutCommands::Audit(_) => "layout audit",
        },
        Commands::Assert(_) => "assert",
        Commands::Screenshot(args) => match args.command {
            ScreenshotCommands::Capture(_) => "screenshot capture",
            ScreenshotCommands::AssertStartupPresented(_) => "screenshot assert-startup-presented",
            ScreenshotCommands::Compare(_) => "screenshot compare",
            ScreenshotCommands::Sequence(_) => "screenshot sequence",
            ScreenshotCommands::ContactSheet(_) => "screenshot contact-sheet",
        },
        Commands::Visual(args) => match args.command {
            VisualCommands::Preflight(_) => "visual preflight",
            VisualCommands::Audit(_) => "visual audit",
            VisualCommands::Isolate(_) => "visual isolate",
        },
        Commands::Event(args) => match args.command {
            EventCommands::Trace(_) => "event trace",
            EventCommands::Summarize(_) => "event summarize",
        },
        Commands::Telemetry(args) => match args.command {
            TelemetryCommands::Record(_) => "telemetry record",
        },
        Commands::Audio(args) => match args.command {
            AudioCommands::Capture(_) => "audio capture",
            AudioCommands::Export(_) => "audio export",
            AudioCommands::Render(_) => "audio render",
            AudioCommands::Analyze(_) => "audio analyze",
        },
        Commands::EditLive(args) => match args.command {
            EditCommands::Set(_) => "edit-live set",
            EditCommands::Call(_) => "edit-live call",
            EditCommands::Diff => "edit-live diff",
            EditCommands::ExportPatch(_) => "edit-live export-patch",
        },
        Commands::ApplyPatch(_) => "apply-patch",
        Commands::Mcp => "mcp",
        Commands::Logs(_) => "logs",
        // Hidden internal command; never surfaced in `--help` or the doctor surface.
        Commands::DebugHost(_) => "__debug-host",
        Commands::PresentedHelper(_) => "__presented-helper",
    }
}

fn print_result(result: &CommandResult, as_json: bool) -> io::Result<()> {
    let mut stdout = io::stdout().lock();
    if as_json {
        serde_json::to_writer_pretty(&mut stdout, result)?;
        writeln!(stdout)?;
        return Ok(());
    }

    let status = if result.ok { "OK" } else { "FAIL" };
    writeln!(stdout, "[{status}] {}: {}", result.command, result.message)?;
    if !result.failures.is_empty() {
        writeln!(stdout, "Failures:")?;
        for failure in &result.failures {
            writeln!(stdout, "- {}", serde_json::to_string(failure)?)?;
        }
    }
    Ok(())
}
