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

/// Route `session reload`: no targets = full-scene reload (unchanged); `--script` =
/// engine-faithful script soft reload; `--resource` = live resource reload; `--scene` =
/// faithful `.tscn` structural live-edit. Requested classes are applied in order
/// (scripts, resources, scenes); the first failure short-circuits and later results are
/// nested under the top-level result's data.
fn session_reload_command(config: &ProjectConfig, args: &SessionReloadArgs) -> CommandResult {
    let to_res = |paths: &[String]| -> Vec<String> {
        paths
            .iter()
            .map(|path| crate::watch::to_res_path(&config.project_root, path))
            .collect()
    };
    let scripts = to_res(&args.scripts);
    let resources = to_res(&args.resources);
    let scenes = to_res(&args.scenes);
    if scripts.is_empty() && resources.is_empty() && scenes.is_empty() {
        return runtime::session_reload_result(config);
    }

    let mut steps: Vec<(&str, Vec<String>)> = Vec::new();
    if !scripts.is_empty() {
        steps.push(("scripts", scripts));
    }
    if !resources.is_empty() {
        steps.push(("resources", resources));
    }
    if !scenes.is_empty() {
        steps.push(("scenes", scenes));
    }

    let mut accumulated: Option<CommandResult> = None;
    for (label, targets) in &steps {
        let result = match *label {
            "scripts" => runtime::session_reload_scripts_result(config, targets),
            "resources" => runtime::session_reload_resources_result(config, targets),
            _ => runtime::session_reload_structure_result(config, targets),
        };
        if !result.ok {
            return result;
        }
        accumulated = Some(match accumulated {
            None => result,
            Some(mut acc) => {
                acc.data.insert(label.to_string(), json!(result.data));
                acc
            }
        });
    }
    accumulated.unwrap_or_else(|| runtime::session_reload_result(config))
}

