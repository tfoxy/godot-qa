//! Owned, isolated compositor capture for the first presented live-session frame.
//! The public command starts Gamescope; only its fixed child helper can capture.

use crate::{Cli, PresentedHelperArgs, PresentedStartupArgs};
use godot_qa_core::config::{ProjectConfig, load_project_config};
use godot_qa_core::hash::{Sha256, to_lower_hex};
use godot_qa_core::results::{CommandResult, FailureDetail};
use godot_qa_runtime as runtime;
use godot_qa_scenario::screenshots;
use image::GenericImageView;
use serde_json::{Map, Value, json};
use std::fs::{self, File};
use std::io::{self, Read};
use std::os::fd::AsRawFd;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const COMMAND: &str = "screenshot assert-startup-presented";
const WRAPPER_TIMEOUT: Duration = Duration::from_secs(55);
const SETTLE: Duration = Duration::from_secs(2);

fn failure(
    kind: &str,
    message: impl Into<String>,
    expected: Value,
    actual: Value,
    out: &Path,
) -> CommandResult {
    let mut result = CommandResult::success(COMMAND, "presented startup assertion failed");
    result.ok = false;
    result.exit_code = 1;
    result.data.insert("artifacts".into(), json!({"root": out}));
    result.failures.push(FailureDetail {
        kind: kind.into(),
        command: Some(COMMAND.into()),
        message: Some(message.into()),
        expected: Some(expected),
        actual: Some(actual),
        path: Some(out.display().to_string()),
        ..FailureDetail::default()
    });
    result
}

fn parse_viewport(raw: &str) -> Option<(u32, u32)> {
    let (w, h) = raw.split_once('x')?;
    let w = w.parse::<u32>().ok()?;
    let h = h.parse::<u32>().ok()?;
    (w > 0 && h > 0 && w <= 8192 && h <= 8192).then_some((w, h))
}

fn resolved_path(root: &Path, raw: &str) -> PathBuf {
    let path = Path::new(raw);
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        root.join(path)
    }
}

fn unique_out(config: &ProjectConfig) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    config
        .artifacts_dir
        .join("presented-startup")
        .join(format!("{nanos}-{}", std::process::id()))
}

