use super::capture;
#[cfg(unix)]
use super::display::effective_launch_prefix;
use super::display::{self, append_project_display_driver_args_with_env, project_prefers_wayland};
#[cfg(unix)]
use super::launch::run_godot_command_with_completion_path;
use super::launch::{run_godot_command, run_godot_command_with_timeout};
#[cfg(unix)]
use super::lifecycle;
use super::lifecycle::{exit_poll_backoff, is_zombie_process, stale_owned_child_cleanup_failure};
#[cfg(target_os = "linux")]
use super::probes;
#[cfg(unix)]
use super::stop_godot_process;
use super::{
    GodotOneShotLaunch, launch_godot_check, launch_godot_import, launch_godot_one_shot,
    launch_godot_patch_runner, launch_godot_session, pid_is_running,
    probe_godot_version_with_metadata, run_safe_probe, stop_godot_process_with_cleanup,
};
use godot_qa_test_support::fixture;
use std::env;
use std::fs::{self, File};
use std::io::Write;
#[cfg(unix)]
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
fn safe_probe_drains_large_streams_with_bounded_retention() {
    let temp_dir = unique_temp_dir().join("probe with spaces");
    let stdout = "o".repeat(4 * 1024 * 1024);
    let stderr = "e".repeat(4 * 1024 * 1024);
    let probe = fixture(
        &temp_dir.join("large output"),
        &[("stdout", &stdout), ("stderr", &stderr)],
    );

    let result = run_safe_probe(&probe, &[], Duration::from_secs(2));
    fs::remove_dir_all(temp_dir).unwrap();

    assert!(
        !result.timed_out,
        "probe timed out with status {:?}",
        result.status
    );
    assert_eq!(result.status, Some(0));
    assert_eq!(result.error_kind, None);
    assert!(!result.stdout.is_empty());
    assert!(!result.stderr.is_empty());
    assert!(result.stdout.len() <= 64 * 1024);
    assert!(result.stderr.len() <= 64 * 1024);
    assert!(result.stdout.bytes().all(|byte| byte == b'o'));
    assert!(result.stderr.bytes().all(|byte| byte == b'e'));
}

#[cfg(unix)]
#[test]
fn safe_probe_reaps_descendant_holding_pipes_after_leader_exit() {
    let temp_dir = unique_temp_dir().join("probe with spaces");
    let sleeper = fixture(&temp_dir.join("sleeper"), &[("sleep_ms", "30000")]);
    let pid_file = temp_dir.join("sleeper.pid");
    let probe = fixture(
        &temp_dir.join("probe"),
        &[
            ("child_exe", sleeper.to_str().unwrap()),
            ("child_pid_file", pid_file.to_str().unwrap()),
            ("stdout", "leader exited\n"),
        ],
    );
    let (sender, receiver) = std::sync::mpsc::channel();
    let worker = std::thread::spawn(move || {
        let result = run_safe_probe(&probe, &[], Duration::from_millis(300));
        let _ = sender.send(result);
    });
    let result = receiver.recv_timeout(Duration::from_secs(2));
    let pid = fs::read_to_string(pid_file).unwrap().parse().unwrap();
    let cleaned_up = super::wait_for_godot_process_exit(pid, 0.5);
    // Ensure a failed regression does not leave the fixture or reader blocked.
    if !cleaned_up {
        lifecycle::send_signal(pid, 9).unwrap();
    }
    worker.join().unwrap();
    fs::remove_dir_all(temp_dir).unwrap();

    let result = result.expect("probe must return while descendant would still be sleeping");
    assert!(result.duration < Duration::from_secs(2));
    assert_eq!(result.status, Some(0));
    assert!(!result.timed_out);
    assert_eq!(result.stdout, "leader exited\n");
    assert!(cleaned_up, "probe must terminate the owned descendant");
}

