#[cfg(test)]
mod android_tests {
    use super::*;
    use godot_qa_runtime::android_artifacts::AndroidArtifactMapper;
    use godot_qa_runtime::{AndroidSessionMetadata, SessionRecord, SessionTarget};
    #[cfg(unix)]
    use godot_qa_runtime::PooledSessionHandle;
    #[cfg(unix)]
    use std::io::{BufRead, BufReader, Write as _};
    #[cfg(unix)]
    use std::net::TcpListener;
    #[cfg(unix)]
    use std::thread;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn unique_temp(name: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!("godot-qa-android-{name}-{unique}"));
        fs::create_dir_all(&path).unwrap();
        path
    }

    fn options<'a>(preset: Option<&'a str>) -> ScenarioRunAndroidOptions<'a> {
        ScenarioRunAndroidOptions {
            preset,
            device: None,
            package: None,
            no_apk_cache: false,
            audio: false,
            no_audio: false,
            headless: false,
            windowed: false,
            visual_risk: false,
            visual_risk_strict: false,
        }
    }

    // --- Option validation ------------------------------------------------

    #[test]
    fn option_rejection_table() {
        // Each rejected display/audio/visual flag maps to android_option_unsupported.
        let rejected: [(&str, fn(&mut ScenarioRunAndroidOptions)); 5] = [
            ("--headless", |o| o.headless = true),
            ("--windowed", |o| o.windowed = true),
            ("--no-audio", |o| o.no_audio = true),
            ("--visual-risk", |o| o.visual_risk = true),
            ("--visual-risk-strict", |o| o.visual_risk_strict = true),
        ];
        for (label, apply) in rejected {
            let mut opts = options(Some("Android QA"));
            apply(&mut opts);
            let failure = validate_android_scenario_options(&opts).unwrap_err();
            assert_eq!(failure.kind, "android_option_unsupported", "{label}");
            assert!(
                failure.message.as_deref().unwrap().contains(label),
                "message should name {label}"
            );
        }
        // Missing preset -> android_preset_required.
        let failure = validate_android_scenario_options(&options(None)).unwrap_err();
        assert_eq!(failure.kind, "android_preset_required");
        let failure = validate_android_scenario_options(&options(Some("  "))).unwrap_err();
        assert_eq!(failure.kind, "android_preset_required");
        // --audio + a valid preset is accepted (device audio is real).
        let mut ok = options(Some("Android QA"));
        ok.audio = true;
        assert_eq!(validate_android_scenario_options(&ok).unwrap(), "Android QA");
    }

    #[test]
    fn desktop_rejects_android_flags_table() {
        assert!(desktop_rejects_android_scenario_flags(None, None, None, false).is_none());
        for failure in [
            desktop_rejects_android_scenario_flags(Some("p"), None, None, false),
            desktop_rejects_android_scenario_flags(None, Some("d"), None, false),
            desktop_rejects_android_scenario_flags(None, None, Some("pkg"), false),
            desktop_rejects_android_scenario_flags(None, None, None, true),
        ] {
            let failure = failure.expect("android flag on desktop must be rejected");
            assert_eq!(failure.kind, "invalid_params");
            assert!(
                failure
                    .message
                    .as_deref()
                    .unwrap()
                    .contains("requires --target android")
            );
        }
    }

    // --- Scenario preflight ------------------------------------------------

    #[test]
    fn classify_android_support_table() {
        let cases: [(Value, Option<&str>); 8] = [
            (json!({"name": "x", "scene": "res://x.tscn"}), None),
            (
                json!({"name": "x", "scene": "res://x.tscn", "steps": [{"wait_frames": 1}], "assert": [{"exists": "qa:btn"}]}),
                None,
            ),
            (
                json!({"name": "x", "scene": "res://x.tscn", "godot_args": ["--single-window"]}),
                Some("godot_args"),
            ),
            (
                json!({"name": "x", "scene": "res://x.tscn", "determinism": {"fixed_fps": 30}}),
                Some("fixed_fps"),
            ),
            (
                json!({"name": "x", "scene": "res://x.tscn", "viewport": {"width": 800, "height": 600}}),
                Some("viewport_override"),
            ),
            (
                json!({"name": "x", "scene": "res://x.tscn", "viewports": [[1920, 1080], [1280, 720]]}),
                Some("multi_viewport"),
            ),
            (
                json!({"name": "x", "scene": "res://x.tscn", "assert": [{"layout_audit": "default"}]}),
                Some("layout_matrix"),
            ),
            (
                json!({"name": "x", "scene": "res://x.tscn", "assert": [{"screenshot_match": {"baseline": "b.png"}}]}),
                Some("screenshot_match"),
            ),
        ];
        for (scenario, expected) in cases {
            match (classify_android_support(&scenario), expected) {
                (Ok(()), None) => {}
                (Err(failure), Some(reason)) => {
                    assert_eq!(failure.kind, "android_scenario_unsupported");
                    assert_eq!(
                        failure.details.get("reason"),
                        Some(&json!(reason)),
                        "scenario {scenario} should reject with reason {reason}"
                    );
                }
                (result, expected) => {
                    panic!("scenario {scenario}: got {result:?}, expected {expected:?}")
                }
            }
        }
    }

    #[test]
    fn normalized_minimal_document_passes_preflight() {
        // A scenario that omits `viewport` becomes `viewport: {}` after
        // normalization; an empty normalized object is not a declaration, so the
        // Android one-shot preflight must still accept it.
        let document = json!({
            "name": "smoke",
            "scene": "res://smoke.tscn",
            "steps": [{"wait_frames": 1}],
        });
        let normalized = crate::scenario_doc::normalize_scenario_document(&document);
        assert_eq!(normalized.get("viewport"), Some(&json!({})));
        assert!(
            classify_android_support(&normalized).is_ok(),
            "normalized minimal document must pass preflight: {normalized}"
        );
    }

    #[test]
    fn audio_capture_and_save_screenshot_are_supported() {
        // Audio evidence and windowed-media save_screenshot are allowed on device.
        for scenario in [
            json!({"name": "x", "scene": "res://x.tscn", "steps": [{"audio_capture": {"name": "m", "duration_seconds": 0.5}}]}),
            json!({"name": "x", "scene": "res://x.tscn", "snapshots": {"save_screenshot": true}}),
        ] {
            assert!(classify_android_support(&scenario).is_ok(), "{scenario}");
        }
    }

    #[test]
    fn android_session_active_failure_shape() {
        let record = fake_record("sess-conflict", 0);
        let failure = android_session_active_failure(&record);
        assert_eq!(failure.kind, "android_session_active");
        assert!(
            failure
                .message
                .as_deref()
                .unwrap()
                .contains("session stop")
        );
    }

    // --- Request rewrite ---------------------------------------------------

    fn run_artifacts(root: &Path) -> artifacts::RunArtifacts {
        artifacts::RunArtifacts {
            run_id: "run-1".to_string(),
            root_dir: root.to_path_buf(),
            meta_path: root.join("meta.json"),
            request_path: root.join("request.json"),
            result_path: root.join("result.json"),
            logs_path: root.join("logs.txt"),
            progress_path: root.join("progress.jsonl"),
            slow_summary_path: root.join("slow-summary.json"),
            snapshots_dir: root.join("snapshots"),
            temporal_dir: root.join("temporal"),
            audio_dir: root.join("audio"),
        }
    }

    #[test]
    fn request_rewrite_devices_paths_strips_frame_budget_and_leaves_res_paths() {
        let root = unique_temp("request-rewrite");
        let artifacts = run_artifacts(&root);
        let host_request = json!({
            "scenario": {"name": "x", "scene": "res://main.tscn", "project_args": ["--seed", "7"]},
            "artifacts": artifacts.to_json(),
            "run": {
                "id": "run-1",
                "godot_args": [],
                "frame_budget": {"frames": 300, "fixed_fps": 60},
            },
        });
        write_json(&artifacts.request_path, &host_request).unwrap();

        let device_root = "user://godot_qa/live/sess-1/run";
        let mapper = AndroidArtifactMapper::new(&root, device_root).unwrap();
        let bytes = android_scenario_request_bytes(&mapper, &artifacts).unwrap();
        let device_request: Value = serde_json::from_slice(&bytes).unwrap();

        // Every artifact string is device-rooted; frame_budget is gone; res:// stays.
        assert_eq!(device_request["artifacts"]["run_root"], json!(device_root));
        assert_eq!(
            device_request["artifacts"]["result_path"],
            json!(format!("{device_root}/result.json"))
        );
        assert_eq!(
            device_request["artifacts"]["snapshots_dir"],
            json!(format!("{device_root}/snapshots"))
        );
        assert!(device_request["run"].get("frame_budget").is_none());
        assert_eq!(device_request["scenario"]["scene"], json!("res://main.tscn"));

        // The re-persisted host copy keeps host paths but also lost frame_budget.
        let host_after: Value =
            serde_json::from_str(&fs::read_to_string(&artifacts.request_path).unwrap()).unwrap();
        assert!(host_after["run"].get("frame_budget").is_none());
        assert_eq!(host_after["artifacts"]["run_root"], json!(root));

        let _ = fs::remove_dir_all(&root);
    }

    // --- Artifacts-echo substitution --------------------------------------

    #[test]
    fn echoed_run_artifacts_are_substituted_everywhere() {
        let root = unique_temp("echo-subst");
        let artifacts = run_artifacts(&root);
        let host_artifacts = artifacts.to_json().as_object().cloned().unwrap();
        let device_root = "user://godot_qa/live/sess-1/run";
        let mut result = json!({
            "artifacts": {
                "run_root": device_root,
                "logs_path": format!("{device_root}/logs.txt"),
                "result_path": format!("{device_root}/result.json"),
            },
            "failures": [{
                "type": "x",
                "artifacts": {
                    "logs_path": format!("{device_root}/logs.txt"),
                    "screenshot": format!("{device_root}/failure.png"),
                },
            }],
            "note": "res://ui/main.tscn",
        });
        substitute_echoed_run_artifacts(&mut result, &host_artifacts);

        // Echoed run-level keys become host paths at every "artifacts" object.
        assert_eq!(result["artifacts"]["logs_path"], json!(artifacts.logs_path));
        assert_eq!(result["artifacts"]["run_root"], json!(artifacts.root_dir));
        assert_eq!(
            result["failures"][0]["artifacts"]["logs_path"],
            json!(artifacts.logs_path)
        );
        // A non-run-level evidence key (screenshot) is left for materialize to pull.
        assert_eq!(
            result["failures"][0]["artifacts"]["screenshot"],
            json!(format!("{device_root}/failure.png"))
        );
        // A plain res:// string is untouched.
        assert_eq!(result["note"], json!("res://ui/main.tscn"));
        let _ = fs::remove_dir_all(&root);
    }

    // --- Collection walk with a fake puller -------------------------------

    #[test]
    fn collection_walk_pulls_only_evidence_and_rewrites_progress() {
        let host_root = unique_temp("collect-walk");
        let artifacts = run_artifacts(&host_root);
        let host_artifacts = artifacts.to_json().as_object().cloned().unwrap();
        let device_root = "user://godot_qa/live/sess-1/run";
        let mapper = AndroidArtifactMapper::new(&host_root, device_root).unwrap();

        let device_result = json!({
            "ok": false,
            "artifacts": {
                "run_root": device_root,
                "logs_path": format!("{device_root}/logs.txt"),
                "result_path": format!("{device_root}/result.json"),
                "snapshots_dir": format!("{device_root}/snapshots"),
            },
            "snapshot_artifacts": {
                "current": {
                    "root": format!("{device_root}/snapshots/current"),
                    "screenshot": format!("{device_root}/snapshots/current/screenshot.png"),
                }
            },
            "failures": [{
                "type": "focused_mismatch",
                "artifacts": {
                    "logs_path": format!("{device_root}/logs.txt"),
                    "screenshot": format!("{device_root}/failure.png"),
                }
            }],
        });
        let bytes = serde_json::to_vec(&device_result).unwrap();

        let mut pulled: Vec<String> = Vec::new();
        let mut puller = |device_path: &str, temporary: &Path| -> Result<(), FailureDetail> {
            pulled.push(device_path.to_string());
            fs::write(temporary, b"evidence").map_err(|error| android_artifact_io_failure(temporary, &error))
        };
        let result = rewrite_collected_result(&bytes, &host_artifacts, &mapper, &mut puller).unwrap();

        // Only real evidence is pulled: the failure screenshot and the snapshot
        // screenshot. Host-only run-level files (logs.txt) are never pulled.
        pulled.sort();
        assert_eq!(
            pulled,
            vec![
                format!("{device_root}/failure.png"),
                format!("{device_root}/snapshots/current/screenshot.png"),
            ]
        );
        // Echoed run-level artifacts are host paths; evidence is host paths; the
        // snapshot bundle root is created as a directory.
        assert_eq!(result["artifacts"]["logs_path"], json!(artifacts.logs_path));
        assert_eq!(
            result["failures"][0]["artifacts"]["screenshot"],
            json!(host_root.join("failure.png"))
        );
        assert!(host_root.join("snapshots/current").is_dir());

        // Progress is rewritten path-only (never pulled); out-of-root lines drop.
        let progress = format!(
            "{}\n{}\n{}\n",
            json!({"result_path": format!("{device_root}/result.json"), "note": "res://x.tscn"}),
            "not json",
            json!("user://godot_qa/live/other/leak.jsonl"),
        );
        let rewritten = rewrite_progress_lines(&progress, &mapper);
        let lines: Vec<&str> = rewritten.lines().collect();
        assert_eq!(lines.len(), 2, "the out-of-root JSON line is dropped");
        let first: Value = serde_json::from_str(lines[0]).unwrap();
        assert_eq!(first["result_path"], json!(host_root.join("result.json")));
        assert_eq!(first["note"], json!("res://x.tscn"));
        assert_eq!(lines[1], "not json");

        let _ = fs::remove_dir_all(&host_root);
    }

    // --- Full post-deploy loop --------------------------------------------

    fn fake_record(session_id: &str, port: u16) -> SessionRecord {
        SessionRecord {
            process_identity: None,
            protocol_version: "0.1".to_string(),
            session_id: session_id.to_string(),
            pid: 0,
            host: "127.0.0.1".to_string(),
            port,
            scene: String::new(),
            viewport: vec![1080, 1920],
            artifact_dir: String::new(),
            request_path: String::new(),
            token_file: String::new(),
            started_at: "0Z".to_string(),
            headless: false,
            project_args: Vec::new(),
            frame_pacing: None,
            target: SessionTarget::Android,
            android: Some(AndroidSessionMetadata {
                device_serial: "serial-1".to_string(),
                package: "dev.qa.sample".to_string(),
                preset: "Android QA".to_string(),
                device_pid: 4242,
                device_port: 32400,
                forwarded_host_port: 32400,
                adb_path: "adb".to_string(),
                device_artifact_root: format!("user://godot_qa/live/{session_id}"),
                logcat_pid: None,
            logcat_process_identity: None,
                deployment_log_path: String::new(),
                logcat_log_path: String::new(),
            }),
        }
    }

    #[cfg(unix)]
    fn write_executable(path: &Path, contents: &str) -> PathBuf {
        use std::os::unix::fs::PermissionsExt;
        fs::write(path, contents).unwrap();
        let mut permissions = fs::metadata(path).unwrap().permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(path, permissions).unwrap();
        path.to_path_buf()
    }

    fn test_config(root: &Path) -> ProjectConfig {
        ProjectConfig {
            project_root: root.to_path_buf(),
            repo_root: root.to_path_buf(),
            verbose: false,
            godot_binary: Some(root.join("missing-godot").to_string_lossy().into_owned()),
            qa_dir: root.join("qa"),
            artifacts_dir: root.join(".qa/artifacts"),
            scenarios_dir: root.join("qa/scenarios"),
            addon_dir: root.join("addons/godot_qa"),
            repo_addon_dir: root.join("addons/godot_qa"),
            godot_launch_prefix: Vec::new(),
            godot_windowed_launch_prefix: Vec::new(),
            godot_args: Vec::new(),
            godot_project_args: Vec::new(),
            godot_launch_mode: godot_qa_core::config::GodotLaunchMode::Auto,
            godot_audio_mode: godot_qa_core::config::GodotAudioMode::Auto,
            scenario_warm_default: false,
            scenario_warm_strict_default: false,
            scenario_jobs_default: None,
            project_config_failure: None,
            artifacts_keep_runs: None,
            scenario_schedule_default: None,
            log_ignore_patterns: Vec::new(),
            config_warnings: Vec::new(),
        }
    }

    #[test]
    fn roundtrip_failure_classification_table() {
        let root = unique_temp("classify");
        let config = test_config(&root);
        let android = fake_record("sess-x", 0).android.take().unwrap();

        // App still alive -> reuse the desktop scenario_run_timeout kind.
        let alive = android_roundtrip_failure_from_probe(
            &config,
            &android,
            Some(Duration::from_secs(30)),
            Ok(Some(4242)),
        );
        assert_eq!(alive.kind, "scenario_run_timeout");
        assert_eq!(alive.details["transport_budget_seconds"], json!(30.0));
        assert!(alive.details.contains_key("logcat"));

        // App gone -> new android_app_crashed, pointing at retained host logs.
        let crashed = android_roundtrip_failure_from_probe(&config, &android, None, Ok(None));
        assert_eq!(crashed.kind, "android_app_crashed");
        assert!(crashed.details.contains_key("logcat"));
        assert!(crashed.details.contains_key("deployment_log"));

        // Device unqueryable -> reuse android_device_unavailable.
        let probe_error = FailureDetail {
            kind: "android_pid_query_failed".to_string(),
            actual: Some(json!("no devices/emulators found")),
            ..FailureDetail::default()
        };
        let unavailable =
            android_roundtrip_failure_from_probe(&config, &android, None, Err(probe_error));
        assert_eq!(unavailable.kind, "android_device_unavailable");
        assert_eq!(unavailable.details["device"], json!(android.device_serial));

        let _ = fs::remove_dir_all(&root);
    }

    // --- Process-driven harness -------------------------------------------

    /// A hand-built post-deploy scenario, wired to a fake TCP bridge + fake ADB so
    /// the mandatory cleanup path exercises real ADB command sequencing.
    #[cfg(unix)]
    struct AndroidHarness {
        root: PathBuf,
        config: ProjectConfig,
        handle: PooledSessionHandle,
        artifacts: artifacts::RunArtifacts,
        scenario: Value,
        scenario_path: PathBuf,
        calls: PathBuf,
        session_id: &'static str,
    }

    #[cfg(unix)]
    impl AndroidHarness {
        fn run(&self) -> CommandResult {
            execute_android_scenario_on_session(
                &self.config,
                &self.handle,
                &self.scenario_path,
                &self.scenario,
                &self.artifacts,
                None,
                false,
            )
        }

        fn adb_log(&self) -> String {
            fs::read_to_string(&self.calls).unwrap()
        }
    }

    /// Build the harness. `write_result` controls whether the bridge writes the
    /// device result.json (false forces a mid-run collection failure);
    /// `force_stop_code` is the exit code of the cleanup `am force-stop` (non-zero
    /// forces a cleanup-step failure).
    #[cfg(unix)]
    fn android_harness(name: &str, write_result: bool, force_stop_code: i32) -> AndroidHarness {
        let root = unique_temp(name);
        let device_tree = root.join("device");
        let session_id = "scenario-sess-1";
        let device_run_root = format!("user://godot_qa/live/{session_id}/run");
        let result_device_path = format!("{device_run_root}/result.json");
        let device_result_file = device_tree
            .join("files/godot_qa/live")
            .join(session_id)
            .join("run/result.json");

        let calls = root.join("adb-calls.log");
        let adb = write_executable(
            &root.join("adb"),
            &format!(
                "#!/bin/sh\nprintf '%s\\n' \"$*\" >> '{calls}'\nlast=\"\"\nfor a in \"$@\"; do last=\"$a\"; done\ncase \" $* \" in\n  *'umask 077; cat > '*)\n    /bin/cat > /dev/null\n    exit 0 ;;\n  *' exec-out run-as '*' cat '*)\n    full='{device_tree}/'\"$last\"\n    if [ -f \"$full\" ]; then /bin/cat \"$full\"; exit $?; fi\n    exit 1 ;;\n  *' shell pidof '*)\n    printf '4242\\n'\n    exit 0 ;;\n  *' shell am force-stop '*)\n    exit {force_stop_code} ;;\n  *' forward --list '*)\n    printf 'serial-1 tcp:32400 tcp:32400\\n'\n    exit 0 ;;\nesac\nexit 0\n",
                calls = calls.display(),
                device_tree = device_tree.display(),
            ),
        );

        let port = spawn_fake_bridge(device_result_file, result_device_path, write_result);

        let mut record = fake_record(session_id, port);
        record.artifact_dir = root.join("session").to_string_lossy().into_owned();
        if let Some(android) = record.android.as_mut() {
            android.adb_path = adb.to_string_lossy().into_owned();
        }
        let handle = PooledSessionHandle {
            record,
            token: "test-token".to_string(),
        };

        let config = test_config(&root);
        let scenario = json!({"name": "android_smoke", "scene": "res://main.tscn", "steps": [{"wait_frames": 1}]});
        let scenario_path = root.join("qa/scenarios/android_smoke.json");
        let scenario_for_artifacts =
            scenario_with_effective_launch_args(&config, &runtime_scenario_payload(&scenario));
        let artifacts = artifacts::prepare_run_artifacts(
            &config.artifacts_dir,
            &scenario_for_artifacts,
            &config.project_root,
            &scenario_path,
            "scenario run android_smoke --target android",
            config.godot_binary.as_deref(),
            &[],
            &[],
            None,
            None,
        )
        .unwrap();

        AndroidHarness {
            root,
            config,
            handle,
            artifacts,
            scenario,
            scenario_path,
            calls,
            session_id,
        }
    }

    /// Fake bridge: on `scenario.run` optionally writes a realistic flat result.json
    /// into the fake device tree, then replies liveness-only. Anything else replies ok.
    #[cfg(unix)]
    fn spawn_fake_bridge(
        device_result_file: PathBuf,
        result_device_path: String,
        write_result: bool,
    ) -> u16 {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let result_json = serde_json::to_string(&json!({
            "ok": true,
            "scenario": "android_smoke",
            "scene": "res://main.tscn",
            "duration_ms": 5,
            "display_server": "android",
            "snapshots": [],
            "snapshot_artifacts": {},
            "failures": [],
            "artifacts": {
                "run_root": result_device_path.trim_end_matches("/result.json"),
                "result_path": result_device_path,
                "logs_path": format!("{}/logs.txt", result_device_path.trim_end_matches("/result.json")),
            },
        }))
        .unwrap();
        thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { continue };
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                let mut line = String::new();
                if reader.read_line(&mut line).is_err() {
                    continue;
                }
                let request: Value = serde_json::from_str(line.trim()).unwrap_or(Value::Null);
                let command = request.get("command").and_then(Value::as_str).unwrap_or("");
                let id = request.get("id").cloned().unwrap_or(Value::Null);
                let response = if command == "scenario.run" {
                    if write_result {
                        if let Some(parent) = device_result_file.parent() {
                            let _ = fs::create_dir_all(parent);
                        }
                        let _ = fs::write(&device_result_file, &result_json);
                    }
                    json!({
                        "protocol_version": "0.1",
                        "ok": true,
                        "id": id,
                        "data": {
                            "scenario": "android_smoke",
                            "ok": true,
                            "write_ok": true,
                            "result_path": result_device_path,
                            "failure_count": 0,
                        },
                    })
                } else {
                    json!({"protocol_version": "0.1", "ok": true, "id": id, "data": {}})
                };
                let mut bytes = serde_json::to_vec(&response).unwrap();
                bytes.push(b'\n');
                let _ = stream.write_all(&bytes);
                let _ = stream.flush();
            }
        });
        port
    }

    #[cfg(unix)]
    fn assert_cleanup_order(log: &str, session_id: &str) {
        let force_stop = log.find("am force-stop dev.qa.sample").expect("force-stop");
        let forward_remove = log.find("forward --remove tcp:32400").expect("forward removal");
        let private_rm = log
            .find(&format!("run-as dev.qa.sample rm -rf files/godot_qa/live/{session_id}"))
            .expect("private session rm");
        let bootstrap_rm = log
            .find("run-as dev.qa.sample rm -f files/godot_qa/android_live_bootstrap.json")
            .expect("bootstrap rm");
        assert!(
            force_stop < forward_remove && forward_remove < private_rm && private_rm < bootstrap_rm,
            "cleanup order wrong:\n{log}"
        );
    }

    #[cfg(unix)]
    #[test]
    fn fake_adb_consumes_input_and_reads_result_with_empty_path() {
        let harness = android_harness("empty-path", false, 0);
        let adb = harness.root.join("adb");
        let device_path = "files/godot_qa/live/scenario-sess-1/run/result.json";
        let source = harness.root.join("device").join(device_path);
        fs::create_dir_all(source.parent().unwrap()).unwrap();
        fs::write(&source, b"{\"ok\":true}").unwrap();

        let mut push = std::process::Command::new(&adb)
            .args([
                "-s", "serial-1", "shell", "run-as", "dev.qa.sample", "sh", "-c",
                "umask 077; cat > files/godot_qa/live/scenario-sess-1/run/request.json",
            ])
            .env("PATH", "")
            .stdin(std::process::Stdio::piped())
            .spawn()
            .unwrap();
        push.stdin.take().unwrap().write_all(b"request body").unwrap();
        assert!(push.wait().unwrap().success());

        let pulled = std::process::Command::new(&adb)
            .args(["-s", "serial-1", "exec-out", "run-as", "dev.qa.sample", "cat", device_path])
            .env("PATH", "")
            .output()
            .unwrap();
        assert!(pulled.status.success());
        assert_eq!(pulled.stdout, b"{\"ok\":true}");
        let _ = fs::remove_dir_all(harness.root);
    }

    #[cfg(unix)]
    #[test]
    fn full_post_deploy_loop_produces_android_verdict_and_cleans_up() {
        let harness = android_harness("full-loop", true, 0);
        let result = harness.run();

        // Desktop-identical verdict shape, androidized.
        assert!(result.ok, "verdict should pass: {:?}", result.failures);
        assert_eq!(result.command, "scenario run");
        assert_eq!(result.data["target"], json!("android"));
        assert_eq!(result.data["android"]["session_id"], json!(harness.session_id));
        assert_eq!(result.data["android"]["cleanup_complete"], json!(true));
        assert_eq!(result.data["android"]["deployment_mode"], json!("debug_apk"));
        assert_eq!(result.data["duration_ms"], json!(5));
        assert!(result.data.contains_key("process"));
        assert!(harness.artifacts.result_path.is_file());

        let log = harness.adb_log();
        assert_cleanup_order(&log, harness.session_id);
        // The request write (push) precedes the first result read (dispatch/collect).
        let push = log.find("cat > files/godot_qa/live").expect("request push write");
        let first_read = log.find("exec-out run-as dev.qa.sample cat").expect("result read");
        assert!(push < first_read, "request push must precede dispatch:\n{log}");

        let _ = fs::remove_dir_all(&harness.root);
    }

    #[cfg(unix)]
    #[test]
    fn mid_run_failure_still_cleans_up_in_order() {
        // The bridge never writes result.json, so the round-trip pull fails and the
        // run fails, yet cleanup still runs in the exact order. The precise
        // round-trip classification depends on a live device probe (whose ADB spawn
        // can vary under host load) and is pinned deterministically by
        // `roundtrip_failure_classification_table`; here we only require an ordered
        // teardown after a mid-run failure.
        let harness = android_harness("mid-run-fail", false, 0);
        let result = harness.run();

        assert!(!result.ok);
        assert_eq!(result.exit_code, 1);
        assert!(!result.failures.is_empty());
        assert_eq!(result.data["android"]["cleanup_complete"], json!(true));
        // Host artifacts (request.json) are retained on the mid-run failure.
        assert!(harness.artifacts.request_path.is_file());
        assert_cleanup_order(&harness.adb_log(), harness.session_id);

        let _ = fs::remove_dir_all(&harness.root);
    }

    #[cfg(unix)]
    #[test]
    fn cleanup_step_failure_forces_incomplete_and_nonzero() {
        // The run succeeds, but `am force-stop` exits non-zero during cleanup.
        let harness = android_harness("cleanup-fail", true, 7);
        let result = harness.run();

        assert!(!result.ok, "a cleanup failure must fail the run");
        assert_eq!(result.exit_code, 1);
        assert_eq!(result.data["android"]["cleanup_complete"], json!(false));
        assert!(
            result
                .failures
                .iter()
                .any(|failure| failure.kind == "android_force_stop_failed"),
            "cleanup failure should be appended: {:?}",
            result.failures
        );

        let _ = fs::remove_dir_all(&harness.root);
    }
}