pub(crate) fn runtime_result(cli: &Cli) -> Option<CommandResult> {
    let config = runtime_config(cli);
    match &cli.command {
        Commands::Session(args) => match &args.command {
            SessionCommands::Start(args) => {
                let target = match args.target {
                    SessionTargetArg::Desktop => runtime::SessionTarget::Desktop,
                    SessionTargetArg::Android => runtime::SessionTarget::Android,
                };
                let frame_pacing_request = if target == runtime::SessionTarget::Android {
                    if args.fastest || args.fixed_fps.is_some() {
                        Some(args.fixed_fps)
                    } else {
                        None
                    }
                } else {
                    match session_frame_pacing_request(
                        args.fastest,
                        args.fixed_fps,
                        "session start",
                    ) {
                        Ok(request) => request,
                        Err(result) => return Some(result),
                    }
                };
                let headless = if target == runtime::SessionTarget::Android {
                    args.headless
                } else {
                    args.headless || config.launch_mode_headless_override().unwrap_or(false)
                };
                // Audio axis is independent of display: honor --audio/--no-audio,
                // then the project config, else the pre-audio-axis default (real
                // audio only when windowed; `--headless` already forces Dummy).
                let audio = crate::scenario::audio_override(args.audio, args.no_audio)
                    .or_else(|| config.audio_mode_override())
                    .unwrap_or(!headless);
                Some(append_config_warnings(
                    runtime::session_start_target_result(
                        &config,
                        runtime::SessionStartRequest {
                            target,
                            scene: args.scene.as_deref(),
                            viewport: args.viewport.as_deref(),
                            headless,
                            audio,
                            no_audio: args.no_audio,
                            project_args: &args.project_args,
                            frame_pacing_request,
                            fastest: args.fastest,
                            hot_reload: args.hot_reload,
                            preset: args.preset.as_deref(),
                            device: args.device.as_deref(),
                            package: args.package_id.as_deref(),
                            no_apk_cache: args.no_apk_cache,
                        },
                    ),
                    &config,
                ))
            }
            SessionCommands::Status => Some(runtime::session_status_result(&config)),
            SessionCommands::List => Some(runtime::session_list_result(&config)),
            SessionCommands::RunScript(args) => Some(session_run_script_result(&config, args)),
            SessionCommands::Pause => Some(runtime::run_live_protocol_command(
                &config,
                "session pause",
                runtime::SESSION_PAUSE,
                serde_json::Map::new(),
            )),
            SessionCommands::Resume => Some(runtime::run_live_protocol_command(
                &config,
                "session resume",
                runtime::SESSION_RESUME,
                serde_json::Map::new(),
            )),
            SessionCommands::Wait(args) => Some(runtime::run_live_protocol_command(
                &config,
                "session wait",
                runtime::SESSION_WAIT_FRAMES,
                serde_json::Map::from_iter([("frames".to_string(), json!(args.frames))]),
            )),
            SessionCommands::TimeScale(args) => match session_time_scale_params(args) {
                Ok(params) => Some(runtime::run_live_protocol_command(
                    &config,
                    "session time-scale",
                    runtime::SESSION_SET_TIME_SCALE,
                    params,
                )),
                Err(failure) => Some(invalid_params_result("session time-scale", failure)),
            },
            SessionCommands::Reload(args) => Some(session_reload_command(&config, args)),
            SessionCommands::Watch(_) => None,
            SessionCommands::Step(args) => {
                if args.frames <= 0 {
                    return Some(CommandResult {
                        ok: false,
                        command: "session step".to_string(),
                        message: "live command failed".to_string(),
                        data: serde_json::Map::new(),
                        failures: vec![FailureDetail {
                            kind: "invalid_params".to_string(),
                            command: Some("session step".to_string()),
                            message: Some(
                                "session step requires a positive integer frame count".to_string(),
                            ),
                            expected: Some(json!("positive integer frames")),
                            actual: Some(json!(args.frames)),
                            ..FailureDetail::default()
                        }],
                        exit_code: 1,
                    });
                }
                Some(runtime::run_live_protocol_command(
                    &config,
                    "session step",
                    runtime::SESSION_STEP,
                    serde_json::Map::from_iter([("frames".to_string(), json!(args.frames))]),
                ))
            }
            SessionCommands::Profile(args) => {
                if let Some(result) = run_setup_script_before(
                    &config,
                    "session profile",
                    args.setup_script.as_deref(),
                ) {
                    return Some(result);
                }
                let result = session_profile_result(&config, args);
                let result = attach_setup_script_metadata(result, args.setup_script.as_deref());
                Some(maybe_summary_json_result(result, args.summary_json))
            }
            SessionCommands::Stop => Some(runtime::session_stop_result(&config)),
        },
        Commands::Snapshot(args) => {
            let params = match snapshot_filter_params("snapshot", &args.filters) {
                Ok(mut params) => {
                    params.insert("name".to_string(), json!(args.name.clone()));
                    params
                }
                Err(failure) => return Some(invalid_params_result("snapshot", failure)),
            };
            Some(runtime::run_live_screenshot_protocol_command(
                &config,
                "snapshot",
                runtime::SNAPSHOT_CAPTURE,
                params,
            ))
        }
        Commands::Tree(args) => {
            let params = match snapshot_filter_params("tree", &args.filters) {
                Ok(params) => params,
                Err(failure) => return Some(invalid_params_result("tree", failure)),
            };
            let session_path = runtime::session_json_path(&config);
            if session_path.exists() {
                Some(runtime::run_live_protocol_command(
                    &config,
                    "tree",
                    runtime::TREE_DUMP,
                    params,
                ))
            } else {
                Some(godot_qa_scenario::run::run_generated_tree_once(
                    &config, &params, "tree",
                ))
            }
        }
        Commands::Ui => Some(runtime::run_live_protocol_command(
            &config,
            "ui",
            runtime::UI_DUMP,
            serde_json::Map::new(),
        )),
        Commands::Focus(args) => Some(match args.command {
            Some(FocusCommands::Graph) => runtime::run_live_protocol_command(
                &config,
                "focus graph",
                runtime::FOCUS_GRAPH,
                serde_json::Map::new(),
            ),
            None => runtime::run_live_protocol_command(
                &config,
                "focus",
                runtime::FOCUS_GET,
                serde_json::Map::new(),
            ),
        }),
        Commands::Node(args) => Some(match &args.command {
            NodeCommands::Inspect(inspect) => {
                let session_path = runtime::session_json_path(&config);
                if session_path.exists() {
                    runtime::run_live_protocol_command(
                        &config,
                        "node inspect",
                        runtime::NODE_INSPECT,
                        serde_json::Map::from_iter([(
                            "target".to_string(),
                            json!(inspect.target.clone()),
                        )]),
                    )
                } else {
                    godot_qa_scenario::run::run_generated_inspect_once(
                        &config,
                        &inspect.target,
                        "node inspect",
                    )
                }
            }
        }),
        Commands::Physics(args) => Some(match &args.command {
            PhysicsCommands::Inspect(inspect) => match physics_inspect_params(inspect) {
                Ok(params) => runtime::physics_inspect_result(&config, params),
                Err(failure) => invalid_params_result("physics inspect", failure),
            },
            PhysicsCommands::Overlay(overlay) => match physics_overlay_params(overlay) {
                Ok(params) => match (&overlay.scenario, &overlay.snapshot) {
                    (Some(scenario), Some(snapshot)) => {
                        physics_overlay::physics_overlay_from_snapshot_result(
                            &config,
                            params,
                            scenario,
                            snapshot,
                            overlay.out.clone(),
                        )
                    }
                    _ => physics_overlay::physics_overlay_result(
                        &config,
                        params,
                        overlay.out.clone(),
                    ),
                },
                Err(failure) => invalid_params_result("physics overlay", failure),
            },
            PhysicsCommands::CollisionReport(args) => match physics_collision_report_params(args) {
                Ok(params) => {
                    if let Some(result) = run_setup_script_before(
                        &config,
                        "physics collision-report",
                        args.setup_script.as_deref(),
                    ) {
                        result
                    } else {
                        let result = physics_collision_report_result(&config, args, params);
                        let result =
                            attach_setup_script_metadata(result, args.setup_script.as_deref());
                        maybe_summary_json_result(result, args.summary_json)
                    }
                }
                Err(failure) => invalid_params_result("physics collision-report", failure),
            },
        }),
        Commands::Surface(args) => Some(match &args.command {
            SurfaceCommands::Audit(args) => match surface_audit_params(args) {
                Ok(params) => {
                    if let Some(result) = run_setup_script_before(
                        &config,
                        "surface audit",
                        args.setup_script.as_deref(),
                    ) {
                        result
                    } else {
                        attach_setup_script_metadata(
                            surface_audit::surface_audit_result(&config, params),
                            args.setup_script.as_deref(),
                        )
                    }
                }
                Err(failure) => invalid_params_result("surface audit", failure),
            },
        }),
        Commands::Trace(args) => Some(match &args.command {
            TraceCommands::Record(record) => match trace_record_params(record) {
                Ok(params) => {
                    if let Some(result) = run_setup_script_before_with_timeout(
                        &config,
                        "trace record",
                        record.setup_script.as_deref(),
                        record.setup_timeout_seconds,
                    ) {
                        result
                    } else {
                        let result = runtime::trace_record_result(&config, params);
                        let result =
                            attach_setup_script_metadata(result, record.setup_script.as_deref());
                        maybe_summary_json_result(result, record.summary_json)
                    }
                }
                Err(failure) => invalid_params_result("trace record", failure),
            },
        }),
        Commands::Event(args) => Some(match &args.command {
            EventCommands::Trace(args) => match event_trace_params(args) {
                Ok(params) => {
                    if let Some(result) = run_setup_script_before(
                        &config,
                        "event trace",
                        args.setup_script.as_deref(),
                    ) {
                        result
                    } else {
                        let result = runtime::event_trace_result(&config, params);
                        let result =
                            attach_setup_script_metadata(result, args.setup_script.as_deref());
                        maybe_summary_json_result(result, args.summary_json)
                    }
                }
                Err(failure) => invalid_params_result("event trace", failure),
            },
            EventCommands::Summarize(args) => match event_summarize_params(args) {
                Ok(params) => runtime::event_summarize_result(
                    &params.events,
                    params.pairing_config.as_deref(),
                    params.strict_warnings,
                ),
                Err(failure) => invalid_params_result("event summarize", failure),
            },
        }),
        Commands::Telemetry(args) => Some(match &args.command {
            TelemetryCommands::Record(args) => match telemetry_record_params(args) {
                Ok(params) => {
                    runtime::telemetry_record_result(&config, params, args.out.as_deref())
                }
                Err(failure) => invalid_params_result("telemetry record", failure),
            },
        }),
        Commands::Audio(args) => Some(match &args.command {
            AudioCommands::Capture(args) => match audio_capture_request(&config, args) {
                Ok(request) => {
                    if let Some(result) = run_setup_script_before(
                        &config,
                        "audio capture",
                        args.setup_script.as_deref(),
                    ) {
                        result
                    } else {
                        attach_setup_script_metadata(
                            runtime::audio_capture_result(&config, request),
                            args.setup_script.as_deref(),
                        )
                    }
                }
                Err(failure) => invalid_params_result("audio capture", failure),
            },
            AudioCommands::Export(args) => match audio_export_request(&config, args) {
                Ok(request) => runtime::audio_export_result(&config, request),
                Err(failure) => invalid_params_result("audio export", failure),
            },
            AudioCommands::Render(args) => match audio_render_request(&config, args) {
                Ok(request) => runtime::audio_render_result(&config, request),
                Err(failure) => invalid_params_result("audio render", failure),
            },
            AudioCommands::Analyze(_) => return None,
        }),
        Commands::Errors => Some(runtime::run_live_protocol_command(
            &config,
            "errors",
            runtime::ERRORS_READ,
            serde_json::Map::new(),
        )),
        Commands::Logs(args) => Some(runtime::run_live_protocol_command(
            &config,
            "logs",
            runtime::LOGS_READ,
            serde_json::Map::from_iter([("since".to_string(), json!(args.since.clone()))]),
        )),
        Commands::Input(args) => Some(match &args.command {
            InputCommands::Tap(args) => runtime::run_live_protocol_command(
                &config,
                "input tap",
                runtime::INPUT_TAP_ACTION,
                serde_json::Map::from_iter([("action".to_string(), json!(args.action.clone()))]),
            ),
            InputCommands::Hold(args) => runtime::run_live_protocol_command(
                &config,
                "input hold",
                runtime::INPUT_HOLD_ACTION,
                serde_json::Map::from_iter([
                    ("action".to_string(), json!(args.action.clone())),
                    ("frames".to_string(), json!(args.frames)),
                ]),
            ),
            InputCommands::Release(args) => runtime::run_live_protocol_command(
                &config,
                "input release",
                runtime::INPUT_RELEASE_ACTION,
                serde_json::Map::from_iter([("action".to_string(), json!(args.action.clone()))]),
            ),
            InputCommands::Click(args) => runtime::run_live_protocol_command(
                &config,
                "input click",
                runtime::INPUT_CLICK_TARGET,
                serde_json::Map::from_iter([("target".to_string(), json!(args.target.clone()))]),
            ),
            InputCommands::Mouse(args) => match &args.command {
                InputMouseCommands::Click(click) => runtime::run_live_protocol_command(
                    &config,
                    "input mouse click",
                    runtime::INPUT_MOUSE_CLICK,
                    serde_json::Map::from_iter([
                        ("x".to_string(), json!(click.x)),
                        ("y".to_string(), json!(click.y)),
                    ]),
                ),
                InputMouseCommands::Wheel(wheel) => runtime::run_live_protocol_command(
                    &config,
                    "input mouse wheel",
                    runtime::INPUT_MOUSE_WHEEL,
                    serde_json::Map::from_iter([
                        ("x".to_string(), json!(wheel.x)),
                        ("y".to_string(), json!(wheel.y)),
                        ("dy".to_string(), json!(wheel.dy)),
                        ("dx".to_string(), json!(wheel.dx)),
                    ]),
                ),
            },
            InputCommands::Touch(args) => match &args.command {
                InputTouchCommands::Down(down) => runtime::run_live_protocol_command(
                    &config,
                    "input touch down",
                    runtime::INPUT_TOUCH_DOWN,
                    serde_json::Map::from_iter([
                        ("x".to_string(), json!(down.x)),
                        ("y".to_string(), json!(down.y)),
                        ("index".to_string(), json!(down.index)),
                    ]),
                ),
                InputTouchCommands::Move(move_args) => runtime::run_live_protocol_command(
                    &config,
                    "input touch move",
                    runtime::INPUT_TOUCH_MOVE,
                    serde_json::Map::from_iter([
                        ("x".to_string(), json!(move_args.x)),
                        ("y".to_string(), json!(move_args.y)),
                        ("index".to_string(), json!(move_args.index)),
                        ("frames".to_string(), json!(move_args.frames)),
                    ]),
                ),
                InputTouchCommands::Up(up) => runtime::run_live_protocol_command(
                    &config,
                    "input touch up",
                    runtime::INPUT_TOUCH_UP,
                    serde_json::Map::from_iter([
                        ("x".to_string(), json!(up.x)),
                        ("y".to_string(), json!(up.y)),
                        ("index".to_string(), json!(up.index)),
                    ]),
                ),
                InputTouchCommands::Tap(tap) => runtime::run_live_protocol_command(
                    &config,
                    "input touch tap",
                    runtime::INPUT_TOUCH_TAP,
                    serde_json::Map::from_iter([
                        ("x".to_string(), json!(tap.x)),
                        ("y".to_string(), json!(tap.y)),
                        ("index".to_string(), json!(tap.index)),
                    ]),
                ),
            },
            InputCommands::Drag(drag) => {
                let pointer = match drag.pointer {
                    PointerKind::Touch => "touch",
                    PointerKind::Mouse => "mouse",
                };
                runtime::run_live_protocol_command(
                    &config,
                    "input drag",
                    runtime::INPUT_DRAG,
                    serde_json::Map::from_iter([
                        ("from_x".to_string(), json!(drag.from_x)),
                        ("from_y".to_string(), json!(drag.from_y)),
                        ("to_x".to_string(), json!(drag.to_x)),
                        ("to_y".to_string(), json!(drag.to_y)),
                        ("frames".to_string(), json!(drag.frames)),
                        ("pointer".to_string(), json!(pointer)),
                        ("index".to_string(), json!(drag.index)),
                    ]),
                )
            }
            InputCommands::Key(args) => runtime::run_live_protocol_command(
                &config,
                "input key",
                runtime::INPUT_KEY,
                serde_json::Map::from_iter([("key".to_string(), json!(args.key.clone()))]),
            ),
            InputCommands::Type(args) => runtime::run_live_protocol_command(
                &config,
                "input type",
                runtime::INPUT_TEXT,
                serde_json::Map::from_iter([("text".to_string(), json!(args.text.clone()))]),
            ),
            InputCommands::Joy(args) => match &args.command {
                InputJoyCommands::Button(button) => runtime::run_live_protocol_command(
                    &config,
                    "input joy button",
                    runtime::INPUT_JOY_BUTTON,
                    serde_json::Map::from_iter([
                        ("button".to_string(), json!(button.button.clone())),
                        ("device".to_string(), json!(button.device)),
                        ("frames".to_string(), json!(button.frames)),
                    ]),
                ),
                InputJoyCommands::Motion(motion) => runtime::run_live_protocol_command(
                    &config,
                    "input joy motion",
                    runtime::INPUT_JOY_MOTION,
                    serde_json::Map::from_iter([
                        ("axis".to_string(), json!(motion.axis.clone())),
                        ("value".to_string(), json!(motion.value)),
                        ("device".to_string(), json!(motion.device)),
                        ("frames".to_string(), json!(motion.frames)),
                    ]),
                ),
            },
        }),
        _ => None,
    }
}