pub(crate) fn assert_startup_presented(
    config: &ProjectConfig,
    args: &PresentedStartupArgs,
) -> CommandResult {
    let out = args
        .out
        .as_deref()
        .map(|value| resolved_path(&config.project_root, value))
        .unwrap_or_else(|| unique_out(config));
    let Some((w, h)) = parse_viewport(&args.viewport) else {
        return failure(
            "invalid_viewport",
            "viewport must be a positive WIDTHxHEIGHT up to 8192x8192",
            json!("WIDTHxHEIGHT"),
            json!(args.viewport),
            &out,
        );
    };
    if !args.threshold.is_finite() || !(0.0..=1.0).contains(&args.threshold) {
        return failure(
            "invalid_threshold",
            "threshold must be between 0 and 1",
            json!("0.0..=1.0"),
            json!(args.threshold),
            &out,
        );
    }
    if let Some(problem) = config.project_config_failure_for_command(COMMAND) {
        let mut result = failure(
            "project_config_invalid",
            "project configuration is invalid",
            json!("valid project configuration"),
            json!(problem.kind),
            &out,
        );
        result.failures.push(problem);
        return result;
    }
    if !config.godot_launch_prefix.is_empty()
        || !config.godot_windowed_launch_prefix.is_empty()
        || config
            .godot_args
            .iter()
            .any(|arg| arg == "--display-driver" || arg.starts_with("--display-driver="))
    {
        return failure(
            "presented_capture_unsupported",
            "strict display ownership requires no configured launch wrapper or display-driver override",
            json!("direct Godot launch in owned Gamescope"),
            json!({"launch_prefix": config.godot_launch_prefix, "windowed_launch_prefix": config.godot_windowed_launch_prefix, "godot_args": config.godot_args}),
            &out,
        );
    }
    let baseline = args
        .baseline
        .as_deref()
        .map(|path| resolved_path(&config.project_root, path));
    if baseline.as_ref().is_some_and(|path| !path.is_file()) {
        return failure(
            "baseline_not_found",
            "presented baseline PNG is missing",
            json!("existing PNG"),
            json!(baseline),
            &out,
        );
    }
    if let Some(parent) = out.parent() {
        if let Err(error) = fs::create_dir_all(parent) {
            return failure(
                "artifact_write_failed",
                error.to_string(),
                json!("writable output parent"),
                json!(parent),
                &out,
            );
        }
    }
    if let Err(error) = fs::create_dir(&out) {
        let kind = if error.kind() == io::ErrorKind::AlreadyExists {
            "artifact_dir_exists"
        } else {
            "artifact_write_failed"
        };
        return failure(
            kind,
            error.to_string(),
            json!("new writable output directory"),
            json!(out),
            &out,
        );
    }
    let Some(binary) = config.godot_binary.as_deref() else {
        return failure(
            "missing_godot",
            "Godot binary was not found",
            json!("Godot binary"),
            json!("missing"),
            &out,
        );
    };
    let executable = match std::env::current_exe() {
        Ok(path) => path,
        Err(error) => {
            return failure(
                "presented_capture_unsupported",
                error.to_string(),
                json!("current CLI executable"),
                json!("unavailable"),
                &out,
            );
        }
    };
    let mut nonce_bytes = [0_u8; 16];
    if let Err(error) =
        File::open("/dev/urandom").and_then(|mut file| file.read_exact(&mut nonce_bytes))
    {
        return failure(
            "presented_capture_unsupported",
            error.to_string(),
            json!("random run nonce"),
            json!("unavailable"),
            &out,
        );
    }
    let nonce = nonce_bytes
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    let owner_path = out.join("owner.json");
    let result_path = out.join("helper-result.json");
    let log_path = out.join("gamescope.log");
    let log = match File::create(&log_path) {
        Ok(file) => file,
        Err(error) => {
            return failure(
                "artifact_write_failed",
                error.to_string(),
                json!("writable Gamescope log"),
                json!(log_path),
                &out,
            );
        }
    };
    let stderr = match log.try_clone() {
        Ok(file) => file,
        Err(error) => {
            return failure(
                "artifact_write_failed",
                error.to_string(),
                json!("writable Gamescope log"),
                json!(log_path),
                &out,
            );
        }
    };
    let _project_lease = match godot_qa_process::project_access::LaunchLease::acquire(
        &config.project_root,
        godot_qa_process::project_access::LaunchClass::RuntimePreview,
    ) {
        Ok(lease) => lease,
        Err(problem) => {
            let mut result = failure(
                &problem.kind,
                "project is reserved by an asset trial",
                json!("compatible project lease"),
                json!(problem.actual),
                &out,
            );
            result.failures = vec![problem];
            return result;
        }
    };
    let mut cmd = Command::new("gamescope");
    cmd.args([
        "--backend",
        "headless",
        "--expose-wayland",
        "-W",
        &w.to_string(),
        "-H",
        &h.to_string(),
        "-w",
        &w.to_string(),
        "-h",
        &h.to_string(),
        "--",
    ])
    .arg(executable)
    .arg("--json")
    .arg("--project")
    .arg(&config.project_root)
    .arg("--godot-bin")
    .arg(binary)
    .arg("__presented-helper")
    .arg("--owner")
    .arg(&owner_path)
    .arg("--result")
    .arg(&result_path)
    .arg("--nonce")
    .arg(&nonce)
    .arg("--scene")
    .arg(&args.scene)
    .arg("--viewport")
    .arg(&args.viewport)
    .arg("--backend")
    .arg(&args.display_backend)
    .arg("--out")
    .arg(&out)
    .arg("--threshold")
    .arg(args.threshold.to_string())
    .arg("--channel-tolerance")
    .arg(args.channel_tolerance.to_string())
    .env("XMODIFIERS", "@im=none")
    .stdout(Stdio::from(log))
    .stderr(Stdio::from(stderr));
    if let Some(baseline) = &baseline {
        cmd.arg("--baseline").arg(baseline);
    }
    unsafe {
        cmd.pre_exec(|| {
            if libc::setsid() == -1 {
                Err(io::Error::last_os_error())
            } else {
                Ok(())
            }
        });
    }
    let mut child = match cmd.spawn() {
        Ok(child) => child,
        Err(error) => {
            return failure(
                "presented_capture_unsupported",
                format!("could not start headless Gamescope: {error}"),
                json!("installed runnable gamescope"),
                json!(error.to_string()),
                &out,
            );
        }
    };
    let _project_child = match godot_qa_process::project_access::register_child(child.id() as i64) {
        Ok(receipt) => receipt,
        Err(error) => {
            terminate_group(child.id() as i32, &mut child);
            return failure(
                "project_access_failed",
                error.to_string(),
                json!("persisted child identity"),
                json!("receipt write failed"),
                &out,
            );
        }
    };
    let owner_pid = child.id() as i32;
    if let Err(error) = fs::write(
        &owner_path,
        json!({"nonce": nonce, "pid": owner_pid}).to_string(),
    ) {
        terminate_group(owner_pid, &mut child);
        return failure(
            "artifact_write_failed",
            error.to_string(),
            json!("owner marker"),
            json!(owner_path),
            &out,
        );
    }
    let deadline = Instant::now() + WRAPPER_TIMEOUT;
    let wrapper_status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Some(status),
            Ok(None) if Instant::now() >= deadline => {
                terminate_group(owner_pid, &mut child);
                break None;
            }
            Ok(None) => thread::sleep(Duration::from_millis(50)),
            Err(_) => {
                terminate_group(owner_pid, &mut child);
                break None;
            }
        }
    };
    let helper = fs::read(&result_path)
        .ok()
        .and_then(|bytes| serde_json::from_slice::<CommandResult>(&bytes).ok())
        .filter(|result| {
            result.data.get("run_nonce").and_then(Value::as_str) == Some(&nonce)
                && result.data.get("owner_pid").and_then(Value::as_i64)
                    == Some(i64::from(owner_pid))
        });
    // The helper can die before its finally-style session stop. Its project arg
    // identifies only this run's Godot process, so an existing user session is safe.
    let orphan_cleanup = cleanup_owned_orphan(config, &nonce);
    let mut result = helper.unwrap_or_else(|| failure("presented_result_missing", "Gamescope child did not produce a valid result", json!("valid run-matched helper JSON"), json!({"wrapper_exit": wrapper_status.as_ref().and_then(|s| s.code()), "timed_out": wrapper_status.is_none()}), &out));
    result.command = COMMAND.into();
    result.data.insert("wrapper".into(), json!({"exit_code": wrapper_status.as_ref().and_then(|s| s.code()), "timed_out": wrapper_status.is_none(), "pid": owner_pid, "log": log_path}));
    if let Some(cleanup) = orphan_cleanup {
        result.ok = false;
        result.data.insert("orphan_cleanup".into(), json!(cleanup));
        result.failures.push(FailureDetail {
            kind: "presented_teardown_failed".into(),
            command: Some(COMMAND.into()),
            message: Some("helper left its owned Godot session running".into()),
            path: Some(out.display().to_string()),
            ..FailureDetail::default()
        });
    }
    if wrapper_status.is_none() {
        result.ok = false;
        result.failures.push(FailureDetail {
            kind: "presented_wrapper_timeout".into(),
            command: Some(COMMAND.into()),
            message: Some("owned Gamescope process did not finish within 55 seconds".into()),
            path: Some(log_path.display().to_string()),
            ..FailureDetail::default()
        });
    } else if !wrapper_status
        .as_ref()
        .is_some_and(|status| status.success())
        && result.ok
    {
        result.ok = false;
        result.failures.push(FailureDetail {
            kind: "presented_wrapper_failed".into(),
            command: Some(COMMAND.into()),
            message: Some("owned Gamescope process exited unsuccessfully".into()),
            actual: Some(json!(wrapper_status.and_then(|s| s.code()))),
            path: Some(log_path.display().to_string()),
            ..FailureDetail::default()
        });
    }
    result.exit_code = if result.ok { 0 } else { 1 };
    let finalized_path = out.join("result.json");
    if let Some(artifacts) = result
        .data
        .get_mut("artifacts")
        .and_then(Value::as_object_mut)
    {
        artifacts.insert("result".into(), json!(finalized_path));
    }
    if let Err(error) = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&finalized_path)
        .and_then(|file| serde_json::to_writer_pretty(file, &result).map_err(io::Error::other))
    {
        result.ok = false;
        result.exit_code = 1;
        result.failures.push(FailureDetail {
            kind: "artifact_write_failed".into(),
            command: Some(COMMAND.into()),
            message: Some(error.to_string()),
            path: Some(finalized_path.display().to_string()),
            ..FailureDetail::default()
        });
    }
    result
}

