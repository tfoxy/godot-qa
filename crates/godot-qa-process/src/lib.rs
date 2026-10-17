#![allow(clippy::result_large_err)]

pub mod android;
pub mod apk_cache;
pub mod directory_link;
#[cfg(windows)]
mod windows;

use godot_qa_core::project_file::read_project_godot_cached;
use godot_qa_core::results::FailureDetail;
use serde_json::json;
use std::env;
use std::fs;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
#[cfg(unix)]
use std::os::unix::process::CommandExt;
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::thread;
use std::time::{Duration, Instant};

const GODOT_VERSION_PROBE_FAILED: &str = "godot_version_probe_failed";
const CAPTURED_STREAM_LIMIT_BYTES: u64 = 8 * 1024 * 1024;

#[derive(Debug, Clone, serde::Serialize)]
pub struct OutputTruncation {
    pub stdout_total_bytes: u64,
    pub stderr_total_bytes: u64,
    pub retained_bytes_per_stream: u64,
    pub raw_stdout_path: String,
    pub raw_stderr_path: String,
}

#[derive(Debug, Clone)]
pub struct GodotRunResult {
    pub command: Vec<String>,
    pub returncode: i32,
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

#[derive(Debug, Clone)]
pub struct DisplayLaunchMetadata {
    pub headless: bool,
    pub selected_mode: String,
    pub platform: String,
    pub linuxbsd_target: bool,
    pub x11_display_available: bool,
    pub wayland_display_available: bool,
    pub selected_display_driver: Option<String>,
    pub project_prefers_wayland: bool,
    pub requested_display_driver: Option<String>,
    pub virtual_display_used: bool,
    pub virtual_display_command: Option<Vec<String>>,
    pub fallback_reason: Option<String>,
    pub launch_prefix_used: bool,
    pub windowed_launch_prefix_used: bool,
    pub effective_command_prefix: Vec<String>,
    pub display_backend_available: bool,
    pub startup_attempted: bool,
    pub virtual_display_exit_code: Option<i32>,
    pub startup_failure_kind: Option<String>,
    pub startup_failure_retryable: bool,
    pub startup_failure_guidance: Option<String>,
}

#[derive(Debug, Clone)]
pub struct ProcessCleanupResult {
    pub status: String,
    pub failure_kind: Option<String>,
    pub pid: Option<i64>,
    pub process_group: Option<i64>,
    pub owned: bool,
    pub message: Option<String>,
}

#[derive(Debug, Clone)]
pub struct GodotVersionProbeResult {
    pub version: String,
    pub metadata: GodotVersionProbeMetadata,
}

#[derive(Debug, Clone)]
pub struct GodotVersionProbeMetadata {
    pub attempts: Vec<GodotVersionProbeAttempt>,
    pub used_launch_prefix: bool,
    pub fallback_used: bool,
    pub fallback_reason: Option<String>,
    pub selected_command: Vec<String>,
    pub failure_type: Option<String>,
}

#[derive(Debug, Clone)]
pub struct GodotVersionProbeAttempt {
    pub command: Vec<String>,
    pub used_launch_prefix: bool,
    pub status: Option<i32>,
    pub stdout: String,
    pub stderr: String,
    pub failure_type: Option<String>,
    pub failure_reason: Option<String>,
}

#[derive(Debug, Clone)]
pub struct SafeProbeResult {
    pub command: Vec<String>,
    pub status: Option<i32>,
    pub stdout: String,
    pub stderr: String,
    pub duration: Duration,
    pub timed_out: bool,
    pub error_kind: Option<String>,
    pub error_message: Option<String>,
}

impl GodotVersionProbeMetadata {
    pub fn to_json(&self) -> serde_json::Value {
        json!({
            "attempts": self.attempts.iter().map(GodotVersionProbeAttempt::to_json).collect::<Vec<_>>(),
            "used_launch_prefix": self.used_launch_prefix,
            "fallback_used": self.fallback_used,
            "fallback_reason": self.fallback_reason,
            "selected_command": self.selected_command,
            "selected_command_line": format_command(&self.selected_command),
            "failure_type": self.failure_type,
        })
    }
}

impl GodotVersionProbeAttempt {
    pub fn to_json(&self) -> serde_json::Value {
        json!({
            "command": self.command,
            "command_line": format_command(&self.command),
            "used_launch_prefix": self.used_launch_prefix,
            "status": self.status,
            "stdout": self.stdout,
            "stderr": self.stderr,
            "failure_type": self.failure_type,
            "failure_reason": self.failure_reason,
        })
    }
}

pub fn resolve_godot_binary(override_binary: Option<&str>) -> Option<String> {
    if let Some(binary) = override_binary {
        return Some(binary.to_string());
    }
    if let Ok(binary) = env::var("GODOT_BIN")
        && !binary.is_empty()
    {
        return Some(binary);
    }
    env::var_os("PATH").and_then(|path_var| {
        env::split_paths(&path_var)
            .flat_map(|entry| executable_candidates(&entry, "godot"))
            .find(|candidate| candidate.is_file())
            .map(|candidate| candidate.to_string_lossy().into_owned())
    })
}

/// Native executable candidates; explicit versioned Godot paths remain supported by
/// --godot-bin / GODOT_BIN. Do not execute batch files through an implicit shell.
pub fn executable_candidates(directory: &Path, name: &str) -> Vec<std::path::PathBuf> {
    if cfg!(windows) {
        vec![directory.join(format!("{name}.exe")), directory.join(name)]
    } else {
        vec![directory.join(name)]
    }
}

/// Detach background helpers from the launching console while keeping redirected I/O.
pub fn configure_background_command(command: &mut Command) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        use windows_sys::Win32::System::Threading::{CREATE_NEW_PROCESS_GROUP, DETACHED_PROCESS};
        command.creation_flags(CREATE_NEW_PROCESS_GROUP | DETACHED_PROCESS);
    }
    #[cfg(not(windows))]
    let _ = command;
}

/// Windows creation time is an opaque process identity, persisted with session metadata.
pub fn process_identity(pid: i64) -> Option<u64> {
    #[cfg(windows)]
    {
        windows::identity(pid)
    }
    #[cfg(not(windows))]
    {
        let _ = pid;
        None
    }
}

pub fn process_identity_matches(pid: i64, expected: Option<u64>) -> bool {
    pid_is_running(pid)
        && expected
            .is_none_or(|expected| process_identity(pid).is_none_or(|actual| actual == expected))
}

pub fn probe_godot_version(
    binary: &str,
    launch_prefix: &[String],
) -> Result<String, FailureDetail> {
    probe_godot_version_with_metadata(binary, launch_prefix).map(|result| result.version)
}

pub fn probe_godot_version_with_metadata(
    binary: &str,
    launch_prefix: &[String],
) -> Result<GodotVersionProbeResult, FailureDetail> {
    let prefixed_command =
        effective_godot_command(launch_prefix, binary, &["--version".to_string()]);
    let direct_command = effective_godot_command(&[], binary, &["--version".to_string()]);
    let expected = "exit status 0 with version output";
    let mut attempts = Vec::new();

    let first = run_version_probe_attempt(&prefixed_command, !launch_prefix.is_empty());
    if let Some(version) = successful_version(&first) {
        let metadata = GodotVersionProbeMetadata {
            attempts: vec![first],
            used_launch_prefix: !launch_prefix.is_empty(),
            fallback_used: false,
            fallback_reason: None,
            selected_command: prefixed_command,
            failure_type: None,
        };
        return Ok(GodotVersionProbeResult { version, metadata });
    }

    let fallback_reason = first.failure_reason.clone();
    attempts.push(first);
    if !launch_prefix.is_empty() {
        let direct = run_version_probe_attempt(&direct_command, false);
        if let Some(version) = successful_version(&direct) {
            attempts.push(direct);
            let metadata = GodotVersionProbeMetadata {
                attempts,
                used_launch_prefix: false,
                fallback_used: true,
                fallback_reason,
                selected_command: direct_command,
                failure_type: None,
            };
            return Ok(GodotVersionProbeResult { version, metadata });
        }
        attempts.push(direct);
    }

    let final_attempt = attempts
        .last()
        .expect("at least one version probe attempt must be recorded")
        .clone();
    Err(FailureDetail {
        kind: GODOT_VERSION_PROBE_FAILED.to_string(),
        command: Some(format_command(&final_attempt.command)),
        path: final_attempt.command.first().cloned(),
        expected: Some(json!(expected)),
        actual: Some(json!(
            final_attempt
                .failure_reason
                .clone()
                .unwrap_or_else(|| "probe_failed".to_string())
        )),
        message: final_attempt.failure_reason.clone(),
        details: [(
            "version_probe".to_string(),
            GodotVersionProbeMetadata {
                attempts,
                used_launch_prefix: false,
                fallback_used: !launch_prefix.is_empty(),
                fallback_reason,
                selected_command: final_attempt.command.clone(),
                failure_type: Some(GODOT_VERSION_PROBE_FAILED.to_string()),
            }
            .to_json(),
        )]
        .into_iter()
        .collect(),
        ..FailureDetail::default()
    })
}

fn run_version_probe_attempt(
    command: &[String],
    used_launch_prefix: bool,
) -> GodotVersionProbeAttempt {
    match run_command_output(command) {
        Ok(output) => {
            let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
            let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
            let version = if !stdout.is_empty() {
                stdout.clone()
            } else {
                stderr.clone()
            };
            let (failure_type, failure_reason) = if !output.status.success() {
                (
                    Some(GODOT_VERSION_PROBE_FAILED.to_string()),
                    Some(if version.is_empty() {
                        format!("exited with status {}", output.status.code().unwrap_or(-1))
                    } else {
                        version
                    }),
                )
            } else if version.is_empty() {
                (
                    Some(GODOT_VERSION_PROBE_FAILED.to_string()),
                    Some("empty_output".to_string()),
                )
            } else {
                (None, None)
            };
            GodotVersionProbeAttempt {
                command: command.to_vec(),
                used_launch_prefix,
                status: Some(output.status.code().unwrap_or(-1)),
                stdout,
                stderr,
                failure_type,
                failure_reason,
            }
        }
        Err(error) => GodotVersionProbeAttempt {
            command: command.to_vec(),
            used_launch_prefix,
            status: None,
            stdout: String::new(),
            stderr: String::new(),
            failure_type: Some(GODOT_VERSION_PROBE_FAILED.to_string()),
            failure_reason: Some(format!("os_error: {error}")),
        },
    }
}

fn successful_version(attempt: &GodotVersionProbeAttempt) -> Option<String> {
    if attempt.failure_type.is_some() {
        return None;
    }
    let version = if !attempt.stdout.is_empty() {
        attempt.stdout.clone()
    } else {
        attempt.stderr.clone()
    };
    (!version.is_empty()).then_some(version)
}

pub fn format_command(command: &[String]) -> String {
    command
        .iter()
        .map(|part| shell_quote(part))
        .collect::<Vec<_>>()
        .join(" ")
}

pub fn run_safe_probe(executable: &Path, args: &[String], timeout: Duration) -> SafeProbeResult {
    let started = Instant::now();
    let mut command = Vec::with_capacity(1 + args.len());
    command.push(executable.to_string_lossy().into_owned());
    command.extend(args.iter().cloned());

    let mut child_command = Command::new(executable);
    child_command
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(unix)]
    unsafe {
        child_command.pre_exec(|| {
            if libc::setsid() == -1 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }

    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        child_command.creation_flags(windows_sys::Win32::System::Threading::CREATE_SUSPENDED);
    }
    let mut child = match child_command.spawn() {
        Ok(child) => child,
        Err(error) => {
            return SafeProbeResult {
                command,
                status: None,
                stdout: String::new(),
                stderr: String::new(),
                duration: started.elapsed(),
                timed_out: false,
                error_kind: Some(format!("{:?}", error.kind())),
                error_message: Some(error.to_string()),
            };
        }
    };

    #[cfg(windows)]
    let owned_job = match windows::OwnedJob::attach(&child) {
        Ok(job) => job,
        Err(error) => {
            let _ = child.kill();
            let _ = child.wait();
            return SafeProbeResult {
                command,
                status: None,
                stdout: String::new(),
                stderr: String::new(),
                duration: started.elapsed(),
                timed_out: false,
                error_kind: Some(format!("{:?}", error.kind())),
                error_message: Some(error.to_string()),
            };
        }
    };
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                #[cfg(windows)]
                let _ = owned_job.terminate();
                return match child.wait_with_output() {
                    Ok(output) => SafeProbeResult {
                        command,
                        status: output.status.code().or_else(|| status.code()),
                        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
                        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
                        duration: started.elapsed(),
                        timed_out: false,
                        error_kind: None,
                        error_message: None,
                    },
                    Err(error) => SafeProbeResult {
                        command,
                        status: None,
                        stdout: String::new(),
                        stderr: String::new(),
                        duration: started.elapsed(),
                        timed_out: false,
                        error_kind: Some(format!("{:?}", error.kind())),
                        error_message: Some(error.to_string()),
                    },
                };
            }
            Ok(None) if started.elapsed() >= timeout => {
                #[cfg(windows)]
                let _ = owned_job.terminate();
                let kill_error = kill_safe_probe_child(&mut child);
                return match child.wait_with_output() {
                    Ok(output) => SafeProbeResult {
                        command,
                        status: output.status.code(),
                        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
                        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
                        duration: started.elapsed(),
                        timed_out: true,
                        error_kind: Some("timeout".to_string()),
                        error_message: kill_error
                            .map(|error| format!("timed out; kill failed: {error}"))
                            .or_else(|| Some("timed out".to_string())),
                    },
                    Err(error) => SafeProbeResult {
                        command,
                        status: None,
                        stdout: String::new(),
                        stderr: String::new(),
                        duration: started.elapsed(),
                        timed_out: true,
                        error_kind: Some("timeout".to_string()),
                        error_message: Some(format!("timed out; cleanup failed: {error}")),
                    },
                };
            }
            Ok(None) => {
                thread::sleep(Duration::from_millis(10));
            }
            Err(error) => {
                let _ = kill_safe_probe_child(&mut child);
                let _ = child.wait();
                return SafeProbeResult {
                    command,
                    status: None,
                    stdout: String::new(),
                    stderr: String::new(),
                    duration: started.elapsed(),
                    timed_out: false,
                    error_kind: Some(format!("{:?}", error.kind())),
                    error_message: Some(error.to_string()),
                };
            }
        }
    }
}

fn kill_safe_probe_child(child: &mut std::process::Child) -> Option<std::io::Error> {
    #[cfg(unix)]
    {
        send_process_group_signal(child.id() as i64, libc::SIGKILL)
            .err()
            .map(std::io::Error::other)
    }
    #[cfg(not(unix))]
    {
        child.kill().err()
    }
}

fn shell_quote(part: &str) -> String {
    if part.is_empty() {
        return "''".to_string();
    }
    if part
        .bytes()
        .all(|byte| matches!(byte, b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'_' | b'/' | b'.' | b'-' | b'=' | b':'))
    {
        return part.to_string();
    }
    format!("'{}'", part.replace('\'', "'\"'\"'"))
}