pub(crate) fn runtime_config(cli: &Cli) -> ProjectConfig {
    let resolved_binary = godot_qa_process::resolve_godot_binary(cli.godot_bin.as_deref());
    load_project_config(
        cli.project.as_deref().map(Path::new),
        cli.godot_bin.as_deref(),
        cli.artifact_dir.as_deref(),
        cli.verbose,
        resolved_binary,
    )
}

/// Default wall-clock ceiling for setup-script execution. Setup scripts can spawn
/// a live session (`session_start`) that may hang indefinitely if the game never
/// becomes reachable, so every `--setup-script` path is bounded even without an
/// explicit `--setup-timeout-seconds`.
pub(crate) const DEFAULT_SETUP_SCRIPT_TIMEOUT_SECONDS: u64 = 120;

pub(crate) fn run_setup_script_before(
    config: &ProjectConfig,
    command: &str,
    setup_script: Option<&str>,
) -> Option<CommandResult> {
    run_setup_script_before_with_timeout(config, command, setup_script, None)
}

/// Run a `--setup-script` before the main command, bounded by a wall-clock
/// deadline (`timeout_seconds`, or [`DEFAULT_SETUP_SCRIPT_TIMEOUT_SECONDS`] when
/// `None`/`0`). The script runs on a worker thread; if it overruns, any live
/// session it started is stopped so the worker unblocks and the Godot process is
/// reaped, and a structured `setup_script_timeout` failure is returned.
pub(crate) fn run_setup_script_before_with_timeout(
    config: &ProjectConfig,
    command: &str,
    setup_script: Option<&str>,
    timeout_seconds: Option<u64>,
) -> Option<CommandResult> {
    let setup_script = setup_script
        .map(str::trim)
        .filter(|path| !path.is_empty())?;
    let timeout_seconds = timeout_seconds
        .filter(|value| *value > 0)
        .unwrap_or(DEFAULT_SETUP_SCRIPT_TIMEOUT_SECONDS);

    let started = Instant::now();
    let worker_config = config.clone();
    let worker_script = setup_script.to_string();
    // Detached worker: on timeout we stop the session (below), which unblocks any
    // hung `session_start` probe so this thread finishes shortly after.
    let setup_result =
        match run_bounded_setup_script(Duration::from_secs(timeout_seconds), move || {
            session_run_script_result(
                &worker_config,
                &SessionRunScriptArgs {
                    path: worker_script,
                },
            )
        }) {
            Ok(result) => result,
            Err(mpsc::RecvTimeoutError::Timeout) => {
                // Clean up any session the overrunning script started so we do not leak
                // a Godot process, then report the deadline breach.
                let _ = runtime::session_stop_result(config);
                return Some(setup_script_timeout_result(
                    command,
                    setup_script,
                    timeout_seconds,
                    started.elapsed(),
                ));
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                // The worker panicked before sending; surface it as a setup failure.
                return Some(setup_script_worker_failed_result(command, setup_script));
            }
        };
    if setup_result.ok {
        return None;
    }
    Some(CommandResult {
        ok: false,
        command: command.to_string(),
        message: format!("{command} setup script failed"),
        data: Map::from_iter([
            ("setup_script".to_string(), json!(setup_script)),
            ("setup_result".to_string(), json!(setup_result)),
        ]),
        failures: setup_result.failures,
        exit_code: 1,
    })
}

