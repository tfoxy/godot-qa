use godot_qa_core::results::FailureDetail;
use serde_json::json;
use std::fs::{self, File};
#[cfg(unix)]
use std::os::unix::process::CommandExt;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::Duration;

use super::capture::{
    CAPTURED_STREAM_LIMIT_BYTES, OutputTruncation, OwnedChildOutput, ProcessTermination,
    ProcessTiming, apply_joypad_isolation, run_owned_child_output_with_timeout_monitor,
};
use super::display::{
    DisplayLaunchMetadata, append_project_display_driver_args,
    build_project_display_launch_metadata, classify_display_startup_failure,
    classify_launch_spawn_failure, display_launch_metadata_value, display_startup_guidance,
};
use super::lifecycle::{ProcessCleanupResult, configure_background_command, reap_background_child};
use super::probes::{effective_godot_command, format_command};
use super::project_access;

#[derive(Debug, Clone)]
pub struct GodotRunResult {
    pub command: Vec<String>,
    pub returncode: i32,
    pub termination: Option<ProcessTermination>,
    pub process_timing: Option<ProcessTiming>,
    pub stdout: String,
    pub stderr: String,
    pub display: DisplayLaunchMetadata,
    pub pid: Option<i64>,
    pub process_group: Option<i64>,
    pub cleanup: Option<ProcessCleanupResult>,
    pub infrastructure_failure_kind: Option<String>,
    pub timed_out: bool,
    pub timeout_ms: Option<u64>,
    pub output_truncation: Option<OutputTruncation>,
}

pub struct GodotOneShotLaunch<'a> {
    pub binary: &'a str,
    pub launch_prefix: &'a [String],
    pub joypad_isolation: bool,
    pub windowed_launch_prefix: &'a [String],
    pub default_godot_args: &'a [String],
    pub extra_godot_args: &'a [String],
    pub default_project_args: &'a [String],
    pub extra_project_args: &'a [String],
    pub project_root: &'a Path,
    pub request_path: &'a Path,
    pub logs_path: &'a Path,
    pub headless: bool,
    pub audio: bool,
    pub viewport: Option<&'a [i64]>,
    pub timeout: Option<Duration>,
    pub fixed_fps: Option<i64>,
}

#[derive(Debug, Clone)]
pub struct GodotSessionLaunch {
    pub command: Vec<String>,
    pub pid: i64,
    pub logs_path: String,
    pub display: DisplayLaunchMetadata,
}

pub fn launch_godot_import(
    binary: &str,
    launch_prefix: &[String],
    joypad_isolation: bool,
    project_root: &Path,
    logs_path: &Path,
) -> Result<GodotRunResult, FailureDetail> {
    launch_godot_import_with_args(
        binary,
        launch_prefix,
        joypad_isolation,
        &[],
        project_root,
        logs_path,
    )
}

pub fn launch_godot_import_with_args(
    binary: &str,
    launch_prefix: &[String],
    joypad_isolation: bool,
    default_godot_args: &[String],
    project_root: &Path,
    logs_path: &Path,
) -> Result<GodotRunResult, FailureDetail> {
    let _project_lease = project_access::LaunchLease::acquire(
        project_root,
        project_access::LaunchClass::ImportOrEditor,
    )?;
    let display = build_project_display_launch_metadata(project_root, true, launch_prefix);
    let mut args = vec![
        "--headless".to_string(),
        "--log-file".to_string(),
        godot_log_path(logs_path).to_string_lossy().into_owned(),
    ];
    args.extend(default_godot_args.iter().cloned());
    args.extend([
        "--path".to_string(),
        project_root.to_string_lossy().into_owned(),
        "--import".to_string(),
    ]);
    let command = effective_godot_command(launch_prefix, binary, &args);
    run_godot_command(&command, joypad_isolation, logs_path, display)
}