fn cleanup_owned_orphan(config: &ProjectConfig, nonce: &str) -> Option<CommandResult> {
    let record = runtime::load_session_record(config, COMMAND).ok()?;
    if !record
        .project_args
        .iter()
        .any(|arg| arg == &format!("--qa-presented-run={nonce}"))
    {
        return None;
    }
    Some(runtime::session_stop_result(config))
}

fn terminate_group(pid: i32, child: &mut std::process::Child) {
    unsafe {
        libc::kill(-pid, libc::SIGTERM);
    }
    if child.try_wait().ok().flatten().is_none() {
        thread::sleep(Duration::from_millis(300));
    }
    if child.try_wait().ok().flatten().is_none() {
        unsafe {
            libc::kill(-pid, libc::SIGKILL);
        }
    }
    let _ = child.wait();
}

pub(crate) fn helper_result(cli: &Cli, args: &PresentedHelperArgs) -> CommandResult {
    let mut result = helper_result_inner(cli, args);
    result.data.insert("run_nonce".into(), json!(args.nonce));
    result.data.insert(
        "owner_pid".into(),
        json!(
            fs::read(&args.owner)
                .ok()
                .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
                .and_then(|owner| owner.get("pid").cloned())
                .unwrap_or(Value::Null)
        ),
    );
    result.exit_code = if result.ok { 0 } else { 1 };
    if let Err(error) = fs::write(
        &args.result,
        serde_json::to_vec_pretty(&result).unwrap_or_default(),
    ) {
        result.ok = false;
        result.exit_code = 1;
        result.failures.push(FailureDetail {
            kind: "artifact_write_failed".into(),
            command: Some(COMMAND.into()),
            message: Some(error.to_string()),
            path: Some(args.result.display().to_string()),
            ..FailureDetail::default()
        });
    }
    result
}