/// Run `work` on a detached worker thread, returning its result or a
/// `RecvTimeoutError` if it does not finish within `timeout` (or panics). Kept
/// separate so the deadline mechanism can be unit-tested without a Godot binary.
pub(crate) fn run_bounded_setup_script<F>(
    timeout: Duration,
    work: F,
) -> Result<CommandResult, mpsc::RecvTimeoutError>
where
    F: FnOnce() -> CommandResult + Send + 'static,
{
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        let _ = tx.send(work());
    });
    rx.recv_timeout(timeout)
}

pub(crate) fn setup_script_timeout_result(
    command: &str,
    setup_script: &str,
    timeout_seconds: u64,
    elapsed: Duration,
) -> CommandResult {
    let elapsed_seconds = elapsed.as_secs_f64();
    CommandResult {
        ok: false,
        command: command.to_string(),
        message: format!("{command} setup script timed out"),
        data: Map::from_iter([
            ("setup_script".to_string(), json!(setup_script)),
            ("setup_timeout_seconds".to_string(), json!(timeout_seconds)),
            ("elapsed_seconds".to_string(), json!(elapsed_seconds)),
        ]),
        failures: vec![FailureDetail {
            kind: "setup_script_timeout".to_string(),
            command: Some(command.to_string()),
            path: Some(setup_script.to_string()),
            message: Some(format!(
                "setup script {setup_script} did not finish within {timeout_seconds}s (elapsed {elapsed_seconds:.1}s); any live session it started was stopped"
            )),
            expected: Some(json!(format!(
                "setup script completes within {timeout_seconds}s"
            ))),
            actual: Some(json!(format!("still running after {elapsed_seconds:.1}s"))),
            ..FailureDetail::default()
        }],
        exit_code: 1,
    }
}

