    #[test]
    fn profile_percentiles_empty_samples_are_deterministic() {
        assert_eq!(
            profile_percentiles(&[]),
            ProfilePercentiles {
                sample_count: 0,
                min: None,
                max: None,
                p50: None,
                p90: None,
                p95: None,
                p99: None,
            }
        );
    }

    #[test]
    fn profile_percentiles_small_samples_use_nearest_rank_ceiling() {
        assert_eq!(
            profile_percentiles(&[4.0, 1.0, 2.0]),
            ProfilePercentiles {
                sample_count: 3,
                min: Some(1.0),
                max: Some(4.0),
                p50: Some(2.0),
                p90: Some(4.0),
                p95: Some(4.0),
                p99: Some(4.0),
            }
        );
    }

    #[test]
    fn profile_percentiles_larger_samples_use_ceiling_rank() {
        assert_eq!(
            profile_percentiles(&[1.0, 100.0, 2.0, 3.0, 4.0]),
            ProfilePercentiles {
                sample_count: 5,
                min: Some(1.0),
                max: Some(100.0),
                p50: Some(3.0),
                p90: Some(100.0),
                p95: Some(100.0),
                p99: Some(100.0),
            }
        );
    }

    #[test]
    fn profile_result_writes_bounded_artifact_and_summary_path() {
        let root = temp_dir("profile-artifact");
        let artifacts = test_run_artifacts(&root);
        let mut base_data = serde_json::Map::from_iter([
            ("duration_ms".to_string(), json!(123)),
            ("launch_display".to_string(), json!({"headless": true})),
            ("artifacts".to_string(), artifacts.to_json()),
        ]);
        let runtime_result = godot_qa_core::results::CommandResult {
            ok: true,
            command: "scenario profile".to_string(),
            message: "scenario profile passed".to_string(),
            data: base_data.clone(),
            failures: Vec::new(),
            exit_code: 0,
        };
        let raw = json!({
            "ok": true,
            "profile": {
                "frame_times_ms": [16.0, 18.0, 20.0],
                "monitor_samples": [
                    {"name": "time/process", "value": 1.5},
                    {"name": "MEMORY_STATIC", "value": 1000.0},
                    {"name": "MEMORY_STATIC", "value": 1500.0},
                    {"name": "MEMORY_STATIC_MAX", "value": 2000.0}
                ],
                "step_timings": [{"step": 0, "duration_ms": 3}],
                "top_script_methods": {
                    "status": "available",
                    "instrumentation": "opt_in",
                    "method_count": 1,
                    "sample_count": 5,
                    "methods": [{
                        "label": "MainMenu._process",
                        "call_count": 5,
                        "total_ms": 1.25,
                        "self_ms": 1.0,
                        "avg_ms": 0.25,
                        "max_ms": 0.4,
                        "last_ms": 0.2
                    }]
                }
            }
        });

        let result = profile_result_from_runtime(
            "scenario profile",
            &json!({"name": "profiled", "scene": "res://main.tscn"}),
            &mut base_data,
            &artifacts,
            &raw,
            &runtime_result,
        )
        .unwrap();

        let profile_path = result.data["artifacts"]["profile"].as_str().unwrap();
        let artifact: Value =
            serde_json::from_str(&fs::read_to_string(profile_path).unwrap()).unwrap();
        assert_eq!(
            result.data["profile"]["frame_time_percentiles_ms"]["p90"],
            20.0
        );
        assert_eq!(artifact["scenario"], "profiled");
        assert_eq!(artifact["frame_times_ms"].as_array().unwrap().len(), 3);
        assert_eq!(
            result.data["profile"]["top_script_methods"]["methods"][0]["label"],
            "MainMenu._process"
        );
        assert_eq!(
            artifact["top_script_methods"]["methods"][0]["label"],
            "MainMenu._process"
        );
        // Static-memory monitors flow through the generic counter summary so leak
        // trends (min/max/latest bytes) survive the round-trip into data + artifact.
        assert_eq!(
            result.data["profile"]["monitor_counters"]["MEMORY_STATIC"],
            json!({"sample_count": 2, "min": 1000.0, "max": 1500.0, "latest": 1500.0})
        );
        assert_eq!(
            result.data["profile"]["monitor_counters"]["MEMORY_STATIC_MAX"]["latest"],
            2000.0
        );
        // The raw memory samples are also preserved in the bounded artifact history.
        assert!(
            artifact["monitor_samples"]
                .as_array()
                .unwrap()
                .iter()
                .any(|sample| sample["name"] == "MEMORY_STATIC_MAX" && sample["value"] == 2000.0)
        );

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn scenario_progress_heartbeat_contract() {
        let root = temp_dir("scenario-progress-heartbeat-contract");
        let scenario_path = root.join("qa/scenarios/progress_contract.json");
        let scenario = json!({
            "name": "progress_contract",
            "scene": "res://main.tscn",
            "steps": [{"click": "qa:start_button"}],
            "assert": [{"visible": "qa:status"}]
        });
        let artifacts = crate::artifacts::prepare_run_artifacts(
            &root.join(".qa/artifacts"),
            &scenario,
            &root,
            &scenario_path,
            "scenario run progress_contract",
            None,
            &[],
            &[],
            Some("run-001"),
            Some("0Z"),
        )
        .unwrap();
        write(
            &artifacts.progress_path,
            &format!(
                "{}\n{}\n{}\n{}\n",
                json!({
                    "event": "start",
                    "operation": "step",
                    "index": 0,
                    "kind": "input",
                    "command": "input.click_target",
                    "target": "qa:start_button",
                    "started_at": "10Z",
                    "elapsed_ms": 0.0,
                    "artifacts": {"progress": artifacts.progress_path}
                }),
                json!({
                    "event": "finish",
                    "operation": "step",
                    "index": 0,
                    "kind": "input",
                    "command": "input.click_target",
                    "target": "qa:start_button",
                    "started_at": "10Z",
                    "elapsed_ms": 1.0,
                    "ok": true,
                    "failure_count_delta": 0,
                    "artifacts": {"progress": artifacts.progress_path}
                }),
                json!({
                    "event": "start",
                    "operation": "assertion",
                    "index": 0,
                    "kind": "assert",
                    "command": "assert.visible",
                    "target": "qa:status",
                    "started_at": "11Z",
                    "elapsed_ms": 0.0,
                    "artifacts": {"progress": artifacts.progress_path}
                }),
                json!({
                    "event": "finish",
                    "operation": "assertion",
                    "index": 0,
                    "kind": "assert",
                    "command": "assert.visible",
                    "target": "qa:status",
                    "started_at": "11Z",
                    "elapsed_ms": 2.0,
                    "ok": true,
                    "failure_count_delta": 0,
                    "artifacts": {"progress": artifacts.progress_path}
                })
            ),
        );

        let request: Value =
            serde_json::from_str(&fs::read_to_string(&artifacts.request_path).unwrap()).unwrap();
        assert_eq!(
            request["artifacts"]["progress"],
            json!(artifacts.progress_path)
        );
        assert_eq!(
            request["artifacts"]["progress_path"],
            json!(artifacts.progress_path)
        );

        let runtime_request_path = artifacts.root_dir.join("runtime-request.json");
        write_runtime_request(
            &runtime_request_path,
            &scenario,
            &root,
            &artifacts,
            "scenario run progress_contract",
            RuntimeExecutionOptions::default(),
        )
        .unwrap();
        let runtime_request: Value =
            serde_json::from_str(&fs::read_to_string(runtime_request_path).unwrap()).unwrap();
        assert!(request["run"]["nonce"].as_str().is_some_and(|nonce| !nonce.is_empty()));
        assert_eq!(runtime_request["run"]["nonce"], request["run"]["nonce"]);
        assert_eq!(
            runtime_request["artifacts"]["progress"],
            json!(artifacts.progress_path)
        );

        let mut base_data = serde_json::Map::from_iter([
            ("scenario".to_string(), json!("progress_contract")),
            ("scene".to_string(), json!("res://main.tscn")),
            ("artifacts".to_string(), artifacts.to_json()),
        ]);
        let result = validate_runtime_command_result(
            "scenario run",
            &scenario,
            &mut base_data,
            &artifacts.result_path,
            &artifacts.to_json(),
            &test_process_result(&root, 0),
            &json!({"ok": true, "snapshots": [], "failures": [], "duration_ms": 12}),
            &artifacts.logs_path,
            &[],
        );
        assert!(result.ok);
        assert_eq!(
            result.data["progress"]["path"],
            json!(artifacts.progress_path)
        );
        assert_eq!(result.data["progress"]["record_count"], json!(4));
        assert_eq!(result.data["progress"]["exists"], json!(true));
        assert_eq!(
            result.data["artifacts"]["progress"],
            json!(artifacts.progress_path)
        );

        let progress_lines: Vec<Value> = fs::read_to_string(&artifacts.progress_path)
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect();
        assert_eq!(progress_lines[0]["operation"], json!("step"));
        assert_eq!(progress_lines[0]["index"], json!(0));
        assert_eq!(progress_lines[0]["kind"], json!("input"));
        assert_eq!(progress_lines[0]["target"], json!("qa:start_button"));
        assert!(progress_lines[0].get("project_args").is_none());
        assert!(progress_lines[0].get("session_token").is_none());
        assert!(progress_lines[0].get("expected").is_none());
        assert!(progress_lines[0].get("args").is_none());
        assert_eq!(progress_lines[3]["failure_count_delta"], json!(0));

        let mut profile_base_data = serde_json::Map::from_iter([
            ("scenario".to_string(), json!("progress_contract")),
            ("scene".to_string(), json!("res://main.tscn")),
            ("artifacts".to_string(), artifacts.to_json()),
            ("duration_ms".to_string(), json!(12)),
        ]);
        let profile_runtime_result = CommandResult {
            ok: true,
            command: "scenario profile".to_string(),
            message: "scenario profile passed".to_string(),
            data: serde_json::Map::from_iter([("artifacts".to_string(), artifacts.to_json())]),
            failures: vec![],
            exit_code: 0,
        };
        let profile_result = profile_result_from_runtime(
            "scenario profile",
            &scenario,
            &mut profile_base_data,
            &artifacts,
            &json!({
                "ok": true,
                "profile": {
                    "frame_times_ms": [16.0],
                    "step_timings": [{"label": "step", "index": 0, "duration_ms": 1.0}]
                }
            }),
            &profile_runtime_result,
        )
        .unwrap();
        assert_eq!(
            profile_result.data["profile"]["step_timing_count"],
            json!(1)
        );
        assert_eq!(
            profile_result.data["profile"]["step_timings"][0]["label"],
            json!("step")
        );
        assert_eq!(
            profile_result.data["artifacts"]["profile"],
            json!(artifacts.root_dir.join("profile.json"))
        );

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn slow_summary_contract_scenario_progress_is_bounded_sorted_artifact_backed_and_non_failing() {
        let root = temp_dir("scenario-slow-summary-contract");
        let progress_path = root.join("progress.jsonl");
        let artifact_path = root.join("slow-summary.json");
        write(
            &progress_path,
            &format!(
                "{}\n{}\n{}\n",
                json!({"event":"finish","operation":"step","index":0,"kind":"wait_frames","command":"wait_frames","elapsed_ms":5.0,"ok":true,"artifacts":{"progress":progress_path}}),
                json!({"event":"finish","operation":"assertion","index":0,"kind":"assert","command":"assert.no_errors","elapsed_ms":75.0,"ok":true,"artifacts":{"progress":progress_path}}),
                json!({"event":"finish","operation":"step","index":1,"kind":"wait_frames","command":"wait_frames","elapsed_ms":125.0,"ok":true,"artifacts":{"progress":progress_path}})
            ),
        );
        let summary = slow_scenario_summary_json(
            &json!({"name":"slow","slow_summary_threshold_ms":50}),
            &progress_path.to_string_lossy(),
            &artifact_path,
        );
        write_json(&artifact_path, &summary).unwrap();

        assert_eq!(summary["slow_count"], json!(2));
        assert_eq!(summary["slow_operations"][0]["elapsed_ms"], json!(125.0));
        assert_eq!(summary["slow_operations"][1]["elapsed_ms"], json!(75.0));
        assert_eq!(summary["slow_operations"][0]["operation"], json!("step"));
        assert_eq!(summary["artifacts"]["slow_summary"], json!(artifact_path));
        assert!(artifact_path.is_file());

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn scenario_timeout_diagnostic_contract() {
        let root = temp_dir("scenario-timeout-diagnostic-contract");
        let scenario_path = root.join("qa/scenarios/timeout_contract.json");
        let scenario = json!({
            "name": "timeout_contract",
            "scene": "res://main.tscn",
            "timeout": "250ms",
            "steps": [{"wait_frames": 999999}]
        });
        assert_eq!(
            scenario_timeout_duration("scenario run", &scenario_path, &scenario)
                .unwrap()
                .map(duration_millis_u64),
            Some(250)
        );
        for invalid in [
            json!({"name":"bad","scene":"res://main.tscn","timeout_seconds":0,"steps":[]}),
            json!({"name":"bad","scene":"res://main.tscn","timeout_seconds":-1,"steps":[]}),
            json!({"name":"bad","scene":"res://main.tscn","timeout":"5frames","steps":[]}),
            json!({"name":"bad","scene":"res://main.tscn","timeout":"fast","steps":[]}),
        ] {
            let failure = scenario_timeout_duration("scenario run", &scenario_path, &invalid)
                .expect_err("invalid timeout rejected");
            assert_eq!(failure.kind, "scenario_validation_failed");
        }

        let artifacts = crate::artifacts::prepare_run_artifacts(
            &root.join(".qa/artifacts"),
            &scenario,
            &root,
            &scenario_path,
            "scenario run timeout_contract",
            None,
            &[],
            &[],
            Some("run-001"),
            Some("0Z"),
        )
        .unwrap();
        // A declared timeout with frame pacing puts the engine-enforced frame
        // budget in the request: ceil(0.25s x default 60fps) = 15 frames.
        let request: Value =
            serde_json::from_str(&fs::read_to_string(&artifacts.request_path).unwrap()).unwrap();
        assert_eq!(request["run"]["frame_budget"]["frames"], json!(15));
        assert_eq!(request["run"]["frame_budget"]["fixed_fps"], json!(60));
        write(
            &artifacts.progress_path,
            &format!(
                "{}\n",
                json!({
                    "event": "start",
                    "operation": "assertion",
                    "index": 2,
                    "kind": "assert",
                    "command": "assert.visible",
                    "target": "qa:status",
                    "started_at": "10Z",
                    "elapsed_ms": 12.0,
                    "artifacts": {"progress": artifacts.progress_path}
                })
            ),
        );
        let mut process_result = test_process_result(&root, -1);
        process_result.timed_out = true;
        process_result.timeout_ms = Some(250);
        process_result.infrastructure_failure_kind = Some("owned_process_timeout".to_string());
        process_result.cleanup = Some(godot_qa_process::ProcessCleanupResult {
            status: "timeout_killed".to_string(),
            failure_kind: Some("owned_process_timeout".to_string()),
            pid: Some(123),
            process_group: Some(123),
            owned: true,
            message: Some(
                "owned Godot process exceeded configured timeout and was killed".to_string(),
            ),
        });
        let artifacts_map = artifacts.to_json().as_object().cloned().unwrap();
        let failure = scenario_run_timeout_failure(
            "scenario run",
            &artifacts.result_path,
            &artifacts_map,
            &process_result,
            Some(Duration::from_millis(250)),
        );
        assert_eq!(failure.kind, "scenario_run_timeout");
        assert_ne!(failure.kind, "assertion_failed");
        assert_eq!(failure.details["failure_category"], json!("process"));
        let actual = failure.actual.as_ref().unwrap();
        assert_eq!(actual["timeout_ms"], json!(250));
        assert_eq!(actual["active_assertion"]["index"], json!(2));
        assert_eq!(actual["cleanup"]["owned"], json!(true));
        assert_eq!(
            failure.artifacts.as_ref().unwrap()["progress"],
            json!(artifacts.progress_path)
        );

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn scenario_frame_budget_derivation_contract() {
        let root = temp_dir("scenario-frame-budget-derivation");
        write(
            &root.join("project.godot"),
            "[application]\nconfig/name=\"sample\"\n",
        );

        // Declared numeric timeout at scenario-pinned fps.
        let scenario = json!({
            "name": "budgeted", "scene": "res://main.tscn",
            "timeout_seconds": 2.5,
            "determinism": {"fixed_fps": 60},
            "steps": []
        });
        let budget = super::scenario_frame_budget_json(&scenario, &root).expect("budget derived");
        assert_eq!(budget["frames"], json!(150));
        assert_eq!(budget["timeout_seconds"], json!(2.5));
        assert_eq!(budget["fixed_fps"], json!(60));
        assert_eq!(budget["source"], json!("scenario"));

        // Duration-string alias at project-default pacing.
        let scenario = json!({
            "name": "budgeted", "scene": "res://main.tscn",
            "run_timeout": "500ms",
            "steps": []
        });
        let budget = super::scenario_frame_budget_json(&scenario, &root).expect("budget derived");
        assert_eq!(budget["frames"], json!(30));
        assert_eq!(budget["source"], json!("godot_default"));

        // Numeric alias with a scenario fps override.
        let scenario = json!({
            "name": "budgeted", "scene": "res://main.tscn",
            "run_timeout_seconds": 1,
            "determinism": {"fixed_fps": 120},
            "steps": []
        });
        let budget = super::scenario_frame_budget_json(&scenario, &root).expect("budget derived");
        assert_eq!(budget["frames"], json!(120));

        // No declared timeout -> no budget (unbounded scenarios keep exact
        // current behavior).
        let scenario = json!({"name": "plain", "scene": "res://main.tscn", "steps": []});
        assert!(super::scenario_frame_budget_json(&scenario, &root).is_none());

        // time_control disables frame pacing -> wall-clock semantics, no budget.
        let scenario = json!({
            "name": "realtime", "scene": "res://main.tscn",
            "timeout_seconds": 30,
            "time_control": {"time_scale": 1.0},
            "steps": []
        });
        assert!(super::scenario_frame_budget_json(&scenario, &root).is_none());

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn scenario_timeout_alias_and_conflict_contract() {
        let scenario_path = PathBuf::from("qa/scenarios/alias.json");
        let aliased = json!({"name":"a","scene":"res://main.tscn","run_timeout_seconds":2,"steps":[]});
        assert_eq!(
            scenario_timeout_duration("scenario run", &scenario_path, &aliased)
                .unwrap()
                .map(duration_millis_u64),
            Some(2000)
        );
        let aliased = json!({"name":"a","scene":"res://main.tscn","run_timeout":"3s","steps":[]});
        assert_eq!(
            scenario_timeout_duration("scenario run", &scenario_path, &aliased)
                .unwrap()
                .map(duration_millis_u64),
            Some(3000)
        );
        // Invalid alias values are rejected like their canonical fields.
        for invalid in [
            json!({"name":"a","scene":"res://main.tscn","run_timeout_seconds":0,"steps":[]}),
            json!({"name":"a","scene":"res://main.tscn","run_timeout":"fast","steps":[]}),
        ] {
            let failure = scenario_timeout_duration("scenario run", &scenario_path, &invalid)
                .expect_err("invalid alias timeout rejected");
            assert_eq!(failure.kind, "scenario_validation_failed");
        }
        // More than one of the four timeout fields conflicts, in the run path
        // and at document validation time.
        let conflicted = json!({
            "name":"a","scene":"res://main.tscn",
            "timeout_seconds":2,"run_timeout":"3s","steps":[]
        });
        let failure = scenario_timeout_duration("scenario run", &scenario_path, &conflicted)
            .expect_err("conflicting timeout fields rejected");
        assert_eq!(failure.kind, "scenario_validation_failed");
        let err = validate_scenario_document(&conflicted, &scenario_path)
            .expect_err("conflicting timeout fields rejected by validation");
        assert_eq!(err.failure.kind, "scenario_validation_failed");
        // Valid aliases pass document validation and survive normalization.
        let aliased = json!({"name":"a","scene":"res://main.tscn","run_timeout":"3s","steps":[]});
        validate_scenario_document(&aliased, &scenario_path).expect("alias timeout validates");
        let normalized = crate::scenario_doc::normalize_scenario_document(&aliased);
        assert_eq!(normalized["run_timeout"], json!("3s"));
    }

    #[test]
    fn frame_budget_wall_backstop_contract() {
        // Without frame pacing the declared wall timeout is untouched.
        assert_eq!(
            super::frame_budget_wall_backstop(Some(Duration::from_secs(120)), false),
            Some(Duration::from_secs(120))
        );
        // With a frame budget the wall clock is a 3x hang backstop...
        assert_eq!(
            super::frame_budget_wall_backstop(Some(Duration::from_secs(120)), true),
            Some(Duration::from_secs(360))
        );
        // ...capped at 1800s...
        assert_eq!(
            super::frame_budget_wall_backstop(Some(Duration::from_secs(900)), true),
            Some(Duration::from_secs(1800))
        );
        // ...but never below the declared wall.
        assert_eq!(
            super::frame_budget_wall_backstop(Some(Duration::from_secs(2000)), true),
            Some(Duration::from_secs(2000))
        );
        assert_eq!(super::frame_budget_wall_backstop(None, true), None);
    }

    #[test]
    fn scenario_progress_redaction_contract() {
        let root = temp_dir("scenario-progress-redaction-contract");
        let progress_path = root.join("progress.jsonl");
        write(
            &progress_path,
            &format!(
                "{}\n",
                json!({
                    "event": "start",
                    "operation": "step",
                    "index": 0,
                    "kind": "input",
                    "target": "qa:start",
                    "session_token": "live-token",
                    "project_args": ["--api-key", "secret-value", "--token=abc123", "--safe", "ok"],
                    "args": ["--password", "hunter2"]
                })
            ),
        );
        let active = latest_active_progress_event(&progress_path.to_string_lossy()).unwrap();
        let rendered = serde_json::to_string(&active).unwrap();
        assert!(!rendered.contains("live-token"));
        assert!(!rendered.contains("secret-value"));
        assert!(!rendered.contains("abc123"));
        assert!(!rendered.contains("hunter2"));
        assert!(rendered.contains("<redacted:"));

        let redacted = process_summary_json(&GodotRunResult {
            command: vec![
                "godot".to_string(),
                "--token=abc123".to_string(),
                "--password".to_string(),
                "hunter2".to_string(),
            ],
            returncode: -1,
            stdout: String::new(),
            stderr: String::new(),
            display: godot_qa_process::resolve_project_display_launch_metadata(&root, &[], true),
            pid: None,
            process_group: None,
            cleanup: None,
            infrastructure_failure_kind: None,
            timed_out: true,
            timeout_ms: Some(100),
            output_truncation: None,
        });
        let rendered = serde_json::to_string(&redacted).unwrap();
        assert!(!rendered.contains("abc123"));
        assert!(!rendered.contains("hunter2"));
        assert!(rendered.contains("<redacted:"));

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn time_control_profile_artifact_contract() {
        let root = temp_dir("time-control-profile-artifact-contract");
        let artifacts = test_run_artifacts(&root);
        let time_control = json!({
            "target": "scenario",
            "requested": {"time_scale": 4.0},
            "effective": {"time_scale": 4.0},
            "restoration_status": "restored",
            "warnings": [{"type": "time_control_physics_pairing_missing"}]
        });
        let mut base_data = serde_json::Map::from_iter([
            ("duration_ms".to_string(), json!(123)),
            ("artifacts".to_string(), artifacts.to_json()),
            ("time_control".to_string(), time_control.clone()),
        ]);
        let runtime_result = godot_qa_core::results::CommandResult {
            ok: true,
            command: "scenario profile".to_string(),
            message: "scenario profile passed".to_string(),
            data: base_data.clone(),
            failures: Vec::new(),
            exit_code: 0,
        };
        let raw = json!({
            "ok": true,
            "time_control": time_control,
            "profile": {
                "frame_times_ms": [16.0, 18.0],
                "monitor_samples": [],
                "step_timings": []
            }
        });

        let result = profile_result_from_runtime(
            "scenario profile",
            &json!({"name": "profiled", "scene": "res://main.tscn"}),
            &mut base_data,
            &artifacts,
            &raw,
            &runtime_result,
        )
        .unwrap();

        assert_eq!(result.data["time_control"], time_control);
        let profile_path = result.data["artifacts"]["profile"].as_str().unwrap();
        let artifact: Value =
            serde_json::from_str(&fs::read_to_string(profile_path).unwrap()).unwrap();
        assert_eq!(artifact["time_control"], time_control);

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn profile_result_truncates_frame_sample_history_to_limit() {
        let root = temp_dir("profile-artifact-bounded");
        let artifacts = test_run_artifacts(&root);
        let mut base_data = serde_json::Map::from_iter([
            ("duration_ms".to_string(), json!(123)),
            ("artifacts".to_string(), artifacts.to_json()),
        ]);
        let runtime_result = godot_qa_core::results::CommandResult {
            ok: true,
            command: "scenario profile".to_string(),
            message: "scenario profile passed".to_string(),
            data: base_data.clone(),
            failures: Vec::new(),
            exit_code: 0,
        };
        let samples = (0..(super::PROFILE_MAX_FRAME_SAMPLES + 25))
            .map(|sample| json!(sample as f64))
            .collect::<Vec<_>>();
        let raw = json!({
            "ok": true,
            "profile": {
                "frame_times_ms": samples,
                "monitor_samples": [],
                "step_timings": []
            }
        });

        let result = profile_result_from_runtime(
            "scenario profile",
            &json!({"name": "profiled", "scene": "res://main.tscn"}),
            &mut base_data,
            &artifacts,
            &raw,
            &runtime_result,
        )
        .unwrap();

        let profile_path = result.data["artifacts"]["profile"].as_str().unwrap();
        let artifact: Value =
            serde_json::from_str(&fs::read_to_string(profile_path).unwrap()).unwrap();
        assert_eq!(
            artifact["frame_times_ms"].as_array().unwrap().len(),
            super::PROFILE_MAX_FRAME_SAMPLES
        );
        assert_eq!(
            result.data["profile"]["frame_time_ms"]["sample_count"],
            super::PROFILE_MAX_FRAME_SAMPLES + 25
        );

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn profile_result_rejects_malformed_profile_payload() {
        let root = temp_dir("profile-malformed");
        let artifacts = test_run_artifacts(&root);
        let mut base_data =
            serde_json::Map::from_iter([("artifacts".to_string(), artifacts.to_json())]);
        let runtime_result = godot_qa_core::results::CommandResult {
            ok: true,
            command: "scenario profile".to_string(),
            message: "scenario profile passed".to_string(),
            data: base_data.clone(),
            failures: Vec::new(),
            exit_code: 0,
        };

        let result = profile_result_from_runtime(
            "scenario profile",
            &json!({"name": "profiled", "scene": "res://main.tscn"}),
            &mut base_data,
            &artifacts,
            &json!({"ok": true, "profile": {"frame_times_ms": ["slow"]}}),
            &runtime_result,
        )
        .unwrap_err();

        assert_eq!(result.failures[0].kind, "scenario_result_invalid");
        assert_eq!(result.command, "scenario profile");

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn profile_result_rejects_malformed_monitor_and_step_payloads() {
        for (name, runtime_payload, expected) in [
            (
                "profile-malformed-monitor",
                json!({"ok": true, "profile": {"monitor_samples": "unsupported"}}),
                "profile.monitor_samples must be an array of objects",
            ),
            (
                "profile-malformed-step",
                json!({"ok": true, "profile": {"step_timings": [1]}}),
                "profile.step_timings must be an array of objects",
            ),
        ] {
            let root = temp_dir(name);
            let artifacts = test_run_artifacts(&root);
            let mut base_data =
                serde_json::Map::from_iter([("artifacts".to_string(), artifacts.to_json())]);
            let runtime_result = godot_qa_core::results::CommandResult {
                ok: true,
                command: "scenario profile".to_string(),
                message: "scenario profile passed".to_string(),
                data: base_data.clone(),
                failures: Vec::new(),
                exit_code: 0,
            };

            let result = profile_result_from_runtime(
                "scenario profile",
                &json!({"name": "profiled", "scene": "res://main.tscn"}),
                &mut base_data,
                &artifacts,
                &runtime_payload,
                &runtime_result,
            )
            .unwrap_err();

            assert_eq!(result.failures[0].kind, "scenario_result_invalid");
            assert_eq!(result.failures[0].message.as_deref(), Some(expected));

            fs::remove_dir_all(root).unwrap();
        }
    }

    #[test]
    fn profile_result_normalizes_unsupported_monitor_samples_out_of_summary() {
        let root = temp_dir("profile-monitor-normalization");
        let artifacts = test_run_artifacts(&root);
        let mut base_data =
            serde_json::Map::from_iter([("artifacts".to_string(), artifacts.to_json())]);
        let runtime_result = godot_qa_core::results::CommandResult {
            ok: true,
            command: "scenario profile".to_string(),
            message: "scenario profile passed".to_string(),
            data: base_data.clone(),
            failures: Vec::new(),
            exit_code: 0,
        };
        let raw = json!({
            "ok": true,
            "profile": {
                "frame_times_ms": [16.0],
                "monitor_samples": [
                    {"name": "time/process", "value": 1.0},
                    {"name": "time/process", "value": 3.0},
                    {"name": "unsupported", "value": null},
                    {"counter": "object/node_count", "value": 7.0},
                    {"value": 99.0}
                ],
                "step_timings": []
            }
        });

        let result = profile_result_from_runtime(
            "scenario profile",
            &json!({"name": "profiled", "scene": "res://main.tscn"}),
            &mut base_data,
            &artifacts,
            &raw,
            &runtime_result,
        )
        .unwrap();

        assert_eq!(
            result.data["profile"]["monitor_counters"]["time/process"],
            json!({"sample_count": 2, "min": 1.0, "max": 3.0, "latest": 3.0})
        );
        assert_eq!(
            result.data["profile"]["monitor_counters"]["object/node_count"]["latest"],
            7.0
        );
        assert!(
            result.data["profile"]["monitor_counters"]
                .get("unsupported")
                .is_none()
        );

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn profile_result_shapes_artifact_write_failed() {
        let root = temp_dir("profile-artifact-write-failed");
        let artifacts = test_run_artifacts(&root);
        let mut base_data =
            serde_json::Map::from_iter([("artifacts".to_string(), artifacts.to_json())]);
        let runtime_result = godot_qa_core::results::CommandResult {
            ok: true,
            command: "scenario profile".to_string(),
            message: "scenario profile passed".to_string(),
            data: base_data.clone(),
            failures: Vec::new(),
            exit_code: 0,
        };
        fs::write(root.join("profile.json"), "already a directory blocker").unwrap();
        fs::remove_file(root.join("profile.json")).unwrap();
        fs::create_dir(root.join("profile.json")).unwrap();

        let result = profile_result_from_runtime(
            "scenario profile",
            &json!({"name": "profiled", "scene": "res://main.tscn"}),
            &mut base_data,
            &artifacts,
            &json!({"ok": true, "profile": {"frame_times_ms": [16.0]}}),
            &runtime_result,
        )
        .unwrap_err();

        assert!(!result.ok);
        assert_eq!(result.exit_code, 1);
        assert_eq!(result.failures[0].kind, "artifact_write_failed");
        assert_eq!(
            result.failures[0].command.as_deref(),
            Some("scenario profile")
        );
        assert!(
            result.failures[0]
                .path
                .as_deref()
                .unwrap()
                .ends_with("profile.json")
        );

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn profile_runtime_assertion_failure_remains_runtime_failure() {
        let root = temp_dir("profile-runtime-assertion-failure");
        let result_path = root.join("result.json");
        let logs_path = root.join("logs.txt");
        write(&logs_path, "");
        let artifacts = json!({
            "run_root": root,
            "result_path": result_path,
            "logs_path": logs_path,
        });
        let mut base_data = serde_json::Map::new();
        let runtime_result = json!({
            "ok": false,
            "snapshots": [],
            "failures": [{
                "type": "assertion_failed",
                "message": "expected node to be visible"
            }],
            "profile": {"frame_times_ms": [16.0]}
        });

        let result = validate_runtime_command_result(
            "scenario profile",
            &json!({}),
            &mut base_data,
            &result_path,
            &artifacts,
            &test_process_result(&root, 0),
            &runtime_result,
            &logs_path,
            &[],
        );

        assert_eq!(result.failures[0].kind, "assertion_failed");
        assert_eq!(result.message, "scenario profile failed");

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn runtime_time_control_result_contract() {
        let root = temp_dir("runtime-time-control-result-contract");
        let time_control = json!({
            "target": "scenario",
            "original": {
                "time_scale": 1.0,
                "physics_ticks_per_second": 60,
                "max_physics_steps_per_frame": 8
            },
            "requested": {
                "time_scale": 4.0,
                "physics_ticks_per_second": 240,
                "max_physics_steps_per_frame": 16
            },
            "effective": {
                "time_scale": 4.0,
                "physics_ticks_per_second": 240,
                "max_physics_steps_per_frame": 16
            },
            "restored": {
                "time_scale": 1.0,
                "physics_ticks_per_second": 60,
                "max_physics_steps_per_frame": 8
            },
            "restored_matches_original": true,
            "restoration_status": "restored",
            "warnings": []
        });
        let runtime_result = json!({
            "ok": true,
            "snapshots": [],
            "failures": [],
            "time_control": time_control
        });

        let result = strict_summary_result(&root, 0, &runtime_result, "");

        assert!(result.ok);
        assert_eq!(result.data["time_control"], time_control);

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn runtime_time_control_omitted_when_unrequested() {
        let root = temp_dir("runtime-time-control-omitted-when-unrequested");
        let runtime_result = json!({
            "ok": true,
            "snapshots": [],
            "failures": []
        });

        let result = strict_summary_result(&root, 0, &runtime_result, "");

        assert!(result.ok);
        assert!(result.data.get("time_control").is_none());

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn time_control_request_artifact_contract() {
        let root = temp_dir("time-control-request-artifact-contract");
        let scenario_path = root.join("qa/scenarios/accelerated.json");
        let scenario = json!({
            "name": "accelerated",
            "scene": "res://main.tscn",
            "time_control": {"time_scale": 4.0, "physics_ticks_per_second": 240, "max_physics_steps_per_frame": 16},
            "steps": []
        });
        let artifacts = crate::artifacts::prepare_run_artifacts(
            &root.join(".qa/artifacts"),
            &scenario,
            &root,
            &scenario_path,
            "scenario run accelerated",
            None,
            &[],
            &[],
            Some("run-001"),
            Some("0Z"),
        )
        .unwrap();

        let meta: Value =
            serde_json::from_str(&fs::read_to_string(&artifacts.meta_path).unwrap()).unwrap();
        let request: Value =
            serde_json::from_str(&fs::read_to_string(&artifacts.request_path).unwrap()).unwrap();
        assert_eq!(meta["time_control"], scenario["time_control"]);
        assert_eq!(
            request["scenario"]["time_control"],
            scenario["time_control"]
        );
        assert_eq!(request["run"]["time_control"], scenario["time_control"]);

        let runtime_request_path = artifacts.root_dir.join("runtime-request.json");
        write_runtime_request(
            &runtime_request_path,
            &scenario,
            &root,
            &artifacts,
            "scenario run accelerated",
            RuntimeExecutionOptions::default(),
        )
        .unwrap();
        let runtime_request: Value =
            serde_json::from_str(&fs::read_to_string(runtime_request_path).unwrap()).unwrap();
        assert_eq!(runtime_request["time_control"], scenario["time_control"]);
        assert_eq!(
            runtime_request["run"]["time_control"],
            scenario["time_control"]
        );
        assert_eq!(runtime_request["time_control_warnings"], json!([]));

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn time_control_visual_warning_contract() {
        let scenario = json!({
            "name": "accelerated_visual",
            "scene": "res://main.tscn",
            "time_control": {"time_scale": 8.0},
            "snapshots": {"save_screenshot": true},
            "steps": []
        });
        let warnings = time_control_warnings(
            "scenario run",
            &scenario,
            RuntimeExecutionOptions {
                visual_risk: VisualRiskExecutionOptions {
                    enabled: true,
                    strict: false,
                },
                ..RuntimeExecutionOptions::default()
            },
        );

        assert_eq!(warnings.len(), 1);
        assert_eq!(warnings[0].kind, "time_control_physics_pairing_missing");
        assert_eq!(warnings[0].severity.as_deref(), Some("warning"));
        assert_eq!(
            warnings[0].details["risk_sources"],
            json!(["screenshot", "visual_risk"])
        );

        let paired = json!({
            "name": "paired_visual",
            "time_control": {"time_scale": 8.0, "physics_ticks_per_second": 480, "max_physics_steps_per_frame": 16},
            "snapshots": {"save_screenshot": true}
        });
        assert!(
            time_control_warnings("scenario run", &paired, RuntimeExecutionOptions::default())
                .is_empty()
        );
    }

    #[test]
    fn run_generated_tree_once_scenario_preserves_snapshot_filters() {
        let filters = json!({
            "target": "qa:start_button",
            "max_depth": 1,
            "qa_only": true,
            "include_metadata": true,
            "metadata_keys": ["qa_id", "role"]
        })
        .as_object()
        .cloned()
        .unwrap();

        let scenario = generated_tree_scenario("res://main.tscn", &filters);

        assert_eq!(scenario["name"], "generated_tree_once");
        assert_eq!(scenario["scene"], "res://main.tscn");
        assert_eq!(
            scenario["steps"],
            json!([{"wait_frames": 1}, {"snapshot": "current"}])
        );
        assert_eq!(scenario["snapshots"]["save_screenshot"], false);
        assert_eq!(scenario["snapshots"]["target"], "qa:start_button");
        assert_eq!(scenario["snapshots"]["max_depth"], 1);
        assert_eq!(scenario["snapshots"]["qa_only"], true);
        assert_eq!(scenario["snapshots"]["include_metadata"], true);
        assert_eq!(
            scenario["snapshots"]["metadata_keys"],
            json!(["qa_id", "role"])
        );
    }

    #[test]
    fn run_generated_tree_once_missing_tree_artifacts_are_actionable() {
        let data = json!({
            "snapshot_artifacts": {
                "current": {
                    "ui": "/tmp/ui.json",
                    "focus": "/tmp/focus.json",
                    "errors": "/tmp/errors.json"
                }
            }
        })
        .as_object()
        .cloned()
        .unwrap();
        let artifacts = snapshot_artifacts_for(&data, "current").unwrap();

        let failure = super::path_from_artifacts(&artifacts, "tree").unwrap_err();

        assert_eq!(failure.kind, "artifact_read_failed");
        assert_eq!(
            failure.expected,
            Some(json!(["tree", "ui", "focus", "errors"]))
        );
        assert_eq!(failure.actual, Some(json!(["tree"])));
        assert_eq!(failure.artifacts.unwrap()["ui"], "/tmp/ui.json");
    }

    #[test]
    fn runtime_log_failures_ignores_known_wayland_startup_warnings() {
        let root = temp_dir("wayland-warnings");
        let logs_path = root.join("logs.txt");
        write(
            &logs_path,
            "WARNING: Can't obtain the XDG decoration manager. Libdecor will be used for drawing CSDs, if available.\nWARNING: xdg-toplevel-icon protocol not found! Cannot set window icon.\nWARNING: FIFO protocol not found! Frame pacing will be degraded.\nWARNING: The requested V-Sync mode Disabled is not available. Falling back to V-Sync mode Enabled.\n",
        );
        let artifacts = json!({"logs_path": logs_path})
            .as_object()
            .cloned()
            .unwrap();

        let (failures, ignored) = runtime_log_failures("scenario run", &logs_path, &artifacts, &[]);

        let _ = fs::remove_dir_all(&root);

        assert!(failures.is_empty());
        assert_eq!(ignored, 4);
    }

    #[test]
    fn runtime_log_failures_ignores_known_virtual_display_startup_noise() {
        let root = temp_dir("virtual-display-noise");
        let logs_path = root.join("logs.txt");
        write(
            &logs_path,
            "ERROR: Required extension VK_KHR_surface not found.\nERROR: Condition \"err != OK\" is true. Returning: err\nWARNING: Your video card drivers seem not to support the required Vulkan version, switching to OpenGL 3.\nWARNING: Could not set V-Sync mode, as changing V-Sync mode is not supported by the graphics driver.\nERROR: Condition \"status < 0\" is true. Returning: ERR_CANT_OPEN\nWARNING: All audio drivers failed, falling back to the dummy driver.\n",
        );
        let artifacts = json!({"logs_path": logs_path})
            .as_object()
            .cloned()
            .unwrap();

        let (failures, ignored) = runtime_log_failures("scenario run", &logs_path, &artifacts, &[]);

        let _ = fs::remove_dir_all(&root);

        assert!(failures.is_empty());
        assert_eq!(ignored, 6);
    }

    /// A warm run's logs.txt is a slice of the pooled session log INCLUDING the
    /// two `scenario-log` marker lines that bound it, so the shared cold log
    /// scanner must treat those lines as ordinary output. They carry no
    /// `ERROR:`/`WARNING:` prefix, so this needs no ignore-list entry — pin that.
    #[test]
    fn runtime_log_failures_ignores_warm_scenario_log_markers() {
        let root = temp_dir("warm-scenario-log-markers");
        let logs_path = root.join("logs.txt");
        let prefix = super::WARM_LOG_MARKER_PREFIX;
        write(
            &logs_path,
            &format!(
                "{prefix} begin 20260101T000000Z\nWARNING: All audio drivers failed, falling back \
                 to the dummy driver.\n{prefix} end 20260101T000000Z\n"
            ),
        );
        let artifacts = json!({"logs_path": logs_path})
            .as_object()
            .cloned()
            .unwrap();

        let (failures, ignored) = runtime_log_failures("scenario run", &logs_path, &artifacts, &[]);

        let _ = fs::remove_dir_all(&root);

        assert!(failures.is_empty(), "{failures:?}");
        // The two marker lines carry no ERROR:/WARNING: prefix, so only the audio
        // WARNING is an ignore-list hit.
        assert_eq!(ignored, 1);
    }

    /// A project's `logs.ignore` patterns silence engine lines the built-in lists
    /// do not know about — including real `ERROR:` lines, which is the whole point
    /// and the whole risk. Anything the project did NOT list still fails.
    #[test]
    fn runtime_log_failures_honours_project_log_ignore_patterns() {
        let root = temp_dir("project-log-ignore");
        let logs_path = root.join("logs.txt");
        write(
            &logs_path,
            "ERROR: Cannot call method 'emit' on a previously freed instance.\nWARNING: Shader \
             compilation took 900ms.\nERROR: something the project did not silence\n",
        );
        let artifacts = json!({"logs_path": logs_path})
            .as_object()
            .cloned()
            .unwrap();
        let patterns = vec![
            "previously freed instance".to_string(),
            "Shader compilation took".to_string(),
        ];

        let (failures, ignored) =
            runtime_log_failures("scenario run", &logs_path, &artifacts, &patterns);

        let _ = fs::remove_dir_all(&root);

        assert_eq!(ignored, 2);
        assert_eq!(failures.len(), 1, "{failures:?}");
        assert_eq!(failures[0].kind, "godot_log_error");
        assert_eq!(
            failures[0].actual,
            Some(json!("ERROR: something the project did not silence"))
        );
    }

    /// Without the project list the very same log is a hard failure: the ignore
    /// list is the only thing standing between this line and a red run.
    #[test]
    fn runtime_log_failures_without_project_patterns_still_flags_the_line() {
        let root = temp_dir("project-log-ignore-absent");
        let logs_path = root.join("logs.txt");
        write(
            &logs_path,
            "ERROR: Cannot call method 'emit' on a previously freed instance.\n",
        );
        let artifacts = json!({"logs_path": logs_path})
            .as_object()
            .cloned()
            .unwrap();

        let (failures, ignored) = runtime_log_failures("scenario run", &logs_path, &artifacts, &[]);

        let _ = fs::remove_dir_all(&root);

        assert_eq!(ignored, 0);
        assert_eq!(failures.len(), 1, "{failures:?}");
        assert_eq!(failures[0].kind, "godot_log_error");
    }

    /// `data.logs` publishes what the ignore list did: `ignored_count` for the
    /// silenced lines and `ignore_patterns_applied` for how many project patterns
    /// were in force, so a silenced run is never indistinguishable from a clean one.
    #[test]
    fn validate_runtime_command_result_reports_log_ignore_counters() {
        let root = temp_dir("log-ignore-counters");
        let logs_path = root.join("logs.txt");
        let result_path = root.join("result.json");
        write(
            &logs_path,
            "ERROR: Cannot call method 'emit' on a previously freed instance.\nERROR: Cannot call \
             method 'emit' on a previously freed instance.\n",
        );
        let artifacts = json!({
            "run_root": root,
            "result_path": result_path,
            "logs_path": logs_path,
        });
        let mut base_data =
            serde_json::Map::from_iter([("artifacts".to_string(), artifacts.clone())]);

        let result = validate_runtime_command_result(
            "scenario run",
            &json!({}),
            &mut base_data,
            &result_path,
            &artifacts,
            &test_process_result(&root, 0),
            &json!({"ok": true, "snapshots": [], "failures": []}),
            &logs_path,
            &["previously freed instance".to_string()],
        );

        let _ = fs::remove_dir_all(&root);

        assert!(result.ok, "{:?}", result.failures);
        assert_eq!(result.data["logs"]["status"], json!("clean"));
        assert_eq!(result.data["logs"]["error_count"], json!(0));
        assert_eq!(result.data["logs"]["warning_count"], json!(0));
        assert_eq!(result.data["logs"]["ignored_count"], json!(2));
        assert_eq!(result.data["logs"]["ignore_patterns_applied"], json!(1));
    }

    #[test]
    fn scenario_requests_screenshot_when_snapshot_saving_is_enabled() {
        let scenario = json!({
            "name": "main_menu_screenshot",
            "scene": "res://ui/main_menu.tscn",
            "steps": [{"snapshot": "after_start"}],
            "snapshots": {"save_screenshot": true},
        });

        assert!(scenario_requests_screenshot(&scenario));
    }

    #[test]
    fn scenario_requests_screenshot_when_screenshot_match_is_present() {
        let scenario = json!({
            "name": "main_menu_start_opens_pause",
            "scene": "res://ui/main_menu.tscn",
            "steps": [{"snapshot": "after_start"}],
            "assert": [{
                "screenshot_match": {
                    "snapshot": "after_start",
                    "baseline": "qa/baselines/main_menu_start_opens_pause/after_start.png",
                    "threshold": 0.0
                }
            }],
        });

        assert!(scenario_requests_screenshot(&scenario));
    }

    #[test]
    fn visual_assertions_request_screenshot_without_baseline_approval() {
        for assertion in [
            json!({
                "visible_pixels": {
                    "target": "qa:important_actor",
                    "snapshot": "after",
                    "min_ratio": 0.25
                }
            }),
            json!({
                "not_occluded": {
                    "target": "qa:important_actor",
                    "occluder": "qa:overlay",
                    "snapshot": "after"
                }
            }),
            json!({
                "renders_above": {
                    "target": "qa:important_actor",
                    "other": "qa:background",
                    "snapshot": "after"
                }
            }),
            json!({
                "renders_below": {
                    "target": "qa:background",
                    "other": "qa:important_actor",
                    "snapshot": "after"
                }
            }),
        ] {
            let scenario = json!({
                "name": "visual_occlusion_demo",
                "scene": "res://ui/main_menu.tscn",
                "steps": [{"snapshot": "after"}],
                "assert": [assertion],
            });

            assert!(scenario_requests_screenshot(&scenario));
            assert!(!scenario.to_string().contains("qa/baselines"));
        }
    }

    #[test]
    fn step_local_visual_assertions_request_screenshot() {
        let scenario = json!({
            "name": "step_visual_payload",
            "scene": "res://main.tscn",
            "steps": [
                {"snapshot": "after"},
                {"assert": [
                    {"visible_pixels": {"target": "qa:important_actor", "snapshot": "after", "min_ratio": 0.25}}
                ]}
            ],
            "assert": []
        });

        assert!(scenario_contains_visual_assertion(&scenario));
        assert!(scenario_requests_screenshot(&scenario));
    }

    #[test]
    fn visual_assertion_detection_requires_real_visual_evidence() {
        assert!(scenario_contains_visual_assertion(&json!({
            "assert": [{"visible_pixels": {"target": "qa:actor"}}],
        })));
        assert!(scenario_contains_visual_assertion(&json!({
            "assert": [{"not_occluded": {"target": "qa:actor", "occluder": "qa:cover"}}],
        })));
        assert!(!scenario_contains_visual_assertion(&json!({
            "snapshots": {"save_screenshot": true},
            "assert": [{"visible": "qa:actor"}],
        })));
    }