fn helper_result_inner(cli: &Cli, args: &PresentedHelperArgs) -> CommandResult {
    let Some((w, h)) = parse_viewport(&args.viewport) else {
        return failure(
            "invalid_viewport",
            "invalid helper viewport",
            json!("WIDTHxHEIGHT"),
            json!(args.viewport),
            &args.out,
        );
    };
    let owner_deadline = Instant::now() + Duration::from_secs(2);
    let owner: Value = match loop {
        if let Some(owner) = fs::read(&args.owner)
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        {
            break Some(owner);
        }
        if Instant::now() >= owner_deadline {
            break None;
        }
        thread::sleep(Duration::from_millis(20));
    } {
        Some(owner) => owner,
        None => {
            return failure(
                "presented_capture_unsupported",
                "owner marker is unavailable",
                json!("owned Gamescope marker"),
                json!(args.owner),
                &args.out,
            );
        }
    };
    let owner_pid = owner.get("pid").and_then(Value::as_i64).unwrap_or(0) as i32;
    if owner.get("nonce").and_then(Value::as_str) != Some(&args.nonce) || owner_pid <= 0 {
        return failure(
            "presented_capture_unsupported",
            "owner marker did not match this run",
            json!(args.nonce),
            owner,
            &args.out,
        );
    }
    // The parent starts Gamescope with setsid. A helper outside that newly
    // created session cannot claim an arbitrary compositor using a marker.
    let helper_session = unsafe { libc::getsid(0) };
    if helper_session != owner_pid {
        return failure(
            "presented_capture_unsupported",
            "helper is outside the owned Gamescope session",
            json!(owner_pid),
            json!(helper_session),
            &args.out,
        );
    }
    let socket_name = match std::env::var("GAMESCOPE_WAYLAND_DISPLAY") {
        Ok(name) if !name.is_empty() => name,
        _ => {
            return failure(
                "presented_capture_unsupported",
                "Gamescope did not expose its Wayland socket",
                json!("GAMESCOPE_WAYLAND_DISPLAY"),
                json!("missing"),
                &args.out,
            );
        }
    };
    let socket = Path::new(&socket_name);
    let socket = if socket.is_absolute() {
        socket.to_path_buf()
    } else {
        PathBuf::from(std::env::var("XDG_RUNTIME_DIR").unwrap_or_default()).join(socket)
    };
    let peer_pid = match compositor_peer_pid(&socket) {
        Ok(pid) => pid,
        Err(error) => {
            return failure(
                "presented_capture_unsupported",
                format!("cannot identify owned compositor socket: {error}"),
                json!("live Gamescope Wayland socket"),
                json!(socket),
                &args.out,
            );
        }
    };
    let peer_group = unsafe { libc::getpgid(peer_pid) };
    if peer_group != owner_pid {
        return failure(
            "presented_capture_unsupported",
            "compositor socket is outside the owned process group",
            json!(owner_pid),
            json!({"peer_pid": peer_pid, "peer_group": peer_group}),
            &args.out,
        );
    }
    if args.backend == "wayland" {
        // This fixed helper has not started any threads before launching Godot.
        unsafe {
            std::env::set_var("WAYLAND_DISPLAY", &socket_name);
        }
    } else if args.backend == "x11" {
        if std::env::var("DISPLAY")
            .ok()
            .filter(|value| !value.is_empty())
            .is_none()
        {
            return failure(
                "presented_capture_unsupported",
                "Gamescope nested X11 display is unavailable",
                json!("nested DISPLAY"),
                json!("missing"),
                &args.out,
            );
        }
    } else {
        return failure(
            "presented_capture_unsupported",
            "unsupported display backend",
            json!(["wayland", "x11"]),
            json!(args.backend),
            &args.out,
        );
    }
    let mut config = load_project_config(
        cli.project.as_deref().map(Path::new),
        cli.godot_bin.as_deref(),
        None,
        false,
        godot_qa_process::resolve_godot_binary(cli.godot_bin.as_deref()),
    );
    config
        .godot_args
        .extend(["--display-driver".to_string(), args.backend.clone()]);
    let started = Instant::now();
    let start = runtime::session_start_result(
        &config,
        Some(&args.scene),
        &args.viewport,
        false,
        false,
        &[format!("--qa-presented-run={}", args.nonce)],
        None,
        false,
    );
    if !start.ok {
        let mut result = failure(
            "presented_session_start_failed",
            "owned Godot session did not start",
            json!("windowed session ready"),
            json!(start.failures),
            &args.out,
        );
        result.data.insert("session_start".into(), json!(start));
        return result;
    }
    if args.backend == "x11" {
        if let Ok(output) = Command::new("xwininfo").args(["-root", "-tree"]).output() {
            let _ = fs::write(args.out.join("x11-window-tree.txt"), output.stdout);
        }
    }
    let result = capture_and_compare(&config, args, (w, h), &socket, peer_pid, started, &start);
    let stop = runtime::session_stop_result(&config);
    let mut result = result;
    let clean_stop = stop.ok
        && stop.message == "live command completed"
        && stop.data.get("stopping") == Some(&json!(true));
    result.data.insert("session_stop".into(), json!(stop));
    if !clean_stop {
        result.ok = false;
        result.failures.push(FailureDetail {
            kind: "presented_teardown_failed".into(),
            command: Some(COMMAND.into()),
            message: Some("owned Godot session did not exit cleanly".into()),
            actual: Some(json!(stop.message)),
            path: Some(args.out.display().to_string()),
            ..FailureDetail::default()
        });
    }
    result.exit_code = if result.ok { 0 } else { 1 };
    result
}