#[allow(clippy::too_many_arguments)]
pub fn launch_godot_check(
    binary: &str,
    launch_prefix: &[String],
    joypad_isolation: bool,
    default_godot_args: &[String],
    default_project_args: &[String],
    project_root: &Path,
    request_path: &Path,
    logs_path: &Path,
) -> Result<GodotRunResult, FailureDetail> {
    launch_godot_check_with_timeout(
        binary,
        launch_prefix,
        joypad_isolation,
        default_godot_args,
        default_project_args,
        project_root,
        request_path,
        logs_path,
        None,
    )
}

#[allow(clippy::too_many_arguments)]
pub fn launch_godot_check_with_timeout(
    binary: &str,
    launch_prefix: &[String],
    joypad_isolation: bool,
    default_godot_args: &[String],
    default_project_args: &[String],
    project_root: &Path,
    request_path: &Path,
    logs_path: &Path,
    timeout: Option<Duration>,
) -> Result<GodotRunResult, FailureDetail> {
    launch_godot_check_with_timeout_and_fixed_fps(
        binary,
        launch_prefix,
        joypad_isolation,
        default_godot_args,
        default_project_args,
        project_root,
        request_path,
        logs_path,
        timeout,
        None,
    )
}

#[allow(clippy::too_many_arguments)]
pub fn launch_godot_check_with_timeout_and_fixed_fps(
    binary: &str,
    launch_prefix: &[String],
    joypad_isolation: bool,
    default_godot_args: &[String],
    default_project_args: &[String],
    project_root: &Path,
    request_path: &Path,
    logs_path: &Path,
    timeout: Option<Duration>,
    fixed_fps: Option<i64>,
) -> Result<GodotRunResult, FailureDetail> {
    launch_godot_check_with_timeout_monitor(
        binary,
        launch_prefix,
        joypad_isolation,
        default_godot_args,
        default_project_args,
        project_root,
        request_path,
        logs_path,
        timeout,
        fixed_fps,
        None,
    )
}

/// Poll a caller-owned deadline monitor while retaining owned-child timeout cleanup.
/// Returning a duration expires the run and records that budget in timeout evidence.
#[allow(clippy::too_many_arguments)]
pub fn launch_godot_check_with_timeout_monitor(
    binary: &str,
    launch_prefix: &[String],
    joypad_isolation: bool,
    default_godot_args: &[String],
    default_project_args: &[String],
    project_root: &Path,
    request_path: &Path,
    logs_path: &Path,
    timeout: Option<Duration>,
    fixed_fps: Option<i64>,
    timeout_monitor: Option<&mut dyn FnMut() -> Option<Duration>>,
) -> Result<GodotRunResult, FailureDetail> {
    let _project_lease = project_access::LaunchLease::acquire(
        project_root,
        project_access::LaunchClass::ImportOrEditor,
    )?;
    let display = build_project_display_launch_metadata(project_root, true, launch_prefix);
    let mut args = vec![
        "--headless".to_string(),
        "--log-file".to_string(),
        godot_log_path(logs_path).to_string_lossy().into_owned(),
    ];
    append_fixed_fps_args(&mut args, fixed_fps);
    args.extend(default_godot_args.iter().cloned());
    args.extend([
        "--path".to_string(),
        project_root.to_string_lossy().into_owned(),
        "--script".to_string(),
        "res://addons/godot_qa/qa_runner.gd".to_string(),
        "--".to_string(),
        "--qa".to_string(),
        format!("--qa-check-request={}", request_path.to_string_lossy()),
    ]);
    args.extend(default_project_args.iter().cloned());
    let command = effective_godot_command(launch_prefix, binary, &args);
    run_godot_command_with_timeout_monitor(
        &command,
        joypad_isolation,
        logs_path,
        display,
        timeout,
        None,
        timeout_monitor,
    )
}

