use godot_qa_core::results::FailureDetail;
use serde_json::json;
use std::env;
use std::io::Read;
#[cfg(unix)]
use std::os::unix::process::CommandExt;
use std::path::Path;
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

#[cfg(windows)]
use super::windows;

const GODOT_VERSION_PROBE_FAILED: &str = "godot_version_probe_failed";
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
    let output = run_safe_probe(
        Path::new(&command[0]),
        &command[1..],
        Duration::from_secs(2),
    );
    let stdout = output.stdout.trim().to_string();
    let stderr = output.stderr.trim().to_string();
    let version = if !stdout.is_empty() { &stdout } else { &stderr };
    let failure_reason = if output.timed_out {
        Some("timeout".to_string())
    } else if let Some(error) = output.error_message {
        Some(format!("os_error: {error}"))
    } else if output.status != Some(0) {
        Some(if version.is_empty() {
            format!("exited with status {}", output.status.unwrap_or(-1))
        } else {
            version.clone()
        })
    } else if version.is_empty() {
        Some("empty_output".to_string())
    } else {
        None
    };
    GodotVersionProbeAttempt {
        command: command.to_vec(),
        used_launch_prefix,
        status: output.status,
        stdout,
        stderr,
        failure_type: failure_reason
            .as_ref()
            .map(|_| GODOT_VERSION_PROBE_FAILED.to_string()),
        failure_reason,
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
    let mut child = match spawn_probe_child(&mut child_command) {
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
    let stdout = ProbeCapture::spawn(child.stdout.take().expect("piped stdout"));
    let stderr = ProbeCapture::spawn(child.stderr.take().expect("piped stderr"));
    let mut status = None;
    let mut error = None;
    let mut timed_out = false;
    loop {
        if status.is_none() {
            match child.try_wait() {
                Ok(Some(exited)) => {
                    status = Some(exited);
                    // The leader may leave descendants holding the pipes open.
                    #[cfg(windows)]
                    let cleanup = owned_job.terminate().err();
                    #[cfg(not(windows))]
                    let cleanup = kill_safe_probe_child(&mut child);
                    error = cleanup;
                }
                Ok(None) => {}
                Err(wait_error) => {
                    error = Some(wait_error);
                    break;
                }
            }
        }
        if status.is_some() && stdout.worker.is_finished() && stderr.worker.is_finished() {
            break;
        }
        if started.elapsed() >= timeout {
            timed_out = true;
            break;
        }
        thread::sleep(Duration::from_millis(1));
    }
    if status.is_none() {
        #[cfg(windows)]
        let _ = owned_job.terminate();
        if let Some(kill_error) = kill_safe_probe_child(&mut child) {
            error.get_or_insert(kill_error);
        } else {
            match child.wait() {
                Ok(exited) => status = Some(exited),
                Err(wait_error) => {
                    error.get_or_insert(wait_error);
                }
            }
        }
    }
    // Never join a reader still waiting for EOF: even pipe collection shares the deadline.
    let (stdout, stdout_error) = stdout.finish();
    let (stderr, stderr_error) = stderr.finish();
    error = error.or(stdout_error).or(stderr_error);
    SafeProbeResult {
        command,
        status: status.and_then(|status| status.code()),
        stdout,
        stderr,
        duration: started.elapsed(),
        timed_out,
        error_kind: if timed_out {
            Some("timeout".to_string())
        } else {
            error.as_ref().map(|error| format!("{:?}", error.kind()))
        },
        error_message: if timed_out {
            Some(error.map_or_else(
                || "timed out".to_string(),
                |error| format!("timed out; cleanup failed: {error}"),
            ))
        } else {
            error.map(|error| error.to_string())
        },
    }
}

struct ProbeCapture {
    bytes: std::sync::Arc<std::sync::Mutex<Vec<u8>>>,
    worker: thread::JoinHandle<std::io::Result<()>>,
}

impl ProbeCapture {
    fn spawn(mut stream: impl Read + Send + 'static) -> Self {
        const LIMIT: usize = 64 * 1024;
        let bytes = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let captured = bytes.clone();
        let worker = thread::spawn(move || {
            let mut buffer = [0; 8192];
            loop {
                let count = match stream.read(&mut buffer) {
                    Ok(0) => return Ok(()),
                    Ok(count) => count,
                    Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
                    Err(error) => return Err(error),
                };
                let mut bytes = captured.lock().expect("probe capture lock");
                let retained = count.min(LIMIT - bytes.len());
                bytes.extend_from_slice(&buffer[..retained]);
            }
        });
        Self { bytes, worker }
    }

    fn finish(self) -> (String, Option<std::io::Error>) {
        let error = if self.worker.is_finished() {
            self.worker.join().expect("probe reader thread").err()
        } else {
            None
        };
        let bytes = self.bytes.lock().expect("probe capture lock");
        (String::from_utf8_lossy(&bytes).into_owned(), error)
    }
}

fn kill_safe_probe_child(child: &mut std::process::Child) -> Option<std::io::Error> {
    #[cfg(unix)]
    {
        // Signal only the session we created, including after its leader has exited.
        if unsafe { libc::kill(-(child.id() as libc::pid_t), libc::SIGKILL) } == 0 {
            return None;
        }
        let error = std::io::Error::last_os_error();
        (error.raw_os_error() != Some(libc::ESRCH)).then_some(error)
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
pub(super) fn effective_godot_command(
    launch_prefix: &[String],
    binary: &str,
    args: &[String],
) -> Vec<String> {
    let mut command = Vec::with_capacity(launch_prefix.len() + 1 + args.len());
    command.extend(launch_prefix.iter().cloned());
    command.push(binary.to_string());
    command.extend(args.iter().cloned());
    command
}
pub(super) fn spawn_probe_child(command: &mut Command) -> std::io::Result<std::process::Child> {
    for attempt in 0..3 {
        match command.spawn() {
            Ok(child) => return Ok(child),
            Err(error) if error.raw_os_error() == Some(26) && attempt < 2 => {
                thread::sleep(Duration::from_millis(10));
            }
            Err(error) => return Err(error),
        }
    }
    unreachable!("final spawn attempt returns its result")
}