pub fn migration_marker() -> &'static str {
    "m29"
}

/// Sleep schedule between process-exit poll attempts, mirroring
/// `startup_probe_backoff` in the runtime transport: start fine-grained so a
/// process that exits promptly is observed within ~5ms instead of the old flat
/// 100ms poll (which added up to ~100ms of dead time to every stop), doubling
/// back to the same 100ms pace for processes that linger.
fn exit_poll_backoff(previous: Duration) -> Duration {
    (previous * 2).min(Duration::from_millis(100))
}

pub fn wait_for_godot_process_exit(pid: i64, wait_timeout: f64) -> bool {
    if pid <= 0 {
        return false;
    }
    let deadline = Instant::now() + Duration::from_secs_f64(wait_timeout);
    let mut backoff = Duration::from_millis(5);
    while Instant::now() < deadline {
        if !pid_is_running(pid) || is_zombie_process(pid) {
            return true;
        }
        thread::sleep(backoff);
        backoff = exit_poll_backoff(backoff);
    }
    !pid_is_running(pid) || is_zombie_process(pid)
}

pub fn stop_godot_process(pid: i64, wait_timeout: f64) -> Result<String, FailureDetail> {
    stop_godot_process_with_cleanup(pid, wait_timeout).map(|cleanup| cleanup.status)
}

pub fn stop_godot_process_with_identity(
    pid: i64,
    wait_timeout: f64,
    identity: Option<u64>,
) -> Result<String, FailureDetail> {
    if cfg!(windows) && identity.is_none() && pid_is_running(pid) {
        return Err(FailureDetail {
            kind: "godot_stop_failed".into(),
            command: Some(format!("session stop (pid={pid})")),
            message: Some("saved session lacks a Windows process identity; stop through its authenticated bridge or restart the session before using process fallback".into()),
            expected: Some(json!("saved process creation identity")),
            actual: Some(json!("missing")),
            ..FailureDetail::default()
        });
    }
    stop_process_with_identity(pid, wait_timeout, identity).map(|cleanup| cleanup.status)
}

pub fn stop_godot_process_with_cleanup(
    pid: i64,
    wait_timeout: f64,
) -> Result<ProcessCleanupResult, FailureDetail> {
    stop_process_with_identity(pid, wait_timeout, None)
}

fn stop_process_with_identity(
    pid: i64,
    wait_timeout: f64,
    identity: Option<u64>,
) -> Result<ProcessCleanupResult, FailureDetail> {
    if pid <= 0 {
        return Err(FailureDetail {
            kind: "godot_stop_failed".to_string(),
            command: Some(format!("session stop (pid={pid})")),
            message: Some("live session pid was invalid".to_string()),
            expected: Some(json!("positive live session pid")),
            actual: Some(json!(pid)),
            ..FailureDetail::default()
        });
    }
    let process_group = current_run_process_group(pid);
    terminate_process_tree(pid, false, identity).map_err(|error| FailureDetail {
        kind: "godot_stop_failed".to_string(),
        command: Some(format!("session stop (pid={pid})")),
        message: Some(error),
        expected: Some(json!("process accepts termination signal")),
        actual: Some(json!("OSError")),
        ..FailureDetail::default()
    })?;
    if wait_for_godot_process_exit(pid, wait_timeout) {
        return Ok(ProcessCleanupResult {
            status: "terminated".to_string(),
            failure_kind: None,
            pid: Some(pid),
            process_group,
            owned: true,
            message: None,
        });
    }
    terminate_process_tree(pid, true, identity).map_err(|error| FailureDetail {
        kind: "godot_stop_failed".to_string(),
        command: Some(format!("session stop (pid={pid})")),
        message: Some(error),
        expected: Some(json!(
            "process accepts kill signal after termination timeout"
        )),
        actual: Some(json!("OSError")),
        ..FailureDetail::default()
    })?;
    if wait_for_godot_process_exit(pid, wait_timeout) {
        return Ok(ProcessCleanupResult {
            status: "killed".to_string(),
            failure_kind: None,
            pid: Some(pid),
            process_group,
            owned: true,
            message: None,
        });
    }
    Ok(ProcessCleanupResult {
        status: "cleanup_failed".to_string(),
        failure_kind: Some("stale_process_cleanup_failed".to_string()),
        pid: Some(pid),
        process_group,
        owned: true,
        message: Some("process remained alive after terminate/kill fallback".to_string()),
    })
}

pub fn launch_godot_import(
    binary: &str,
    launch_prefix: &[String],
    project_root: &Path,
    logs_path: &Path,
) -> Result<GodotRunResult, FailureDetail> {
    launch_godot_import_with_args(binary, launch_prefix, &[], project_root, logs_path)
}

pub fn launch_godot_import_with_args(
    binary: &str,
    launch_prefix: &[String],
    default_godot_args: &[String],
    project_root: &Path,
    logs_path: &Path,
) -> Result<GodotRunResult, FailureDetail> {
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
    run_godot_command(&command, logs_path, display)
}

