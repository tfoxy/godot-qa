use std::fs::{self, File};
use std::io::{Read, Seek, SeekFrom, Write};
#[cfg(unix)]
use std::os::unix::process::CommandExt;
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::thread;
use std::time::{Duration, Instant};

#[cfg(unix)]
use super::lifecycle::send_process_group_signal;
use super::lifecycle::{
    ProcessCleanupResult, current_run_process_group, reap_background_child,
    terminate_owned_child_after_timeout,
};
#[cfg(unix)]
use super::owned_group;
use super::probes::spawn_probe_child;
use super::project_access;
#[cfg(windows)]
use super::windows;

pub(super) const CAPTURED_STREAM_LIMIT_BYTES: u64 = 8 * 1024 * 1024;

#[derive(Debug, Clone, serde::Serialize)]
pub struct OutputTruncation {
    pub stdout_total_bytes: u64,
    pub stderr_total_bytes: u64,
    pub retained_bytes_per_stream: u64,
    pub raw_stdout_path: String,
    pub raw_stderr_path: String,
}

/// Native status of the process launched by the CLI (possibly a wrapper).
#[derive(Debug, Clone, serde::Serialize)]
pub struct ProcessTermination {
    pub scope: &'static str,
    pub exit_code: Option<i32>,
    pub signal: Option<i32>,
    pub core_dumped: Option<bool>,
    pub native_status: String,
    pub tool_initiated: bool,
}

/// CLI monotonic observations; these do not claim to measure engine CPU time.
#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct ProcessTiming {
    pub spawn_ms: u64,
    pub runtime_started_observed_ms: Option<u64>,
    pub result_observed_ms: Option<u64>,
    pub post_result_exit_ms: Option<u64>,
    pub cleanup_ms: u64,
    pub total_ms: u64,
}

pub(super) struct OwnedChildOutput {
    pub(super) status_code: i32,
    pub(super) termination: Option<ProcessTermination>,
    pub(super) process_timing: ProcessTiming,
    pub(super) stdout: Vec<u8>,
    pub(super) stderr: Vec<u8>,
    pub(super) pid: Option<i64>,
    pub(super) process_group: Option<i64>,
    pub(super) cleanup: Option<ProcessCleanupResult>,
    pub(super) timed_out: bool,
    pub(super) timeout_ms: Option<u64>,
    pub(super) output_truncation: Option<OutputTruncation>,
}

/// Override SDL's physical-controller allowlist only for opted-in desktop children.
/// Leaving the command environment untouched preserves caller values when disabled.
pub(super) fn apply_joypad_isolation(command: &mut Command, enabled: bool) {
    if enabled {
        command.env("SDL_GAMECONTROLLER_IGNORE_DEVICES_EXCEPT", "0xffff/0xffff");
    }
}

#[cfg(test)]
pub(super) fn run_owned_child_output(
    command: &[String],
    joypad_isolation: bool,
    timeout: Option<Duration>,
    completion_path: Option<&Path>,
    file_capture_path: Option<&Path>,
) -> std::io::Result<OwnedChildOutput> {
    run_owned_child_output_with_timeout_monitor(
        command,
        joypad_isolation,
        timeout,
        completion_path,
        file_capture_path,
        None,
        None,
    )
}