pub fn launch_godot_one_shot(
    options: GodotOneShotLaunch<'_>,
) -> Result<GodotRunResult, FailureDetail> {
    let mut launch_args = options.default_godot_args.to_vec();
    launch_args.extend_from_slice(options.extra_godot_args);
    let class = project_access::class_with_engine_args(
        project_access::LaunchClass::RuntimePreview,
        &launch_args,
    );
    let _project_lease = project_access::LaunchLease::acquire(options.project_root, class)?;
    let mut args = Vec::new();
    let display = append_project_display_driver_args(
        &mut args,
        options.project_root,
        options.launch_prefix,
        options.windowed_launch_prefix,
        options.headless,
    );
    append_media_axis_args(&mut args, options.headless, options.audio, options.viewport);
    args.extend([
        "--log-file".to_string(),
        godot_log_path(options.logs_path)
            .to_string_lossy()
            .into_owned(),
    ]);
    append_fixed_fps_args(&mut args, options.fixed_fps);
    args.extend(options.default_godot_args.iter().cloned());
    args.extend(options.extra_godot_args.iter().cloned());
    args.extend([
        "--path".to_string(),
        options.project_root.to_string_lossy().into_owned(),
        "--script".to_string(),
        "res://addons/godot_qa/qa_runner.gd".to_string(),
        "--".to_string(),
        "--qa".to_string(),
        format!(
            "--qa-run-request={}",
            options.request_path.to_string_lossy()
        ),
    ]);
    args.extend(options.default_project_args.iter().cloned());
    args.extend(options.extra_project_args.iter().cloned());
    let command = effective_godot_command(&display.effective_command_prefix, options.binary, &args);
    let result_path = options
        .request_path
        .parent()
        .map(|parent| parent.join("result.json"));
    run_godot_command_with_completion_path(
        &command,
        options.joypad_isolation,
        options.logs_path,
        display,
        options.timeout,
        result_path.as_deref(),
    )
}

/// Plain-runtime asset writer/probe. Only an in-process trial coordinator may
/// launch this helper; it adds no runtime bridge filesystem permissions.
pub fn launch_godot_asset_trial_runner(
    options: GodotOneShotLaunch<'_>,
) -> Result<GodotRunResult, FailureDetail> {
    let _project_lease = project_access::LaunchLease::acquire(
        options.project_root,
        project_access::LaunchClass::CoordinatorChild,
    )?;
    if options
        .default_godot_args
        .iter()
        .chain(options.extra_godot_args)
        .any(|arg| matches!(arg.as_str(), "--editor" | "-e" | "--import"))
    {
        return Err(FailureDetail {
            kind: "asset_trial_engine_args_invalid".into(),
            message: Some("asset trial helpers must run without editor/import mode".into()),
            ..FailureDetail::default()
        });
    }
    let mut args = Vec::new();
    let display = append_project_display_driver_args(
        &mut args,
        options.project_root,
        options.launch_prefix,
        options.windowed_launch_prefix,
        options.headless,
    );
    append_media_axis_args(&mut args, options.headless, options.audio, options.viewport);
    args.extend([
        "--log-file".into(),
        godot_log_path(options.logs_path)
            .to_string_lossy()
            .into_owned(),
    ]);
    append_fixed_fps_args(&mut args, options.fixed_fps);
    args.extend(options.default_godot_args.iter().cloned());
    args.extend(options.extra_godot_args.iter().cloned());
    args.extend([
        "--path".into(),
        options.project_root.to_string_lossy().into_owned(),
        "--script".into(),
        "res://addons/godot_qa/qa_asset_trial_runner.gd".into(),
        "--".into(),
        "--qa".into(),
        "--request".into(),
        options.request_path.to_string_lossy().into_owned(),
    ]);
    let command = effective_godot_command(&display.effective_command_prefix, options.binary, &args);
    run_godot_command_with_timeout(
        &command,
        options.joypad_isolation,
        options.logs_path,
        display,
        options.timeout,
    )
}

fn append_fixed_fps_args(args: &mut Vec<String>, fixed_fps: Option<i64>) {
    if let Some(fixed_fps) = fixed_fps {
        args.push("--fixed-fps".to_string());
        args.push(fixed_fps.to_string());
        args.push("--disable-vsync".to_string());
    }
}