fn setup_script_worker_failed_result(command: &str, setup_script: &str) -> CommandResult {
    CommandResult {
        ok: false,
        command: command.to_string(),
        message: format!("{command} setup script failed"),
        data: Map::from_iter([("setup_script".to_string(), json!(setup_script))]),
        failures: vec![FailureDetail {
            kind: "setup_script_failed".to_string(),
            command: Some(command.to_string()),
            path: Some(setup_script.to_string()),
            message: Some(format!(
                "setup script {setup_script} worker terminated unexpectedly before reporting a result"
            )),
            expected: Some(json!("setup script runs to completion")),
            actual: Some(json!("worker_terminated")),
            ..FailureDetail::default()
        }],
        exit_code: 1,
    }
}

pub(crate) fn attach_setup_script_metadata(
    mut result: CommandResult,
    setup_script: Option<&str>,
) -> CommandResult {
    if let Some(setup_script) = setup_script.map(str::trim).filter(|path| !path.is_empty()) {
        result
            .data
            .insert("setup_script".to_string(), json!(setup_script));
    }
    result
}

pub(crate) fn maybe_summary_json_result(
    result: CommandResult,
    summary_json: bool,
) -> CommandResult {
    if summary_json {
        summary_json_result(result)
    } else {
        result
    }
}

pub(crate) fn summary_json_result(mut result: CommandResult) -> CommandResult {
    let mut data = Value::Object(result.data);
    prune_summary_json_payload(&mut data);
    if let Value::Object(mut object) = data {
        object.insert("summary_json".to_string(), json!(true));
        result.data = object;
    } else {
        result.data = Map::from_iter([("summary_json".to_string(), json!(true))]);
    }
    result
}

fn prune_summary_json_payload(value: &mut Value) {
    prune_summary_json_payload_inner(value, false);
}

fn prune_summary_json_payload_inner(value: &mut Value, preserve_keys: bool) {
    match value {
        Value::Object(object) => {
            if !preserve_keys {
                for key in [
                    "samples",
                    "events",
                    "frame_times_ms",
                    "frame_samples",
                    "monitor_samples",
                    "step_timings",
                ] {
                    object.remove(key);
                }
            }
            for (key, value) in object.iter_mut() {
                prune_summary_json_payload_inner(value, preserve_keys || key == "artifacts");
            }
        }
        Value::Array(values) => {
            for value in values {
                prune_summary_json_payload_inner(value, preserve_keys);
            }
        }
        _ => {}
    }
}
