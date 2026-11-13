#[cfg(unix)]
#[test]
fn session_tokens_are_owner_only_under_umask_022() {
    use std::os::unix::fs::PermissionsExt;
    use std::os::unix::process::CommandExt;

    // Set the process-wide umask only in an isolated child before it starts.
    const CHILD: &str = "GODOT_QA_TOKEN_PERMISSION_CHILD";
    if std::env::var_os(CHILD).is_none() {
        let mut child = std::process::Command::new(std::env::current_exe().unwrap());
        child.args([
            "--exact",
            "tests::session_tokens_are_owner_only_under_umask_022",
        ]);
        child.env(CHILD, "1");
        unsafe {
            child.pre_exec(|| {
                libc::umask(0o022);
                Ok(())
            });
        }
        assert!(child.status().unwrap().success());
        return;
    }
    let fx = deploy_fixture("token-permissions");
    let record = test_session_record(&fx.config, &fx.base, 12345);
    persist_session(&fx.config, &record, "singleton-secret").unwrap();
    let singleton = session_token_path(&fx.config);
    let request = AndroidScenarioSessionRequest {
        preset: "Android QA",
        device: Some("device-1"),
        package: None,
        project_args: &[],
        no_apk_cache: false,
        deployment_log: fx.deployment_log.clone(),
        logcat_log: fx.logcat_log.clone(),
    };
    let handle = deploy_android_scenario_session(
        &fx.config,
        &request,
        fx.godot.to_str().unwrap(),
        fx.adb.to_str().unwrap(),
        &fx.device,
        &fx.preset,
        &|_, _| Ok(json!({"ok": true, "data": {"viewport": [1080, 1920]}})),
    )
    .unwrap();
    let pooled = PathBuf::from(&handle.record.token_file);
    let modes: Vec<_> = [&singleton, &pooled]
        .into_iter()
        .map(|path| {
            (
                path.clone(),
                fs::metadata(path).unwrap().permissions().mode() & 0o777,
            )
        })
        .collect();
    let _ = fs::remove_dir_all(&fx.base);
    assert!(modes.iter().all(|(_, mode)| *mode == 0o600), "{modes:?}");
}

#[cfg(unix)]
#[test]
fn existing_session_token_permissions_are_repaired() {
    use std::os::unix::fs::PermissionsExt;
    let root = temp_root("existing-token-permissions");
    let config = test_config(&root);
    let record = test_session_record(&config, &root, 12345);
    persist_session(&config, &record, "old-secret").unwrap();
    let path = session_token_path(&config);
    fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
    assert_eq!(
        read_session_token(&record, SESSION_STATUS).unwrap(),
        "old-secret"
    );
    assert_eq!(
        fs::metadata(&path).unwrap().permissions().mode() & 0o777,
        0o600
    );
    fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
    persist_session(&config, &record, "new-secret").unwrap();
    assert_eq!(
        fs::metadata(&path).unwrap().permissions().mode() & 0o777,
        0o600
    );
    assert_eq!(fs::read_to_string(&path).unwrap(), "new-secret");
    let _ = fs::remove_dir_all(root);
}

#[cfg(unix)]
#[test]
fn file_transport_requests_are_owner_only() {
    use std::os::unix::fs::PermissionsExt;
    let root = temp_root("rpc-token-permissions");
    let config = test_config(&root);
    let record = test_session_record(&config, &root, 12345);
    let rpc = Path::new(&record.artifact_dir).join("rpc");
    let peer = thread::spawn(move || {
        let path = rpc.join("requests/permission-test.json");
        let deadline = std::time::Instant::now() + Duration::from_secs(3);
        while !path.exists() {
            assert!(std::time::Instant::now() < deadline);
            thread::sleep(Duration::from_millis(5));
        }
        let mode = fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        fs::write(rpc.join("responses/permission-test.json"), b"{}").unwrap();
        mode
    });
    send_file_live_request(
        &record,
        Path::new(&record.request_path),
        SESSION_STATUS,
        "permission-test",
        &json!({"token": "secret"}),
        "permission denied",
        Duration::from_secs(3),
    )
    .unwrap();
    let mode = peer.join().unwrap();
    let _ = fs::remove_dir_all(root);
    assert_eq!(mode, 0o600);
}

