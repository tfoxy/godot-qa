use super::*;

#[cfg(target_os = "linux")]
#[test]
fn stale_session_identity_leaves_unrelated_process_untouched() {
    let root = std::env::temp_dir().join(format!("gqa-stale-identity-{}", unique_session_id()));
    fs::create_dir_all(&root).unwrap();
    fs::write(root.join("project.godot"), "[application]\n").unwrap();
    let config = godot_qa_core::config::load_project_config(Some(&root), None, None, false, None);
    let mut prepared =
        prepare_live_session(&config, "res://main.tscn", vec![1280, 720], true, &[], None).unwrap();
    let mut child = std::process::Command::new("sleep")
        .arg("30")
        .spawn()
        .unwrap();
    prepared.record.pid = i64::from(child.id());
    prepared.record.process_identity = Some(0);
    persist_session(&config, &prepared.record, &prepared.token).unwrap();
    let result = session_stop_result(&config);
    let survived = !godot_qa_process::wait_for_godot_process_exit(prepared.record.pid, 0.1);
    let _ = child.kill();
    child.wait().unwrap();
    fs::remove_dir_all(root).unwrap();
    assert!(
        survived,
        "stale session metadata signalled an unrelated process"
    );
    assert!(result.ok, "stale metadata cleanup failed: {result:?}");
}

#[test]
fn debug_host_arguments_use_resolved_launch_scene() {
    let root = std::env::temp_dir().join(format!("gqa-launch-scene-{}", unique_session_id()));
    fs::create_dir_all(&root).unwrap();
    fs::write(
        root.join("project.godot"),
        "[application]\nrun/main_scene=\"res://main.tscn\"\n",
    )
    .unwrap();
    let config = godot_qa_core::config::load_project_config(Some(&root), None, None, false, None);
    for explicit in [Some("res://alternate.tscn"), Some(""), None] {
        let scene = resolve_session_start_scene(&config, explicit).unwrap();
        let command = debug_host_command(Path::new("godot-qa"), &config, &scene);
        let args: Vec<_> = command.get_args().collect();
        let launch_scene = args
            .windows(2)
            .find(|pair| pair[0] == "--scene")
            .map(|pair| pair[1].to_str().unwrap());
        assert_eq!(
            launch_scene,
            Some(scene.as_str()),
            "explicit scene {explicit:?}"
        );
    }
    let _ = fs::remove_dir_all(root);
}