/// Emit the media-axis engine flags for the two independent bools `headless`
/// (display off/on) and `audio` (audio off/on), producing the four grid cells:
///
/// - display on  (`!headless`): `--resolution WxH` (the windowed display-driver /
///   xvfb args are already appended by `append_project_display_driver_args`). When
///   audio is off, `append_audio_driver_args` adds `--audio-driver Dummy`.
/// - display off + audio off (`headless && !audio`): `--headless`, which forces
///   BOTH the null display driver and the Dummy audio driver, so no separate
///   `--audio-driver Dummy` is needed.
/// - display off + audio on  (`headless && audio`): `--display-driver headless`
///   nulls only the display and leaves the real audio driver in place.
fn append_media_axis_args(
    args: &mut Vec<String>,
    headless: bool,
    audio: bool,
    viewport: Option<&[i64]>,
) {
    if headless {
        if audio {
            args.push("--display-driver".to_string());
            args.push("headless".to_string());
        } else {
            args.push("--headless".to_string());
        }
    } else if let Some(resolution) = viewport_resolution_arg(viewport) {
        args.push("--resolution".to_string());
        args.push(resolution);
    }
    append_audio_driver_args(args, audio);
}

/// Push `--audio-driver Dummy` when audio is off AND `--headless` was not already
/// emitted (`--headless` already forces the Dummy audio driver, so a second one is
/// redundant). When audio is on, the real audio driver stays in place.
fn append_audio_driver_args(args: &mut Vec<String>, audio: bool) {
    if !audio && !args.iter().any(|arg| arg == "--headless") {
        args.push("--audio-driver".to_string());
        args.push("Dummy".to_string());
    }
}

pub fn launch_godot_patch_runner(
    binary: &str,
    launch_prefix: &[String],
    joypad_isolation: bool,
    default_godot_args: &[String],
    project_root: &Path,
    request_path: &Path,
    logs_path: &Path,
) -> Result<GodotRunResult, FailureDetail> {
    let _project_lease = project_access::LaunchLease::acquire(
        project_root,
        project_access::LaunchClass::SourceWriter,
    )?;
    let display = build_project_display_launch_metadata(project_root, true, launch_prefix);
    let mut args = vec![
        "--headless".to_string(),
        "--log-file".to_string(),
        godot_log_path(logs_path).to_string_lossy().into_owned(),
    ];
    args.extend(default_godot_args.iter().cloned());
    args.extend([
        "--path".to_string(),
        project_root.to_string_lossy().into_owned(),
        "--script".to_string(),
        "res://addons/godot_qa/qa_patch_runner.gd".to_string(),
        "--".to_string(),
        "--qa".to_string(),
        format!("--qa-patch-request={}", request_path.to_string_lossy()),
    ]);
    let command = effective_godot_command(launch_prefix, binary, &args);
    run_godot_command(&command, joypad_isolation, logs_path, display)
}