#[test]
fn delayed_mutation_response_does_not_resend_request() {
    use std::io::BufRead;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

    let root = temp_root("delayed-mutation");
    let config = test_config(&root);
    let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
    listener.set_nonblocking(true).unwrap();
    let record = test_session_record(&config, &root, listener.local_addr().unwrap().port());
    persist_session(&config, &record, "mutation-token").unwrap();
    let executions = Arc::new(AtomicUsize::new(0));
    let stop = Arc::new(AtomicBool::new(false));
    let peer_executions = executions.clone();
    let peer_stop = stop.clone();
    let response_delay = live_request_timeout(INPUT_CLICK_TARGET, &Map::new()).unwrap()
        + LIVE_QUEUE_WAIT_ALLOWANCE
        + Duration::from_millis(500);
    let peer = thread::spawn(move || {
        let mut clients = Vec::new();
        while !peer_stop.load(Ordering::SeqCst) {
            match listener.accept() {
                Ok((mut stream, _)) => {
                    let executions = peer_executions.clone();
                    clients.push(thread::spawn(move || {
                        stream
                            .set_read_timeout(Some(Duration::from_secs(2)))
                            .unwrap();
                        let mut line = String::new();
                        std::io::BufReader::new(&mut stream)
                            .read_line(&mut line)
                            .unwrap();
                        let request: Value = serde_json::from_str(&line).unwrap();
                        assert_eq!(request["command"], INPUT_CLICK_TARGET);
                        let execution = executions.fetch_add(1, Ordering::SeqCst);
                        if execution == 0 {
                            thread::sleep(response_delay);
                        }
                        let response = json!({
                            "protocol_version": PROTOCOL_VERSION,
                            "id": request["id"], "ok": true, "data": {}
                        });
                        let _ = writeln!(stream, "{response}");
                    }));
                }
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    thread::sleep(Duration::from_millis(5));
                }
                Err(error) => panic!("fake bridge accept failed: {error}"),
            }
        }
        for client in clients {
            client.join().unwrap();
        }
    });
    let result = run_live_protocol_command(
        &config,
        "input click",
        INPUT_CLICK_TARGET,
        Map::from_iter([("target".to_string(), json!("qa:mutate"))]),
    );
    stop.store(true, Ordering::SeqCst);
    peer.join().unwrap();
    let count = executions.load(Ordering::SeqCst);
    let _ = fs::remove_dir_all(root);
    assert_eq!(count, 1, "a delayed response must not duplicate a mutation");
    assert!(!result.ok);
    assert_eq!(result.failures[0].kind, "session_request_failed");
    assert!(!result.data.contains_key("retry"));
}

#[test]
fn file_transport_response_timeout_is_not_retryable() {
    let root = temp_root("rpc-response-no-retry");
    let config = test_config(&root);
    let record = test_session_record(&config, &root, 12345);
    let failure = send_file_live_request(
        &record,
        Path::new(&record.request_path),
        INPUT_CLICK_TARGET,
        "timeout-test",
        &json!({"token": "secret"}),
        "permission denied",
        Duration::ZERO,
    )
    .unwrap_err();
    assert_eq!(failure.kind, "session_request_failed");
    assert!(!live_command_failure_is_retryable(&failure));
    assert!(
        !Path::new(&record.artifact_dir)
            .join("rpc/requests/timeout-test.json")
            .exists()
    );
    let _ = fs::remove_dir_all(root);
}

fn read_fake_bridge_request(stream: &mut TcpStream) -> Value {
    use std::io::BufRead;
    stream
        .set_read_timeout(Some(Duration::from_secs(2)))
        .unwrap();
    let mut line = String::new();
    std::io::BufReader::new(stream)
        .read_line(&mut line)
        .unwrap();
    serde_json::from_str(&line).unwrap()
}