fn compositor_peer_pid(path: &Path) -> io::Result<i32> {
    let stream = std::os::unix::net::UnixStream::connect(path)?;
    let mut credential: libc::ucred = unsafe { std::mem::zeroed() };
    let mut len = std::mem::size_of::<libc::ucred>() as libc::socklen_t;
    let status = unsafe {
        libc::getsockopt(
            stream.as_raw_fd(),
            libc::SOL_SOCKET,
            libc::SO_PEERCRED,
            (&mut credential as *mut libc::ucred).cast(),
            &mut len,
        )
    };
    if status == -1 {
        Err(io::Error::last_os_error())
    } else {
        Ok(credential.pid)
    }
}

fn sha256(path: &Path) -> io::Result<String> {
    let mut file = File::open(path)?;
    let mut digest = Sha256::new();
    let mut chunk = [0_u8; 65536];
    loop {
        let count = file.read(&mut chunk)?;
        if count == 0 {
            break;
        }
        digest.update(&chunk[..count]);
    }
    Ok(to_lower_hex(&digest.finish()))
}

fn capture_gamescope_png(path: &Path, log_path: &Path) -> io::Result<(u32, u32)> {
    let log = File::create(log_path)?;
    let stderr = log.try_clone()?;
    let mut child = Command::new("gamescopectl")
        .arg("screenshot")
        .arg(path)
        .stdout(Stdio::from(log))
        .stderr(Stdio::from(stderr))
        .spawn()?;
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        match child.try_wait()? {
            Some(status) if status.success() => break,
            Some(status) => {
                return Err(io::Error::other(format!(
                    "gamescopectl exited with {status}"
                )));
            }
            None if Instant::now() >= deadline => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(io::Error::other("gamescopectl timed out after 10 seconds"));
            }
            None => thread::sleep(Duration::from_millis(20)),
        }
    }
    let deadline = Instant::now() + Duration::from_secs(8);
    loop {
        if let Ok(image) = image::open(path) {
            return Ok(image.dimensions());
        }
        if Instant::now() >= deadline {
            return Err(io::Error::other(
                "Gamescope did not write a decodable PNG within eight seconds",
            ));
        }
        thread::sleep(Duration::from_millis(30));
    }
}