#[allow(clippy::too_many_arguments)]
pub fn launch_godot_session(
    binary: &str,
    launch_prefix: &[String],
    joypad_isolation: bool,
    windowed_launch_prefix: &[String],
    project_root: &Path,
    request_path: &Path,
    logs_path: &Path,
    port: u16,
    session_token: &str,
    headless: bool,
    audio: bool,
    viewport: Option<&[i64]>,
    fixed_fps: Option<i64>,
    default_godot_args: &[String],
    default_project_args: &[String],
    extra_project_args: &[String],
    remote_debug: Option<&str>,
) -> Result<GodotSessionLaunch, FailureDetail> {
    let class = if remote_debug.is_some() {
        project_access::LaunchClass::SourceWriter
    } else {
        project_access::class_with_engine_args(
            project_access::LaunchClass::RuntimePreview,
            default_godot_args,
        )
    };
    let _project_lease = project_access::LaunchLease::acquire(project_root, class)?;
    let mut args = Vec::new();
    let display = append_project_display_driver_args(
        &mut args,
        project_root,
        launch_prefix,
        windowed_launch_prefix,
        headless,
    );
    append_media_axis_args(&mut args, headless, audio, viewport);
    args.extend([
        "--log-file".to_string(),
        godot_log_path(logs_path).to_string_lossy().into_owned(),
    ]);
    // `--remote-debug` makes the game dial our debug-host listener, enabling
    // engine-faithful hot reload. It is an engine flag, so it belongs in the
    // engine-arg region before `--path`.
    if let Some(remote_debug) = remote_debug {
        args.push("--remote-debug".to_string());
        args.push(remote_debug.to_string());
    }
    append_fixed_fps_args(&mut args, fixed_fps);
    args.extend(default_godot_args.iter().cloned());
    args.extend([
        "--path".to_string(),
        project_root.to_string_lossy().into_owned(),
        "--script".to_string(),
        "res://addons/godot_qa/qa_session_runner.gd".to_string(),
        "--".to_string(),
        "--qa".to_string(),
        format!("--qa-session-request={}", request_path.to_string_lossy()),
        format!("--qa-session-port={port}"),
        format!("--qa-session-token={session_token}"),
    ]);
    args.extend(default_project_args.iter().cloned());
    args.extend(extra_project_args.iter().cloned());
    let command = effective_godot_command(&display.effective_command_prefix, binary, &args);
    let formatted_command = format_command(&command);
    if let Some(parent) = logs_path.parent() {
        fs::create_dir_all(parent).map_err(|error| FailureDetail {
            kind: "artifact_write_failed".to_string(),
            command: Some(formatted_command.clone()),
            path: Some(logs_path.to_string_lossy().into_owned()),
            message: Some(error.to_string()),
            expected: Some(json!("Godot logs written to the artifact path")),
            actual: Some(json!("OSError")),
            ..FailureDetail::default()
        })?;
    }
    let log_file = File::create(logs_path).map_err(|error| FailureDetail {
        kind: "artifact_write_failed".to_string(),
        command: Some(formatted_command.clone()),
        path: Some(logs_path.to_string_lossy().into_owned()),
        message: Some(error.to_string()),
        expected: Some(json!("Godot logs written to the artifact path")),
        actual: Some(json!("OSError")),
        ..FailureDetail::default()
    })?;
    let log_stderr = log_file.try_clone().map_err(|error| FailureDetail {
        kind: "artifact_write_failed".to_string(),
        command: Some(formatted_command.clone()),
        path: Some(logs_path.to_string_lossy().into_owned()),
        message: Some(error.to_string()),
        expected: Some(json!("Godot logs written to the artifact path")),
        actual: Some(json!("OSError")),
        ..FailureDetail::default()
    })?;
    let mut child_command = Command::new(command.first().cloned().unwrap_or_default());
    apply_joypad_isolation(&mut child_command, joypad_isolation);
    child_command
        .args(command.iter().skip(1))
        .stdout(Stdio::from(log_file))
        .stderr(Stdio::from(log_stderr));
    #[cfg(unix)]
    unsafe {
        child_command.pre_exec(|| {
            if libc::setsid() == -1 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
    configure_background_command(&mut child_command);
    child_command.stdin(Stdio::null());
    let mut child = child_command.spawn().map_err(|error| FailureDetail {
        kind: "godot_launch_failed".to_string(),
        command: Some(formatted_command),
        path: command.first().cloned(),
        message: Some(error.to_string()),
        expected: Some(json!("Godot process starts successfully")),
        actual: Some(json!("OSError")),
        ..FailureDetail::default()
    })?;
    let _project_child = match project_access::register_child(child.id() as i64) {
        Ok(receipt) => receipt,
        Err(error) => {
            let _ = child.kill();
            let _ = child.wait();
            return Err(FailureDetail {
                kind: "project_access_failed".into(),
                message: Some(error.to_string()),
                ..FailureDetail::default()
            });
        }
    };
    let launch = GodotSessionLaunch {
        command,
        pid: child.id() as i64,
        logs_path: logs_path.to_string_lossy().into_owned(),
        display: DisplayLaunchMetadata {
            joypad_isolation,
            ..display
        },
    };
    reap_background_child(child);
    Ok(launch)
}

fn viewport_resolution_arg(viewport: Option<&[i64]>) -> Option<String> {
    let viewport = viewport?;
    let width = *viewport.first()?;
    let height = *viewport.get(1)?;
    if width <= 0 || height <= 0 {
        return None;
    }
    Some(format!("{width}x{height}"))
}

pub(super) fn run_godot_command(
    command: &[String],
    joypad_isolation: bool,
    logs_path: &Path,
    display: DisplayLaunchMetadata,
) -> Result<GodotRunResult, FailureDetail> {
    run_godot_command_with_timeout(command, joypad_isolation, logs_path, display, None)
}

pub(super) fn run_godot_command_with_timeout(
    command: &[String],
    joypad_isolation: bool,
    logs_path: &Path,
    display: DisplayLaunchMetadata,
    timeout: Option<Duration>,
) -> Result<GodotRunResult, FailureDetail> {
    run_godot_command_with_completion_path(
        command,
        joypad_isolation,
        logs_path,
        display,
        timeout,
        None,
    )
}

pub(super) fn run_godot_command_with_completion_path(
    command: &[String],
    joypad_isolation: bool,
    logs_path: &Path,
    display: DisplayLaunchMetadata,
    timeout: Option<Duration>,
    completion_path: Option<&Path>,
) -> Result<GodotRunResult, FailureDetail> {
    run_godot_command_with_timeout_monitor(
        command,
        joypad_isolation,
        logs_path,
        display,
        timeout,
        completion_path,
        None,
    )
}

fn run_godot_command_with_timeout_monitor(
    command: &[String],
    joypad_isolation: bool,
    logs_path: &Path,
    mut display: DisplayLaunchMetadata,
    timeout: Option<Duration>,
    completion_path: Option<&Path>,
    timeout_monitor: Option<&mut dyn FnMut() -> Option<Duration>>,
) -> Result<GodotRunResult, FailureDetail> {
    ensure_log_parent(logs_path, command)?;
    display.joypad_isolation = joypad_isolation;
    match run_owned_child_output_with_timeout_monitor(
        command,
        joypad_isolation,
        timeout,
        completion_path,
        Some(logs_path),
        timeout_monitor,
        None,
    ) {
        Ok(child_output) => {
            let OwnedChildOutput {
                status_code,
                termination,
                process_timing,
                stdout,
                stderr,
                pid,
                process_group,
                mut cleanup,
                timed_out,
                timeout_ms,
                output_truncation,
            } = child_output;
            let stdout = String::from_utf8_lossy(&stdout).into_owned();
            let mut stderr = String::from_utf8_lossy(&stderr).into_owned();
            if let Some(truncation) = &output_truncation {
                stderr.push_str(&format!(
                    "\n[godot-qa output capture truncated; stdout={} bytes, stderr={} bytes, limit={} bytes per stream; complete originals: {} and {}]\n",
                    truncation.stdout_total_bytes,
                    truncation.stderr_total_bytes,
                    truncation.retained_bytes_per_stream,
                    truncation.raw_stdout_path,
                    truncation.raw_stderr_path,
                ));
            }
            display.startup_attempted = true;
            let infrastructure_failure_kind = if timed_out {
                Some("owned_process_timeout".to_string())
            } else {
                classify_display_startup_failure(&display, Some(status_code), &stdout, &stderr)
            };
            if let Some(kind) = infrastructure_failure_kind.clone() {
                display.startup_failure_kind = Some(kind);
                display.startup_failure_retryable = !timed_out;
                display.startup_failure_guidance = Some(if timed_out {
                    "Increase the owned Godot process timeout or inspect partial logs for the active run.".to_string()
                } else {
                    display_startup_guidance(&display)
                });
                if display.virtual_display_used {
                    display.virtual_display_exit_code = Some(status_code);
                }
            }
            fs::write(logs_path, combined_output(&stdout, &stderr)).map_err(|error| {
                FailureDetail {
                    kind: "artifact_write_failed".to_string(),
                    command: Some(format_command(command)),
                    path: Some(logs_path.to_string_lossy().into_owned()),
                    message: Some(error.to_string()),
                    expected: Some(json!("Godot logs written to the artifact path")),
                    actual: Some(json!("OSError")),
                    ..FailureDetail::default()
                }
            })?;
            // Foreground validation must not pass after diagnostics were omitted.
            if completion_path.is_none()
                && status_code == 0
                && !timed_out
                && let Some(truncation) = &output_truncation
            {
                return Err(FailureDetail {
                    kind: "output_capture_truncated".to_string(),
                    command: Some(format_command(command)),
                    path: Some(logs_path.to_string_lossy().into_owned()),
                    expected: Some(json!({"complete_output": true, "max_bytes_per_stream": CAPTURED_STREAM_LIMIT_BYTES})),
                    target: command.windows(2).find(|args| args[0] == "--path").map(|args| args[1].clone()),
                    actual: Some(json!(truncation)),
                    message: Some("Foreground Godot output exceeded the capture limit; inspect the complete raw artifacts, reduce output, and rerun validation.".to_string()),
                    artifacts: Some(serde_json::Map::from_iter([
                        ("logs".to_string(), json!(logs_path)),
                        ("raw_stdout".to_string(), json!(truncation.raw_stdout_path)),
                        ("raw_stderr".to_string(), json!(truncation.raw_stderr_path)),
                    ])),
                    ..FailureDetail::default()
                });
            }
            if status_code == 0
                && !timed_out
                && completion_path.is_some()
                && output_truncation.is_none()
            {
                let _ = fs::remove_file(logs_path.with_extension("stdout.txt"));
                let _ = fs::remove_file(logs_path.with_extension("stderr.txt"));
            }
            Ok(GodotRunResult {
                command: command.to_vec(),
                returncode: status_code,
                termination,
                process_timing: Some(process_timing),
                stdout,
                stderr,
                display,
                pid,
                process_group,
                cleanup: cleanup.take(),
                infrastructure_failure_kind,
                timed_out,
                timeout_ms,
                output_truncation,
            })
        }
        Err(error) => {
            let failure_kind = classify_launch_spawn_failure(&display, command);
            display.startup_attempted = true;
            display.startup_failure_kind = Some(failure_kind.clone());
            display.startup_failure_retryable = failure_kind != "ordinary_launch_failed";
            display.startup_failure_guidance = Some(display_startup_guidance(&display));
            Err(FailureDetail {
                kind: "godot_launch_failed".to_string(),
                command: Some(format_command(command)),
                path: command.first().cloned(),
                message: Some(error.to_string()),
                expected: Some(json!("Godot process starts successfully")),
                actual: Some(json!("OSError")),
                details: [(
                    "launch_infrastructure".to_string(),
                    json!({
                        "failure_kind": failure_kind,
                        "display": display_launch_metadata_value(&display),
                    }),
                )]
                .into_iter()
                .collect(),
                ..FailureDetail::default()
            })
        }
    }
}

#[test]
fn combined_output_preserves_stream_line_boundaries() {
    assert_eq!(
        combined_output("out", "SCRIPT ERROR: fault\n"),
        "out\nSCRIPT ERROR: fault\n"
    );
    assert_eq!(
        combined_output("out\n", "ERROR: fault"),
        "out\nERROR: fault"
    );
    assert_eq!(combined_output("", "WARNING: fault"), "WARNING: fault");
    assert_eq!(combined_output("out", ""), "out");
}

fn combined_output(stdout: &str, stderr: &str) -> String {
    let boundary = if !stdout.is_empty() && !stdout.ends_with('\n') && !stderr.is_empty() {
        "\n"
    } else {
        ""
    };
    format!("{stdout}{boundary}{stderr}")
}

fn godot_log_path(logs_path: &Path) -> std::path::PathBuf {
    logs_path
        .parent()
        .map(|parent| parent.join("godot.log"))
        .unwrap_or_else(|| Path::new("godot.log").to_path_buf())
}

fn ensure_log_parent(logs_path: &Path, command: &[String]) -> Result<(), FailureDetail> {
    if let Some(parent) = logs_path.parent() {
        fs::create_dir_all(parent).map_err(|error| FailureDetail {
            kind: "artifact_write_failed".to_string(),
            command: Some(format_command(command)),
            path: Some(parent.to_string_lossy().into_owned()),
            message: Some(error.to_string()),
            expected: Some(json!("Godot logs written to the artifact path")),
            actual: Some(json!("OSError")),
            ..FailureDetail::default()
        })?;
    }
    Ok(())
}
