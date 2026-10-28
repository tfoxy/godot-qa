    #[test]
    fn scenario_does_not_request_screenshot_without_screenshot_fields() {
        let scenario = json!({
            "name": "main_menu_start_opens_pause",
            "scene": "res://ui/main_menu.tscn",
            "steps": [{"wait_frames": 5}],
            "assert": [{"no_errors": true}],
        });

        assert!(!scenario_requests_screenshot(&scenario));
    }

    #[test]
    fn display_backend_screenshot_scenarios_default_to_windowed() {
        let scenario = json!({
            "name": "main_menu_screenshot",
            "scene": "res://ui/main_menu.tscn",
            "steps": [{"snapshot": "after"}],
            "snapshots": {"save_screenshot": true},
        });

        assert!(!selected_headless_for_scenario(
            None,
            scenario_requests_screenshot(&scenario)
        ));
    }

    #[test]
    fn scenario_launch_selection_defaults_plain_state_to_headless() {
        let scenario = json!({
            "name": "state_only",
            "scene": "res://ui/main_menu.tscn",
            "steps": [{"wait_frames": 5}],
            "assert": [{"state_matches": {"target": "qa:start", "source": "inspect", "path": "visible", "operator": "eq", "expected": true}}],
        });

        assert!(!scenario_requires_windowed_media_evidence(&scenario, false));
        assert!(selected_headless_for_scenario(
            None,
            scenario_requires_windowed_media_evidence(&scenario, false)
        ));
    }

    #[test]
    fn audio_capture_requires_audio_evidence() {
        let scenario = json!({
            "name": "capture_music",
            "scene": "res://main.tscn",
            "steps": [
                {"wait_frames": 1},
                {"group": [{"audio_capture": {"name": "music", "duration_seconds": 0.5}}]}
            ],
        });

        assert!(scenario_contains_audio_capture(&scenario));
        assert!(scenario_requires_audio_evidence(&scenario));
        // Audio evidence no longer forces a window: a headless-display + real-audio
        // launch (the display-off + audio-on grid cell) is valid.
        assert!(!scenario_requires_windowed_media_evidence(&scenario, false));
        assert!(selected_headless_for_scenario(
            None,
            scenario_requires_windowed_media_evidence(&scenario, false)
        ));
        // Audio required -> the real audio driver is selected by default.
        assert!(selected_audio_for_scenario(
            None,
            scenario_requires_audio_evidence(&scenario)
        ));
    }

    #[test]
    fn screenshot_requires_video_not_audio() {
        let scenario = json!({
            "name": "main_menu_screenshot",
            "scene": "res://ui/main_menu.tscn",
            "steps": [{"snapshot": "after"}],
            "snapshots": {"save_screenshot": true},
        });

        // A screenshot scenario requires VIDEO (a window) but NOT audio.
        assert!(!scenario_contains_audio_capture(&scenario));
        assert!(!scenario_requires_audio_evidence(&scenario));
        assert!(scenario_requires_windowed_media_evidence(
            &scenario,
            scenario_requests_screenshot(&scenario)
        ));
        assert!(!selected_headless_for_scenario(
            None,
            scenario_requires_windowed_media_evidence(
                &scenario,
                scenario_requests_screenshot(&scenario)
            )
        ));
        // Video required, audio not required -> audio stays off (Dummy driver).
        assert!(!selected_audio_for_scenario(
            None,
            scenario_requires_audio_evidence(&scenario)
        ));
    }

    #[test]
    fn explicit_headless_and_windowed_override_media_auto_detection() {
        assert!(selected_headless_for_scenario(Some(true), true));
        assert!(!selected_headless_for_scenario(Some(false), false));
    }

    #[test]
    fn explicit_audio_override_wins_over_auto_detection() {
        // --audio forces real audio even when the scenario captures none; --no-audio
        // forces Dummy even when the scenario would otherwise request audio.
        assert!(selected_audio_for_scenario(Some(true), false));
        assert!(!selected_audio_for_scenario(Some(false), true));
        // Auto (None) follows the requires-audio signal.
        assert!(selected_audio_for_scenario(None, true));
        assert!(!selected_audio_for_scenario(None, false));
    }

    #[test]
    fn scenario_godot_args_read_normalized_payload() {
        let scenario = json!({
            "name": "main_menu_logic",
            "scene": "res://ui/main_menu.tscn",
            "godot_args": ["--single-window", "--rendering-driver", "opengl3"],
            "steps": [{"wait_frames": 1}],
        });

        assert_eq!(
            scenario_godot_args(&scenario),
            vec![
                "--single-window".to_string(),
                "--rendering-driver".to_string(),
                "opengl3".to_string(),
            ]
        );
    }

    #[test]
    fn scenario_project_args_read_normalized_payload() {
        let scenario = json!({
            "name": "main_menu_logic",
            "scene": "res://ui/main_menu.tscn",
            "project_args": ["--save-slot", "qa", "--seed=42"],
            "steps": [{"wait_frames": 1}],
        });

        assert_eq!(
            scenario_project_args(&scenario),
            vec![
                "--save-slot".to_string(),
                "qa".to_string(),
                "--seed=42".to_string(),
            ]
        );
    }

    #[test]
    fn scenario_project_args_default_to_empty_for_normal_scenarios() {
        let scenario = json!({
            "name": "main_menu_logic",
            "scene": "res://ui/main_menu.tscn",
            "steps": [{"wait_frames": 1}],
        });

        assert_eq!(scenario_project_args(&scenario), Vec::<String>::new());
    }

    #[test]
    fn scenario_project_args_are_threaded_into_runtime_launch() {
        let root = temp_dir("scenario-project-args-launch");
        fs::create_dir_all(&root).expect("create temp dir");
        let artifacts = test_run_artifacts(&root);
        fs::create_dir_all(&artifacts.root_dir).expect("create artifact dir");
        let scenario_path = root.join("qa/scenarios/project_args.json");
        let mut config = test_config(&root);
        config.godot_binary = Some(
            godot_qa_test_support::fixture(&root.join("fake-godot"), &[])
                .to_string_lossy()
                .into_owned(),
        );
        config.godot_args = vec!["--rendering-driver".to_string(), "opengl3".to_string()];
        config.godot_project_args = vec![
            "--no-countdown".to_string(),
            "--token".to_string(),
            "abcdef".to_string(),
        ];
        let scenario = json!({
            "name": "project_args",
            "scene": "res://ui/main_menu.tscn",
            "godot_args": ["--single-window"],
            "project_args": ["--save-slot", "qa profile", "--seed=42"],
            "steps": [{"wait_frames": 1}],
        });

        let result = execute_runtime_run(
            "scenario run",
            "scenario run failed",
            &config,
            &scenario_path,
            &scenario,
            &artifacts,
            "godot-qa scenario run project_args",
            false,
            true,
            false,
            false,
            RuntimeExecutionOptions::default(),
        );

        let _ = fs::remove_dir_all(&root);

        let command = result.data["process"]["command"]
            .as_array()
            .expect("process command")
            .iter()
            .map(|arg| arg.as_str().expect("command arg").to_string())
            .collect::<Vec<_>>();
        let separator_index = command
            .iter()
            .position(|arg| arg == "--")
            .expect("qa runner separator");
        let godot_arg_index = command
            .iter()
            .position(|arg| arg == "--rendering-driver")
            .expect("config Godot arg");
        let scenario_godot_arg_index = command
            .iter()
            .position(|arg| arg == "--single-window")
            .expect("scenario Godot arg");
        let fixed_fps_index = command
            .iter()
            .position(|arg| arg == "--fixed-fps")
            .expect("generated fixed FPS arg");
        let qa_request_index = command
            .iter()
            .position(|arg| arg.starts_with("--qa-run-request="))
            .expect("QA request flag");
        assert!(fixed_fps_index < godot_arg_index);
        assert!(godot_arg_index < scenario_godot_arg_index);
        assert_eq!(command[fixed_fps_index + 1], "60");
        assert!(command.contains(&"--disable-vsync".to_string()));
        assert!(scenario_godot_arg_index < separator_index);
        assert_eq!(
            result.data["godot_args"],
            json!(["--rendering-driver", "opengl3", "--single-window"])
        );
        assert_eq!(
            result.data["project_args"],
            json!([
                "--no-countdown",
                "--token",
                "<redacted:6>",
                "--save-slot",
                "qa profile",
                "--seed=42"
            ])
        );
        assert_eq!(result.data["frame_pacing"]["fixed_fps"], 60);
        assert_eq!(result.data["frame_pacing"]["source"], "godot_default");
        assert_eq!(
            &command[qa_request_index + 1..qa_request_index + 7],
            [
                "--no-countdown",
                "--token",
                "<redacted:6>",
                "--save-slot",
                "qa profile",
                "--seed=42"
            ]
        );
        let artifact_scenario = scenario_with_effective_launch_args(&config, &scenario);
        let prepared = artifacts::prepare_run_artifacts(
            &root.join(".qa/artifacts"),
            &artifact_scenario,
            &config.project_root,
            &scenario_path,
            "godot-qa scenario run project_args",
            config.godot_binary.as_deref(),
            &config.godot_launch_prefix,
            &config.godot_windowed_launch_prefix,
            None,
            None,
        )
        .expect("prepare artifacts");
        let meta = serde_json::from_str::<Value>(
            &fs::read_to_string(prepared.meta_path).expect("read meta"),
        )
        .expect("parse meta");
        let request = serde_json::from_str::<Value>(
            &fs::read_to_string(prepared.request_path).expect("read request"),
        )
        .expect("parse request");
        assert_eq!(
            meta["godot_args"],
            json!(["--rendering-driver", "opengl3", "--single-window"])
        );
        assert_eq!(
            request["run"]["project_args"],
            json!([
                "--no-countdown",
                "--token",
                "<redacted:6>",
                "--save-slot",
                "qa profile",
                "--seed=42"
            ])
        );
    }

    #[test]
    fn config_launch_mode_headless_is_overridden_by_explicit_windowed() {
        let root = temp_dir("scenario-config-launch-mode");
        let scenario = json!({
            "name": "main_menu_logic",
            "scene": "res://ui/main_menu.tscn",
            "steps": [{"wait_frames": 1}],
        });
        let config = ProjectConfig {
            godot_launch_mode: godot_qa_core::config::GodotLaunchMode::Headless,
            godot_audio_mode: godot_qa_core::config::GodotAudioMode::Auto,
            ..test_config(&root)
        };

        let config_default = selected_headless_for_scenario(
            config.launch_mode_headless_override(),
            scenario_requests_screenshot(&scenario),
        );
        let explicit_windowed = selected_headless_for_scenario(
            Some(false).or_else(|| config.launch_mode_headless_override()),
            scenario_requests_screenshot(&scenario),
        );

        let _ = fs::remove_dir_all(&root);
        assert!(config_default);
        assert!(!explicit_windowed);
    }

    #[test]
    fn runtime_result_preserves_node_inspection_evidence() {
        let root = temp_dir("scenario-node-inspection-evidence");
        let runtime_result = json!({
            "ok": true,
            "snapshots": [],
            "failures": [],
            "node_inspections": {
                "0:qa:start_button": {
                    "step_index": 0,
                    "target": "qa:start_button",
                    "resolved": "path:/root/Main/StartButton",
                    "node": {
                        "name": "StartButton",
                        "type": "Button",
                        "metadata": {"qa_id": "start_button"},
                        "control": {"visible": true}
                    },
                    "parent": {"name": "MenuButtons"},
                    "children": [],
                    "script_path": ""
                }
            }
        });
        let result = strict_summary_result(&root, 0, &runtime_result, "");

        let _ = fs::remove_dir_all(&root);

        assert!(result.ok);
        assert_eq!(
            result.data["node_inspections"]["0:qa:start_button"]["target"],
            json!("qa:start_button")
        );
        assert_eq!(
            result.data["node_inspections"]["0:qa:start_button"]["node"]["metadata"]["qa_id"],
            json!("start_button")
        );
    }

    #[test]
    fn runtime_result_rejects_invalid_node_inspection_evidence() {
        let root = temp_dir("scenario-node-inspection-invalid");
        let runtime_result = json!({
            "ok": true,
            "snapshots": [],
            "failures": [],
            "node_inspections": ["qa:start_button"]
        });
        let result = strict_summary_result(&root, 0, &runtime_result, "");

        let _ = fs::remove_dir_all(&root);

        assert!(!result.ok);
        assert_eq!(result.failures[0].kind, "scenario_result_invalid");
        assert!(
            result.failures[0]
                .message
                .as_deref()
                .unwrap_or_default()
                .contains("node_inspections")
        );
    }

    #[test]
    fn physics_overlay_snapshot_evidence_is_preserved() {
        let root = temp_dir("scenario-physics-overlay-evidence");
        let runtime_result = json!({
            "ok": true,
            "snapshots": ["after"],
            "failures": [],
            "physics_overlays": {
                "1:actor": {
                    "step_index": 1,
                    "name": "actor",
                    "target": "qa:actor",
                    "snapshot": "after",
                    "metadata": {
                        "target": "qa:actor",
                        "shapes": [],
                        "contacts": [],
                        "project_state": {},
                        "metadata": {"shape_count": 0}
                    }
                }
            }
        });
        let result = strict_summary_result(&root, 0, &runtime_result, "");

        let _ = fs::remove_dir_all(&root);

        assert!(result.ok);
        assert_eq!(
            result.data["physics_overlays"]["1:actor"]["snapshot"],
            json!("after")
        );
        assert_eq!(
            result.data["physics_overlays"]["1:actor"]["metadata"]["target"],
            json!("qa:actor")
        );
    }

    #[test]
    fn physics_overlay_snapshot_evidence_rejects_invalid_entries() {
        let root = temp_dir("scenario-physics-overlay-invalid");
        let runtime_result = json!({
            "ok": true,
            "snapshots": [],
            "failures": [],
            "physics_overlays": ["qa:actor"]
        });
        let result = strict_summary_result(&root, 0, &runtime_result, "");

        let _ = fs::remove_dir_all(&root);

        assert!(!result.ok);
        assert_eq!(result.failures[0].kind, "scenario_result_invalid");
        assert!(
            result.failures[0]
                .message
                .as_deref()
                .unwrap_or_default()
                .contains("physics_overlays")
        );
    }

    #[test]
    fn runtime_result_project_args() {
        let root = temp_dir("runtime-result-project-args");
        fs::create_dir_all(&root).expect("create temp dir");
        let scenario = json!({
            "name": "project_args",
            "scene": "res://ui/main_menu.tscn",
            "godot_args": ["--single-window"],
            "project_args": ["--slot", "qa", "--token", "abcdef"],
            "steps": [],
        });
        let runtime_result = json!({
            "ok": true,
            "godot_args": ["--single-window"],
            "project_args": ["--slot", "qa", "--token", "abcdef"],
            "snapshots": [],
            "failures": [],
        });

        let result = summary_result_with_scenario(&root, 0, &runtime_result, "", &scenario);

        assert!(result.ok, "{:#?}", result.failures);
        assert_eq!(result.data["godot_args"], json!(["--single-window"]));
        assert_eq!(
            result.data["project_args"],
            json!(["--slot", "qa", "--token", "abcdef"])
        );
        assert!(result.data.get("launch_args").is_none());
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn display_backend_non_screenshot_scenarios_default_to_headless() {
        let scenario = json!({
            "name": "main_menu_logic",
            "scene": "res://ui/main_menu.tscn",
            "steps": [{"wait_frames": 1}],
            "assert": [{"no_errors": true}],
        });

        assert!(selected_headless_for_scenario(
            None,
            scenario_requests_screenshot(&scenario)
        ));
    }

    #[cfg(any(
        target_os = "linux",
        target_os = "freebsd",
        target_os = "netbsd",
        target_os = "openbsd",
        target_os = "dragonfly"
    ))]
    #[test]
    fn display_backend_unavailable_fails_before_runtime_result_loading() {
        let _guard = ENV_LOCK.lock().unwrap();
        let old_display = std::env::var_os("DISPLAY");
        let old_wayland = std::env::var_os("WAYLAND_DISPLAY");
        let old_path = std::env::var_os("PATH");
        // SAFETY: This test serializes its environment mutation with ENV_LOCK and restores
        // the previous values before releasing the lock.
        unsafe {
            std::env::remove_var("DISPLAY");
            std::env::remove_var("WAYLAND_DISPLAY");
            std::env::set_var("PATH", "");
        }

        let root = temp_dir("display-backend-unavailable");
        write(&root.join("project.godot"), "config_version=5\n");
        let scenario_path = root.join("qa/scenarios/main_menu_screenshot.json");
        let scenario = json!({
            "name": "main_menu_screenshot",
            "scene": "res://ui/main_menu.tscn",
            "steps": [{"snapshot": "after"}],
            "snapshots": {"save_screenshot": true},
        });
        let result = super::run_scenario_once(
            &test_config(&root),
            &scenario_path,
            &scenario,
            "scenario run main_menu_screenshot",
            None,
            None,
            VisualRiskExecutionOptions::default(),
        );

        if let Some(value) = old_display {
            // SAFETY: See the ENV_LOCK note above.
            unsafe { std::env::set_var("DISPLAY", value) };
        } else {
            // SAFETY: See the ENV_LOCK note above.
            unsafe { std::env::remove_var("DISPLAY") };
        }
        if let Some(value) = old_wayland {
            // SAFETY: See the ENV_LOCK note above.
            unsafe { std::env::set_var("WAYLAND_DISPLAY", value) };
        } else {
            // SAFETY: See the ENV_LOCK note above.
            unsafe { std::env::remove_var("WAYLAND_DISPLAY") };
        }
        if let Some(value) = old_path {
            // SAFETY: See the ENV_LOCK note above.
            unsafe { std::env::set_var("PATH", value) };
        } else {
            // SAFETY: See the ENV_LOCK note above.
            unsafe { std::env::remove_var("PATH") };
        }
        let _ = fs::remove_dir_all(&root);

        assert!(!result.ok);
        assert_eq!(result.failures[0].kind, "display_backend_unavailable");
        assert_eq!(result.data["headless"], json!(false));
        assert_eq!(
            result.data["launch_display"]["fallback_reason"],
            json!("display_backend_unavailable")
        );
        assert_eq!(
            result.data["launch_display"]["display_backend_available"],
            json!(false)
        );
        assert_eq!(
            result.failures[0].expected,
            Some(json!(
                "real display backend or detected virtual display wrapper"
            ))
        );
        assert!(
            !result.data["artifacts"]["result_path"]
                .as_str()
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn visual_preflight_windowed_display_available_reports_visual_support() {
        let _guard = ENV_LOCK.lock().unwrap();
        let old_display = std::env::var_os("DISPLAY");
        let old_wayland = std::env::var_os("WAYLAND_DISPLAY");
        // SAFETY: This test serializes environment mutation with ENV_LOCK and restores
        // the previous values before releasing the lock.
        unsafe {
            std::env::set_var("DISPLAY", ":99");
            std::env::remove_var("WAYLAND_DISPLAY");
        }

        let root = temp_dir("visual-preflight-display-available");
        write(&root.join("project.godot"), "config_version=5\n");
        let result =
            super::run_visual_preflight(&test_config(&root), "visual preflight --windowed", false);

        if let Some(value) = old_display {
            // SAFETY: See the ENV_LOCK note above.
            unsafe { std::env::set_var("DISPLAY", value) };
        } else {
            // SAFETY: See the ENV_LOCK note above.
            unsafe { std::env::remove_var("DISPLAY") };
        }
        if let Some(value) = old_wayland {
            // SAFETY: See the ENV_LOCK note above.
            unsafe { std::env::set_var("WAYLAND_DISPLAY", value) };
        } else {
            // SAFETY: See the ENV_LOCK note above.
            unsafe { std::env::remove_var("WAYLAND_DISPLAY") };
        }
        let _ = fs::remove_dir_all(&root);

        assert!(result.ok);
        assert_eq!(result.command, "visual preflight");
        assert_eq!(result.data["headless"], json!(false));
        assert_eq!(
            result.data["visual_support"]["real_viewport_pixels"]["available"],
            json!(true)
        );
        assert_eq!(
            result.data["visual_support"]["real_viewport_pixels"]["reason"],
            json!("display_backend_available")
        );
        assert_eq!(
            result.data["visual_support"]["target_hidden_capture"]["available"],
            json!(true)
        );
        assert_eq!(
            result.data["visual_support"]["mask_generation"]["available"],
            json!(true)
        );
        assert_eq!(
            result.data["visual_support"]["saved_snapshot_object_pixels"]["available"],
            json!(true)
        );
    }

    #[test]
    fn visual_preflight_forced_headless_reports_real_pixels_unavailable() {
        let root = temp_dir("visual-preflight-headless");
        write(&root.join("project.godot"), "config_version=5\n");
        let result =
            super::run_visual_preflight(&test_config(&root), "visual preflight --headless", true);
        let _ = fs::remove_dir_all(&root);

        assert!(result.ok);
        assert_eq!(result.data["headless"], json!(true));
        assert_eq!(
            result.data["visual_support"]["real_viewport_pixels"]["available"],
            json!(false)
        );
        assert_eq!(
            result.data["visual_support"]["real_viewport_pixels"]["reason"],
            json!("forced_headless")
        );
        assert_eq!(
            result.data["visual_support"]["target_hidden_capture"]["reason"],
            json!("unavailable_real_viewport_pixels_forced_headless")
        );
        assert_eq!(
            result.data["visual_support"]["saved_snapshot_object_pixels"]["available"],
            json!(false)
        );
    }

    #[cfg(any(
        target_os = "linux",
        target_os = "freebsd",
        target_os = "netbsd",
        target_os = "openbsd",
        target_os = "dragonfly"
    ))]
    #[test]
    fn visual_preflight_unavailable_display_reports_launch_diagnostics() {
        let _guard = ENV_LOCK.lock().unwrap();
        let old_display = std::env::var_os("DISPLAY");
        let old_wayland = std::env::var_os("WAYLAND_DISPLAY");
        let old_path = std::env::var_os("PATH");
        // SAFETY: This test serializes environment mutation with ENV_LOCK and restores
        // the previous values before releasing the lock.
        unsafe {
            std::env::remove_var("DISPLAY");
            std::env::remove_var("WAYLAND_DISPLAY");
            std::env::set_var("PATH", "");
        }

        let root = temp_dir("visual-preflight-display-unavailable");
        write(&root.join("project.godot"), "config_version=5\n");
        let result =
            super::run_visual_preflight(&test_config(&root), "visual preflight --windowed", false);

        if let Some(value) = old_display {
            // SAFETY: See the ENV_LOCK note above.
            unsafe { std::env::set_var("DISPLAY", value) };
        } else {
            // SAFETY: See the ENV_LOCK note above.
            unsafe { std::env::remove_var("DISPLAY") };
        }
        if let Some(value) = old_wayland {
            // SAFETY: See the ENV_LOCK note above.
            unsafe { std::env::set_var("WAYLAND_DISPLAY", value) };
        } else {
            // SAFETY: See the ENV_LOCK note above.
            unsafe { std::env::remove_var("WAYLAND_DISPLAY") };
        }
        if let Some(value) = old_path {
            // SAFETY: See the ENV_LOCK note above.
            unsafe { std::env::set_var("PATH", value) };
        } else {
            // SAFETY: See the ENV_LOCK note above.
            unsafe { std::env::remove_var("PATH") };
        }
        let _ = fs::remove_dir_all(&root);

        if result.data["launch_display"]["linuxbsd_target"] != json!(true) {
            return;
        }
        assert!(result.ok);
        assert_eq!(
            result.data["launch_display"]["fallback_reason"],
            json!("display_backend_unavailable")
        );
        assert_eq!(
            result.data["visual_support"]["real_viewport_pixels"]["available"],
            json!(false)
        );
        assert_eq!(
            result.data["visual_support"]["real_viewport_pixels"]["reason"],
            json!("display_backend_unavailable")
        );
        assert_eq!(
            result.data["visual_support"]["mask_generation"]["reason"],
            json!("unavailable_real_viewport_pixels_display_backend_unavailable")
        );
    }

    #[test]
    fn missing_runtime_result_includes_crash_context_from_logs() {
        let root = temp_dir("missing-result-crash-context");
        let result_path = root.join("result.json");
        let logs_path = root.join("logs.txt");
        write(
            &logs_path,
            &[
                "Godot Engine v4.6.2",
                "SCRIPT ERROR: Invalid access to property or key 'global_position'",
                "          at: build_screen.gd:42",
                "ERROR: FATAL: Signal 11",
                "Native backtrace:",
                "  [0] Object::emit_signalp",
            ]
            .join("\n"),
        );
        let artifacts = json!({
            "run_root": root,
            "result_path": result_path,
            "logs_path": logs_path,
        });

        let failure =
            load_runtime_result("scenario run", &result_path, &artifacts, None).unwrap_err();

        assert_eq!(failure.kind, "scenario_result_missing");
        assert_eq!(failure.path.as_deref(), Some(result_path.to_str().unwrap()));
        assert_eq!(failure.artifacts.unwrap()["logs_path"], json!(logs_path));
        assert_eq!(
            failure.actual.unwrap(),
            json!({
                "result": "missing",
                "result_path": result_path,
                "logs_path": logs_path,
                "crash_signal": "ERROR: FATAL: Signal 11",
                "script_backtrace": [
                    "SCRIPT ERROR: Invalid access to property or key 'global_position'",
                    "          at: build_screen.gd:42",
                ],
                "last_relevant_log_lines": [
                    "SCRIPT ERROR: Invalid access to property or key 'global_position'",
                    "          at: build_screen.gd:42",
                    "ERROR: FATAL: Signal 11",
                    "Native backtrace:",
                    "  [0] Object::emit_signalp",
                ],
            })
        );
    }

    #[test]
    fn scenario_result_missing_remains_distinct_from_display_backend_failures() {
        let root = temp_dir("scenario-result-missing-distinct");
        let result_path = root.join("result.json");
        let artifacts = json!({
            "run_root": root,
            "result_path": result_path,
            "logs_path": root.join("logs.txt"),
        });

        let failure =
            load_runtime_result("scenario run", &result_path, &artifacts, None).unwrap_err();

        let _ = fs::remove_dir_all(&root);

        assert_eq!(failure.kind, "scenario_result_missing");
    }

    #[test]
    fn scenario_result_missing_after_display_startup_failure() {
        let root = temp_dir("scenario-result-missing-display-startup");
        let result_path = root.join("result.json");
        let artifacts = json!({
            "run_root": root,
            "result_path": result_path,
            "logs_path": root.join("logs.txt"),
        });
        let display = display_startup_failure(&root);

        let failure = load_runtime_result("scenario run", &result_path, &artifacts, Some(&display))
            .unwrap_err();

        let _ = fs::remove_dir_all(&root);

        assert_eq!(failure.kind, "scenario_result_missing");
        assert_eq!(
            failure.details["failure_category"],
            json!("display_infrastructure")
        );
        assert_eq!(
            failure.details["infrastructure_failure_kind"],
            json!("display_startup_exhausted")
        );
        assert_eq!(failure.details["retryable"], json!(true));
        assert!(failure.details.get("retry_guidance").is_some());
    }

    #[test]
    fn display_startup_failure_contract() {
        let root = temp_dir("display-startup-failure-contract");
        let display = display_startup_failure(&root);
        let process_result = GodotRunResult {
            command: vec!["godot".to_string()],
            returncode: 1,
            termination: None,
            process_timing: None,
            stdout: String::new(),
            stderr: String::new(),
            display,
            pid: Some(123),
            process_group: Some(123),
            cleanup: Some(godot_qa_process::ProcessCleanupResult {
                status: "cleaned".to_string(),
                failure_kind: None,
                pid: Some(123),
                process_group: Some(123),
                owned: true,
                message: None,
            }),
            infrastructure_failure_kind: Some("display_startup_exhausted".to_string()),
            timed_out: false,
            timeout_ms: None,
            output_truncation: None,
        };

        let process = super::process_summary_json(&process_result);
        let display = super::display_launch_metadata_json(&process_result.display);

        let _ = fs::remove_dir_all(&root);

        assert_eq!(process["failure_category"], json!("display_infrastructure"));
        assert_eq!(process["retryable"], json!(true));
        assert_eq!(process["cleanup"]["status"], json!("cleaned"));
        assert_eq!(display["failure_category"], json!("display_infrastructure"));
        assert_eq!(display["retryable"], json!(true));
        assert_eq!(display["lifecycle"]["startup_failed"], json!(true));
    }

    #[test]
    fn scenario_result_invalid_remains_distinct_from_display_backend_failures() {
        let root = temp_dir("scenario-result-invalid-distinct");
        let result_path = root.join("result.json");
        write(&result_path, "{not json");
        let artifacts = json!({
            "run_root": root,
            "result_path": result_path,
        });

        let failure =
            load_runtime_result("scenario run", &result_path, &artifacts, None).unwrap_err();

        let _ = fs::remove_dir_all(&root);

        assert_eq!(failure.kind, "scenario_result_invalid");
        assert_eq!(failure.actual, Some(json!("JSONDecodeError")));
        assert_eq!(failure.details["failure_category"], json!("artifact"));
        assert_eq!(failure.details["retryable"], json!(false));
    }

    #[test]
    fn malformed_result_json_is_not_retryable_infrastructure() {
        let root = temp_dir("malformed-result-not-infrastructure");
        let result_path = root.join("result.json");
        write(&result_path, "{not json");
        let artifacts = json!({
            "run_root": root,
            "result_path": result_path,
        });

        let failure =
            load_runtime_result("scenario run", &result_path, &artifacts, None).unwrap_err();

        let _ = fs::remove_dir_all(&root);

        assert_eq!(failure.kind, "scenario_result_invalid");
        assert_eq!(failure.details["failure_category"], json!("artifact"));
        assert_eq!(failure.details["retryable"], json!(false));
    }

    #[test]
    fn scenario_result_runtime_assertion_failures_remain_distinct() {
        let artifacts = json!({"run_root":"/tmp/run","result_path":"/tmp/run/result.json"})
            .as_object()
            .cloned()
            .unwrap();

        let failures = validate_runtime_failures(
            "scenario run",
            Path::new("/tmp/run/result.json"),
            Some(&json!([{
                "type": "assertion_failed",
                "message": "expected button to be visible",
                "expected": true,
                "actual": false,
                "target": "qa:start_button"
            }])),
            false,
            &artifacts,
        )
        .unwrap();

        assert_eq!(failures[0].kind, "assertion_failed");
        assert_eq!(failures[0].expected, Some(json!(true)));
        assert_eq!(failures[0].actual, Some(json!(false)));
        assert_eq!(
            failures[0].details["failure_category"],
            json!("gameplay_assertion")
        );
        assert_eq!(failures[0].details["retryable"], json!(false));
    }

    #[test]
    fn assertion_failures_are_not_retryable_infrastructure() {
        let root = temp_dir("assertion-failures-not-infrastructure");
        let result = strict_summary_result(
            &root,
            0,
            &json!({
                "ok": false,
                "snapshots": [],
                "failures": [{
                    "type": "assertion_failed",
                    "message": "expected button to be visible",
                    "expected": true,
                    "actual": false
                }]
            }),
            "",
        );

        let _ = fs::remove_dir_all(&root);

        assert!(!result.ok);
        assert_eq!(
            result.failures[0].details["failure_category"],
            json!("gameplay_assertion")
        );
        assert_eq!(result.failures[0].details["retryable"], json!(false));
        assert_eq!(
            result.data["outcome"]["failure_category"],
            json!("gameplay_assertion")
        );
        assert_eq!(result.data["outcome"]["retryable"], json!(false));
    }

    #[test]
    fn scenario_process_outcome_strict_clean_pass_has_summaries() {
        let root = temp_dir("scenario-process-outcome-clean-pass");
        let result = strict_summary_result(
            &root,
            0,
            &json!({"ok": true, "snapshots": [], "failures": [], "duration_ms": 12}),
            "",
        );

        let _ = fs::remove_dir_all(&root);

        assert!(result.ok);
        assert_eq!(result.exit_code, 0);
        assert_strict_summaries(&result);
        assert_eq!(result.data["process"]["status"], json!("passed"));
        assert_eq!(result.data["process"]["exit_code"], json!(0));
        assert_eq!(result.data["runtime_result"]["status"], json!("passed"));
        assert_eq!(result.data["runtime_result"]["ok"], json!(true));
        assert_eq!(result.data["runtime_result"]["failure_count"], json!(0));
        assert_eq!(result.data["logs"]["status"], json!("clean"));
        assert_eq!(
            result.data["outcome"],
            json!({"status":"pass","reason":null})
        );
    }

    #[test]
    fn scenario_process_outcome_strict_runtime_assertion_failure_is_distinct() {
        let root = temp_dir("scenario-process-outcome-runtime-failure");
        let result = strict_summary_result(
            &root,
            0,
            &json!({
                "ok": false,
                "snapshots": [],
                "failures": [{
                    "type": "assertion_failed",
                    "message": "expected button to be visible",
                    "expected": true,
                    "actual": false,
                    "target": "qa:start_button"
                }]
            }),
            "",
        );

        let _ = fs::remove_dir_all(&root);

        assert!(!result.ok);
        assert_eq!(result.exit_code, 1);
        assert_strict_summaries(&result);
        assert_eq!(result.failures[0].kind, "assertion_failed");
        assert_eq!(result.data["process"]["status"], json!("passed"));
        assert_eq!(result.data["runtime_result"]["status"], json!("failed"));
        assert_eq!(result.data["runtime_result"]["failure_count"], json!(1));
        assert_eq!(
            result.data["outcome"],
            json!({
                "status":"fail",
                "reason":"assertion_failed",
                "failure_category":"gameplay_assertion",
                "retryable":false
            })
        );
    }

    #[test]
    fn scenario_process_outcome_strict_missing_result_json_is_distinct() {
        let root = temp_dir("scenario-process-outcome-missing-result");
        let result_path = root.join("result.json");
        let logs_path = root.join("logs.txt");
        write(&logs_path, "");
        let artifacts = json!({
            "run_root": root,
            "result_path": result_path,
            "logs_path": logs_path,
        });
        let failure =
            load_runtime_result("scenario run", &result_path, &artifacts, None).unwrap_err();
        let process_result = test_process_result(&root, 0);
        let mut data = serde_json::Map::new();
        data.insert(
            "process".to_string(),
            super::process_summary_json(&process_result),
        );
        data.insert(
            "runtime_result".to_string(),
            super::runtime_result_summary_json(&result_path, "missing", None, 0),
        );
        data.insert(
            "logs".to_string(),
            super::log_status_summary_json(&logs_path, &[], 0, 0),
        );
        data.insert(
            "outcome".to_string(),
            super::outcome_summary_json(
                "fail",
                Some(&failure.kind),
                Some(std::slice::from_ref(&failure)),
            ),
        );
        let result = godot_qa_core::results::CommandResult {
            ok: false,
            command: "scenario run".to_string(),
            message: "scenario run failed".to_string(),
            data,
            failures: vec![failure],
            exit_code: 1,
        };

        let _ = fs::remove_dir_all(&root);

        assert_strict_summaries(&result);
        assert_eq!(result.failures[0].kind, "scenario_result_missing");
        assert_eq!(result.data["runtime_result"]["status"], json!("missing"));
        assert_eq!(result.data["runtime_result"]["ok"], Value::Null);
        assert_eq!(
            result.data["outcome"],
            json!({
                "status":"fail",
                "reason":"scenario_result_missing",
                "failure_category":"artifact",
                "retryable":false
            })
        );
    }
