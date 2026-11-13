use godot_qa_core::results::FailureDetail;
use serde_json::json;
use std::fs;
use std::process::Command;
use std::thread;
use std::time::{Duration, Instant};

#[cfg(unix)]
use super::owned_group;
use super::proc_stat;
#[cfg(windows)]
use super::windows;

#[derive(Debug, Clone)]
pub struct ProcessCleanupResult {
    pub status: String,
    pub failure_kind: Option<String>,
    pub pid: Option<i64>,
    pub process_group: Option<i64>,
    pub owned: bool,
    pub message: Option<String>,
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

/// Reap a background child without blocking the caller or keeping the CLI alive.
/// The child remains detached when the launching CLI exits.
pub fn reap_background_child(mut child: std::process::Child) {
    thread::spawn(move || {
        let _ = child.wait();
    });
}

/// Process birth identity, persisted with session metadata (Linux start ticks or Windows FILETIME).
pub fn process_identity(pid: i64) -> Option<u64> {
    #[cfg(target_os = "linux")]
    {
        let stat = fs::read(format!("/proc/{pid}/stat")).ok()?;
        proc_stat::fields(&stat)?.nth(19)?.parse().ok()
    }
    #[cfg(windows)]
    {
        windows::identity(pid)
    }
    #[cfg(not(any(target_os = "linux", windows)))]
    {
        let _ = pid;
        None
    }
}

pub fn process_identity_matches(pid: i64, expected: Option<u64>) -> bool {
    pid_is_running(pid) && expected.is_none_or(|expected| process_identity(pid) == Some(expected))
}

/// Sleep schedule between process-exit poll attempts, mirroring
/// `startup_probe_backoff` in the runtime transport: start fine-grained so a
/// process that exits promptly is observed within ~5ms instead of the old flat
/// 100ms poll (which added up to ~100ms of dead time to every stop), doubling
/// back to the same 100ms pace for processes that linger.
pub(super) fn exit_poll_backoff(previous: Duration) -> Duration {
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

#[cfg(unix)]
pub(super) fn terminate_owned_child_after_timeout(
    child: &mut std::process::Child,
    pid: i64,
    process_group: Option<i64>,
) -> ProcessCleanupResult {
    let (mut status, mut failure_kind, mut message) = match owned_group::terminate(pid) {
        Ok(killed) => (
            if killed {
                "timeout_killed"
            } else {
                "timeout_terminated"
            },
            "owned_process_timeout",
            format!(
                "owned Godot process group exceeded configured timeout and was {}",
                if killed { "killed" } else { "terminated" }
            ),
        ),
        Err(error) => (
            "cleanup_failed",
            "owned_process_timeout_cleanup_failed",
            format!("owned process group {pid} timeout cleanup failed: {error}"),
        ),
    };
    let deadline = Instant::now() + Duration::from_secs(1);
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if Instant::now() < deadline => {
                thread::sleep(Duration::from_millis(10));
                continue;
            }
            Ok(None) => message.push_str("; owned leader did not exit within 1 second"),
            Err(error) => message.push_str(&format!("; owned leader exit check failed: {error}")),
        }
        status = "cleanup_failed";
        failure_kind = "owned_process_timeout_cleanup_failed";
        break;
    }
    ProcessCleanupResult {
        status: status.to_string(),
        failure_kind: Some(failure_kind.to_string()),
        pid: Some(pid),
        process_group,
        owned: true,
        message: Some(message),
    }
}

#[cfg(not(unix))]
pub(super) fn terminate_owned_child_after_timeout(
    child: &mut std::process::Child,
    pid: i64,
    process_group: Option<i64>,
) -> ProcessCleanupResult {
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

#[cfg(test)]
pub(super) fn stale_owned_child_cleanup_failure(
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

pub(super) fn is_zombie_process(pid: i64) -> bool {
    let stat_path = format!("/proc/{pid}/stat");
    if let Ok(contents) = fs::read(stat_path) {
        return proc_stat::fields(&contents).and_then(|mut fields| fields.next()) == Some("Z");
    }
    false
}

pub(super) fn current_run_process_group(pid: i64) -> Option<i64> {
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
        let signal = if force { libc::SIGKILL } else { libc::SIGTERM };
        if let Some(expected) = expected {
            let actual = process_identity(pid);
            if actual != Some(expected) {
                return Err(format!(
                    "refusing to signal pid {pid}: expected process birth identity {expected}, actual {actual:?}"
                ));
            }
            if current_run_process_group(pid) == Some(pid) {
                return send_process_group_signal(pid, signal);
            }
        }
        // Legacy records establish only a PID, never ownership of its group.
        send_signal(pid, signal)
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = (pid, force, expected);
        Err("process termination unsupported on this platform".into())
    }
}

#[cfg(unix)]
pub(super) fn send_signal(pid: i64, signal: i32) -> Result<(), String> {
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
pub(super) fn send_process_group_signal(pid: i64, signal: i32) -> Result<(), String> {
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