#[test]
fn queued_status_waits_for_long_operation_without_retry() {
    let root = temp_root("queued-status");
    let config = test_config(&root);
    let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let record = test_session_record(&config, &root, listener.local_addr().unwrap().port());
    persist_session(&config, &record, "queue-token").unwrap();
    let (started, waiting) = std::sync::mpsc::channel();
    let peer = thread::spawn(move || {
        let (mut long, _) = listener.accept().unwrap();
        let request = read_fake_bridge_request(&mut long);
        assert_eq!(request["command"], SESSION_WAIT_FRAMES);
        started.send(()).unwrap();
        let (mut queued, _) = listener.accept().unwrap();
        let queued_request = read_fake_bridge_request(&mut queued);
        assert_eq!(queued_request["command"], SESSION_STATUS);
        // The serialized bridge cannot answer status until this operation ends.
        thread::sleep(Duration::from_secs(20));
        for (stream, request) in [(&mut long, request), (&mut queued, queued_request)] {
            let _ = writeln!(
                stream,
                "{}",
                json!({
                    "protocol_version": PROTOCOL_VERSION, "id": request["id"],
                    "ok": true, "data": {}
                })
            );
        }
        listener.set_nonblocking(true).unwrap();
        assert!(listener.accept().is_err(), "queued command was resubmitted");
    });
    let long_config = config.clone();
    let long = thread::spawn(move || {
        run_live_protocol_command(
            &long_config,
            "session wait",
            SESSION_WAIT_FRAMES,
            Map::from_iter([("frames".to_string(), json!(600))]),
        )
    });
    waiting.recv_timeout(Duration::from_secs(3)).unwrap();
    let result = run_live_protocol_command(&config, "session status", SESSION_STATUS, Map::new());
    peer.join().unwrap();
    assert!(long.join().unwrap().ok);
    let _ = fs::remove_dir_all(root);
    assert!(result.ok, "{:?}", result.failures);
    assert!(!result.data.contains_key("retry"));
}

#[test]
fn trickling_response_obeys_absolute_deadline() {
    let root = temp_root("trickling-response");
    let config = test_config(&root);
    let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let record = test_session_record(&config, &root, listener.local_addr().unwrap().port());
    let peer = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        read_fake_bridge_request(&mut stream);
        for _ in 0..10 {
            if stream.write_all(b" ").is_err() {
                return;
            }
            thread::sleep(Duration::from_millis(60));
        }
        let _ = stream.write_all(b"{}\n");
    });
    let started = std::time::Instant::now();
    let result = send_live_request_with_timeout(
        &record,
        &session_json_path(&config),
        SESSION_STATUS,
        Map::new(),
        Some("trickle-token"),
        Duration::from_millis(200),
    );
    let elapsed = started.elapsed();
    peer.join().unwrap();
    let _ = fs::remove_dir_all(root);
    let failure = result.unwrap_err();
    assert_eq!(failure.kind, "session_request_failed");
    assert!(elapsed < Duration::from_millis(500), "{elapsed:?}");
    assert!(!live_command_failure_is_retryable(&failure));
}

#[test]
fn oversized_response_returns_structured_failure() {
    let root = temp_root("oversized-response");
    let config = test_config(&root);
    let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let record = test_session_record(&config, &root, listener.local_addr().unwrap().port());
    let peer = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        read_fake_bridge_request(&mut stream);
        stream
            .set_write_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        for _ in 0..1025 {
            if stream.write_all(&[b' '; 65536]).is_err() {
                return;
            }
        }
        let _ = stream.write_all(b"{}\n");
    });
    let result = send_live_request_with_timeout(
        &record,
        &session_json_path(&config),
        TREE_DUMP,
        Map::new(),
        Some("oversize-token"),
        Duration::from_secs(3),
    );
    peer.join().unwrap();
    let _ = fs::remove_dir_all(root);
    let failure = result.unwrap_err();
    assert_eq!(failure.kind, "session_request_failed");
    assert_eq!(failure.command.as_deref(), Some(TREE_DUMP));
    assert_eq!(
        failure.expected,
        Some(json!({"max_response_bytes": 64 * 1024 * 1024}))
    );
    assert!(
        failure.actual.as_ref().unwrap()["response_bytes"]
            .as_u64()
            .unwrap()
            > 64 * 1024 * 1024
    );
    assert!(!live_command_failure_is_retryable(&failure));
}