#[test]
fn owned_child_file_capture_without_deadline_returns_output() {
    let _signal_guard = PROCESS_SIGNAL_LOCK.lock().expect("signal test lock");
    let temp_dir = unique_temp_dir();
    fs::create_dir_all(&temp_dir).expect("create temp dir");
    let output = capture::run_owned_child_output(
        &[fixture(
            &temp_dir.join("output"),
            &[("stdout", "out"), ("stderr", "err")],
        )
        .to_string_lossy()
        .into_owned()],
        false,
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
fn version_probe_hanging_prefix_times_out_before_direct_fallback() {
    let temp_dir = unique_temp_dir().join("version probe with spaces");
    let wrapper = fixture(
        &temp_dir.join("wrapper"),
        &[
            ("stdout", "prefix started\n"),
            ("sleep_ms", "3000"),
            ("exit", "19"),
        ],
    );
    let godot = fixture(&temp_dir.join("godot"), &[("stdout", "Godot_v4.5\n")]);
    let started = std::time::Instant::now();
    let result = probe_godot_version_with_metadata(
        godot.to_str().unwrap(),
        &[wrapper.to_string_lossy().into_owned()],
    )
    .expect("direct probe succeeds after prefix timeout");
    let elapsed = started.elapsed();
    fs::remove_dir_all(temp_dir).unwrap();

    assert!(elapsed < Duration::from_millis(2800), "{elapsed:?}");
    assert_eq!(result.version, "Godot_v4.5");
    assert!(result.metadata.fallback_used);
    assert_eq!(result.metadata.fallback_reason.as_deref(), Some("timeout"));
    let metadata = result.metadata.to_json();
    assert_eq!(metadata["attempts"].as_array().unwrap().len(), 2);
    assert_eq!(metadata["attempts"][0]["failure_reason"], "timeout");
    assert_eq!(
        metadata["attempts"][0]["failure_type"],
        "godot_version_probe_failed"
    );
    assert_eq!(metadata["attempts"][0]["stdout"], "prefix started");
    assert_eq!(metadata["attempts"][1]["status"], 0);
    assert_eq!(
        metadata["selected_command"][0],
        godot.to_string_lossy().as_ref()
    );
}

#[test]
fn version_probe_hanging_direct_probe_reports_timeout() {
    let temp_dir = unique_temp_dir().join("version probe with spaces");
    let godot = fixture(
        &temp_dir.join("godot"),
        &[("stdout", "Godot_v4.5\n"), ("sleep_ms", "3000")],
    );
    let started = std::time::Instant::now();
    let result = probe_godot_version_with_metadata(godot.to_str().unwrap(), &[]);
    let elapsed = started.elapsed();
    fs::remove_dir_all(temp_dir).unwrap();

    assert!(elapsed < Duration::from_millis(2800), "{elapsed:?}");
    let failure = result.expect_err("output from a timed-out probe must not be a version");
    assert_eq!(failure.kind, "godot_version_probe_failed");
    assert_eq!(failure.actual, Some(serde_json::json!("timeout")));
    let metadata = &failure.details["version_probe"];
    assert_eq!(metadata["fallback_used"], false);
    assert_eq!(metadata["attempts"].as_array().unwrap().len(), 1);
    assert_eq!(metadata["attempts"][0]["stdout"], "Godot_v4.5");
    assert_eq!(metadata["attempts"][0]["failure_reason"], "timeout");
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
        joypad_isolation: false,
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
        joypad_isolation: false,
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
        joypad_isolation: false,
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
        joypad_isolation: false,
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
        joypad_isolation: false,
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
        joypad_isolation: false,
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
        joypad_isolation: false,
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
        joypad_isolation: false,
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
        joypad_isolation: false,
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
        joypad_isolation: false,
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
        false,
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
        false,
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
        false,
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
        false,
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

#[cfg(unix)]
#[test]
fn owned_group_cleanup_precedes_stdin_writer_wait() {
    let root = unique_temp_dir();
    fs::create_dir_all(&root).unwrap();
    let wrapper = write_executable(
        &root.join("stdin-holder.sh"),
        "#!/bin/sh\nexec 3<&0\nsleep 30 <&3 3<&- >/dev/null 2>&1 &\nprintf '%s' \"$!\" > \"$1\"\nexit 0\n",
    );
    let pid_file = root.join("child.pid");
    let started = std::time::Instant::now();
    let result = capture::run_owned_child_output_with_timeout_monitor(
        &[
            wrapper.to_string_lossy().into_owned(),
            pid_file.to_string_lossy().into_owned(),
        ],
        false,
        Some(Duration::from_secs(5)),
        None,
        None,
        None,
        Some(&vec![0; 1024 * 1024]),
    );
    let pid = fs::read_to_string(pid_file).unwrap().parse().unwrap();
    let gone = super::wait_for_godot_process_exit(pid, 0.5);
    if !gone {
        lifecycle::send_signal(pid, libc::SIGKILL).unwrap();
    }
    fs::remove_dir_all(root).unwrap();
    let output = result.expect("leader exit must survive a descendant holding stdin");
    assert!(gone);
    assert_eq!(output.status_code, 0);
    assert!(started.elapsed() < Duration::from_secs(2));
}

#[cfg(unix)]
#[test]
fn owned_group_timeout_kills_term_ignoring_descendant() {
    assert_owned_group_cleanup(Some(Duration::from_millis(200)), true);
}

#[cfg(target_os = "linux")]
#[test]
fn owned_group_timeout_handles_non_utf8_comm() {
    let root = unique_temp_dir().join("non ascii process with spaces");
    fs::create_dir_all(&root).unwrap();
    // Linux truncates comm at 15 bytes, splitting this name's final é.
    let sleeper = root.join("godot_v4.5_abcé");
    fs::copy("/bin/sleep", &sleeper).unwrap();
    let wrapper = write_executable(
        &root.join("wrapper.sh"),
        "#!/bin/sh\necho $$ > \"$2\"\nexec \"$1\" 60\n",
    );
    let pid_path = root.join("leader.pid");
    let command = vec![
        wrapper.to_string_lossy().into_owned(),
        sleeper.to_string_lossy().into_owned(),
        pid_path.to_string_lossy().into_owned(),
    ];
    let (sender, receiver) = std::sync::mpsc::channel();
    let started = std::time::Instant::now();
    let worker = std::thread::spawn(move || {
        let _ = sender.send(capture::run_owned_child_output(
            &command,
            false,
            Some(Duration::from_millis(500)),
            None,
            None,
        ));
    });
    let deadline = std::time::Instant::now() + Duration::from_secs(2);
    while !pid_path.exists() && std::time::Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(5));
    }
    let pid: i64 = fs::read_to_string(pid_path)
        .unwrap()
        .trim()
        .parse()
        .unwrap();
    let result = receiver.recv_timeout(Duration::from_millis(3500));
    let returned_in_time = result.is_ok();
    // Unblock the original hanging cleanup and leave no fixture behind.
    unsafe {
        libc::kill(-(pid as libc::pid_t), libc::SIGKILL);
    }
    let output = result
        .or_else(|_| receiver.recv_timeout(Duration::from_secs(2)))
        .unwrap();
    worker.join().unwrap();
    let reaped = !pid_is_running(pid);
    fs::remove_dir_all(root).unwrap();
    assert!(returned_in_time, "non-UTF-8 comm blocked timeout cleanup");
    let output = output.expect("owned timeout output");
    assert!(output.timed_out);
    assert_eq!(output.cleanup.unwrap().status, "timeout_terminated");
    assert!(reaped, "owned leader {pid} survived or was not reaped");
    assert!(started.elapsed() < Duration::from_secs(4));
}

#[cfg(unix)]
#[test]
fn owned_group_normal_exit_kills_term_ignoring_descendant() {
    for timeout in [None, Some(Duration::from_secs(5))] {
        assert_owned_group_cleanup(timeout, false);
    }
}

#[cfg(unix)]
fn assert_owned_group_cleanup(timeout: Option<Duration>, timed_out: bool) {
    let root = unique_temp_dir().join("owned group with spaces");
    fs::create_dir_all(&root).unwrap();
    let wrapper = write_executable(
        &root.join("wrapper.sh"),
        "#!/bin/sh\n\
             (trap '' TERM; printf ready > \"$1/ready\"; exec sleep 30) </dev/null >/dev/null 2>&1 &\n\
             printf '%s' \"$!\" > \"$1/child.pid\"\n\
             while [ ! -f \"$1/ready\" ]; do sleep 0.01; done\n\
             printf 'leader output'\n\
             if [ \"$2\" = timeout ]; then exec sleep 30; fi\n\
             exit 23\n",
    );
    let started = std::time::Instant::now();
    let result = capture::run_owned_child_output(
        &[
            wrapper.to_string_lossy().into_owned(),
            root.to_string_lossy().into_owned(),
            if timed_out { "timeout" } else { "normal" }.into(),
        ],
        false,
        timeout,
        None,
        None,
    );
    let descendant = fs::read_to_string(root.join("child.pid"))
        .unwrap()
        .parse()
        .unwrap();
    let cleaned_up = super::wait_for_godot_process_exit(descendant, 0.5);
    // Leave no running fixture behind when the regression fails.
    if !cleaned_up {
        lifecycle::send_signal(descendant, libc::SIGKILL).unwrap();
    }
    fs::remove_dir_all(root).unwrap();
    let output = result.expect("owned wrapper output");
    assert!(cleaned_up, "owned descendant survived leader exit");
    assert!(started.elapsed() < Duration::from_secs(5));
    assert_eq!(output.stdout, b"leader output");
    assert_eq!(output.timed_out, timed_out);
    let termination = output.termination.unwrap();
    if timed_out {
        assert_eq!(output.cleanup.unwrap().status, "timeout_killed");
        assert_eq!(termination.signal, Some(libc::SIGTERM));
        assert!(termination.tool_initiated);
    } else {
        assert_eq!(output.status_code, 23);
        assert_eq!(termination.exit_code, Some(23));
        assert!(!termination.tool_initiated);
        assert_eq!(output.cleanup.unwrap().status, "exited");
    }
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
        false,
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
        false,
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
        false,
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
    let timing = result.process_timing.as_ref().unwrap();
    assert!(timing.result_observed_ms.unwrap() >= timing.spawn_ms);
    assert!(timing.post_result_exit_ms.unwrap() >= 400);
    assert!(timing.total_ms >= timing.result_observed_ms.unwrap());
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
        false,
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
fn completion_identity_is_cached_and_rejects_stale_or_partial_results() {
    let temp_dir = unique_temp_dir();
    fs::create_dir_all(&temp_dir).unwrap();
    let result_path = temp_dir.join("result.json");
    let progress_path = temp_dir.join("progress.jsonl");
    fs::write(
        temp_dir.join("request.json"),
        serde_json::to_vec(&serde_json::json!({
            "run": {"id": "current", "nonce": "fresh"},
            "artifacts": {"progress": progress_path},
        }))
        .unwrap(),
    )
    .unwrap();
    let identity = capture::CompletionIdentity::read(&result_path).unwrap();
    // Polling uses the request identity captured before process launch.
    fs::remove_file(temp_dir.join("request.json")).unwrap();
    for invalid in [
        r#"{"run_identity":{"run_id":"stale","nonce":"fresh"},"ok":true}"#,
        r#"{"run_identity":{"run_id":"current","nonce":"old"},"ok":true}"#,
        r#"{"run_identity":{"run_id":"current","nonce":"fresh"}}"#,
        r#"{"run_identity":{"run_id":"current","nonce":"fresh"},"ok":"true"}"#,
        r#"{"run_identity":{"run_id":"current","nonce":"fresh"},"ok":tru"#,
    ] {
        fs::write(&result_path, invalid).unwrap();
        assert!(
            !identity.matches(&result_path),
            "accepted invalid result: {invalid}"
        );
    }
    fs::write(
        &result_path,
        r#"{"run_identity":{"run_id":"current","nonce":"fresh"},"ok":false}"#,
    )
    .unwrap();
    assert!(
        identity.matches(&result_path),
        "failed assertions still mark completion"
    );
    fs::write(
        &progress_path,
        r#"{"event":"runtime_started","run_id":"current","nonce":"old"}"#,
    )
    .unwrap();
    assert!(!identity.runtime_started());
    fs::write(
        &progress_path,
        r#"{"event":"runtime_started","run_id":"current","nonce":"fresh"}"#,
    )
    .unwrap();
    assert!(identity.runtime_started());
    let _ = fs::remove_dir_all(temp_dir);
}

#[cfg(unix)]
#[test]
fn native_signal_is_distinct_from_tool_timeout() {
    let _signal_guard = PROCESS_SIGNAL_LOCK.lock().unwrap();
    let natural = capture::run_owned_child_output(
        &["/bin/sh".into(), "-c".into(), "kill -TERM $$".into()],
        false,
        None,
        None,
        None,
    )
    .unwrap();
    assert_eq!(natural.status_code, -1);
    assert!(!natural.timed_out);
    let termination = natural.termination.unwrap();
    assert_eq!(termination.scope, "direct_child");
    assert_eq!(termination.exit_code, None);
    assert_eq!(termination.signal, Some(libc::SIGTERM));
    assert_eq!(termination.core_dumped, Some(false));
    assert!(!termination.tool_initiated);
    assert!(!termination.native_status.is_empty());

    let timeout = capture::run_owned_child_output(
        &["/bin/sh".into(), "-c".into(), "sleep 30".into()],
        false,
        Some(Duration::from_millis(50)),
        None,
        None,
    )
    .unwrap();
    assert_eq!(timeout.status_code, -1);
    assert!(timeout.timed_out);
    let termination = timeout.termination.unwrap();
    assert!(termination.tool_initiated);
    assert!(matches!(
        termination.signal,
        Some(libc::SIGTERM | libc::SIGKILL)
    ));
    assert!(timeout.process_timing.total_ms >= timeout.process_timing.spawn_ms);
    assert!(timeout.process_timing.runtime_started_observed_ms.is_none());
    assert!(timeout.process_timing.result_observed_ms.is_none());
}

#[cfg(unix)]
#[test]
fn stale_or_partial_completion_never_extends_process_deadline() {
    let _signal_guard = PROCESS_SIGNAL_LOCK.lock().unwrap();
    let temp_dir = unique_temp_dir();
    fs::create_dir_all(&temp_dir).unwrap();
    let result_path = temp_dir.join("result.json");
    fs::write(
        temp_dir.join("request.json"),
        r#"{"run":{"id":"run-1","nonce":"fresh"}}"#,
    )
    .unwrap();
    for invalid in [
        r#"{"run_identity":{"run_id":"run-1","nonce":"stale"},"ok":true}"#,
        r#"{"run_identity":{"run_id":"run-1","nonce":"fresh"},"ok":tru"#,
    ] {
        fs::write(&result_path, invalid).unwrap();
        let started = std::time::Instant::now();
        let output = capture::run_owned_child_output(
            &["/bin/sh".into(), "-c".into(), "sleep 30".into()],
            false,
            Some(Duration::from_millis(50)),
            Some(&result_path),
            Some(&temp_dir.join("logs.txt")),
        )
        .unwrap();
        assert!(output.timed_out);
        assert_eq!(output.timeout_ms, Some(50));
        assert!(output.process_timing.result_observed_ms.is_none());
        assert!(
            started.elapsed() < Duration::from_secs(3),
            "invalid result granted exit grace"
        );
    }
    let _ = fs::remove_dir_all(temp_dir);
}

#[test]
fn foreground_file_capture_keeps_bounded_streams_and_complete_artifacts() {
    let temp_dir = unique_temp_dir().join("foreground capture with spaces");
    let stdout = format!(
        "stdout head\n{}\nstdout tail\n",
        "x".repeat(9 * 1024 * 1024)
    );
    let stderr = format!(
        "stderr head\n{}\nstderr tail\n",
        "y".repeat(9 * 1024 * 1024)
    );
    let child = fixture(
        &temp_dir.join("noisy godot"),
        &[("stdout", &stdout), ("stderr", &stderr)],
    );
    for timeout in [None, Some(Duration::from_secs(10))] {
        let logs = temp_dir.join("logs.txt");
        let result = capture::run_owned_child_output(
            &[child.to_string_lossy().into_owned()],
            false,
            timeout,
            None,
            Some(&logs),
        )
        .unwrap();
        assert_eq!(result.status_code, 0);
        assert!(!result.timed_out);
        for (capture, original, stream) in [
            (&result.stdout, &stdout, "stdout"),
            (&result.stderr, &stderr, "stderr"),
        ] {
            assert!(capture.len() <= capture::CAPTURED_STREAM_LIMIT_BYTES as usize + 1024);
            let excerpt = String::from_utf8_lossy(capture);
            assert!(excerpt.starts_with(&format!("{stream} head\n")));
            assert!(excerpt.ends_with(&format!("{stream} tail\n")));
            assert!(excerpt.contains("omitted"));
            assert_eq!(
                fs::read(logs.with_extension(format!("{stream}.txt"))).unwrap(),
                original.as_bytes()
            );
        }
        let truncation = result.output_truncation.unwrap();
        assert_eq!(truncation.stdout_total_bytes, stdout.len() as u64);
        assert_eq!(truncation.stderr_total_bytes, stderr.len() as u64);
    }
    fs::remove_dir_all(temp_dir).unwrap();
}

#[test]
fn foreground_launches_fail_on_truncated_capture_and_keep_raw_artifacts() {
    let temp_dir = unique_temp_dir().join("foreground helpers with spaces");
    let output = "x".repeat(9 * 1024 * 1024);
    let child = fixture(
        &temp_dir.join("noisy godot"),
        &[("stdout", &output), ("stderr", &output)],
    );
    let binary = child.to_str().unwrap();
    let request = temp_dir.join("request.json");
    for helper in ["check", "check_timeout", "import", "patch"] {
        let logs = temp_dir.join(format!("{helper}.logs.txt"));
        let result = match helper {
            "check" => launch_godot_check(binary, &[], false, &[], &[], &temp_dir, &request, &logs),
            "check_timeout" => super::launch_godot_check_with_timeout(
                binary,
                &[],
                false,
                &[],
                &[],
                &temp_dir,
                &request,
                &logs,
                Some(Duration::from_secs(10)),
            ),
            "import" => launch_godot_import(binary, &[], false, &temp_dir, &logs),
            "patch" => {
                launch_godot_patch_runner(binary, &[], false, &[], &temp_dir, &request, &logs)
            }
            _ => unreachable!(),
        };
        let failure = result.expect_err("truncated validation output must fail");
        assert_eq!(failure.kind, "output_capture_truncated");
        assert!(failure.command.unwrap().contains("noisy godot"));
        assert_eq!(failure.target.as_deref(), temp_dir.to_str());
        assert_eq!(failure.actual.unwrap()["stdout_total_bytes"], output.len());
        let artifacts = failure.artifacts.unwrap();
        assert_eq!(artifacts["logs"], logs.to_string_lossy().as_ref());
        assert!(
            fs::metadata(&logs).unwrap().len() <= 2 * capture::CAPTURED_STREAM_LIMIT_BYTES + 4096
        );
        for stream in ["stdout", "stderr"] {
            let raw = logs.with_extension(format!("{stream}.txt"));
            assert_eq!(
                artifacts[&format!("raw_{stream}")],
                raw.to_string_lossy().as_ref()
            );
            assert_eq!(fs::read(raw).unwrap(), output.as_bytes());
        }
    }
    fs::remove_dir_all(temp_dir).unwrap();
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
        (capture::CAPTURED_STREAM_LIMIT_BYTES + 1024) as usize
    ])
    .unwrap();
    file.write_all(b"ERROR: last teardown\n").unwrap();
    drop(file);
    let (capture, total) = capture::read_bounded_capture(&path).unwrap();
    let capture = String::from_utf8(capture).unwrap();
    assert!(total > capture::CAPTURED_STREAM_LIMIT_BYTES);
    assert!(capture.len() < capture::CAPTURED_STREAM_LIMIT_BYTES as usize + 1024);
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
        false,
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
    assert!(truncation.stdout_total_bytes > capture::CAPTURED_STREAM_LIMIT_BYTES);
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
        false,
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
fn joypad_isolation_overrides_existing_command_env_only_when_enabled() {
    const NAME: &str = "SDL_GAMECONTROLLER_IGNORE_DEVICES_EXCEPT";
    let mut command = std::process::Command::new("godot");
    capture::apply_joypad_isolation(&mut command, false);
    assert_eq!(command.get_envs().count(), 0);
    command.env(NAME, "caller-value");
    capture::apply_joypad_isolation(&mut command, false);
    assert_eq!(
        command.get_envs().find(|(key, _)| *key == NAME).unwrap().1,
        Some(std::ffi::OsStr::new("caller-value"))
    );
    capture::apply_joypad_isolation(&mut command, true);
    assert_eq!(
        command.get_envs().find(|(key, _)| *key == NAME).unwrap().1,
        Some(std::ffi::OsStr::new("0xffff/0xffff"))
    );
}

#[test]
fn desktop_launches_apply_joypad_isolation_to_spawned_children() {
    const NAME: &str = "SDL_GAMECONTROLLER_IGNORE_DEVICES_EXCEPT";
    for enabled in [true, false] {
        for mode in [
            "check",
            "check_timeout",
            "check_fixed_fps",
            "scenario",
            "session",
            "import",
            "import_args",
            "patch",
            "asset_trial",
        ] {
            let temp_dir = unique_temp_dir().join("project with spaces");
            fs::create_dir_all(&temp_dir).expect("create temp dir");
            let env_path = temp_dir.join("env.txt");
            let binary_path = fixture(
                &temp_dir.join("fake godot"),
                &[(
                    "env_file:SDL_GAMECONTROLLER_IGNORE_DEVICES_EXCEPT",
                    env_path.to_str().unwrap(),
                )],
            );
            let binary = binary_path.to_str().unwrap();
            let request = temp_dir.join("request.json");
            fs::write(&request, "{}\n").unwrap();
            let logs = temp_dir.join("logs.txt");
            // Exercise a native launch prefix as well as direct launches.
            let wrapper = fixture(&temp_dir.join("wrapper"), &[("mode", "wrapper")]);
            let prefix = if enabled {
                vec![wrapper.to_string_lossy().into_owned()]
            } else {
                vec![]
            };
            let result = match mode {
                "check" => launch_godot_check(
                    binary,
                    &prefix,
                    enabled,
                    &[],
                    &[],
                    &temp_dir,
                    &request,
                    &logs,
                ),
                "check_timeout" => super::launch_godot_check_with_timeout(
                    binary,
                    &prefix,
                    enabled,
                    &[],
                    &[],
                    &temp_dir,
                    &request,
                    &logs,
                    Some(Duration::from_secs(5)),
                ),
                "check_fixed_fps" => super::launch_godot_check_with_timeout_and_fixed_fps(
                    binary,
                    &prefix,
                    enabled,
                    &[],
                    &[],
                    &temp_dir,
                    &request,
                    &logs,
                    None,
                    Some(60),
                ),
                "import" => launch_godot_import(binary, &prefix, enabled, &temp_dir, &logs),
                "import_args" => super::launch_godot_import_with_args(
                    binary,
                    &prefix,
                    enabled,
                    &[],
                    &temp_dir,
                    &logs,
                ),
                "patch" => launch_godot_patch_runner(
                    binary,
                    &prefix,
                    enabled,
                    &[],
                    &temp_dir,
                    &request,
                    &logs,
                ),
                "scenario" | "asset_trial" => {
                    let options = GodotOneShotLaunch {
                        binary,
                        launch_prefix: &prefix,
                        joypad_isolation: enabled,
                        windowed_launch_prefix: &[],
                        default_godot_args: &[],
                        extra_godot_args: &[],
                        default_project_args: &[],
                        extra_project_args: &[],
                        project_root: &temp_dir,
                        request_path: &request,
                        logs_path: &logs,
                        headless: true,
                        audio: false,
                        viewport: None,
                        timeout: Some(Duration::from_secs(5)),
                        fixed_fps: None,
                    };
                    if mode == "scenario" {
                        launch_godot_one_shot(options)
                    } else {
                        let _lease = super::project_access::ProjectLease::exclusive(&temp_dir)
                            .expect("coordinator lease");
                        let _scope = super::project_access::CoordinatorScope::enter(&temp_dir);
                        super::launch_godot_asset_trial_runner(options)
                    }
                }
                "session" => {
                    let launch = launch_godot_session(
                        binary,
                        &prefix,
                        enabled,
                        &[],
                        &temp_dir,
                        &request,
                        &logs,
                        43124,
                        "session-token",
                        true,
                        false,
                        None,
                        None,
                        &[],
                        &[],
                        &[],
                        None,
                    )
                    .expect("launch fake session");
                    assert_eq!(launch.display.joypad_isolation, enabled);
                    assert!(
                        super::wait_for_godot_process_exit(launch.pid, 5.0),
                        "session fixture exits"
                    );
                    // Detached session launch returns before the child writes its file.
                    assert_eq!(
                        fs::read_to_string(&env_path).expect("child env file"),
                        if enabled {
                            "0xffff/0xffff".to_owned()
                        } else {
                            env::var(NAME).unwrap_or_else(|_| "<unset>".into())
                        },
                        "mode={mode}, isolation={enabled}"
                    );
                    let _ = fs::remove_dir_all(temp_dir.parent().unwrap());
                    continue;
                }
                _ => unreachable!(),
            }
            .expect("run fake Godot");
            assert_eq!(result.returncode, 0, "mode={mode}");
            assert_eq!(result.display.joypad_isolation, enabled, "mode={mode}");
            assert_eq!(
                display::display_launch_metadata_value(&result.display)["joypad_isolation"],
                enabled,
                "mode={mode}"
            );
            assert_eq!(
                fs::read_to_string(&env_path).expect("child env file"),
                if enabled {
                    "0xffff/0xffff".to_owned()
                } else {
                    env::var(NAME).unwrap_or_else(|_| "<unset>".into())
                },
                "mode={mode}, isolation={enabled}"
            );
            let _ = fs::remove_dir_all(temp_dir.parent().unwrap());
        }
    }
}

#[test]
fn launch_godot_import_includes_artifact_log_file() {
    let temp_dir = unique_temp_dir();
    let binary_path = fixture(&temp_dir.join("true"), &[]);
    let logs_path = temp_dir.join("logs.txt");

    let result = launch_godot_import(
        binary_path.to_str().expect("binary path"),
        &[],
        false,
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
        false,
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
        false,
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
fn background_session_children_are_reaped() {
    let root = unique_temp_dir().join("sessions with spaces");
    fs::create_dir_all(&root).unwrap();
    let binary = fixture(&root.join("fake-godot"), &[("sleep_ms", "30000")]);
    let mut pids = Vec::new();
    for _ in 0..12 {
        let launch = launch_godot_session(
            binary.to_str().unwrap(),
            &[],
            false,
            &[],
            &root,
            &root.join("request.json"),
            &root.join("logs.txt"),
            43123,
            "test-token",
            true,
            false,
            None,
            None,
            &[],
            &[],
            &[],
            None,
        )
        .unwrap();
        super::stop_godot_process_with_identity(
            launch.pid,
            0.5,
            super::process_identity(launch.pid),
        )
        .unwrap();
        pids.push(launch.pid);
    }
    let deadline = std::time::Instant::now() + Duration::from_secs(2);
    while pids.iter().any(|pid| pid_is_running(*pid)) && std::time::Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(10));
    }
    let unreaped: Vec<_> = pids
        .iter()
        .copied()
        .filter(|pid| pid_is_running(*pid))
        .collect();
    for pid in pids {
        // Clean up zombies when testing the original handle-dropping bug.
        unsafe {
            libc::waitpid(pid as libc::pid_t, std::ptr::null_mut(), libc::WNOHANG);
        }
    }
    fs::remove_dir_all(root).unwrap();
    assert!(
        unreaped.is_empty(),
        "background session children were not reaped: {unreaped:?}"
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
        false,
        &[],
        &temp_dir,
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
            temp_dir.to_string_lossy().into_owned(),
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
        false,
        &[],
        &temp_dir,
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
        temp_dir.to_string_lossy().into_owned(),
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
        false,
        &[],
        &temp_dir,
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

#[cfg(target_os = "linux")]
#[test]
fn unix_process_identity_is_start_time() {
    let pid = i64::from(std::process::id());
    let stat = fs::read(format!("/proc/{pid}/stat")).unwrap();
    let expected = super::proc_stat::fields(&stat)
        .unwrap()
        .nth(19)
        .unwrap()
        .parse()
        .unwrap();
    assert_eq!(super::process_identity(pid), Some(expected));
    assert!(super::process_identity_matches(pid, Some(expected)));
    assert!(!super::process_identity_matches(pid, Some(expected + 1)));
    assert!(!super::process_identity_matches(pid, Some(0)));
}

#[cfg(target_os = "linux")]
#[test]
fn unix_mismatched_identity_refuses_termination() {
    use std::os::unix::process::CommandExt;
    let mut command = std::process::Command::new("sleep");
    command.arg("30").process_group(0);
    let mut child = command.spawn().unwrap();
    let pid = i64::from(child.id());
    let result = super::terminate_process_tree(pid, false, Some(0));
    let survived = !super::wait_for_godot_process_exit(pid, 0.1);
    let _ = child.kill();
    child.wait().unwrap();
    assert!(
        result.is_err(),
        "mismatched identity must refuse signalling"
    );
    assert!(survived, "unrelated process must remain alive");
}

#[cfg(target_os = "linux")]
#[test]
fn unix_legacy_identity_does_not_signal_group() {
    use std::os::unix::process::CommandExt;
    let root = unique_temp_dir();
    fs::create_dir_all(&root).unwrap();
    let wrapper = write_executable(
        &root.join("legacy.sh"),
        "#!/bin/sh\nsleep 30 &\nprintf '%s' \"$!\" > \"$1\"\nexec sleep 30\n",
    );
    let pid_file = root.join("child.pid");
    let mut command = std::process::Command::new(wrapper);
    command.arg(&pid_file).process_group(0);
    let mut child = probes::spawn_probe_child(&mut command).unwrap();
    let deadline = std::time::Instant::now() + Duration::from_secs(2);
    let descendant = loop {
        if let Some(pid) = fs::read_to_string(&pid_file)
            .ok()
            .and_then(|value| value.parse().ok())
        {
            break pid;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "child PID was not published"
        );
        std::thread::sleep(Duration::from_millis(10));
    };
    let result = super::stop_godot_process_with_identity(i64::from(child.id()), 0.5, None);
    let survived = !super::wait_for_godot_process_exit(descendant, 0.1);
    let _ = lifecycle::send_signal(descendant, libc::SIGKILL);
    let _ = child.kill();
    child.wait().unwrap();
    fs::remove_dir_all(root).unwrap();
    assert!(result.is_ok());
    assert!(
        survived,
        "legacy metadata must not authorize group signalling"
    );
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
    let output = capture::run_owned_child_output(
        &[wrapper.to_string_lossy().into_owned()],
        false,
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
    let contents = fs::read(stat_path).expect("read stat");
    super::proc_stat::fields(&contents)
        .unwrap()
        .nth(3)
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