fn capture_and_compare(
    config: &ProjectConfig,
    args: &PresentedHelperArgs,
    dimensions: (u32, u32),
    socket: &Path,
    peer_pid: i32,
    started: Instant,
    start: &CommandResult,
) -> CommandResult {
    let (w, h) = dimensions;
    let presented = args.out.join("presented.png");
    let viewport = args.out.join("viewport.png");
    let diff = args.out.join("diff.png");
    let metrics = args.out.join("metrics.json");
    let mut data = Map::from_iter([
        (
            "artifacts".into(),
            json!({"root": args.out, "presented": presented, "viewport": viewport, "diff": diff, "metrics": metrics}),
        ),
        (
            "session_id".into(),
            start.data.get("session_id").cloned().unwrap_or(Value::Null),
        ),
        (
            "pid".into(),
            start.data.get("pid").cloned().unwrap_or(Value::Null),
        ),
        (
            "started_at".into(),
            start.data.get("started_at").cloned().unwrap_or(Value::Null),
        ),
        (
            "display_server".into(),
            start
                .data
                .get("display_server")
                .cloned()
                .unwrap_or(Value::Null),
        ),
        (
            "capture".into(),
            json!({"source": "presented_surface", "adapter": "gamescopectl", "output": "isolated_gamescope", "socket": socket, "socket_peer_pid": peer_pid, "dimensions_expected": [w,h], "draw_forced": false, "session_pid": start.data.get("pid")}),
        ),
    ]);
    if let Some(pid) = start.data.get("pid").and_then(Value::as_i64) {
        let environment = fs::read(format!("/proc/{pid}/environ")).ok().map(|bytes| {
            bytes
                .split(|byte| *byte == 0)
                .filter_map(|item| std::str::from_utf8(item).ok())
                .filter(|item| {
                    item.starts_with("DISPLAY=")
                        || item.starts_with("WAYLAND_DISPLAY=")
                        || item.starts_with("GAMESCOPE_WAYLAND_DISPLAY=")
                })
                .map(str::to_string)
                .collect::<Vec<_>>()
        });
        data.insert("child_display_environment".into(), json!(environment));
    }
    if start
        .data
        .get("display_server")
        .and_then(Value::as_str)
        .map(str::to_ascii_lowercase)
        .as_deref()
        != Some(&args.backend)
    {
        let mut result = failure(
            "presented_backend_mismatch",
            "Godot used a different display backend",
            json!(args.backend),
            start
                .data
                .get("display_server")
                .cloned()
                .unwrap_or(Value::Null),
            &args.out,
        );
        result.data.extend(data);
        return result;
    }
    thread::sleep(SETTLE);
    let capture_at = started.elapsed().as_millis();
    let dims = match capture_gamescope_png(&presented, &args.out.join("gamescopectl.log")) {
        Ok(dimensions) => dimensions,
        Err(error) => {
            let mut result = failure(
                "presented_capture_failed",
                error.to_string(),
                json!("valid compositor PNG"),
                json!(presented),
                &args.out,
            );
            result.data.extend(data);
            return result;
        }
    };
    data.insert(
        "capture_order".into(),
        json!({"presented_monotonic_ms": capture_at}),
    );
    data.insert(
        "dimensions".into(),
        json!({"presented": [dims.0,dims.1], "expected": [w,h]}),
    );
    // The first bridge screenshot is deliberately after the compositor capture.
    data.insert("capture_order".into(), json!({"presented_monotonic_ms": capture_at, "viewport_monotonic_ms": started.elapsed().as_millis()}));
    let viewport_result = runtime::run_live_screenshot_protocol_command(
        config,
        COMMAND,
        runtime::SCREENSHOT_CAPTURE,
        Map::from_iter([
            ("name".into(), json!("presented-startup-viewport")),
            ("allow_fallback".into(), json!(false)),
        ]),
    );
    if !viewport_result.ok {
        let mut result = failure(
            "viewport_capture_failed",
            "diagnostic viewport capture failed after compositor capture",
            json!("valid viewport PNG"),
            json!(viewport_result.failures),
            &args.out,
        );
        result.data.extend(data);
        return result;
    }
    if viewport_result.data.get("session_id") != start.data.get("session_id")
        || viewport_result
            .data
            .get("capture_source")
            .and_then(Value::as_str)
            != Some("godot_viewport")
        || viewport_result
            .data
            .get("allow_fallback")
            .and_then(Value::as_bool)
            != Some(false)
    {
        let mut result = failure(
            "viewport_capture_failed",
            "strict viewport capture did not confirm source and session identity",
            json!({"session_id": start.data.get("session_id"), "capture_source": "godot_viewport", "allow_fallback": false}),
            json!({"session_id": viewport_result.data.get("session_id"), "capture_source": viewport_result.data.get("capture_source"), "allow_fallback": viewport_result.data.get("allow_fallback")}),
            &args.out,
        );
        result.data.extend(data);
        return result;
    }
    let staged = viewport_result
        .data
        .get("artifacts")
        .and_then(Value::as_object)
        .and_then(|artifacts| artifacts.get("screenshot"))
        .and_then(Value::as_str);
    let Some(staged) = staged else {
        let mut result = failure(
            "viewport_capture_failed",
            "strict viewport response omitted screenshot path",
            json!("artifacts.screenshot"),
            json!(viewport_result.data),
            &args.out,
        );
        result.data.extend(data);
        return result;
    };
    if let Err(error) = fs::copy(staged, &viewport) {
        let mut result = failure(
            "viewport_capture_failed",
            error.to_string(),
            json!("copyable strict viewport PNG"),
            json!(staged),
            &args.out,
        );
        result.data.extend(data);
        return result;
    }
    data.insert("capture_provenance".into(), json!({
        "presented": {"source": "isolated_compositor", "draw_forced": false, "sha256": sha256(&presented).ok()},
        "viewport": {"source": "godot_viewport", "draw_forced": viewport_result.data.get("draw_forced"), "allow_fallback": false, "sha256": sha256(&viewport).ok()},
        "comparison_reference": if args.baseline.is_some() { "explicit_baseline" } else { "later_viewport" },
        "baseline_sha256": args.baseline.as_deref().and_then(|path| sha256(path).ok()),
    }));
    let viewport_dims = image::open(&viewport).map(|image| image.dimensions());
    if let Some(dimensions) = data.get_mut("dimensions").and_then(Value::as_object_mut) {
        dimensions.insert("viewport".into(), json!(viewport_dims.as_ref().ok()));
    }
    if !matches!(viewport_dims, Ok((vw, vh)) if vw == w && vh == h) {
        let mut result = failure(
            "presented_size_mismatch",
            "viewport and compositor dimensions differ; scaling is unsupported",
            json!([w, h]),
            json!(viewport_dims.ok()),
            &args.out,
        );
        result.data.extend(data);
        return result;
    }
    if dims != (w, h) {
        let mut result = failure(
            "presented_size_mismatch",
            "compositor output dimensions differ from the requested viewport",
            json!([w, h]),
            json!([dims.0, dims.1]),
            &args.out,
        );
        result.data.extend(data);
        return result;
    }
    let presented_quality = screenshots::analyze_screenshot_file(&presented);
    let viewport_quality = screenshots::analyze_screenshot_file(&viewport);
    let presented_low = presented_quality
        .as_ref()
        .ok()
        .and_then(|quality| quality.get("low_information"))
        .and_then(Value::as_bool)
        .unwrap_or(true);
    let viewport_low = viewport_quality
        .as_ref()
        .ok()
        .and_then(|quality| quality.get("low_information"))
        .and_then(Value::as_bool)
        .unwrap_or(true);
    data.insert(
        "capture_quality".into(),
        json!({"presented": presented_quality.ok(), "viewport": viewport_quality.ok()}),
    );
    if presented_low || viewport_low {
        let mut result = failure(
            "presented_capture_low_information",
            "strict startup images lack useful rendered detail",
            json!({"presented_low_information": false, "viewport_low_information": false}),
            json!({"presented_low_information": presented_low, "viewport_low_information": viewport_low}),
            &args.out,
        );
        result.data.extend(data);
        return result;
    }
    let reference = args.baseline.as_deref().unwrap_or(&viewport);
    let comparison = screenshots::compare_presented_startup_files(
        COMMAND,
        &presented,
        reference,
        args.threshold,
        args.channel_tolerance,
        Some(&diff),
        Some(&metrics),
    );
    data.insert("comparison".into(), json!(comparison.data));
    let mut result = CommandResult {
        ok: comparison.ok,
        command: COMMAND.into(),
        message: if comparison.ok {
            "presented startup assertion passed"
        } else {
            "presented startup assertion failed"
        }
        .into(),
        data,
        failures: comparison.failures,
        exit_code: if comparison.ok { 0 } else { 1 },
    };
    for failure in &mut result.failures {
        if failure.kind == "screenshot_mismatch" {
            failure.kind = "presented_startup_mismatch".into();
        }
        if failure.kind == "screenshot_size_mismatch" {
            failure.kind = "presented_size_mismatch".into();
        }
        failure.artifacts = Some(Map::from_iter([
            ("presented".into(), json!(presented)),
            ("viewport".into(), json!(viewport)),
            ("diff".into(), json!(diff)),
        ]));
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn viewport_rejects_zero_and_oversized_values() {
        assert_eq!(parse_viewport("1280x720"), Some((1280, 720)));
        assert_eq!(parse_viewport("0x720"), None);
        assert_eq!(parse_viewport("1280x0"), None);
        assert_eq!(parse_viewport("9000x720"), None);
    }
}