pub(super) fn run_owned_child_output_with_timeout_monitor(
    command: &[String],
    joypad_isolation: bool,
    timeout: Option<Duration>,
    completion_path: Option<&Path>,
    file_capture_path: Option<&Path>,
    mut timeout_monitor: Option<&mut dyn FnMut() -> Option<Duration>>,
    input: Option<&[u8]>,
) -> std::io::Result<OwnedChildOutput> {
    let started = Instant::now();
    let expected = completion_path.and_then(CompletionIdentity::read);
    let Some(executable) = command.first() else {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "empty command",
        ));
    };
    let mut child_command = Command::new(executable);
    apply_joypad_isolation(&mut child_command, joypad_isolation);
    child_command.args(command.iter().skip(1));
    if input.is_some() {
        child_command.stdin(Stdio::piped());
    }
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
    let mut spawned_child = spawn_probe_child(&mut child_command)?;
    let mut process_timing = ProcessTiming {
        spawn_ms: duration_millis_u64(started.elapsed()),
        ..ProcessTiming::default()
    };
    #[cfg(windows)]
    let owned_job = match windows::OwnedJob::attach(&spawned_child) {
        Ok(job) => job,
        Err(error) => {
            let _ = spawned_child.kill();
            let _ = spawned_child.wait();
            return Err(error);
        }
    };
    let pid = spawned_child.id() as i64;
    let _project_child = match project_access::register_child(pid) {
        Ok(receipt) => receipt,
        Err(error) => {
            let _ = spawned_child.kill();
            let _ = spawned_child.wait();
            return Err(error);
        }
    };
    let process_group = current_run_process_group(pid);
    // Keep ownership across every fallible poll, stdin write, and output read.
    let mut cleanup_guard = OwnedChildErrorCleanup {
        child: &mut spawned_child,
        armed: true,
    };
    let child = &mut *cleanup_guard.child;
    let mut input_writer = input.map(|input| {
        let mut stdin = child.stdin.take().expect("piped stdin is available");
        let input = input.to_vec();
        let (sender, receiver) = mpsc::channel();
        thread::spawn(move || {
            let _ = sender.send(stdin.write_all(&input));
        });
        receiver
    });
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
    let mut result_observed_at = None;
    loop {
        if let Some(writer) = &input_writer {
            match writer.try_recv() {
                Ok(result) => {
                    result?;
                    input_writer = None;
                }
                Err(mpsc::TryRecvError::Empty) => {}
                Err(mpsc::TryRecvError::Disconnected) => {
                    return Err(std::io::Error::other(
                        "owned child stdin writer disconnected",
                    ));
                }
            }
        }
        let monitored_timeout = timeout_monitor.as_mut().and_then(|monitor| monitor());
        if process_timing.runtime_started_observed_ms.is_none()
            && expected
                .as_ref()
                .is_some_and(CompletionIdentity::runtime_started)
        {
            process_timing.runtime_started_observed_ms =
                Some(duration_millis_u64(started.elapsed()));
        }
        if let Some(status) = child.try_wait()? {
            let exit_observed = Instant::now();
            // A fast child may exit between polls; inspect its final result too.
            if result_observed_at.is_none()
                && completion_path
                    .zip(expected.as_ref())
                    .is_some_and(|(path, identity)| identity.matches(path))
            {
                result_observed_at = Some(exit_observed);
                process_timing.result_observed_ms =
                    Some(duration_millis_u64(exit_observed.duration_since(started)));
            }
            process_timing.post_result_exit_ms =
                result_observed_at.map(|at| duration_millis_u64(exit_observed.duration_since(at)));
            #[cfg(windows)]
            owned_job.terminate()?;
            #[cfg(unix)]
            {
                let cleanup_started = Instant::now();
                owned_group::terminate(pid).map_err(|error| {
                    std::io::Error::new(
                        error.kind(),
                        format!(
                            "owned process group {pid} cleanup failed after leader exit: {error}"
                        ),
                    )
                })?;
                process_timing.cleanup_ms = duration_millis_u64(cleanup_started.elapsed());
            }
            if let Some(writer) = input_writer.take() {
                let written = writer.recv_timeout(Duration::from_secs(3)).map_err(|_| {
                    std::io::Error::new(
                        std::io::ErrorKind::TimedOut,
                        "owned child stdin writer did not finish",
                    )
                })?;
                // Group cleanup can close stdin held open by a descendant after
                // the leader exits. Preserve that leader's native result.
                if let Err(error) = written
                    && error.kind() != std::io::ErrorKind::BrokenPipe
                {
                    return Err(error);
                }
            }
            let (stdout, stderr, output_truncation) =
                collected_child_output(&file_paths, stdout_reader, stderr_reader)?;
            process_timing.total_ms = duration_millis_u64(started.elapsed());
            cleanup_guard.armed = false;
            return Ok(OwnedChildOutput {
                status_code: status.code().unwrap_or(-1),
                termination: Some(process_termination(status, false)),
                process_timing,
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
                timed_out: monitored_timeout.is_some(),
                timeout_ms: monitored_timeout.or(timeout).map(duration_millis_u64),
                output_truncation,
            });
        }
        if completion_deadline.is_none()
            && deadline.is_none_or(|deadline| Instant::now() < deadline)
            && completion_path
                .zip(expected.as_ref())
                .is_some_and(|(path, identity)| identity.matches(path))
        {
            let observed = Instant::now();
            result_observed_at = Some(observed);
            process_timing.result_observed_ms = Some(duration_millis_u64(started.elapsed()));
            completion_deadline = Some(observed + Duration::from_secs(3));
        }
        // Once a current result has been written, allow the complete exit grace.
        // The declared deadline remains hard until that result exists.
        let overall_expired = completion_deadline.is_none()
            && deadline.is_some_and(|deadline| Instant::now() >= deadline);
        let completion_expired =
            completion_deadline.is_some_and(|deadline| Instant::now() >= deadline);
        if overall_expired || completion_expired || monitored_timeout.is_some() {
            let cleanup_started = Instant::now();
            #[cfg(windows)]
            owned_job.terminate()?;
            let cleanup = terminate_owned_child_after_timeout(child, pid, process_group);
            #[cfg(unix)]
            let termination = child
                .try_wait()
                .ok()
                .flatten()
                .map(|status| process_termination(status, true));
            #[cfg(not(unix))]
            let termination = child
                .wait()
                .ok()
                .map(|status| process_termination(status, true));
            let cleanup_finished = Instant::now();
            process_timing.cleanup_ms =
                duration_millis_u64(cleanup_finished.duration_since(cleanup_started));
            process_timing.post_result_exit_ms = result_observed_at
                .map(|at| duration_millis_u64(cleanup_finished.duration_since(at)));
            cleanup_guard.armed = false;
            drop(cleanup_guard);
            if termination.is_none() {
                // A child stuck after SIGKILL must not block the operation.
                reap_background_child(spawned_child);
            }
            let (stdout, stderr, output_truncation) =
                collected_child_output(&file_paths, stdout_reader, stderr_reader)?;
            process_timing.total_ms = duration_millis_u64(started.elapsed());
            return Ok(OwnedChildOutput {
                status_code: -1,
                termination,
                process_timing,
                stdout,
                stderr,
                pid: Some(pid),
                process_group,
                cleanup: Some(cleanup),
                timed_out: true,
                timeout_ms: Some(if let Some(budget) = monitored_timeout {
                    duration_millis_u64(budget)
                } else if completion_expired {
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

struct OwnedChildErrorCleanup<'a> {
    child: &'a mut std::process::Child,
    armed: bool,
}

impl Drop for OwnedChildErrorCleanup<'_> {
    fn drop(&mut self) {
        if self.armed {
            #[cfg(unix)]
            let _ = send_process_group_signal(self.child.id() as i64, libc::SIGKILL);
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }
}

pub(super) struct CompletionIdentity {
    run_id: String,
    nonce: String,
    progress_path: Option<std::path::PathBuf>,
}

impl CompletionIdentity {
    pub(super) fn read(result_path: &Path) -> Option<Self> {
        let request_bytes = fs::read(result_path.parent()?.join("request.json")).ok()?;
        let request: serde_json::Value = serde_json::from_slice(&request_bytes).ok()?;
        let run = request.get("run")?;
        Some(Self {
            run_id: run.get("id")?.as_str()?.to_owned(),
            nonce: run.get("nonce")?.as_str()?.to_owned(),
            progress_path: request
                .get("artifacts")
                .and_then(|artifacts| {
                    artifacts
                        .get("progress")
                        .or_else(|| artifacts.get("progress_path"))
                })
                .and_then(serde_json::Value::as_str)
                .map(Into::into),
        })
    }

    pub(super) fn matches(&self, result_path: &Path) -> bool {
        let Ok(result_bytes) = fs::read(result_path) else {
            return false;
        };
        let Ok(result) = serde_json::from_slice::<serde_json::Value>(&result_bytes) else {
            return false;
        };
        result
            .get("run_identity")
            .is_some_and(|identity| self.matches_identity(identity, "run_id"))
            && result
                .get("ok")
                .and_then(serde_json::Value::as_bool)
                .is_some()
    }

    fn matches_identity(&self, identity: &serde_json::Value, id_field: &str) -> bool {
        identity.get(id_field).and_then(serde_json::Value::as_str) == Some(self.run_id.as_str())
            && identity.get("nonce").and_then(serde_json::Value::as_str)
                == Some(self.nonce.as_str())
    }

    pub(super) fn runtime_started(&self) -> bool {
        let Some(path) = &self.progress_path else {
            return false;
        };
        let Ok(file) = File::open(path) else {
            return false;
        };
        // The marker is the first executor record. Bound reads while the game runs.
        let mut bytes = Vec::new();
        if file.take(4096).read_to_end(&mut bytes).is_err() {
            return false;
        }
        let contents = String::from_utf8_lossy(&bytes);
        contents
            .lines()
            .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
            .any(|event| {
                event.get("event").and_then(serde_json::Value::as_str) == Some("runtime_started")
                    && self.matches_identity(&event, "run_id")
            })
    }
}

fn process_termination(
    status: std::process::ExitStatus,
    tool_initiated: bool,
) -> ProcessTermination {
    #[cfg(unix)]
    let (signal, core_dumped) = {
        use std::os::unix::process::ExitStatusExt;
        (status.signal(), Some(status.core_dumped()))
    };
    #[cfg(not(unix))]
    let (signal, core_dumped) = (None, None);
    ProcessTermination {
        scope: "direct_child",
        exit_code: status.code(),
        signal,
        core_dumped,
        native_status: status.to_string(),
        tool_initiated,
    }
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

pub(super) fn read_bounded_capture(path: &Path) -> std::io::Result<(Vec<u8>, u64)> {
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