pub fn launch_godot_check(
    binary: &str,
    launch_prefix: &[String],
    default_godot_args: &[String],
    default_project_args: &[String],
    project_root: &Path,
    request_path: &Path,
    logs_path: &Path,
) -> Result<GodotRunResult, FailureDetail> {
    launch_godot_check_with_timeout(
        binary,
        launch_prefix,
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
    default_godot_args: &[String],
    default_project_args: &[String],
    project_root: &Path,
    request_path: &Path,
    logs_path: &Path,
    timeout: Option<Duration>,
    fixed_fps: Option<i64>,
) -> Result<GodotRunResult, FailureDetail> {
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
    run_godot_command_with_timeout(&command, logs_path, display, timeout)
}

pub fn launch_godot_one_shot(
    options: GodotOneShotLaunch<'_>,
) -> Result<GodotRunResult, FailureDetail> {
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
        options.logs_path,
        display,
        options.timeout,
        result_path.as_deref(),
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

pub fn resolve_project_display_launch_metadata(
    project_root: &Path,
    launch_prefix: &[String],
    headless: bool,
) -> DisplayLaunchMetadata {
    build_project_display_launch_metadata(project_root, headless, launch_prefix)
}

pub fn resolve_project_display_launch_metadata_with_windowed_prefix(
    project_root: &Path,
    launch_prefix: &[String],
    windowed_launch_prefix: &[String],
    headless: bool,
) -> DisplayLaunchMetadata {
    build_project_display_launch_metadata_with_windowed_prefix(
        project_root,
        headless,
        launch_prefix,
        windowed_launch_prefix,
    )
}

pub fn launch_godot_patch_runner(
    binary: &str,
    launch_prefix: &[String],
    default_godot_args: &[String],
    project_root: &Path,
    request_path: &Path,
    logs_path: &Path,
) -> Result<GodotRunResult, FailureDetail> {
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
    run_godot_command(&command, logs_path, display)
}

#[allow(clippy::too_many_arguments)]
pub fn launch_godot_session(
    binary: &str,
    launch_prefix: &[String],
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
    let child = child_command.spawn().map_err(|error| FailureDetail {
        kind: "godot_launch_failed".to_string(),
        command: Some(formatted_command),
        path: command.first().cloned(),
        message: Some(error.to_string()),
        expected: Some(json!("Godot process starts successfully")),
        actual: Some(json!("OSError")),
        ..FailureDetail::default()
    })?;
    Ok(GodotSessionLaunch {
        command,
        pid: child.id() as i64,
        logs_path: logs_path.to_string_lossy().into_owned(),
        display,
    })
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

fn append_project_display_driver_args(
    command: &mut Vec<String>,
    project_root: &Path,
    launch_prefix: &[String],
    windowed_launch_prefix: &[String],
    headless: bool,
) -> DisplayLaunchMetadata {
    append_project_display_driver_args_with_env(
        command,
        project_root,
        launch_prefix,
        windowed_launch_prefix,
        headless,
        env_flag_available("DISPLAY"),
        env_flag_available("WAYLAND_DISPLAY"),
        env::var_os("PATH"),
    )
}

#[allow(clippy::too_many_arguments)]
fn append_project_display_driver_args_with_env(
    command: &mut Vec<String>,
    project_root: &Path,
    launch_prefix: &[String],
    windowed_launch_prefix: &[String],
    headless: bool,
    x11_display_available: bool,
    wayland_display_available: bool,
    path: Option<std::ffi::OsString>,
) -> DisplayLaunchMetadata {
    let display = build_project_display_launch_metadata_with_env(
        project_root,
        headless,
        launch_prefix,
        windowed_launch_prefix,
        x11_display_available,
        wayland_display_available,
        path,
    );
    if let Some(display_driver) = display.requested_display_driver.as_deref() {
        command.push("--display-driver".to_string());
        command.push(display_driver.to_string());
    }
    display
}

fn launch_prefix_uses_virtual_x11_display(launch_prefix: &[String]) -> bool {
    launch_prefix.iter().any(|entry| {
        let entry = entry.trim();
        Path::new(entry)
            .file_stem()
            .is_some_and(|name| name == "xvfb-run")
    })
}

fn build_project_display_launch_metadata(
    project_root: &Path,
    headless: bool,
    launch_prefix: &[String],
) -> DisplayLaunchMetadata {
    build_project_display_launch_metadata_with_windowed_prefix(
        project_root,
        headless,
        launch_prefix,
        &[],
    )
}

fn build_project_display_launch_metadata_with_windowed_prefix(
    project_root: &Path,
    headless: bool,
    launch_prefix: &[String],
    windowed_launch_prefix: &[String],
) -> DisplayLaunchMetadata {
    build_project_display_launch_metadata_with_env(
        project_root,
        headless,
        launch_prefix,
        windowed_launch_prefix,
        env_flag_available("DISPLAY"),
        env_flag_available("WAYLAND_DISPLAY"),
        env::var_os("PATH"),
    )
}

fn build_project_display_launch_metadata_with_env(
    project_root: &Path,
    headless: bool,
    launch_prefix: &[String],
    windowed_launch_prefix: &[String],
    x11_display_available: bool,
    wayland_display_available: bool,
    path: Option<std::ffi::OsString>,
) -> DisplayLaunchMetadata {
    let project_prefers_wayland = project_wayland_preference_enabled(project_root);
    let linuxbsd_target = linuxbsd_target();
    let launch_prefix_used = !launch_prefix.is_empty();
    let windowed_launch_prefix_used = !headless && !windowed_launch_prefix.is_empty();
    let configured_virtual_display = launch_prefix_uses_virtual_x11_display(launch_prefix)
        || (!headless && launch_prefix_uses_virtual_x11_display(windowed_launch_prefix));
    let wayland_usable = wayland_display_available && !configured_virtual_display;
    let selected_mode = if headless { "headless" } else { "windowed" }.to_string();
    let auto_virtual_display_command = (!headless
        && linuxbsd_target
        && !x11_display_available
        && !wayland_usable
        && !windowed_launch_prefix_used
        && !configured_virtual_display)
        .then(|| detect_xvfb_run(path.as_deref()))
        .flatten();
    let virtual_display_used = configured_virtual_display || auto_virtual_display_command.is_some();
    let mut effective_command_prefix = Vec::new();
    effective_command_prefix.extend(launch_prefix.iter().cloned());
    if !headless {
        effective_command_prefix.extend(windowed_launch_prefix.iter().cloned());
    }
    if let Some(virtual_display_command) = &auto_virtual_display_command {
        effective_command_prefix.extend(virtual_display_command.iter().cloned());
    }
    let requested_display_driver = if !headless && virtual_display_used && linuxbsd_target {
        Some("x11".to_string())
    } else if !headless && wayland_usable && linuxbsd_target && project_prefers_wayland {
        Some("wayland".to_string())
    } else {
        None
    };
    let selected_display_driver = if headless {
        None
    } else if cfg!(windows) {
        Some("windows".to_string())
    } else if requested_display_driver.as_deref() == Some("x11") {
        Some("x11".to_string())
    } else if requested_display_driver.as_deref() == Some("wayland") {
        Some("wayland".to_string())
    } else if x11_display_available || virtual_display_used {
        Some("x11".to_string())
    } else {
        None
    };
    let display_backend_available =
        headless || !linuxbsd_target || selected_display_driver.is_some() || wayland_usable;
    let fallback_reason = if headless {
        None
    } else if configured_virtual_display {
        Some(
            if !headless && launch_prefix_uses_virtual_x11_display(windowed_launch_prefix) {
                "configured_windowed_launch_prefix"
            } else {
                "configured_launch_prefix"
            }
            .to_string(),
        )
    } else if auto_virtual_display_command.is_some() {
        Some("auto_detected_virtual_display".to_string())
    } else if !display_backend_available {
        Some("display_backend_unavailable".to_string())
    } else if project_prefers_wayland
        && !wayland_usable
        && selected_display_driver.as_deref() == Some("x11")
    {
        Some("wayland_unavailable_using_x11".to_string())
    } else {
        None
    };
    DisplayLaunchMetadata {
        headless,
        selected_mode,
        platform: env::consts::OS.to_string(),
        linuxbsd_target,
        x11_display_available,
        wayland_display_available: wayland_usable,
        selected_display_driver,
        project_prefers_wayland,
        requested_display_driver,
        virtual_display_used,
        virtual_display_command: auto_virtual_display_command,
        fallback_reason,
        launch_prefix_used,
        windowed_launch_prefix_used,
        effective_command_prefix,
        display_backend_available,
        startup_attempted: false,
        virtual_display_exit_code: None,
        startup_failure_kind: None,
        startup_failure_retryable: false,
        startup_failure_guidance: None,
    }
}

#[cfg(test)]
fn project_prefers_wayland(
    project_root: &Path,
    headless: bool,
    wayland_display_available: bool,
) -> bool {
    !headless
        && wayland_display_available
        && linuxbsd_target()
        && project_wayland_preference_enabled(project_root)
}

fn project_wayland_preference_enabled(project_root: &Path) -> bool {
    let Some(project) = read_project_godot_cached(&project_root.join("project.godot")) else {
        return false;
    };
    project.lines().any(|line| {
        let trimmed = line.trim();
        let Some(value) = trimmed.strip_prefix("run/platforms/linuxbsd/prefer_wayland=") else {
            return false;
        };
        value.trim().trim_matches('"') == "true"
    })
}

fn env_flag_available(name: &str) -> bool {
    env::var_os(name).is_some_and(|value| !value.is_empty())
}

fn detect_xvfb_run(path: Option<&std::ffi::OsStr>) -> Option<Vec<String>> {
    let path = path?;
    env::split_paths(path)
        .map(|entry| entry.join("xvfb-run"))
        .find(|candidate| candidate.is_file())
        .map(|candidate| vec![candidate.to_string_lossy().into_owned(), "-a".to_string()])
}

#[cfg(all(test, unix))]
fn effective_launch_prefix(
    launch_prefix: &[String],
    windowed_launch_prefix: &[String],
    display: &DisplayLaunchMetadata,
) -> Vec<String> {
    let mut prefix = Vec::new();
    prefix.extend(launch_prefix.iter().cloned());
    if !display.headless {
        prefix.extend(windowed_launch_prefix.iter().cloned());
    }
    if let Some(virtual_display_command) = &display.virtual_display_command {
        prefix.extend(virtual_display_command.iter().cloned());
    }
    prefix
}

fn linuxbsd_target() -> bool {
    cfg!(any(
        target_os = "linux",
        target_os = "freebsd",
        target_os = "netbsd",
        target_os = "openbsd",
        target_os = "dragonfly"
    ))
}

fn run_godot_command(
    command: &[String],
    logs_path: &Path,
    display: DisplayLaunchMetadata,
) -> Result<GodotRunResult, FailureDetail> {
    run_godot_command_with_timeout(command, logs_path, display, None)
}

fn run_godot_command_with_timeout(
    command: &[String],
    logs_path: &Path,
    display: DisplayLaunchMetadata,
    timeout: Option<Duration>,
) -> Result<GodotRunResult, FailureDetail> {
    run_godot_command_with_completion_path(command, logs_path, display, timeout, None)
}

fn run_godot_command_with_completion_path(
    command: &[String],
    logs_path: &Path,
    mut display: DisplayLaunchMetadata,
    timeout: Option<Duration>,
    completion_path: Option<&Path>,
) -> Result<GodotRunResult, FailureDetail> {
    ensure_log_parent(logs_path, command)?;
    match run_owned_child_output(
        command,
        timeout,
        completion_path,
        completion_path.map(|_| logs_path),
    ) {
        Ok(child_output) => {
            let OwnedChildOutput {
                status_code,
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
            fs::write(logs_path, format!("{stdout}{stderr}")).map_err(|error| FailureDetail {
                kind: "artifact_write_failed".to_string(),
                command: Some(format_command(command)),
                path: Some(logs_path.to_string_lossy().into_owned()),
                message: Some(error.to_string()),
                expected: Some(json!("Godot logs written to the artifact path")),
                actual: Some(json!("OSError")),
                ..FailureDetail::default()
            })?;
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

fn effective_godot_command(launch_prefix: &[String], binary: &str, args: &[String]) -> Vec<String> {
    let mut command = Vec::with_capacity(launch_prefix.len() + 1 + args.len());
    command.extend(launch_prefix.iter().cloned());
    command.push(binary.to_string());
    command.extend(args.iter().cloned());
    command
}

struct OwnedChildOutput {
    status_code: i32,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
    pid: Option<i64>,
    process_group: Option<i64>,
    cleanup: Option<ProcessCleanupResult>,
    timed_out: bool,
    timeout_ms: Option<u64>,
    output_truncation: Option<OutputTruncation>,
}

fn run_owned_child_output(
    command: &[String],
    timeout: Option<Duration>,
    completion_path: Option<&Path>,
    file_capture_path: Option<&Path>,
) -> std::io::Result<OwnedChildOutput> {
    let Some(executable) = command.first() else {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "empty command",
        ));
    };
    let mut child_command = Command::new(executable);
    child_command.args(command.iter().skip(1));
    let file_paths = if let Some(path) = file_capture_path {
        let stdout_path = path.with_extension("stdout.txt");
        let stderr_path = path.with_extension("stderr.txt");
        child_command.stdout(Stdio::from(File::create(&stdout_path)?));
        child_command.stderr(Stdio::from(File::create(&stderr_path)?));
        Some((stdout_path, stderr_path))
    } else {
        child_command.stdout(Stdio::piped()).stderr(Stdio::piped());
        None
    };
    #[cfg(unix)]
    unsafe {
        child_command.pre_exec(|| {
            if libc::setsid() == -1 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        child_command.creation_flags(windows_sys::Win32::System::Threading::CREATE_SUSPENDED);
    }
    let mut child = child_command.spawn()?;
    #[cfg(windows)]
    let owned_job = match windows::OwnedJob::attach(&child) {
        Ok(job) => job,
        Err(error) => {
            let _ = child.kill();
            let _ = child.wait();
            return Err(error);
        }
    };
    let pid = child.id() as i64;
    let process_group = current_run_process_group(pid);
    if !cfg!(windows) && timeout.is_none() && completion_path.is_none() {
        let output = child.wait_with_output()?;
        let (stdout, stderr) = if let Some((stdout_path, stderr_path)) = &file_paths {
            (fs::read(stdout_path)?, fs::read(stderr_path)?)
        } else {
            (output.stdout, output.stderr)
        };
        return Ok(OwnedChildOutput {
            status_code: output.status.code().unwrap_or(-1),
            stdout,
            stderr,
            pid: Some(pid),
            process_group,
            cleanup: Some(ProcessCleanupResult {
                status: "exited".to_string(),
                failure_kind: None,
                pid: Some(pid),
                process_group,
                owned: true,
                message: None,
            }),
            timed_out: false,
            timeout_ms: None,
            output_truncation: None,
        });
    }
    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    let stdout_reader = stdout.map(|mut stdout| {
        let (sender, receiver) = mpsc::channel();
        thread::spawn(move || {
            let mut bytes = Vec::new();
            let result = stdout.read_to_end(&mut bytes);
            let _ = sender.send((bytes, result));
        });
        receiver
    });
    let stderr_reader = stderr.map(|mut stderr| {
        let (sender, receiver) = mpsc::channel();
        thread::spawn(move || {
            let mut bytes = Vec::new();
            let result = stderr.read_to_end(&mut bytes);
            let _ = sender.send((bytes, result));
        });
        receiver
    });
    let deadline = timeout.map(|timeout| Instant::now() + timeout);
    let mut completion_deadline = None;
    loop {
        if let Some(status) = child.try_wait()? {
            #[cfg(windows)]
            owned_job.terminate()?;
            let (stdout, stderr, output_truncation) =
                collected_child_output(&file_paths, stdout_reader, stderr_reader)?;
            return Ok(OwnedChildOutput {
                status_code: status.code().unwrap_or(-1),
                stdout,
                stderr,
                pid: Some(pid),
                process_group,
                cleanup: Some(ProcessCleanupResult {
                    status: "exited".to_string(),
                    failure_kind: None,
                    pid: Some(pid),
                    process_group,
                    owned: true,
                    message: None,
                }),
                timed_out: false,
                timeout_ms: timeout.map(duration_millis_u64),
                output_truncation,
            });
        }
        if completion_deadline.is_none()
            && deadline.is_none_or(|deadline| Instant::now() < deadline)
            && completion_path.is_some_and(completion_matches_current_request)
        {
            completion_deadline = Some(Instant::now() + Duration::from_secs(3));
        }
        // Once a current result has been written, allow the complete exit grace.
        // The declared deadline remains hard until that result exists.
        let overall_expired = completion_deadline.is_none()
            && deadline.is_some_and(|deadline| Instant::now() >= deadline);
        let completion_expired =
            completion_deadline.is_some_and(|deadline| Instant::now() >= deadline);
        if overall_expired || completion_expired {
            #[cfg(windows)]
            owned_job.terminate()?;
            let cleanup = terminate_owned_child_after_timeout(&mut child, pid, process_group);
            let _ = child.wait();
            let (stdout, stderr, output_truncation) =
                collected_child_output(&file_paths, stdout_reader, stderr_reader)?;
            return Ok(OwnedChildOutput {
                status_code: -1,
                stdout,
                stderr,
                pid: Some(pid),
                process_group,
                cleanup: Some(cleanup),
                timed_out: true,
                timeout_ms: Some(if completion_expired {
                    3000
                } else {
                    duration_millis_u64(timeout.unwrap_or(Duration::from_secs(3)))
                }),
                output_truncation,
            });
        }
        thread::sleep(Duration::from_millis(10));
    }
}

fn completion_matches_current_request(result_path: &Path) -> bool {
    let Some(parent) = result_path.parent() else {
        return false;
    };
    let Ok(request_bytes) = fs::read(parent.join("request.json")) else {
        return false;
    };
    let Ok(result_bytes) = fs::read(result_path) else {
        return false;
    };
    let Ok(request) = serde_json::from_slice::<serde_json::Value>(&request_bytes) else {
        return false;
    };
    let Ok(result) = serde_json::from_slice::<serde_json::Value>(&result_bytes) else {
        return false;
    };
    let Some(run) = request.get("run") else {
        return false;
    };
    let Some(identity) = result.get("run_identity") else {
        return false;
    };
    let nonce = run.get("nonce").and_then(serde_json::Value::as_str);
    let run_id = run.get("id").and_then(serde_json::Value::as_str);
    nonce.is_some()
        && run_id.is_some()
        && identity.get("nonce").and_then(serde_json::Value::as_str) == nonce
        && identity.get("run_id").and_then(serde_json::Value::as_str) == run_id
        && result
            .get("ok")
            .and_then(serde_json::Value::as_bool)
            .is_some()
}

fn duration_millis_u64(duration: Duration) -> u64 {
    u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
}

fn join_owned_child_output(
    stdout_reader: Option<Receiver<(Vec<u8>, std::io::Result<usize>)>>,
    stderr_reader: Option<Receiver<(Vec<u8>, std::io::Result<usize>)>>,
) -> std::io::Result<(Vec<u8>, Vec<u8>)> {
    let stdout = join_output_reader(stdout_reader)?;
    let stderr = join_output_reader(stderr_reader)?;
    Ok((stdout, stderr))
}

fn collected_child_output(
    file_paths: &Option<(std::path::PathBuf, std::path::PathBuf)>,
    stdout_reader: Option<Receiver<(Vec<u8>, std::io::Result<usize>)>>,
    stderr_reader: Option<Receiver<(Vec<u8>, std::io::Result<usize>)>>,
) -> std::io::Result<(Vec<u8>, Vec<u8>, Option<OutputTruncation>)> {
    if let Some((stdout_path, stderr_path)) = file_paths {
        let (stdout, stdout_total) = read_bounded_capture(stdout_path)?;
        let (stderr, stderr_total) = read_bounded_capture(stderr_path)?;
        let truncation = (stdout_total > CAPTURED_STREAM_LIMIT_BYTES
            || stderr_total > CAPTURED_STREAM_LIMIT_BYTES)
            .then(|| OutputTruncation {
                stdout_total_bytes: stdout_total,
                stderr_total_bytes: stderr_total,
                retained_bytes_per_stream: CAPTURED_STREAM_LIMIT_BYTES,
                raw_stdout_path: stdout_path.to_string_lossy().into_owned(),
                raw_stderr_path: stderr_path.to_string_lossy().into_owned(),
            });
        return Ok((stdout, stderr, truncation));
    }
    let (stdout, stderr) = join_owned_child_output(stdout_reader, stderr_reader)?;
    Ok((stdout, stderr, None))
}

fn read_bounded_capture(path: &Path) -> std::io::Result<(Vec<u8>, u64)> {
    let mut file = File::open(path)?;
    let total = file.metadata()?.len();
    if total <= CAPTURED_STREAM_LIMIT_BYTES {
        let mut bytes = Vec::with_capacity(total as usize);
        (&mut file)
            .take(CAPTURED_STREAM_LIMIT_BYTES + 1)
            .read_to_end(&mut bytes)?;
        return Ok((bytes, total));
    }
    let half = CAPTURED_STREAM_LIMIT_BYTES / 2;
    let mut bytes = Vec::with_capacity(CAPTURED_STREAM_LIMIT_BYTES as usize + 128);
    (&mut file).take(half).read_to_end(&mut bytes)?;
    bytes.extend_from_slice(
        format!(
            "\n[godot-qa omitted {} middle bytes; raw capture at {}]\n",
            total - CAPTURED_STREAM_LIMIT_BYTES,
            path.display()
        )
        .as_bytes(),
    );
    file.seek(SeekFrom::End(-(half as i64)))?;
    file.take(half).read_to_end(&mut bytes)?;
    Ok((bytes, total))
}

fn join_output_reader(
    reader: Option<Receiver<(Vec<u8>, std::io::Result<usize>)>>,
) -> std::io::Result<Vec<u8>> {
    let Some(reader) = reader else {
        return Ok(Vec::new());
    };
    let (bytes, result) = reader.recv_timeout(Duration::from_secs(3)).map_err(|_| {
        std::io::Error::new(
            std::io::ErrorKind::TimedOut,
            "owned child output reader did not reach EOF",
        )
    })?;
    result.map(|_| bytes)
}

fn terminate_owned_child_after_timeout(
    child: &mut std::process::Child,
    pid: i64,
    process_group: Option<i64>,
) -> ProcessCleanupResult {
    #[cfg(unix)]
    let _ = &mut *child;
    #[cfg(unix)]
    let terminate_error = send_process_group_signal(pid, libc::SIGTERM);
    #[cfg(not(unix))]
    let terminate_error = match child.try_wait() {
        Ok(Some(_)) => Ok(()),
        _ => child.kill().map_err(|error| error.to_string()),
    };
    if let Err(error) = terminate_error {
        return ProcessCleanupResult {
            status: "cleanup_failed".to_string(),
            failure_kind: Some("owned_process_timeout_cleanup_failed".to_string()),
            pid: Some(pid),
            process_group,
            owned: true,
            message: Some(error),
        };
    }
    if wait_for_godot_process_exit(pid, 1.0) {
        return ProcessCleanupResult {
            status: "timeout_terminated".to_string(),
            failure_kind: Some("owned_process_timeout".to_string()),
            pid: Some(pid),
            process_group,
            owned: true,
            message: Some(
                "owned Godot process exceeded configured timeout and was terminated".to_string(),
            ),
        };
    }
    #[cfg(unix)]
    let kill_error = send_process_group_signal(pid, libc::SIGKILL);
    #[cfg(not(unix))]
    let kill_error = child.kill().map_err(|error| error.to_string());
    if let Err(error) = kill_error {
        return ProcessCleanupResult {
            status: "cleanup_failed".to_string(),
            failure_kind: Some("owned_process_timeout_cleanup_failed".to_string()),
            pid: Some(pid),
            process_group,
            owned: true,
            message: Some(error),
        };
    }
    if wait_for_godot_process_exit(pid, 1.0) {
        return ProcessCleanupResult {
            status: "timeout_killed".to_string(),
            failure_kind: Some("owned_process_timeout".to_string()),
            pid: Some(pid),
            process_group,
            owned: true,
            message: Some(
                "owned Godot process exceeded configured timeout and was killed".to_string(),
            ),
        };
    }
    ProcessCleanupResult {
        status: "cleanup_failed".to_string(),
        failure_kind: Some("owned_process_timeout_cleanup_failed".to_string()),
        pid: Some(pid),
        process_group,
        owned: true,
        message: Some("process remained alive after timeout terminate/kill fallback".to_string()),
    }
}

fn run_command_output(command: &[String]) -> std::io::Result<std::process::Output> {
    let Some(executable) = command.first() else {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "empty command",
        ));
    };
    let mut last_error = None;
    for attempt in 0..3 {
        match Command::new(executable)
            .args(command.iter().skip(1))
            .output()
        {
            Ok(output) => return Ok(output),
            Err(error) if error.raw_os_error() == Some(26) && attempt < 2 => {
                last_error = Some(error);
                thread::sleep(Duration::from_millis(10));
            }
            Err(error) => return Err(error),
        }
    }
    Err(last_error.expect("retry loop records executable-busy error"))
}

fn classify_launch_spawn_failure(display: &DisplayLaunchMetadata, command: &[String]) -> String {
    if let Some(virtual_display_command) = &display.virtual_display_command
        && command.first() == virtual_display_command.first()
    {
        return "virtual_display_allocation_failed".to_string();
    }
    if display.virtual_display_used
        && (display.launch_prefix_used || display.windowed_launch_prefix_used)
    {
        return "virtual_display_allocation_failed".to_string();
    }
    "ordinary_launch_failed".to_string()
}

fn classify_display_startup_failure(
    display: &DisplayLaunchMetadata,
    status: Option<i32>,
    stdout: &str,
    stderr: &str,
) -> Option<String> {
    if status == Some(0) || display.headless {
        return None;
    }
    let combined = format!("{stdout}\n{stderr}").to_ascii_lowercase();
    let display_startup_noise = [
        "cannot open display",
        "couldn't connect to display",
        "failed to initialize display",
        "no available display",
        "server is already active for display",
        "xvfb failed",
        "xvfb-run: error",
    ];
    if display_startup_noise
        .iter()
        .any(|needle| combined.contains(needle))
    {
        if display.virtual_display_used {
            return Some("virtual_display_allocation_failed".to_string());
        }
        return Some("display_startup_exhausted".to_string());
    }
    None
}

fn display_startup_guidance(display: &DisplayLaunchMetadata) -> String {
    if display.virtual_display_used {
        "Retry after releasing stale virtual displays or configure a different virtual display range."
            .to_string()
    } else {
        "Retry with a healthy DISPLAY/WAYLAND_DISPLAY or configure a virtual display wrapper."
            .to_string()
    }
}

fn display_launch_metadata_value(display: &DisplayLaunchMetadata) -> serde_json::Value {
    json!({
        "headless": display.headless,
        "selected_mode": display.selected_mode,
        "platform": display.platform,
        "linuxbsd_target": display.linuxbsd_target,
        "x11_display_available": display.x11_display_available,
        "wayland_display_available": display.wayland_display_available,
        "selected_display_driver": display.selected_display_driver,
        "project_prefers_wayland": display.project_prefers_wayland,
        "requested_display_driver": display.requested_display_driver,
        "virtual_display_used": display.virtual_display_used,
        "virtual_display_command": display.virtual_display_command,
        "fallback_reason": display.fallback_reason,
        "launch_prefix_used": display.launch_prefix_used,
        "windowed_launch_prefix_used": display.windowed_launch_prefix_used,
        "effective_command_prefix": display.effective_command_prefix,
        "display_backend_available": display.display_backend_available,
        "startup_attempted": display.startup_attempted,
        "virtual_display_exit_code": display.virtual_display_exit_code,
        "startup_failure_kind": display.startup_failure_kind,
        "startup_failure_retryable": display.startup_failure_retryable,
        "startup_failure_guidance": display.startup_failure_guidance,
    })
}

#[cfg(test)]
fn stale_owned_child_cleanup_failure(
    pid: i64,
    process_group: Option<i64>,
    message: impl Into<String>,
) -> ProcessCleanupResult {
    ProcessCleanupResult {
        status: "cleanup_failed".to_string(),
        failure_kind: Some("stale_process_cleanup_failed".to_string()),
        pid: Some(pid),
        process_group,
        owned: true,
        message: Some(message.into()),
    }
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

pub fn pid_is_running(pid: i64) -> bool {
    if pid <= 0 {
        return false;
    }
    #[cfg(unix)]
    unsafe {
        let result = libc::kill(pid as i32, 0);
        if result == 0 {
            return true;
        }
        let error = std::io::Error::last_os_error()
            .raw_os_error()
            .unwrap_or_default();
        error == libc::EPERM
    }
    #[cfg(windows)]
    {
        windows::is_running(pid)
    }
    #[cfg(not(any(unix, windows)))]
    {
        false
    }
}

fn is_zombie_process(pid: i64) -> bool {
    let stat_path = format!("/proc/{pid}/stat");
    if let Ok(contents) = fs::read_to_string(stat_path) {
        return contents.split_whitespace().nth(2) == Some("Z");
    }
    false
}

fn current_run_process_group(pid: i64) -> Option<i64> {
    if pid <= 0 {
        return None;
    }
    #[cfg(unix)]
    unsafe {
        let group = libc::getpgid(pid as libc::pid_t);
        if group > 0 {
            return Some(group as i64);
        }
        None
    }
    #[cfg(not(unix))]
    {
        let _ = pid;
        None
    }
}

pub fn terminate_process_tree(pid: i64, force: bool, expected: Option<u64>) -> Result<(), String> {
    #[cfg(windows)]
    {
        let _ = force;
        windows::terminate_tree(pid, expected).map_err(|error| error.to_string())
    }
    #[cfg(unix)]
    {
        let _ = expected;
        send_process_group_signal(pid, if force { libc::SIGKILL } else { libc::SIGTERM })
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = (pid, force, expected);
        Err("process termination unsupported on this platform".into())
    }
}

#[cfg(unix)]
fn send_signal(pid: i64, signal: i32) -> Result<(), String> {
    #[cfg(unix)]
    unsafe {
        if libc::kill(pid as i32, signal) == 0 {
            return Ok(());
        }
        Err(std::io::Error::last_os_error().to_string())
    }
    #[cfg(not(unix))]
    {
        let _ = (pid, signal);
        Err("signals unsupported on this platform".to_string())
    }
}

#[cfg(unix)]
fn send_process_group_signal(pid: i64, signal: i32) -> Result<(), String> {
    #[cfg(unix)]
    {
        let process_group_result = unsafe { libc::kill(-(pid as libc::pid_t), signal) };
        if process_group_result == 0 {
            return Ok(());
        }
        let group_error = std::io::Error::last_os_error();
        if group_error.raw_os_error() != Some(libc::ESRCH) {
            return Err(group_error.to_string());
        }
    }
    send_signal(pid, signal)
}

#[cfg(test)]
mod tests {
    use super::{
        GodotOneShotLaunch, append_project_display_driver_args_with_env, exit_poll_backoff,
        is_zombie_process, launch_godot_check, launch_godot_import, launch_godot_one_shot,
        launch_godot_patch_runner, launch_godot_session, pid_is_running,
        probe_godot_version_with_metadata, project_prefers_wayland, run_godot_command,
        run_godot_command_with_timeout, run_safe_probe, stale_owned_child_cleanup_failure,
        stop_godot_process_with_cleanup,
    };
    #[cfg(unix)]
    use super::{
        effective_launch_prefix, run_godot_command_with_completion_path, stop_godot_process,
    };
    use godot_qa_test_support::fixture;
    use std::env;
    use std::fs::{self, File};
    use std::io::Write;
    use std::path::Path;
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{Duration, SystemTime, UNIX_EPOCH};

    static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);
    static PROCESS_SIGNAL_LOCK: Mutex<()> = Mutex::new(());

    fn command_has_godot_suffix(command: &[String], expected_suffix: &[String]) -> bool {
        if command.strip_suffix(expected_suffix).is_some() {
            return true;
        }

        let Some(binary) = expected_suffix.first() else {
            return command.is_empty();
        };
        let Some(binary_index) = command.iter().position(|arg| arg == binary) else {
            return false;
        };

        let actual = &command[binary_index..];
        actual.len() >= 3
            && actual.first() == Some(binary)
            && actual.get(1).map(String::as_str) == Some("--display-driver")
            && actual[3..] == expected_suffix[1..]
    }

    #[test]
    fn safe_probe_success_captures_output_and_status() {
        let _signal_guard = PROCESS_SIGNAL_LOCK.lock().expect("signal test lock");
        let temp_dir = unique_temp_dir();
        fs::create_dir_all(&temp_dir).expect("create temp dir");
        let probe = fixture(
            &temp_dir.join("probe.sh"),
            &[("stdout", "ok:{arg1}\n"), ("stderr", "note\n")],
        );

        let result = run_safe_probe(&probe, &["--version".to_string()], Duration::from_secs(1));

        let _ = fs::remove_dir_all(&temp_dir);

        assert_eq!(
            result.command,
            vec![probe.to_string_lossy(), "--version".into()]
        );
        assert_eq!(result.status, Some(0));
        assert_eq!(result.stdout, "ok:--version\n");
        assert_eq!(result.stderr, "note\n");
        assert!(!result.timed_out);
        assert_eq!(result.error_kind, None);
        assert_eq!(result.error_message, None);
    }

    #[test]
    fn owned_child_file_capture_without_deadline_returns_output() {
        let _signal_guard = PROCESS_SIGNAL_LOCK.lock().expect("signal test lock");
        let temp_dir = unique_temp_dir();
        fs::create_dir_all(&temp_dir).expect("create temp dir");
        let output = super::run_owned_child_output(
            &[fixture(
                &temp_dir.join("output"),
                &[("stdout", "out"), ("stderr", "err")],
            )
            .to_string_lossy()
            .into_owned()],
            None,
            None,
            Some(&temp_dir.join("capture.txt")),
        )
        .expect("run child");
        assert_eq!(output.status_code, 0);
        assert_eq!(output.stdout, b"out");
        assert_eq!(output.stderr, b"err");
        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn safe_probe_non_zero_exit_returns_metadata() {
        let _signal_guard = PROCESS_SIGNAL_LOCK.lock().expect("signal test lock");
        let temp_dir = unique_temp_dir();
        fs::create_dir_all(&temp_dir).expect("create temp dir");
        let probe = fixture(
            &temp_dir.join("probe.sh"),
            &[("stderr", "bad\n"), ("exit", "17")],
        );

        let result = run_safe_probe(
            &probe,
            &["scenario".into(), "--help".into()],
            Duration::from_secs(1),
        );

        let _ = fs::remove_dir_all(&temp_dir);

        assert_eq!(result.status, Some(17));
        assert_eq!(result.stdout, "");
        assert_eq!(result.stderr, "bad\n");
        assert!(!result.timed_out);
        assert_eq!(result.error_kind, None);
        assert_eq!(result.error_message, None);
    }

    #[test]
    fn safe_probe_spawn_error_returns_metadata() {
        let temp_dir = unique_temp_dir();
        let missing = temp_dir.join("missing-godot-qa");

        let result = run_safe_probe(&missing, &["--version".to_string()], Duration::from_secs(1));

        assert_eq!(result.status, None);
        assert_eq!(result.stdout, "");
        assert_eq!(result.stderr, "");
        assert!(!result.timed_out);
        assert!(result.error_kind.is_some());
        assert!(result.error_message.is_some());
    }

    #[test]
    fn safe_probe_timeout_kills_child_and_reports_timeout() {
        let _signal_guard = PROCESS_SIGNAL_LOCK.lock().expect("signal test lock");
        let temp_dir = unique_temp_dir();
        fs::create_dir_all(&temp_dir).expect("create temp dir");
        let probe = fixture(
            &temp_dir.join("probe.sh"),
            &[("stdout", "before loop\n"), ("sleep_ms", "30000")],
        );

        let result = run_safe_probe(&probe, &[], Duration::from_millis(200));

        let _ = fs::remove_dir_all(&temp_dir);

        assert!(result.timed_out);
        assert_eq!(result.error_kind.as_deref(), Some("timeout"));
        assert!(result.duration >= Duration::from_millis(200));
        assert_eq!(result.stdout, "before loop\n");
        assert_ne!(result.status, Some(0));
    }

    #[test]
    fn version_probe_prefixed_failure_falls_back_to_direct_success() {
        let temp_dir = unique_temp_dir();
        fs::create_dir_all(&temp_dir).expect("create temp dir");
        let wrapper = fixture(&temp_dir.join("wrapper.sh"), &[("exit", "19")]);
        let godot = fixture(&temp_dir.join("godot.sh"), &[("stdout", "Godot_v4.3\n")]);

        let result = probe_godot_version_with_metadata(
            godot.to_str().expect("godot path"),
            &[wrapper.to_string_lossy().into_owned()],
        )
        .expect("direct probe succeeds");

        let _ = fs::remove_dir_all(&temp_dir);

        assert_eq!(result.version, "Godot_v4.3");
        assert!(result.metadata.fallback_used);
        assert_eq!(
            result.metadata.fallback_reason,
            Some("exited with status 19".to_string())
        );
        assert_eq!(result.metadata.attempts.len(), 2);
        assert!(result.metadata.attempts[0].used_launch_prefix);
        assert!(!result.metadata.attempts[1].used_launch_prefix);
        assert_eq!(result.metadata.selected_command[0], godot.to_string_lossy());
    }

    #[test]
    fn version_probe_prefixed_empty_output_falls_back_to_direct_success() {
        let temp_dir = unique_temp_dir();
        fs::create_dir_all(&temp_dir).expect("create temp dir");
        let godot = fixture(&temp_dir.join("godot.sh"), &[("stdout", "Godot_v4.4\n")]);

        let result = probe_godot_version_with_metadata(
            godot.to_str().expect("godot path"),
            &[fixture(&temp_dir.join("true"), &[])
                .to_string_lossy()
                .into_owned()],
        )
        .expect("direct probe succeeds");

        let _ = fs::remove_dir_all(&temp_dir);

        assert_eq!(result.version, "Godot_v4.4");
        assert!(result.metadata.fallback_used);
        assert_eq!(
            result.metadata.fallback_reason,
            Some("empty_output".to_string())
        );
    }

    #[test]
    fn version_probe_both_probes_failing_returns_metadata_failure() {
        let temp_dir = unique_temp_dir();
        fs::create_dir_all(&temp_dir).expect("create temp dir");
        let wrapper = fixture(&temp_dir.join("wrapper.sh"), &[("exit", "19")]);
        let godot = fixture(&temp_dir.join("godot.sh"), &[("exit", "21")]);

        let failure = probe_godot_version_with_metadata(
            godot.to_str().expect("godot path"),
            &[wrapper.to_string_lossy().into_owned()],
        )
        .expect_err("both probes fail");

        let _ = fs::remove_dir_all(&temp_dir);

        assert_eq!(failure.kind, "godot_version_probe_failed");
        let probe = failure
            .details
            .get("version_probe")
            .expect("probe metadata");
        assert_eq!(probe["fallback_used"], true);
        assert_eq!(probe["attempts"].as_array().expect("attempts").len(), 2);
    }

    #[test]
    fn version_probe_both_probes_empty_output_returns_metadata_failure() {
        let temp_dir = unique_temp_dir();
        fs::create_dir_all(&temp_dir).expect("create temp dir");
        let godot = fixture(&temp_dir.join("godot.sh"), &[("exit", "0")]);

        let failure = probe_godot_version_with_metadata(
            godot.to_str().expect("godot path"),
            &[fixture(&temp_dir.join("true"), &[])
                .to_string_lossy()
                .into_owned()],
        )
        .expect_err("empty probes fail");

        let _ = fs::remove_dir_all(&temp_dir);

        assert_eq!(failure.kind, "godot_version_probe_failed");
        assert_eq!(failure.actual, Some(serde_json::json!("empty_output")));
        let probe = failure
            .details
            .get("version_probe")
            .expect("probe metadata");
        assert_eq!(probe["attempts"][0]["failure_reason"], "empty_output");
        assert_eq!(probe["attempts"][1]["failure_reason"], "empty_output");
    }

    #[test]
    fn version_probe_explicit_invalid_binary_is_not_bypassed_by_direct_fallback() {
        let temp_dir = unique_temp_dir();
        fs::create_dir_all(&temp_dir).expect("create temp dir");
        let wrapper = fixture(&temp_dir.join("wrapper.sh"), &[("mode", "wrapper")]);
        let missing = temp_dir.join("missing-godot");

        let failure = probe_godot_version_with_metadata(
            missing.to_str().expect("missing path"),
            &[wrapper.to_string_lossy().into_owned()],
        )
        .expect_err("missing explicit binary fails");

        let _ = fs::remove_dir_all(&temp_dir);

        let probe = failure
            .details
            .get("version_probe")
            .expect("probe metadata");
        assert_eq!(probe["attempts"].as_array().expect("attempts").len(), 2);
        assert_eq!(
            probe["attempts"][1]["command"][0],
            missing.to_string_lossy().as_ref()
        );
    }

    #[test]
    fn launch_godot_one_shot_includes_headless_by_default() {
        let temp_dir = unique_temp_dir();
        fs::create_dir_all(&temp_dir).expect("create temp dir");
        let binary_path = fixture(&temp_dir.join("true"), &[]);
        let request_path = temp_dir.join("request.json");
        fs::write(&request_path, "{}\n").expect("write request");
        let logs_path = temp_dir.join("logs.txt");

        let result = launch_godot_one_shot(GodotOneShotLaunch {
            binary: binary_path.to_str().expect("binary path"),
            launch_prefix: &[],
            windowed_launch_prefix: &[],
            default_godot_args: &[],
            extra_godot_args: &[],
            default_project_args: &[],
            extra_project_args: &[],
            project_root: &temp_dir,
            request_path: &request_path,
            logs_path: &logs_path,
            headless: true,
            audio: false,
            viewport: Some(&[1280, 720]),
            timeout: None,
            fixed_fps: None,
        })
        .expect("run fake one-shot binary");

        let _ = fs::remove_dir_all(&temp_dir);

        assert_eq!(
            result.command,
            vec![
                binary_path.to_string_lossy().into_owned(),
                "--headless".to_string(),
                "--log-file".to_string(),
                temp_dir.join("godot.log").to_string_lossy().into_owned(),
                "--path".to_string(),
                temp_dir.to_string_lossy().into_owned(),
                "--script".to_string(),
                "res://addons/godot_qa/qa_runner.gd".to_string(),
                "--".to_string(),
                "--qa".to_string(),
                format!("--qa-run-request={}", request_path.to_string_lossy()),
            ]
        );
    }

    #[test]
    fn launch_godot_one_shot_omits_headless_when_windowed() {
        let temp_dir = unique_temp_dir();
        fs::create_dir_all(&temp_dir).expect("create temp dir");
        let binary_path = fixture(&temp_dir.join("true"), &[]);
        let request_path = temp_dir.join("request.json");
        fs::write(&request_path, "{}\n").expect("write request");
        let logs_path = temp_dir.join("logs.txt");

        let result = launch_godot_one_shot(GodotOneShotLaunch {
            binary: binary_path.to_str().expect("binary path"),
            launch_prefix: &[],
            windowed_launch_prefix: &[],
            default_godot_args: &[],
            extra_godot_args: &[],
            default_project_args: &[],
            extra_project_args: &[],
            project_root: &temp_dir,
            request_path: &request_path,
            logs_path: &logs_path,
            headless: false,
            audio: true,
            viewport: Some(&[1280, 720]),
            timeout: None,
            fixed_fps: None,
        })
        .expect("run fake one-shot binary");

        let _ = fs::remove_dir_all(&temp_dir);

        assert!(!result.command.contains(&"--headless".to_string()));
        let expected_suffix = vec![
            binary_path.to_string_lossy().into_owned(),
            "--resolution".to_string(),
            "1280x720".to_string(),
            "--log-file".to_string(),
            temp_dir.join("godot.log").to_string_lossy().into_owned(),
            "--path".to_string(),
            temp_dir.to_string_lossy().into_owned(),
            "--script".to_string(),
            "res://addons/godot_qa/qa_runner.gd".to_string(),
            "--".to_string(),
            "--qa".to_string(),
            format!("--qa-run-request={}", request_path.to_string_lossy()),
        ];
        assert!(
            command_has_godot_suffix(&result.command, &expected_suffix),
            "windowed one-shot command keeps expected Godot argv suffix"
        );
    }

    fn adjacent_pair(command: &[String], first: &str, second: &str) -> bool {
        command
            .windows(2)
            .any(|pair| pair[0] == first && pair[1] == second)
    }

    fn one_shot_media_command(headless: bool, audio: bool) -> Vec<String> {
        let temp_dir = unique_temp_dir();
        fs::create_dir_all(&temp_dir).expect("create temp dir");
        let binary_path = fixture(&temp_dir.join("true"), &[]);
        let request_path = temp_dir.join("request.json");
        fs::write(&request_path, "{}\n").expect("write request");
        let logs_path = temp_dir.join("logs.txt");

        let result = launch_godot_one_shot(GodotOneShotLaunch {
            binary: binary_path.to_str().expect("binary path"),
            launch_prefix: &[],
            windowed_launch_prefix: &[],
            default_godot_args: &[],
            extra_godot_args: &[],
            default_project_args: &[],
            extra_project_args: &[],
            project_root: &temp_dir,
            request_path: &request_path,
            logs_path: &logs_path,
            headless,
            audio,
            viewport: Some(&[1280, 720]),
            timeout: None,
            fixed_fps: None,
        })
        .expect("run fake one-shot binary");

        let _ = fs::remove_dir_all(&temp_dir);
        result.command
    }

    #[test]
    fn media_axis_display_off_audio_off_uses_headless() {
        // `--headless` nulls both display and audio, so no redundant --audio-driver.
        let command = one_shot_media_command(true, false);
        assert!(command.contains(&"--headless".to_string()));
        assert!(!command.contains(&"--audio-driver".to_string()));
        assert!(!adjacent_pair(&command, "--display-driver", "headless"));
        assert!(!command.contains(&"--resolution".to_string()));
    }

    #[test]
    fn media_axis_display_off_audio_on_nulls_display_only() {
        // display off + audio on: null only the display, keep the real audio driver.
        let command = one_shot_media_command(true, true);
        assert!(!command.contains(&"--headless".to_string()));
        assert!(adjacent_pair(&command, "--display-driver", "headless"));
        assert!(!command.contains(&"--audio-driver".to_string()));
        assert!(!command.contains(&"--resolution".to_string()));
    }

    #[test]
    fn media_axis_display_on_audio_off_adds_dummy_audio() {
        // windowed + audio off: real display, Dummy audio driver.
        let command = one_shot_media_command(false, false);
        assert!(!command.contains(&"--headless".to_string()));
        assert!(adjacent_pair(&command, "--resolution", "1280x720"));
        assert!(adjacent_pair(&command, "--audio-driver", "Dummy"));
        assert!(!adjacent_pair(&command, "--display-driver", "headless"));
    }

    #[test]
    fn media_axis_display_on_audio_on_keeps_real_audio() {
        // windowed + audio on: real display, real audio (no Dummy override).
        let command = one_shot_media_command(false, true);
        assert!(!command.contains(&"--headless".to_string()));
        assert!(adjacent_pair(&command, "--resolution", "1280x720"));
        assert!(!command.contains(&"--audio-driver".to_string()));
        assert!(!adjacent_pair(&command, "--display-driver", "headless"));
    }

    #[test]
    fn launch_godot_one_shot_applies_launch_prefix() {
        let temp_dir = unique_temp_dir();
        fs::create_dir_all(&temp_dir).expect("create temp dir");
        let wrapper_path = fixture(&temp_dir.join("wrapper"), &[("mode", "wrapper")]);
        let binary_path = fixture(&temp_dir.join("true"), &[]);
        let request_path = temp_dir.join("request.json");
        fs::write(&request_path, "{}\n").expect("write request");
        let logs_path = temp_dir.join("logs.txt");
        let prefix = vec![
            wrapper_path.to_string_lossy().into_owned(),
            "--wrapper-flag".to_string(),
        ];

        let result = launch_godot_one_shot(GodotOneShotLaunch {
            binary: binary_path.to_str().expect("binary path"),
            launch_prefix: &prefix,
            windowed_launch_prefix: &[],
            default_godot_args: &[],
            extra_godot_args: &[],
            default_project_args: &[],
            extra_project_args: &[],
            project_root: &temp_dir,
            request_path: &request_path,
            logs_path: &logs_path,
            headless: true,
            audio: false,
            viewport: Some(&[1280, 720]),
            timeout: None,
            fixed_fps: None,
        })
        .expect("run fake one-shot binary through wrapper");

        let _ = fs::remove_dir_all(&temp_dir);

        assert_eq!(result.command[0], prefix[0]);
        assert_eq!(result.command[1], "--wrapper-flag");
        assert_eq!(
            result.command[2],
            binary_path.to_string_lossy().into_owned()
        );
        assert!(result.command.contains(&"--headless".to_string()));
    }

    #[test]
    fn launch_godot_one_shot_headless_uses_only_global_launch_prefix() {
        let temp_dir = unique_temp_dir();
        fs::create_dir_all(&temp_dir).expect("create temp dir");
        let wrapper_path = fixture(&temp_dir.join("wrapper"), &[("mode", "wrapper")]);
        let binary_path = fixture(&temp_dir.join("true"), &[]);
        let request_path = temp_dir.join("request.json");
        fs::write(&request_path, "{}\n").expect("write request");
        let logs_path = temp_dir.join("logs.txt");
        let launch_prefix = vec![
            wrapper_path.to_string_lossy().into_owned(),
            "--wrapper-flag".to_string(),
        ];
        let windowed_launch_prefix = vec!["xvfb-run".to_string(), "-a".to_string()];

        let result = launch_godot_one_shot(GodotOneShotLaunch {
            binary: binary_path.to_str().expect("binary path"),
            launch_prefix: &launch_prefix,
            windowed_launch_prefix: &windowed_launch_prefix,
            default_godot_args: &[],
            extra_godot_args: &[],
            default_project_args: &[],
            extra_project_args: &[],
            project_root: &temp_dir,
            request_path: &request_path,
            logs_path: &logs_path,
            headless: true,
            audio: false,
            viewport: None,
            timeout: None,
            fixed_fps: None,
        })
        .expect("run fake one-shot binary through wrapper");

        let _ = fs::remove_dir_all(&temp_dir);

        assert_eq!(result.command[0], launch_prefix[0]);
        assert_eq!(result.command[1], "--wrapper-flag");
        assert_eq!(result.command[2], binary_path.to_string_lossy());
        assert!(result.display.launch_prefix_used);
        assert!(!result.display.windowed_launch_prefix_used);
        assert_eq!(result.display.effective_command_prefix, launch_prefix);
    }

    #[test]
    fn launch_godot_one_shot_windowed_uses_global_plus_windowed_launch_prefix() {
        let temp_dir = unique_temp_dir();
        fs::create_dir_all(&temp_dir).expect("create temp dir");
        let global_wrapper = fixture(&temp_dir.join("global-wrapper.sh"), &[("mode", "wrapper")]);
        let display_wrapper = fixture(&temp_dir.join("display-wrapper.sh"), &[("mode", "wrapper")]);
        let binary_path = fixture(&temp_dir.join("true"), &[]);
        let request_path = temp_dir.join("request.json");
        fs::write(&request_path, "{}\n").expect("write request");
        let logs_path = temp_dir.join("logs.txt");
        let launch_prefix = vec![
            global_wrapper.to_string_lossy().into_owned(),
            "--global-flag".to_string(),
        ];
        let windowed_launch_prefix = vec![
            display_wrapper.to_string_lossy().into_owned(),
            "--display-flag".to_string(),
        ];

        let result = launch_godot_one_shot(GodotOneShotLaunch {
            binary: binary_path.to_str().expect("binary path"),
            launch_prefix: &launch_prefix,
            windowed_launch_prefix: &windowed_launch_prefix,
            default_godot_args: &[],
            extra_godot_args: &[],
            default_project_args: &[],
            extra_project_args: &[],
            project_root: &temp_dir,
            request_path: &request_path,
            logs_path: &logs_path,
            headless: false,
            audio: false,
            viewport: None,
            timeout: None,
            fixed_fps: None,
        })
        .expect("run fake one-shot binary through nested wrappers");

        let _ = fs::remove_dir_all(&temp_dir);

        assert_eq!(&result.command[0..2], launch_prefix.as_slice());
        assert_eq!(&result.command[2..4], windowed_launch_prefix.as_slice());
        assert_eq!(result.command[4], binary_path.to_string_lossy());
        assert!(result.display.launch_prefix_used);
        assert!(result.display.windowed_launch_prefix_used);
        assert_eq!(
            result.display.effective_command_prefix,
            [launch_prefix, windowed_launch_prefix].concat()
        );
    }

    #[test]
    fn godot_args_are_inserted_before_qa_runner_separator() {
        let temp_dir = unique_temp_dir();
        fs::create_dir_all(&temp_dir).expect("create temp dir");
        let binary_path = fixture(&temp_dir.join("true"), &[]);
        let request_path = temp_dir.join("request.json");
        fs::write(&request_path, "{}\n").expect("write request");
        let logs_path = temp_dir.join("logs.txt");
        let extra_godot_args = vec![
            "--rendering-driver".to_string(),
            "opengl3".to_string(),
            "--single-window".to_string(),
        ];

        let result = launch_godot_one_shot(GodotOneShotLaunch {
            binary: binary_path.to_str().expect("binary path"),
            launch_prefix: &[],
            windowed_launch_prefix: &[],
            default_godot_args: &[],
            extra_godot_args: &extra_godot_args,
            default_project_args: &[],
            extra_project_args: &[],
            project_root: &temp_dir,
            request_path: &request_path,
            logs_path: &logs_path,
            headless: true,
            audio: false,
            viewport: None,
            timeout: None,
            fixed_fps: None,
        })
        .expect("run fake one-shot binary");

        let _ = fs::remove_dir_all(&temp_dir);

        let separator_index = result
            .command
            .iter()
            .position(|arg| arg == "--")
            .expect("qa runner separator");
        let path_index = result
            .command
            .iter()
            .position(|arg| arg == "--path")
            .expect("project path arg");
        let rendering_index = result
            .command
            .iter()
            .position(|arg| arg == "--rendering-driver")
            .expect("extra arg");
        assert!(rendering_index < path_index);
        assert!(path_index < separator_index);
        assert_eq!(
            &result.command[rendering_index..rendering_index + extra_godot_args.len()],
            extra_godot_args.as_slice()
        );
    }

    #[test]
    fn launch_godot_one_shot_includes_generated_fixed_fps_args() {
        let temp_dir = unique_temp_dir();
        fs::create_dir_all(&temp_dir).expect("create temp dir");
        let binary_path = fixture(&temp_dir.join("true"), &[]);
        let request_path = temp_dir.join("request.json");
        fs::write(&request_path, "{}\n").expect("write request");
        let logs_path = temp_dir.join("logs.txt");

        let result = launch_godot_one_shot(GodotOneShotLaunch {
            binary: binary_path.to_str().expect("binary path"),
            launch_prefix: &[],
            windowed_launch_prefix: &[],
            default_godot_args: &[],
            extra_godot_args: &[],
            default_project_args: &[],
            extra_project_args: &[],
            project_root: &temp_dir,
            request_path: &request_path,
            logs_path: &logs_path,
            headless: true,
            audio: false,
            viewport: None,
            timeout: None,
            fixed_fps: Some(144),
        })
        .expect("run fake one-shot binary");

        let _ = fs::remove_dir_all(&temp_dir);

        let fixed_fps_index = result
            .command
            .iter()
            .position(|arg| arg == "--fixed-fps")
            .expect("fixed fps arg");
        let path_index = result
            .command
            .iter()
            .position(|arg| arg == "--path")
            .expect("project path arg");
        assert!(fixed_fps_index < path_index);
        assert_eq!(result.command[fixed_fps_index + 1], "144");
        assert!(result.command.contains(&"--disable-vsync".to_string()));
    }

    #[test]
    fn project_args_are_appended_after_qa_run_request() {
        let temp_dir = unique_temp_dir();
        fs::create_dir_all(&temp_dir).expect("create temp dir");
        let wrapper_path = fixture(&temp_dir.join("wrapper"), &[("mode", "wrapper")]);
        let binary_path = fixture(&temp_dir.join("true"), &[]);
        let request_path = temp_dir.join("request.json");
        fs::write(&request_path, "{}\n").expect("write request");
        let logs_path = temp_dir.join("logs.txt");
        let launch_prefix = vec![
            wrapper_path.to_string_lossy().into_owned(),
            "--wrapper-flag".to_string(),
        ];
        let extra_godot_args = vec![
            "--rendering-driver".to_string(),
            "opengl3".to_string(),
            "--single-window".to_string(),
        ];
        let extra_project_args = vec![
            "--save-slot".to_string(),
            "qa profile".to_string(),
            "--seed=42".to_string(),
        ];

        let result = launch_godot_one_shot(GodotOneShotLaunch {
            binary: binary_path.to_str().expect("binary path"),
            launch_prefix: &launch_prefix,
            windowed_launch_prefix: &[],
            default_godot_args: &[],
            extra_godot_args: &extra_godot_args,
            default_project_args: &[],
            extra_project_args: &extra_project_args,
            project_root: &temp_dir,
            request_path: &request_path,
            logs_path: &logs_path,
            headless: true,
            audio: false,
            viewport: None,
            timeout: None,
            fixed_fps: None,
        })
        .expect("run fake one-shot binary through wrapper");

        let _ = fs::remove_dir_all(&temp_dir);

        assert_eq!(result.command[0], launch_prefix[0]);
        assert_eq!(result.command[1], "--wrapper-flag");
        assert_eq!(
            result.command[2],
            binary_path.to_string_lossy().into_owned()
        );
        let separator_index = result
            .command
            .iter()
            .position(|arg| arg == "--")
            .expect("qa runner separator");
        let path_index = result
            .command
            .iter()
            .position(|arg| arg == "--path")
            .expect("project path arg");
        let rendering_index = result
            .command
            .iter()
            .position(|arg| arg == "--rendering-driver")
            .expect("extra Godot arg");
        let qa_index = result
            .command
            .iter()
            .position(|arg| arg == "--qa")
            .expect("qa runner flag");
        let qa_request_index = result
            .command
            .iter()
            .position(|arg| arg.starts_with("--qa-run-request="))
            .expect("qa run request flag");
        assert!(rendering_index < path_index);
        assert!(path_index < separator_index);
        assert!(separator_index < qa_index);
        assert!(qa_index < qa_request_index);
        assert_eq!(
            &result.command[rendering_index..rendering_index + extra_godot_args.len()],
            extra_godot_args.as_slice()
        );
        assert_eq!(
            &result.command[qa_request_index + 1..qa_request_index + 1 + extra_project_args.len()],
            extra_project_args.as_slice()
        );
    }

    #[test]
    fn default_and_extra_launch_args_preserve_order() {
        let temp_dir = unique_temp_dir();
        fs::create_dir_all(&temp_dir).expect("create temp dir");
        let binary_path = fixture(&temp_dir.join("true"), &[]);
        let request_path = temp_dir.join("request.json");
        fs::write(&request_path, "{}\n").expect("write request");
        let logs_path = temp_dir.join("logs.txt");
        let default_godot_args = vec!["--display-driver".to_string(), "headless".to_string()];
        let extra_godot_args = vec!["--single-window".to_string()];
        let default_project_args = vec!["--no-countdown".to_string()];
        let extra_project_args = vec!["--seed".to_string(), "42".to_string()];

        let result = launch_godot_one_shot(GodotOneShotLaunch {
            binary: binary_path.to_str().expect("binary path"),
            launch_prefix: &[],
            windowed_launch_prefix: &[],
            default_godot_args: &default_godot_args,
            extra_godot_args: &extra_godot_args,
            default_project_args: &default_project_args,
            extra_project_args: &extra_project_args,
            project_root: &temp_dir,
            request_path: &request_path,
            logs_path: &logs_path,
            headless: true,
            audio: false,
            viewport: None,
            timeout: None,
            fixed_fps: None,
        })
        .expect("run fake one-shot binary");

        let _ = fs::remove_dir_all(&temp_dir);

        let path_index = result
            .command
            .iter()
            .position(|arg| arg == "--path")
            .expect("project path arg");
        let default_godot_index = result
            .command
            .iter()
            .position(|arg| arg == "--display-driver")
            .expect("default Godot arg");
        let qa_request_index = result
            .command
            .iter()
            .position(|arg| arg.starts_with("--qa-run-request="))
            .expect("QA request flag");
        assert!(default_godot_index < path_index);
        assert_eq!(
            &result.command[default_godot_index..path_index],
            ["--display-driver", "headless", "--single-window"]
        );
        assert_eq!(
            &result.command[qa_request_index + 1..],
            ["--no-countdown", "--seed", "42"]
        );
    }

    #[cfg(unix)]
    #[test]
    fn project_wayland_preference_requires_windowed_launch() {
        let temp_dir = unique_temp_dir();
        fs::create_dir_all(&temp_dir).expect("create temp dir");
        fs::write(
            temp_dir.join("project.godot"),
            "config_version=5\n\n[application]\nrun/platforms/linuxbsd/prefer_wayland=true\n",
        )
        .expect("write project");

        let windowed = project_prefers_wayland(&temp_dir, false, true);
        let headless = project_prefers_wayland(&temp_dir, true, true);

        let _ = fs::remove_dir_all(&temp_dir);

        assert!(windowed);
        assert!(!headless);
    }

    #[cfg(unix)]
    #[test]
    fn display_driver_args_honor_project_wayland_preference() {
        let temp_dir = unique_temp_dir();
        fs::create_dir_all(&temp_dir).expect("create temp dir");
        fs::write(
            temp_dir.join("project.godot"),
            "config_version=5\n\n[application]\nrun/platforms/linuxbsd/prefer_wayland=true\n",
        )
        .expect("write project");
        let mut command = vec!["godot".to_string()];

        let display = append_project_display_driver_args_with_env(
            &mut command,
            &temp_dir,
            &[],
            &[],
            false,
            false,
            true,
            None,
        );

        let _ = fs::remove_dir_all(&temp_dir);

        assert_eq!(
            command,
            vec![
                "godot".to_string(),
                "--display-driver".to_string(),
                "wayland".to_string(),
            ]
        );
        assert_eq!(display.selected_display_driver.as_deref(), Some("wayland"));
        assert!(display.display_backend_available);
    }

    #[test]
    fn display_driver_args_skip_headless_launches() {
        let temp_dir = unique_temp_dir();
        fs::create_dir_all(&temp_dir).expect("create temp dir");
        fs::write(
            temp_dir.join("project.godot"),
            "config_version=5\n\n[application]\nrun/platforms/linuxbsd/prefer_wayland=true\n",
        )
        .expect("write project");
        let mut command = vec!["godot".to_string()];

        let display = append_project_display_driver_args_with_env(
            &mut command,
            &temp_dir,
            &[],
            &[],
            true,
            false,
            true,
            None,
        );

        let _ = fs::remove_dir_all(&temp_dir);

        assert_eq!(command, vec!["godot".to_string()]);
        assert_eq!(display.selected_mode, "headless");
        assert_eq!(display.selected_display_driver, None);
        assert!(display.display_backend_available);
    }

    #[cfg(unix)]
    #[test]
    fn display_resolver_selects_x11_when_display_available() {
        let temp_dir = unique_temp_dir();
        fs::create_dir_all(&temp_dir).expect("create temp dir");
        let mut command = vec!["godot".to_string()];

        let display = append_project_display_driver_args_with_env(
            &mut command,
            &temp_dir,
            &[],
            &[],
            false,
            true,
            false,
            None,
        );

        let _ = fs::remove_dir_all(&temp_dir);

        assert_eq!(command, vec!["godot".to_string()]);
        assert_eq!(display.selected_mode, "windowed");
        assert_eq!(display.selected_display_driver.as_deref(), Some("x11"));
        assert!(display.x11_display_available);
        assert!(!display.virtual_display_used);
        assert!(display.display_backend_available);
    }

    #[cfg(unix)]
    #[test]
    fn display_resolver_reports_no_backend_without_virtual_wrapper() {
        let temp_dir = unique_temp_dir();
        fs::create_dir_all(&temp_dir).expect("create temp dir");
        let mut command = vec!["godot".to_string()];

        let display = append_project_display_driver_args_with_env(
            &mut command,
            &temp_dir,
            &[],
            &[],
            false,
            false,
            false,
            Some(std::ffi::OsString::new()),
        );

        let _ = fs::remove_dir_all(&temp_dir);

        assert_eq!(command, vec!["godot".to_string()]);
        assert_eq!(display.selected_display_driver, None);
        assert!(!display.virtual_display_used);
        assert_eq!(
            display.fallback_reason.as_deref(),
            Some("display_backend_unavailable")
        );
        assert!(!display.display_backend_available);
    }

    #[cfg(unix)]
    #[test]
    fn display_resolver_records_configured_xvfb_launch_prefix() {
        let temp_dir = unique_temp_dir();
        fs::create_dir_all(&temp_dir).expect("create temp dir");
        let mut command = vec!["godot".to_string()];
        let prefix = vec!["xvfb-run".to_string(), "-a".to_string()];

        let display = append_project_display_driver_args_with_env(
            &mut command,
            &temp_dir,
            &prefix,
            &[],
            false,
            false,
            true,
            None,
        );

        let _ = fs::remove_dir_all(&temp_dir);

        assert_eq!(
            command,
            vec![
                "godot".to_string(),
                "--display-driver".to_string(),
                "x11".to_string(),
            ]
        );
        assert!(display.launch_prefix_used);
        assert!(display.virtual_display_used);
        assert_eq!(display.virtual_display_command, None);
        assert_eq!(display.requested_display_driver.as_deref(), Some("x11"));
        assert_eq!(display.selected_display_driver.as_deref(), Some("x11"));
        assert_eq!(
            display.fallback_reason.as_deref(),
            Some("configured_launch_prefix")
        );
        assert!(!display.wayland_display_available);
    }

    #[cfg(unix)]
    #[test]
    fn display_resolver_records_configured_xvfb_windowed_launch_prefix() {
        let temp_dir = unique_temp_dir();
        fs::create_dir_all(&temp_dir).expect("create temp dir");
        let mut command = vec!["godot".to_string()];
        let global_prefix = vec!["global-wrap".to_string()];
        let windowed_prefix = vec!["xvfb-run".to_string(), "-a".to_string()];

        let display = append_project_display_driver_args_with_env(
            &mut command,
            &temp_dir,
            &global_prefix,
            &windowed_prefix,
            false,
            false,
            true,
            None,
        );

        let _ = fs::remove_dir_all(&temp_dir);

        assert_eq!(
            command,
            vec![
                "godot".to_string(),
                "--display-driver".to_string(),
                "x11".to_string(),
            ]
        );
        assert!(display.launch_prefix_used);
        assert!(display.windowed_launch_prefix_used);
        assert!(display.virtual_display_used);
        assert_eq!(display.virtual_display_command, None);
        assert_eq!(
            display.fallback_reason.as_deref(),
            Some("configured_windowed_launch_prefix")
        );
        assert_eq!(
            display.effective_command_prefix,
            vec![
                "global-wrap".to_string(),
                "xvfb-run".to_string(),
                "-a".to_string()
            ]
        );
    }

    #[cfg(unix)]
    #[test]
    fn display_resolver_auto_detects_xvfb_run_through_path() {
        let temp_dir = unique_temp_dir();
        let bin_dir = temp_dir.join("bin");
        fs::create_dir_all(&bin_dir).expect("create bin dir");
        let xvfb = fixture(&bin_dir.join("xvfb-run"), &[("mode", "wrapper")]);
        let mut command = vec!["godot".to_string()];

        let display = append_project_display_driver_args_with_env(
            &mut command,
            &temp_dir,
            &[],
            &[],
            false,
            false,
            false,
            Some(std::ffi::OsString::from(
                bin_dir.to_string_lossy().into_owned(),
            )),
        );

        let _ = fs::remove_dir_all(&temp_dir);

        assert_eq!(
            command,
            vec![
                "godot".to_string(),
                "--display-driver".to_string(),
                "x11".to_string(),
            ]
        );
        assert!(display.virtual_display_used);
        assert_eq!(display.requested_display_driver.as_deref(), Some("x11"));
        assert_eq!(
            display.virtual_display_command,
            Some(vec![xvfb.to_string_lossy().into_owned(), "-a".to_string()])
        );
        assert_eq!(display.selected_display_driver.as_deref(), Some("x11"));
        assert!(display.display_backend_available);
        assert_eq!(
            effective_launch_prefix(&[], &[], &display),
            vec![xvfb.to_string_lossy().into_owned(), "-a".to_string()]
        );
    }

    #[test]
    fn missing_xvfb_run_is_not_added_to_launch_command() {
        let temp_dir = unique_temp_dir();
        fs::create_dir_all(&temp_dir).expect("create temp dir");
        let mut command = vec!["godot".to_string()];

        let display = append_project_display_driver_args_with_env(
            &mut command,
            &temp_dir,
            &[],
            &[],
            false,
            false,
            false,
            Some(std::ffi::OsString::new()),
        );

        let _ = fs::remove_dir_all(&temp_dir);

        assert_eq!(command, vec!["godot".to_string()]);
        assert_eq!(display.virtual_display_command, None);
        assert!(!display.virtual_display_used);
    }

    #[test]
    fn display_startup_failure_contract() {
        let _signal_guard = PROCESS_SIGNAL_LOCK.lock().expect("signal test lock");
        let temp_dir = unique_temp_dir();
        fs::create_dir_all(&temp_dir).expect("create temp dir");
        let logs_path = temp_dir.join("logs.txt");

        let display_fail = fixture(
            &temp_dir.join("display-fail.sh"),
            &[("stderr", "Failed to initialize display\n"), ("exit", "17")],
        );
        let display = append_project_display_driver_args_with_env(
            &mut vec!["godot".to_string()],
            &temp_dir,
            &[],
            &[],
            false,
            true,
            false,
            None,
        );
        let result = run_godot_command(
            &[display_fail.to_string_lossy().into_owned()],
            &logs_path,
            display,
        )
        .expect("display startup failure is captured as process output");
        assert_eq!(
            result.infrastructure_failure_kind.as_deref(),
            Some("display_startup_exhausted")
        );
        assert_eq!(
            result.display.startup_failure_kind.as_deref(),
            Some("display_startup_exhausted")
        );
        assert!(result.display.startup_failure_retryable);

        let xvfb = fixture(
            &temp_dir.join("xvfb-run"),
            &[
                ("stderr", "xvfb-run: error: Xvfb failed to start\n"),
                ("exit", "3"),
            ],
        );
        let virtual_display = append_project_display_driver_args_with_env(
            &mut vec!["godot".to_string()],
            &temp_dir,
            &[xvfb.to_string_lossy().into_owned()],
            &[],
            false,
            false,
            false,
            None,
        );
        let result = run_godot_command(
            &[
                xvfb.to_string_lossy().into_owned(),
                fixture(&temp_dir.join("true"), &[])
                    .to_string_lossy()
                    .into_owned(),
            ],
            &logs_path,
            virtual_display,
        )
        .expect("virtual display failure is captured as process output");
        assert_eq!(
            result.infrastructure_failure_kind.as_deref(),
            Some("virtual_display_allocation_failed")
        );
        assert_eq!(
            result.display.virtual_display_exit_code,
            Some(result.returncode)
        );

        let failure = run_godot_command(
            &[temp_dir
                .join("missing-godot")
                .to_string_lossy()
                .into_owned()],
            &logs_path,
            append_project_display_driver_args_with_env(
                &mut vec!["godot".to_string()],
                &temp_dir,
                &[],
                &[],
                true,
                false,
                false,
                None,
            ),
        )
        .expect_err("missing executable remains a launch failure");
        assert_eq!(failure.kind, "godot_launch_failed");
        assert_eq!(
            failure.details["launch_infrastructure"]["failure_kind"],
            serde_json::json!("ordinary_launch_failed")
        );

        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn process_cleanup_contract() {
        let temp_dir = unique_temp_dir();
        fs::create_dir_all(&temp_dir).expect("create temp dir");
        let logs_path = temp_dir.join("logs.txt");
        let result = run_godot_command(
            &[fixture(&temp_dir.join("true"), &[])
                .to_string_lossy()
                .into_owned()],
            &logs_path,
            append_project_display_driver_args_with_env(
                &mut vec!["godot".to_string()],
                &temp_dir,
                &[],
                &[],
                true,
                false,
                false,
                None,
            ),
        )
        .expect("owned child exits");
        let cleanup = result.cleanup.expect("owned child cleanup metadata");
        assert_eq!(cleanup.status, "exited");
        assert!(cleanup.owned);
        assert_eq!(cleanup.pid, result.pid);

        let stale = stale_owned_child_cleanup_failure(
            result.pid.unwrap_or(0),
            result.process_group,
            "owned child identity was stale before cleanup completed",
        );
        assert_eq!(stale.status, "cleanup_failed");
        assert_eq!(
            stale.failure_kind.as_deref(),
            Some("stale_process_cleanup_failed")
        );
        assert!(stale.owned);
        assert_eq!(stale.process_group, result.process_group);

        let invalid = stop_godot_process_with_cleanup(0, 0.01).expect_err("invalid pid is unowned");
        assert_eq!(invalid.kind, "godot_stop_failed");
        assert_eq!(invalid.actual, Some(serde_json::json!(0)));

        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn godot_process_timeout_kills_owned_child() {
        let _signal_guard = PROCESS_SIGNAL_LOCK.lock().expect("signal test lock");
        let temp_dir = unique_temp_dir();
        fs::create_dir_all(&temp_dir).expect("create temp dir");
        let logs_path = temp_dir.join("logs.txt");
        let child = fixture(
            &temp_dir.join("slow.sh"),
            &[("stdout", "started\n"), ("sleep_ms", "30000")],
        );

        let result = run_godot_command_with_timeout(
            &[child.to_string_lossy().into_owned()],
            &logs_path,
            append_project_display_driver_args_with_env(
                &mut vec!["godot".to_string()],
                &temp_dir,
                &[],
                &[],
                true,
                false,
                false,
                None,
            ),
            Some(Duration::from_millis(100)),
        )
        .expect("timeout is returned as process metadata");

        assert!(result.timed_out);
        assert_eq!(result.timeout_ms, Some(100));
        assert_eq!(
            result.infrastructure_failure_kind.as_deref(),
            Some("owned_process_timeout")
        );
        let cleanup = result.cleanup.as_ref().expect("cleanup metadata");
        assert!(matches!(
            cleanup.status.as_str(),
            "timeout_terminated" | "timeout_killed"
        ));
        assert_eq!(
            cleanup.failure_kind.as_deref(),
            Some("owned_process_timeout")
        );
        if let Some(pid) = result.pid {
            assert!(!pid_is_running(pid) || is_zombie_process(pid));
        }

        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[cfg(unix)]
    #[test]
    fn completed_result_then_stalled_teardown_is_bounded_and_keeps_output() {
        let _signal_guard = PROCESS_SIGNAL_LOCK.lock().expect("signal test lock");
        let temp_dir = unique_temp_dir();
        fs::create_dir_all(&temp_dir).unwrap();
        let logs_path = temp_dir.join("logs.txt");
        let result_path = temp_dir.join("result.json");
        fs::write(
            temp_dir.join("request.json"),
            r#"{"run":{"id":"run-1","nonce":"fresh"}}"#,
        )
        .unwrap();
        let script = write_executable(
            &temp_dir.join("hang.sh"),
            &format!(
                "#!/bin/sh\nprintf 'first failure evidence\\n'\nprintf '%s' '{{\"run_identity\":{{\"run_id\":\"run-1\",\"nonce\":\"fresh\"}},\"ok\":false,\"failures\":[{{\"type\":\"assertion_failed\"}}]}}' > '{}'\n(sleep 30) &\nwhile :; do sleep 1; done\n",
                result_path.display()
            ),
        );
        let result = run_godot_command_with_completion_path(
            &["/bin/sh".to_string(), script.to_string_lossy().into_owned()],
            &logs_path,
            append_project_display_driver_args_with_env(
                &mut vec!["godot".to_string()],
                &temp_dir,
                &[],
                &[],
                true,
                false,
                false,
                None,
            ),
            None,
            Some(&result_path),
        )
        .unwrap();
        assert!(result.timed_out);
        assert_eq!(result.timeout_ms, Some(3000));
        assert!(result.stdout.contains("first failure evidence"));
        assert!(logs_path.with_extension("stdout.txt").is_file());
        assert!(result.cleanup.is_some());
        let _ = fs::remove_dir_all(temp_dir);
    }

    #[cfg(unix)]
    #[test]
    fn result_near_deadline_still_gets_full_exit_grace() {
        let _signal_guard = PROCESS_SIGNAL_LOCK.lock().unwrap();
        let temp_dir = unique_temp_dir();
        fs::create_dir_all(&temp_dir).unwrap();
        let result_path = temp_dir.join("result.json");
        fs::write(
            temp_dir.join("request.json"),
            r#"{"run":{"id":"run-1","nonce":"fresh"}}"#,
        )
        .unwrap();
        let script = write_executable(
            &temp_dir.join("late.sh"),
            &format!(
                "#!/bin/sh\nsleep 0.9\nprintf '%s' '{{\"run_identity\":{{\"run_id\":\"run-1\",\"nonce\":\"fresh\"}},\"ok\":true,\"failures\":[]}}' > '{}'\nsleep 0.6\n",
                result_path.display()
            ),
        );
        let started = std::time::Instant::now();
        let result = run_godot_command_with_completion_path(
            &["/bin/sh".into(), script.to_string_lossy().into_owned()],
            &temp_dir.join("logs.txt"),
            append_project_display_driver_args_with_env(
                &mut vec!["godot".into()],
                &temp_dir,
                &[],
                &[],
                true,
                false,
                false,
                None,
            ),
            Some(Duration::from_secs(1)),
            Some(&result_path),
        )
        .unwrap();
        assert!(
            !result.timed_out,
            "valid result must receive a full exit grace"
        );
        assert_eq!(result.returncode, 0);
        assert!(started.elapsed() >= Duration::from_millis(1400));
        let _ = fs::remove_dir_all(temp_dir);
    }

    #[cfg(unix)]
    #[test]
    fn stalled_exit_after_late_result_uses_three_second_grace() {
        let _signal_guard = PROCESS_SIGNAL_LOCK.lock().unwrap();
        let temp_dir = unique_temp_dir();
        fs::create_dir_all(&temp_dir).unwrap();
        let result_path = temp_dir.join("result.json");
        fs::write(
            temp_dir.join("request.json"),
            r#"{"run":{"id":"run-1","nonce":"fresh"}}"#,
        )
        .unwrap();
        let script = write_executable(
            &temp_dir.join("late_stall.sh"),
            &format!(
                "#!/bin/sh\nsleep 0.9\nprintf '%s' '{{\"run_identity\":{{\"run_id\":\"run-1\",\"nonce\":\"fresh\"}},\"ok\":true,\"failures\":[]}}' > '{}'\nwhile :; do sleep 1; done\n",
                result_path.display()
            ),
        );
        let started = std::time::Instant::now();
        let result = run_godot_command_with_completion_path(
            &["/bin/sh".into(), script.to_string_lossy().into_owned()],
            &temp_dir.join("logs.txt"),
            append_project_display_driver_args_with_env(
                &mut vec!["godot".into()],
                &temp_dir,
                &[],
                &[],
                true,
                false,
                false,
                None,
            ),
            Some(Duration::from_secs(1)),
            Some(&result_path),
        )
        .unwrap();
        assert!(result.timed_out);
        assert_eq!(result.timeout_ms, Some(3000));
        assert!(started.elapsed() >= Duration::from_millis(3700));
        let _ = fs::remove_dir_all(temp_dir);
    }

    #[test]
    fn file_capture_keeps_bounded_head_and_tail() {
        let temp_dir = unique_temp_dir();
        fs::create_dir_all(&temp_dir).unwrap();
        let path = temp_dir.join("large.stdout.txt");
        let mut file = File::create(&path).unwrap();
        file.write_all(b"ERROR: first failure\n").unwrap();
        file.write_all(&vec![
            b'x';
            (super::CAPTURED_STREAM_LIMIT_BYTES + 1024) as usize
        ])
        .unwrap();
        file.write_all(b"ERROR: last teardown\n").unwrap();
        drop(file);
        let (capture, total) = super::read_bounded_capture(&path).unwrap();
        let capture = String::from_utf8(capture).unwrap();
        assert!(total > super::CAPTURED_STREAM_LIMIT_BYTES);
        assert!(capture.len() < super::CAPTURED_STREAM_LIMIT_BYTES as usize + 1024);
        assert!(capture.contains("ERROR: first failure"));
        assert!(capture.contains("ERROR: last teardown"));
        assert!(capture.contains("omitted"));
        let _ = fs::remove_dir_all(temp_dir);
    }

    #[cfg(unix)]
    #[test]
    fn truncated_success_keeps_complete_raw_streams() {
        let _signal_guard = PROCESS_SIGNAL_LOCK.lock().unwrap();
        let temp_dir = unique_temp_dir();
        fs::create_dir_all(&temp_dir).unwrap();
        let result_path = temp_dir.join("result.json");
        let logs_path = temp_dir.join("logs.txt");
        fs::write(
            temp_dir.join("request.json"),
            r#"{"run":{"id":"run-1","nonce":"fresh"}}"#,
        )
        .unwrap();
        let script = write_executable(
            &temp_dir.join("large.sh"),
            &format!(
                "#!/bin/sh\nprintf 'ERROR: first failure\\n'\nhead -c 8389632 /dev/zero\nprintf '\\nERROR: last teardown\\n'\nprintf '%s' '{{\"run_identity\":{{\"run_id\":\"run-1\",\"nonce\":\"fresh\"}},\"ok\":true,\"failures\":[]}}' > '{}'\n",
                result_path.display()
            ),
        );
        let result = run_godot_command_with_completion_path(
            &["/bin/sh".into(), script.to_string_lossy().into_owned()],
            &logs_path,
            append_project_display_driver_args_with_env(
                &mut vec!["godot".into()],
                &temp_dir,
                &[],
                &[],
                true,
                false,
                false,
                None,
            ),
            Some(Duration::from_secs(10)),
            Some(&result_path),
        )
        .unwrap();
        assert_eq!(result.returncode, 0);
        let truncation = result.output_truncation.unwrap();
        assert!(truncation.stdout_total_bytes > super::CAPTURED_STREAM_LIMIT_BYTES);
        assert!(result.stdout.contains("ERROR: first failure"));
        assert!(result.stdout.contains("ERROR: last teardown"));
        assert!(logs_path.with_extension("stdout.txt").is_file());
        assert_eq!(
            fs::metadata(truncation.raw_stdout_path).unwrap().len(),
            truncation.stdout_total_bytes
        );
        let _ = fs::remove_dir_all(temp_dir);
    }

    #[test]
    fn godot_process_timeout_preserves_partial_output() {
        let _signal_guard = PROCESS_SIGNAL_LOCK.lock().expect("signal test lock");
        let temp_dir = unique_temp_dir();
        fs::create_dir_all(&temp_dir).expect("create temp dir");
        let logs_path = temp_dir.join("logs.txt");
        let child = fixture(
            &temp_dir.join("partial.sh"),
            &[
                ("stdout", "partial stdout\n"),
                ("stderr", "partial stderr\n"),
                ("sleep_ms", "30000"),
            ],
        );

        let result = run_godot_command_with_timeout(
            &[child.to_string_lossy().into_owned()],
            &logs_path,
            append_project_display_driver_args_with_env(
                &mut vec!["godot".to_string()],
                &temp_dir,
                &[],
                &[],
                true,
                false,
                false,
                None,
            ),
            Some(Duration::from_millis(100)),
        )
        .expect("timeout preserves process output");

        assert!(result.timed_out);
        assert_eq!(result.stdout, "partial stdout\n");
        assert_eq!(result.stderr, "partial stderr\n");
        let logs = fs::read_to_string(&logs_path).expect("logs are written");
        assert!(logs.contains("partial stdout"));
        assert!(logs.contains("partial stderr"));

        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn exit_poll_backoff_doubles_to_a_100ms_cap() {
        let mut backoff = Duration::from_millis(5);
        let mut schedule = Vec::new();
        for _ in 0..6 {
            backoff = exit_poll_backoff(backoff);
            schedule.push(backoff.as_millis());
        }
        assert_eq!(schedule, vec![10, 20, 40, 80, 100, 100]);
    }

    #[test]
    fn project_wayland_preference_requires_wayland_display() {
        let temp_dir = unique_temp_dir();
        fs::create_dir_all(&temp_dir).expect("create temp dir");
        fs::write(
            temp_dir.join("project.godot"),
            "config_version=5\n\n[application]\nrun/platforms/linuxbsd/prefer_wayland=true\n",
        )
        .expect("write project");

        let result = project_prefers_wayland(&temp_dir, false, false);

        let _ = fs::remove_dir_all(&temp_dir);

        assert!(!result);
    }

    #[test]
    fn project_wayland_preference_absent_or_false_preserves_default() {
        let absent_dir = unique_temp_dir();
        fs::create_dir_all(&absent_dir).expect("create absent project dir");
        fs::write(absent_dir.join("project.godot"), "config_version=5\n").expect("write absent");

        let false_dir = unique_temp_dir();
        fs::create_dir_all(&false_dir).expect("create false project dir");
        fs::write(
            false_dir.join("project.godot"),
            "config_version=5\n\n[application]\nrun/platforms/linuxbsd/prefer_wayland=false\n",
        )
        .expect("write false project");

        let absent = project_prefers_wayland(&absent_dir, false, true);
        let false_value = project_prefers_wayland(&false_dir, false, true);

        let _ = fs::remove_dir_all(&absent_dir);
        let _ = fs::remove_dir_all(&false_dir);

        assert!(!absent);
        assert!(!false_value);
    }

    #[test]
    fn launch_godot_import_includes_artifact_log_file() {
        let temp_dir = unique_temp_dir();
        let binary_path = fixture(&temp_dir.join("true"), &[]);
        let logs_path = temp_dir.join("logs.txt");

        let result = launch_godot_import(
            binary_path.to_str().expect("binary path"),
            &[],
            &temp_dir,
            &logs_path,
        )
        .expect("run fake import binary");

        let _ = fs::remove_dir_all(&temp_dir);

        assert_eq!(
            result.command,
            vec![
                binary_path.to_string_lossy().into_owned(),
                "--headless".to_string(),
                "--log-file".to_string(),
                temp_dir.join("godot.log").to_string_lossy().into_owned(),
                "--path".to_string(),
                temp_dir.to_string_lossy().into_owned(),
                "--import".to_string(),
            ]
        );
    }

    #[test]
    fn launch_godot_check_includes_artifact_log_file() {
        let temp_dir = unique_temp_dir();
        let binary_path = fixture(&temp_dir.join("true"), &[]);
        let request_path = temp_dir.join("request.json");
        let logs_path = temp_dir.join("logs.txt");

        let result = launch_godot_check(
            binary_path.to_str().expect("binary path"),
            &[],
            &[],
            &[],
            &temp_dir,
            &request_path,
            &logs_path,
        )
        .expect("run fake check binary");

        let _ = fs::remove_dir_all(&temp_dir);

        assert_eq!(
            result.command,
            vec![
                binary_path.to_string_lossy().into_owned(),
                "--headless".to_string(),
                "--log-file".to_string(),
                temp_dir.join("godot.log").to_string_lossy().into_owned(),
                "--path".to_string(),
                temp_dir.to_string_lossy().into_owned(),
                "--script".to_string(),
                "res://addons/godot_qa/qa_runner.gd".to_string(),
                "--".to_string(),
                "--qa".to_string(),
                format!("--qa-check-request={}", request_path.to_string_lossy()),
            ]
        );
    }

    #[test]
    fn launch_godot_patch_runner_includes_artifact_log_file() {
        let temp_dir = unique_temp_dir();
        let binary_path = fixture(&temp_dir.join("true"), &[]);
        let request_path = temp_dir.join("request.json");
        let logs_path = temp_dir.join("logs.txt");

        let result = launch_godot_patch_runner(
            binary_path.to_str().expect("binary path"),
            &[],
            &[],
            &temp_dir,
            &request_path,
            &logs_path,
        )
        .expect("run fake patch binary");

        let _ = fs::remove_dir_all(&temp_dir);

        assert_eq!(
            result.command,
            vec![
                binary_path.to_string_lossy().into_owned(),
                "--headless".to_string(),
                "--log-file".to_string(),
                temp_dir.join("godot.log").to_string_lossy().into_owned(),
                "--path".to_string(),
                temp_dir.to_string_lossy().into_owned(),
                "--script".to_string(),
                "res://addons/godot_qa/qa_patch_runner.gd".to_string(),
                "--".to_string(),
                "--qa".to_string(),
                format!("--qa-patch-request={}", request_path.to_string_lossy()),
            ]
        );
    }

    #[cfg(unix)]
    #[test]
    fn launch_godot_session_starts_child_in_new_session() {
        let _signal_guard = PROCESS_SIGNAL_LOCK.lock().expect("signal test lock");
        let temp_dir = unique_temp_dir();
        fs::create_dir_all(&temp_dir).expect("create temp dir");
        let binary_path = temp_dir.join("fake-godot.sh");
        fs::write(&binary_path, "#!/bin/sh\nwhile true; do sleep 1; done\n")
            .expect("write fake session binary");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&binary_path, fs::Permissions::from_mode(0o755))
                .expect("chmod fake session binary");
        }
        let request_path = temp_dir.join("request.json");
        fs::write(&request_path, "{}\n").expect("write request");
        let logs_path = temp_dir.join("logs.txt");

        let launch = launch_godot_session(
            binary_path.to_str().expect("binary path"),
            &[],
            &[],
            Path::new("/tmp"),
            &request_path,
            &logs_path,
            43123,
            "session-token",
            true,
            false,
            Some(&[1280, 720]),
            None,
            &[],
            &[],
            &[],
            None,
        )
        .expect("launch fake session");

        let parent_sid = read_session_id(std::process::id() as i64);
        let child_sid = read_session_id(launch.pid);

        let stop_result = stop_godot_process(launch.pid, 1.0);
        let _ = fs::remove_dir_all(&temp_dir);

        stop_result.expect("stop fake session");
        assert_ne!(child_sid, parent_sid);
        assert_eq!(child_sid, launch.pid);
        assert_eq!(
            launch.command,
            vec![
                binary_path.to_string_lossy().into_owned(),
                "--headless".to_string(),
                "--log-file".to_string(),
                temp_dir.join("godot.log").to_string_lossy().into_owned(),
                "--path".to_string(),
                "/tmp".to_string(),
                "--script".to_string(),
                "res://addons/godot_qa/qa_session_runner.gd".to_string(),
                "--".to_string(),
                "--qa".to_string(),
                format!("--qa-session-request={}", request_path.to_string_lossy()),
                "--qa-session-port=43123".to_string(),
                "--qa-session-token=session-token".to_string(),
            ]
        );
    }

    #[test]
    fn launch_godot_session_omits_headless_when_windowed() {
        let temp_dir = unique_temp_dir();
        fs::create_dir_all(&temp_dir).expect("create temp dir");
        let binary_path = fixture(&temp_dir.join("true"), &[]);
        let request_path = temp_dir.join("request.json");
        fs::write(&request_path, "{}\n").expect("write request");
        let logs_path = temp_dir.join("logs.txt");

        let launch = launch_godot_session(
            binary_path.to_str().expect("binary path"),
            &[],
            &[],
            Path::new("/tmp"),
            &request_path,
            &logs_path,
            43124,
            "session-token",
            false,
            true,
            Some(&[1280, 720]),
            None,
            &[],
            &[],
            &["--save-slot".to_string(), "qa profile".to_string()],
            None,
        )
        .expect("launch fake session");

        let _ = fs::remove_dir_all(&temp_dir);

        assert!(!launch.command.contains(&"--headless".to_string()));
        let expected_suffix = vec![
            binary_path.to_string_lossy().into_owned(),
            "--resolution".to_string(),
            "1280x720".to_string(),
            "--log-file".to_string(),
            temp_dir.join("godot.log").to_string_lossy().into_owned(),
            "--path".to_string(),
            "/tmp".to_string(),
            "--script".to_string(),
            "res://addons/godot_qa/qa_session_runner.gd".to_string(),
            "--".to_string(),
            "--qa".to_string(),
            format!("--qa-session-request={}", request_path.to_string_lossy()),
            "--qa-session-port=43124".to_string(),
            "--qa-session-token=session-token".to_string(),
            "--save-slot".to_string(),
            "qa profile".to_string(),
        ];
        assert!(
            command_has_godot_suffix(&launch.command, &expected_suffix),
            "windowed session command keeps expected Godot argv suffix"
        );
    }

    #[test]
    fn launch_godot_session_inserts_fixed_fps_before_path() {
        let temp_dir = unique_temp_dir();
        fs::create_dir_all(&temp_dir).expect("create temp dir");
        let binary_path = fixture(&temp_dir.join("true"), &[]);
        let request_path = temp_dir.join("request.json");
        fs::write(&request_path, "{}\n").expect("write request");
        let logs_path = temp_dir.join("logs.txt");

        let launch = launch_godot_session(
            binary_path.to_str().expect("binary path"),
            &[],
            &[],
            Path::new("/tmp"),
            &request_path,
            &logs_path,
            43125,
            "session-token",
            true,
            false,
            Some(&[1280, 720]),
            Some(120),
            &[],
            &[],
            &[],
            None,
        )
        .expect("launch fake session");

        let _ = fs::remove_dir_all(&temp_dir);
        let fixed_fps_position = launch
            .command
            .iter()
            .position(|arg| arg == "--fixed-fps")
            .expect("fixed fps arg");
        let path_position = launch
            .command
            .iter()
            .position(|arg| arg == "--path")
            .expect("path arg");
        assert!(fixed_fps_position < path_position);
        assert_eq!(
            &launch.command[fixed_fps_position..fixed_fps_position + 3],
            ["--fixed-fps", "120", "--disable-vsync"]
        );
    }

    #[cfg(windows)]
    #[test]
    fn windows_process_identity_prevents_stale_termination() {
        let root = unique_temp_dir();
        let binary = fixture(&root.join("sleep"), &[("sleep_ms", "30000")]);
        let mut child = std::process::Command::new(binary).spawn().unwrap();
        let pid = i64::from(child.id());
        let identity = super::process_identity(pid).expect("creation identity");
        assert!(super::process_identity_matches(pid, Some(identity)));
        assert!(!super::process_identity_matches(pid, Some(identity + 1)));
        assert!(super::terminate_process_tree(pid, true, Some(identity + 1)).is_err());
        assert!(
            super::pid_is_running(pid),
            "mismatched identity must not terminate process"
        );
        assert!(super::stop_godot_process_with_identity(pid, 1.0, None).is_err());
        super::stop_godot_process_with_identity(pid, 1.0, Some(identity)).unwrap();
        child.wait().unwrap();
        assert!(!super::pid_is_running(pid));
        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(windows)]
    #[test]
    fn windows_owned_job_reaps_descendant_after_wrapper_exits() {
        let root = unique_temp_dir();
        let child = fixture(&root.join("descendant"), &[("sleep_ms", "30000")]);
        let pid_file = root.join("child.pid");
        let wrapper = fixture(
            &root.join("wrapper"),
            &[
                ("child_exe", child.to_str().unwrap()),
                ("child_pid_file", pid_file.to_str().unwrap()),
                ("stdout", "wrapper done"),
            ],
        );
        let output = super::run_owned_child_output(
            &[wrapper.to_string_lossy().into_owned()],
            Some(Duration::from_secs(5)),
            None,
            None,
        )
        .unwrap();
        let pid = fs::read_to_string(pid_file).unwrap().parse().unwrap();
        assert_eq!(output.status_code, 0);
        assert_eq!(output.stdout, b"wrapper done");
        assert!(super::wait_for_godot_process_exit(pid, 2.0));
        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(windows)]
    #[test]
    fn windows_detached_tree_survives_launcher_and_stops_completely() {
        let root = unique_temp_dir();
        let child = fixture(&root.join("descendant"), &[("sleep_ms", "30000")]);
        let pid_file = root.join("child.pid");
        let wrapper = fixture(
            &root.join("wrapper"),
            &[
                ("child_exe", child.to_str().unwrap()),
                ("child_pid_file", pid_file.to_str().unwrap()),
                ("sleep_ms", "30000"),
            ],
        );
        let mut command = std::process::Command::new(wrapper);
        super::configure_background_command(&mut command);
        command
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null());
        let child = command.spawn().unwrap();
        let pid = i64::from(child.id());
        let identity = super::process_identity(pid).unwrap();
        drop(child); // Like session start: no parent handle remains to keep it alive.
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while !pid_file.exists() && std::time::Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
        let descendant = fs::read_to_string(pid_file).unwrap().parse().unwrap();
        assert!(super::pid_is_running(pid));
        assert!(super::pid_is_running(descendant));
        super::stop_godot_process_with_identity(pid, 2.0, Some(identity)).unwrap();
        assert!(!super::pid_is_running(descendant));
        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(windows)]
    #[test]
    fn windows_display_and_executable_discovery_are_native() {
        let root = unique_temp_dir();
        fs::create_dir_all(&root).unwrap();
        let mut command = vec!["godot".into()];
        let display = append_project_display_driver_args_with_env(
            &mut command,
            &root,
            &[],
            &[],
            false,
            false,
            false,
            None,
        );
        assert_eq!(display.selected_display_driver.as_deref(), Some("windows"));
        assert!(display.display_backend_available);
        assert_eq!(
            super::executable_candidates(&root, "godot")[0],
            root.join("godot.exe")
        );
        assert_eq!(command, vec!["godot"]);
        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    fn read_session_id(pid: i64) -> i64 {
        let stat_path = format!("/proc/{pid}/stat");
        let contents = fs::read_to_string(stat_path).expect("read stat");
        contents
            .split_whitespace()
            .nth(5)
            .and_then(|value| value.parse::<i64>().ok())
            .expect("parse session id")
    }

    fn unique_temp_dir() -> std::path::PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let count = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
        env::temp_dir().join(format!(
            "godot-qa-process-test-{}-{unique}-{count}",
            std::process::id()
        ))
    }

    #[cfg(unix)]
    fn write_executable(path: &Path, contents: &str) -> std::path::PathBuf {
        let mut file = File::create(path).expect("create executable");
        file.write_all(contents.as_bytes())
            .expect("write executable");
        file.sync_all().expect("sync executable");
        drop(file);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(path, fs::Permissions::from_mode(0o755)).expect("chmod executable");
        }
        path.to_path_buf()
    }
}