#[test]
fn full_scene_reload_refreshes_baseline_only_after_success() {
    use godot_qa_debug::host::{DebugHost, DebugHostConfig};
    use std::io::{BufRead, BufReader};
    use std::net::{Shutdown, TcpListener};
    use std::sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    };

    let root = std::env::temp_dir().join(format!("gqa-reload-baseline-{}", unique_session_id()));
    fs::create_dir_all(&root).unwrap();
    fs::write(
        root.join("project.godot"),
        "[application]\nrun/main_scene=\"res://main.tscn\"\n",
    )
    .unwrap();
    let scene_file = root.join("main.tscn");
    fs::write(
        &scene_file,
        "[gd_scene format=3]\n[node name=\"Main\" type=\"Node\"]\n",
    )
    .unwrap();
    let config = godot_qa_core::config::load_project_config(Some(&root), None, None, false, None);
    let mut prepared =
        prepare_live_session(&config, "res://main.tscn", vec![1280, 720], true, &[], None).unwrap();
    let bridge = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    prepared.record.port = bridge.local_addr().unwrap().port();
    prepared.record.pid = std::process::id() as i64;
    prepared.record.process_identity = godot_qa_process::process_identity(prepared.record.pid);
    persist_session(&config, &prepared.record, &prepared.token).unwrap();
    let host = DebugHost::bind(&DebugHostConfig {
        record_path: debug_host_json_path(&config),
        log_path: None,
        game_accept_timeout: Duration::from_secs(5),
        project_root: Some(root.clone()),
        initial_scene: Some("res://main.tscn".into()),
    })
    .unwrap();
    let mut record = load_debug_host_record(&config).unwrap();
    record.process_identity = prepared.record.process_identity;
    write_json(
        &debug_host_json_path(&config),
        &serde_json::to_value(record).unwrap(),
    )
    .unwrap();
    let mut game = TcpStream::connect(("127.0.0.1", host.dbg_port())).unwrap();
    game.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
    let shutdown = Arc::new(AtomicBool::new(false));
    let run_shutdown = Arc::clone(&shutdown);
    let host_thread = thread::spawn(move || host.run(run_shutdown));
    for _ in 0..2 {
        let mut length = [0; 4];
        game.read_exact(&mut length).unwrap();
        game.read_exact(&mut vec![0; u32::from_le_bytes(length) as usize])
            .unwrap();
    }
    let bridge_thread = thread::spawn(move || {
        for ok in [false, true] {
            let (mut stream, _) = bridge.accept().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(3)))
                .unwrap();
            let mut line = String::new();
            BufReader::new(stream.try_clone().unwrap())
                .read_line(&mut line)
                .unwrap();
            let request: Value = serde_json::from_str(&line).unwrap();
            assert_eq!(request["command"], SESSION_RELOAD_SCENE);
            let response = if ok {
                json!({"ok": true, "protocol_version": PROTOCOL_VERSION, "data": {"scene": "res://main.tscn"}})
            } else {
                json!({"ok": false, "protocol_version": PROTOCOL_VERSION, "error": {"code": "scene_load_failed", "message": "test reload rejected"}})
            };
            writeln!(stream, "{response}").unwrap();
        }
    });
    let scenes = vec!["res://main.tscn".to_string()];
    let unsupported =
        "[gd_scene format=3]\n[node name=\"Main\" type=\"Node2D\"]\nmetadata/marker = 1\n";
    fs::write(&scene_file, unsupported).unwrap();
    let fallback = session_reload_structure_result(&config, &scenes);
    assert!(
        fallback.ok && fallback.data.contains_key("fell_back"),
        "{fallback:?}"
    );
    let failed = session_reload_result(&config);
    assert!(!failed.ok, "{failed:?}");
    let still_fallback = session_reload_structure_result(&config, &scenes);
    assert!(
        still_fallback.data.contains_key("fell_back"),
        "failed reload advanced baseline: {still_fallback:?}"
    );
    let reloaded = session_reload_result(&config);
    assert!(reloaded.ok, "{reloaded:?}");
    game.set_read_timeout(Some(Duration::from_millis(100)))
        .unwrap();
    assert!(
        game.read(&mut [0]).is_err(),
        "confirmation sent debugger frames"
    );
    game.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
    fs::write(&scene_file, unsupported.replace("marker = 1", "marker = 2")).unwrap();
    let supported = session_reload_structure_result(&config, &scenes);
    assert!(
        supported.ok && !supported.data.contains_key("fell_back"),
        "successful full reload left stale baseline: {supported:?}"
    );
    assert_eq!(supported.data["ops_applied"], 1);
    shutdown.store(true, Ordering::SeqCst);
    game.shutdown(Shutdown::Both).unwrap();
    host_thread.join().unwrap().unwrap();
    bridge_thread.join().unwrap();
    let _ = fs::remove_dir_all(root);
}

#[test]
fn resource_reload_reports_requested_with_caveat_not_applied() {
    let result = granular_reload_success(
        "session reload",
        "reload_resources",
        vec!["res://ui/theme.tres".to_string()],
        true,
    );
    assert!(result.ok);
    assert_eq!(result.data.get("status"), Some(&json!("requested")));
    // Never claim resources were "applied" (fire-and-forget, DEBUG_ENABLED-gated).
    assert!(result.data.get("applied").is_none());
    assert_eq!(
        result.data.get("requested"),
        Some(&json!(["res://ui/theme.tres"]))
    );
    assert_eq!(
        result.data.get("caveat"),
        Some(&json!(RESOURCE_RELOAD_CAVEAT))
    );
}

#[test]
fn script_reload_reports_applied() {
    let result = granular_reload_success(
        "session reload",
        "reload_scripts",
        vec!["res://player.gd".to_string()],
        true,
    );
    assert!(result.ok);
    assert_eq!(result.data.get("status"), Some(&json!("applied")));
    assert_eq!(
        result.data.get("applied"),
        Some(&json!(["res://player.gd"]))
    );
    assert!(result.data.get("caveat").is_none());
}