#[test]
fn oversized_file_response_returns_structured_failure() {
    let root = temp_root("oversized-file-response");
    let config = test_config(&root);
    let record = test_session_record(&config, &root, 12345);
    let responses = Path::new(&record.artifact_dir).join("rpc/responses");
    fs::create_dir_all(&responses).unwrap();
    let path = responses.join("oversize.json");
    fs::File::create(&path)
        .unwrap()
        .set_len(64 * 1024 * 1024 + 1)
        .unwrap();
    let failure = send_file_live_request(
        &record,
        &session_json_path(&config),
        TREE_DUMP,
        "oversize",
        &json!({}),
        "permission denied",
        Duration::from_secs(2),
    )
    .unwrap_err();
    let _ = fs::remove_dir_all(root);
    assert_eq!(failure.kind, "session_request_failed");
    assert_eq!(
        failure.expected,
        Some(json!({"max_response_bytes": 64 * 1024 * 1024}))
    );
}

#[test]
fn extreme_audio_duration_returns_structured_failure_without_transmission() {
    for command in [AUDIO_CAPTURE, AUDIO_RENDER] {
        assert_extreme_timeout_rejected(
            command,
            Map::from_iter([
                ("duration_seconds".to_string(), json!(1e30)),
                ("max_seconds".to_string(), json!(1e30)),
            ]),
        );
    }
}

#[test]
fn extreme_scenario_frame_backstop_returns_structured_failure_without_transmission() {
    assert_extreme_timeout_rejected(
        SCENARIO_RUN,
        Map::from_iter([
            ("timeout_seconds".to_string(), json!(1e30)),
            ("timeout_frames".to_string(), json!(60)),
        ]),
    );
}

fn assert_extreme_timeout_rejected(command: &str, params: Map<String, Value>) {
    let root = temp_root("extreme-timeout");
    let config = test_config(&root);
    let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
    listener.set_nonblocking(true).unwrap();
    let record = test_session_record(&config, &root, listener.local_addr().unwrap().port());
    persist_session(&config, &record, "duration-token").unwrap();
    let result = run_live_protocol_command(&config, command, command, params);
    assert!(!result.ok);
    assert_eq!(result.exit_code, 1);
    let failure = &result.failures[0];
    assert_eq!(failure.kind, "session_timeout_invalid");
    assert_eq!(failure.command.as_deref(), Some(command));
    assert_eq!(failure.path.as_deref(), session_json_path(&config).to_str());
    assert_eq!(
        failure.expected,
        Some(json!({"max_timeout_seconds": 86400}))
    );
    assert_eq!(failure.actual, Some(json!({"timeout_seconds": 1e30})));
    assert!(!live_command_failure_is_retryable(failure));
    assert!(
        listener.accept().is_err(),
        "invalid timeout must fail before connecting"
    );
    assert!(serde_json::to_value(&result).unwrap().is_object());
    let _ = fs::remove_dir_all(root);
}

#[test]
fn audio_timeout_enforces_practical_upper_bound() {
    for command in [AUDIO_CAPTURE, AUDIO_RENDER] {
        let params = Map::from_iter([("duration_seconds".to_string(), json!(86395))]);
        assert_eq!(
            live_request_timeout(command, &params).unwrap(),
            Duration::from_secs(86400)
        );
        let params = Map::from_iter([("duration_seconds".to_string(), json!(86396))]);
        let failure = live_request_timeout(command, &params).unwrap_err();
        assert_eq!(failure.kind, "session_timeout_invalid");
        assert_eq!(failure.actual, Some(json!({"timeout_seconds": 86401.0})));
    }
}
