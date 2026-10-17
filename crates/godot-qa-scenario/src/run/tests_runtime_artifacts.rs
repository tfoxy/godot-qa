    #[test]
    fn scenario_process_outcome_strict_malformed_result_json_is_distinct() {
        let root = temp_dir("scenario-process-outcome-invalid-result");
        let result_path = root.join("result.json");
        let logs_path = root.join("logs.txt");
        write(&result_path, "{not json");
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
            super::runtime_result_summary_json(&result_path, "invalid", None, 0),
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
        assert_eq!(result.failures[0].kind, "scenario_result_invalid");
        assert_eq!(result.data["runtime_result"]["status"], json!("invalid"));
        assert_eq!(result.data["runtime_result"]["ok"], Value::Null);
        assert_eq!(
            result.data["outcome"],
            json!({
                "status":"fail",
                "reason":"scenario_result_invalid",
                "failure_category":"artifact",
                "retryable":false
            })
        );
    }

    #[test]
    fn scenario_process_outcome_strict_nonzero_process_with_clean_runtime_is_distinct() {
        let root = temp_dir("scenario-process-outcome-process-failure");
        let result = strict_summary_result(
            &root,
            11,
            &json!({"ok": true, "snapshots": [], "failures": []}),
            "",
        );

        let _ = fs::remove_dir_all(&root);

        assert!(!result.ok);
        assert_eq!(result.exit_code, 1);
        assert_strict_summaries(&result);
        assert_eq!(result.failures[0].kind, "godot_run_failed");
        assert_eq!(result.data["process"]["status"], json!("failed"));
        assert_eq!(result.data["process"]["exit_code"], json!(11));
        assert_eq!(result.data["runtime_result"]["status"], json!("passed"));
        assert_eq!(
            result.data["outcome"],
            json!({
                "status":"fail",
                "reason":"godot_run_failed",
                "failure_category":"process",
                "retryable":false
            })
        );
    }

    #[test]
    fn scenario_process_outcome_strict_log_error_and_warning_failures_are_distinct() {
        let root = temp_dir("scenario-process-outcome-log-failures");
        let result = strict_summary_result(
            &root,
            0,
            &json!({"ok": true, "snapshots": [], "failures": []}),
            "WARNING: unexpected warning\nERROR: unexpected error\n",
        );

        let _ = fs::remove_dir_all(&root);

        assert!(!result.ok);
        assert_eq!(result.exit_code, 1);
        assert_strict_summaries(&result);
        assert!(
            result
                .failures
                .iter()
                .any(|failure| failure.kind == "godot_log_error")
        );
        assert!(
            result
                .failures
                .iter()
                .any(|failure| failure.kind == "godot_log_warning")
        );
        assert_eq!(result.data["logs"]["status"], json!("error"));
        assert_eq!(result.data["logs"]["error_count"], json!(1));
        assert_eq!(result.data["logs"]["warning_count"], json!(1));
        assert_eq!(
            result.data["outcome"],
            json!({
                "status":"fail",
                "reason":"godot_log_error",
                "failure_category":"log_health",
                "retryable":false
            })
        );
    }

    #[test]
    fn scenario_process_outcome_policy_configured_wrapper_exit_passes_with_warning() {
        let root = temp_dir("scenario-process-outcome-policy-warning");
        let scenario = json!({
            "process_outcome_policy": {"known_wrapper_exit_codes": [7]},
        });
        let result = summary_result_with_scenario(
            &root,
            7,
            &json!({"ok": true, "snapshots": [], "failures": []}),
            "",
            &scenario,
        );

        let _ = fs::remove_dir_all(&root);

        assert!(result.ok);
        assert!(result.failures.is_empty());
        assert_eq!(result.exit_code, 0);
        assert_eq!(result.message, "scenario run passed with warning");
        assert_eq!(result.data["outcome"]["status"], json!("pass_with_warning"));
        assert_eq!(result.data["outcome"]["ok"], json!(true));
        assert_eq!(result.data["outcome"]["exit_code"], json!(0));
        assert_eq!(
            result.data["process"]["warnings"][0]["type"],
            json!("godot_run_failed")
        );
        assert_eq!(
            result.data["process"]["warnings"][0]["severity"],
            json!("warning")
        );
        assert_eq!(result.data["process"]["warnings"][0]["expected"], json!(0));
        assert_eq!(result.data["process"]["warnings"][0]["actual"], json!(7));
        assert!(result.data["process"]["warnings"][0]["artifacts"]["result_path"].is_string());
    }

    #[test]
    fn scenario_process_outcome_policy_absent_keeps_strict_failure() {
        let root = temp_dir("scenario-process-outcome-policy-absent");
        let result = strict_summary_result(
            &root,
            7,
            &json!({"ok": true, "snapshots": [], "failures": []}),
            "",
        );

        let _ = fs::remove_dir_all(&root);

        assert!(!result.ok);
        assert_eq!(result.exit_code, 1);
        assert_eq!(result.failures[0].kind, "godot_run_failed");
        assert_eq!(result.data["outcome"]["status"], json!("fail"));
    }

    #[test]
    fn scenario_process_outcome_policy_unlisted_exit_keeps_strict_failure() {
        let root = temp_dir("scenario-process-outcome-policy-unlisted");
        let scenario = json!({
            "process_outcome_policy": {"known_wrapper_exit_codes": [8]},
        });
        let result = summary_result_with_scenario(
            &root,
            7,
            &json!({"ok": true, "snapshots": [], "failures": []}),
            "",
            &scenario,
        );

        let _ = fs::remove_dir_all(&root);

        assert!(!result.ok);
        assert_eq!(result.exit_code, 1);
        assert_eq!(result.failures[0].kind, "godot_run_failed");
    }

    #[test]
    fn scenario_process_outcome_policy_runtime_ok_false_keeps_strict_failure() {
        let root = temp_dir("scenario-process-outcome-policy-runtime-ok-false");
        let scenario = json!({
            "process_outcome_policy": {"known_wrapper_exit_codes": [7]},
        });
        let result = summary_result_with_scenario(
            &root,
            7,
            &json!({"ok": false, "snapshots": [], "failures": []}),
            "",
            &scenario,
        );

        let _ = fs::remove_dir_all(&root);

        assert!(!result.ok);
        assert_eq!(result.exit_code, 1);
        assert_eq!(result.failures[0].kind, "scenario_result_invalid");
    }

    #[test]
    fn scenario_process_outcome_policy_runtime_failures_keep_strict_failure() {
        let root = temp_dir("scenario-process-outcome-policy-runtime-failures");
        let scenario = json!({
            "process_outcome_policy": {"known_wrapper_exit_codes": [7]},
        });
        let result = summary_result_with_scenario(
            &root,
            7,
            &json!({
                "ok": false,
                "snapshots": [],
                "failures": [{"type": "assertion_failed", "message": "expected true"}]
            }),
            "",
            &scenario,
        );

        let _ = fs::remove_dir_all(&root);

        assert!(!result.ok);
        assert_eq!(result.exit_code, 1);
        assert_eq!(result.failures[0].kind, "assertion_failed");
    }

    #[test]
    fn scenario_process_outcome_policy_log_error_keeps_strict_failure() {
        let root = temp_dir("scenario-process-outcome-policy-log-error");
        let scenario = json!({
            "process_outcome_policy": {"known_wrapper_exit_codes": [7]},
        });
        let result = summary_result_with_scenario(
            &root,
            7,
            &json!({"ok": true, "snapshots": [], "failures": []}),
            "ERROR: unexpected error\n",
            &scenario,
        );

        let _ = fs::remove_dir_all(&root);

        assert!(!result.ok);
        assert_eq!(result.exit_code, 1);
        assert!(
            result
                .failures
                .iter()
                .any(|failure| failure.kind == "godot_log_error")
        );
        assert_eq!(result.data["outcome"]["status"], json!("fail"));
    }

    #[test]
    fn scenario_process_outcome_policy_malformed_shape_is_validation_failure() {
        let scenario = json!({
            "name": "bad_policy",
            "scene": "res://main.tscn",
            "steps": [],
            "process_outcome_policy": {"known_wrapper_exit_codes": [0, "7"]},
        });

        let err = validate_scenario_document(&scenario, Path::new("scenario.json")).unwrap_err();

        assert_eq!(err.failure.kind, "scenario_validation_failed");
        assert_eq!(
            err.failure.expected,
            Some(json!("non-empty list of positive integers"))
        );
    }

    #[test]
    fn scenario_result_godot_run_failed_remains_distinct() {
        let root = temp_dir("scenario-result-godot-run-failed");
        let result_path = root.join("result.json");
        let logs_path = root.join("logs.txt");
        write(&logs_path, "");
        let artifacts = json!({
            "run_root": root,
            "result_path": result_path,
            "logs_path": logs_path,
        });
        let mut base_data = serde_json::Map::new();
        let process_result = GodotRunResult {
            command: vec!["godot".to_string()],
            returncode: 11,
            stdout: String::new(),
            stderr: String::new(),
            display: godot_qa_process::resolve_project_display_launch_metadata(&root, &[], true),
            pid: None,
            process_group: None,
            cleanup: None,
            infrastructure_failure_kind: None,
            timed_out: false,
            timeout_ms: None,
            output_truncation: None,
        };
        let runtime_result = json!({"ok": true, "snapshots": [], "failures": []});

        let result = validate_runtime_command_result(
            "scenario run",
            &json!({}),
            &mut base_data,
            &result_path,
            &artifacts,
            &process_result,
            &runtime_result,
            &logs_path,
            &[],
        );

        let _ = fs::remove_dir_all(&root);

        assert_eq!(result.failures[0].kind, "godot_run_failed");
    }

    #[test]
    fn runtime_audio_valid_artifacts_surface_and_write_analysis() {
        let root = temp_dir("runtime-audio-valid-artifacts");
        let wav_path = write_runtime_audio_wav(&root, "music");
        let runtime_result = json!({
            "ok": true,
            "snapshots": [],
            "failures": [],
            "audio_artifacts": {
                "music": {"wav": wav_path}
            }
        });

        let result = runtime_audio_result(&root, &runtime_result);
        let persisted: Value =
            serde_json::from_str(&fs::read_to_string(root.join("result.json")).unwrap()).unwrap();
        let analysis_path = root.join("audio/music.analysis.json");

        assert!(result.ok);
        assert_path_eq(
            result.data["audio_artifacts"]["music"]["analysis"]
                .as_str()
                .unwrap(),
            &analysis_path,
        );
        assert_eq!(
            result.data["audio_analysis"]["music"]["sample_rate"],
            json!(8000)
        );
        assert!(analysis_path.is_file());
        assert_path_eq(
            persisted["data"]["audio_artifacts"]["music"]["analysis"]
                .as_str()
                .unwrap(),
            &analysis_path,
        );
        assert_eq!(
            persisted["data"]["audio_analysis"]["music"]["channels"],
            json!(1)
        );

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn runtime_audio_invalid_artifacts_shape_fails() {
        let root = temp_dir("runtime-audio-invalid-shape");
        let runtime_result = json!({
            "ok": true,
            "snapshots": [],
            "failures": [],
            "audio_artifacts": []
        });

        let result = runtime_audio_result(&root, &runtime_result);

        assert!(!result.ok);
        assert_eq!(result.failures[0].kind, "scenario_result_invalid");
        assert!(
            result.failures[0]
                .message
                .as_deref()
                .unwrap_or_default()
                .contains("field 'audio_artifacts' must be an object")
        );

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn runtime_audio_missing_wav_file_fails() {
        let root = temp_dir("runtime-audio-missing-wav");
        let missing = root.join("audio/missing.wav");
        let runtime_result = json!({
            "ok": true,
            "snapshots": [],
            "failures": [],
            "audio_artifacts": {
                "missing": {"wav": missing}
            }
        });

        let result = runtime_audio_result(&root, &runtime_result);

        assert!(!result.ok);
        assert_eq!(result.failures[0].kind, "scenario_result_invalid");
        assert!(
            result.failures[0]
                .message
                .as_deref()
                .unwrap_or_default()
                .contains("wav file was not found")
        );

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn runtime_audio_path_outside_audio_dir_fails() {
        let root = temp_dir("runtime-audio-outside-dir");
        let outside = root.join("outside.wav");
        fs::write(&outside, wav_pcm16(8_000, 1, &[0.0, 0.1])).unwrap();
        let runtime_result = json!({
            "ok": true,
            "snapshots": [],
            "failures": [],
            "audio_artifacts": {
                "escape": {"wav": outside}
            }
        });

        let result = runtime_audio_result(&root, &runtime_result);

        assert!(!result.ok);
        assert_eq!(result.failures[0].kind, "scenario_result_invalid");
        assert!(
            result.failures[0]
                .message
                .as_deref()
                .unwrap_or_default()
                .contains("must stay under artifacts.audio_dir")
        );

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn runtime_audio_invalid_wav_bytes_fail() {
        let root = temp_dir("runtime-audio-invalid-wav");
        let wav_path = root.join("audio/bad.wav");
        fs::create_dir_all(wav_path.parent().unwrap()).unwrap();
        fs::write(&wav_path, b"not a wave").unwrap();
        let runtime_result = json!({
            "ok": true,
            "snapshots": [],
            "failures": [],
            "audio_artifacts": {
                "bad": {"wav": wav_path}
            }
        });

        let result = runtime_audio_result(&root, &runtime_result);

        assert!(!result.ok);
        assert_eq!(result.failures[0].kind, "scenario_result_invalid");
        assert_eq!(result.failures[0].actual, Some(json!("audio_invalid_riff")));

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn runtime_audio_absent_preserves_non_audio_output_shape() {
        let root = temp_dir("runtime-audio-absent-shape");
        let result =
            runtime_audio_result(&root, &json!({"ok": true, "snapshots": [], "failures": []}));

        assert!(result.ok);
        assert!(result.data.get("audio_artifacts").is_none());
        assert!(result.data.get("audio_analysis").is_none());

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn screenshot_quality_fails_low_information_by_default() {
        let root = temp_dir("screenshot-quality-default-failure");
        let screenshot_path = root.join("snapshots/after/screenshot.png");
        if let Some(parent) = screenshot_path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        image::ImageBuffer::<image::Rgba<u8>, Vec<u8>>::from_vec(64, 64, vec![128u8; 64 * 64 * 4])
            .unwrap()
            .save(&screenshot_path)
            .unwrap();
        let artifacts = json!({
            "run_root": root,
            "snapshots_dir": root.join("snapshots"),
        });
        let scenario = json!({
            "snapshots": {"save_screenshot": true},
            "assert": [{
                "visible_pixels": {
                    "target": "qa:important_actor",
                    "snapshot": "after",
                    "min_ratio": 0.25
                }
            }],
        });
        let result = godot_qa_core::results::CommandResult {
            ok: true,
            command: "scenario run".to_string(),
            message: "scenario run passed".to_string(),
            data: serde_json::Map::from_iter([
                ("artifacts".to_string(), artifacts.clone()),
                ("snapshots".to_string(), json!(["after"])),
            ]),
            failures: Vec::new(),
            exit_code: 0,
        };

        let result = apply_screenshot_quality("scenario run", &scenario, result);

        assert!(!result.ok);
        assert_eq!(result.exit_code, 1);
        assert_eq!(result.failures[0].kind, "screenshot_low_information");
        assert_eq!(result.data["screenshot_quality"]["after"]["width"], 64);
        assert_eq!(result.data["screenshot_quality"]["after"]["height"], 64);
        assert_eq!(
            result.data["screenshot_quality"]["after"]["pixel_count"],
            4096
        );
        assert_eq!(
            result.data["screenshot_quality"]["after"]["dominant_colors"][0]["rgba"],
            json!([128, 128, 128, 128])
        );
        assert_eq!(
            result.data["screenshot_quality"]["after"]["non_background_pixel_count"],
            0
        );
        assert_eq!(
            result.data["screenshot_quality"]["after"]["low_information"],
            true
        );
        assert_eq!(
            result.data["screenshot_quality"]["after"]["low_information_reasons"][0],
            json!("small_dimensions")
        );
        assert!(result.data.get("warnings").is_none());
        let failure = &result.failures[0];
        assert_path_eq(
            failure
                .artifacts
                .as_ref()
                .and_then(|v| v.get("screenshot"))
                .and_then(|v| v.as_str())
                .unwrap(),
            &screenshot_path,
        );
    }

    #[test]
    fn screenshot_quality_compat_flag_still_fails_low_information() {
        let root = temp_dir("screenshot-quality-failure");
        let screenshot_path = root.join("snapshots/after/screenshot.png");
        if let Some(parent) = screenshot_path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        image::ImageBuffer::<image::Rgba<u8>, Vec<u8>>::from_vec(64, 64, vec![128u8; 64 * 64 * 4])
            .unwrap()
            .save(&screenshot_path)
            .unwrap();
        let scenario = json!({
            "snapshots": {
                "save_screenshot": true,
                "fail_on_low_information": true
            },
        });
        let result = godot_qa_core::results::CommandResult {
            ok: true,
            command: "scenario run".to_string(),
            message: "scenario run passed".to_string(),
            data: serde_json::Map::from_iter([
                (
                    "artifacts".to_string(),
                    json!({
                        "run_root": root,
                        "snapshots_dir": root.join("snapshots"),
                    }),
                ),
                ("snapshots".to_string(), json!(["after"])),
            ]),
            failures: Vec::new(),
            exit_code: 0,
        };

        let result = apply_screenshot_quality("scenario run", &scenario, result);

        assert!(!result.ok);
        assert_eq!(result.exit_code, 1);
        assert_eq!(result.failures[0].kind, "screenshot_low_information");
        assert_path_eq(result.failures[0].path.as_deref().unwrap(), &screenshot_path);
    }

    #[test]
    fn visual_risk_warning_mode_preserves_success_and_adds_warnings() {
        let root = temp_dir("visual-risk-warning-run");
        let screenshot_path = root.join("snapshots/after/screenshot.png");
        save_visual_risk_png(&screenshot_path);
        let scenario = json!({
            "name": "visual_risk_demo",
            "snapshots": {"save_screenshot": true},
        });
        let result = visual_risk_base_result(&root);

        let result = apply_visual_risk_analysis(
            "scenario run",
            &scenario,
            result,
            RuntimeExecutionOptions {
                visual_risk: VisualRiskExecutionOptions {
                    enabled: true,
                    strict: false,
                },
                ..RuntimeExecutionOptions::default()
            },
        );

        assert!(result.ok);
        assert_eq!(result.exit_code, 0);
        assert!(result.failures.is_empty());
        assert_eq!(result.data["visual_risk"]["strict"], json!(false));
        assert_eq!(
            result.data["warnings"][0]["rule_id"],
            json!("visual-risk.low_visual_diversity")
        );
        assert_eq!(
            result.data["warnings"][0]["actual"]["unique_color_count"],
            json!(2)
        );
    }

    #[test]
    fn visual_risk_strict_mode_converts_findings_to_failures() {
        let root = temp_dir("visual-risk-strict-run");
        let screenshot_path = root.join("snapshots/after/screenshot.png");
        save_visual_risk_png(&screenshot_path);
        let scenario = json!({
            "name": "visual_risk_demo",
            "snapshots": {"save_screenshot": true},
        });
        let result = visual_risk_base_result(&root);

        let result = apply_visual_risk_analysis(
            "scenario run",
            &scenario,
            result,
            RuntimeExecutionOptions {
                visual_risk: VisualRiskExecutionOptions {
                    enabled: true,
                    strict: true,
                },
                ..RuntimeExecutionOptions::default()
            },
        );

        assert!(!result.ok);
        assert_eq!(result.exit_code, 1);
        assert_eq!(result.data["visual_risk"]["strict"], json!(true));
        assert_eq!(result.failures[0].kind, "visual-risk.low_visual_diversity");
        assert_eq!(
            result.failures[0].details["rule_id"],
            json!("visual-risk.low_visual_diversity")
        );
        assert_eq!(
            result.failures[0].actual.as_ref().unwrap()["unique_color_count"],
            json!(2)
        );
    }

    fn visual_risk_base_result(root: &Path) -> godot_qa_core::results::CommandResult {
        godot_qa_core::results::CommandResult {
            ok: true,
            command: "scenario run".to_string(),
            message: "scenario run passed".to_string(),
            data: serde_json::Map::from_iter([
                (
                    "artifacts".to_string(),
                    json!({
                        "run_root": root,
                        "snapshots_dir": root.join("snapshots"),
                    }),
                ),
                ("snapshots".to_string(), json!(["after"])),
            ]),
            failures: Vec::new(),
            exit_code: 0,
        }
    }

    fn save_visual_risk_png(path: &Path) {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        let image = image::ImageBuffer::from_fn(160, 160, |x, y| {
            if x < 148 || y % 2 == 0 {
                image::Rgba([40u8, 40, 48, 255])
            } else {
                image::Rgba([80u8, 80, 88, 255])
            }
        });
        image.save(path).unwrap();
    }

    #[test]
    fn temporal_artifacts_are_additive_and_preserve_existing_keys() {
        let root = temp_dir("temporal-artifacts-additive");
        let artifacts = crate::artifacts::prepare_run_artifacts(
            &root,
            &json!({"name":"temporal_contract","scene":"res://main.tscn"}),
            &root,
            &root.join("qa/scenarios/temporal_contract.json"),
            "scenario run temporal_contract",
            None,
            &[],
            &[],
            Some("run-001"),
            Some("0Z"),
        )
        .unwrap();
        let map = artifacts.to_json().as_object().cloned().unwrap();
        for key in [
            "run_root",
            "meta_path",
            "request_path",
            "result_path",
            "logs_path",
            "snapshots_dir",
            "temporal_dir",
        ] {
            assert!(
                map.contains_key(key),
                "missing expected artifact key: {key}"
            );
        }
        for legacy_key in ["root", "meta", "request", "result", "logs"] {
            assert!(
                !map.contains_key(legacy_key),
                "legacy artifact key should be absent: {legacy_key}"
            );
        }
        assert!(Path::new(map["temporal_dir"].as_str().unwrap()).is_dir());
    }

    #[test]
    fn temporal_failures_allow_structured_expected_actual() {
        let artifacts = json!({"run_root":"/tmp/run","result_path":"/tmp/run/result.json"})
            .as_object()
            .cloned()
            .unwrap();
        let failures = validate_runtime_failures(
            "scenario run",
            Path::new("/tmp/run/result.json"),
            Some(&json!([{
                "type": "assertion_failed",
                "message": "max displacement exceeded",
                "expected": {"max": 32.0, "window_frames": 12},
                "actual": {"max": 81.25, "sample_frame": 17},
                "target": "qa:demo_actor",
                "severity": "error",
                "artifacts": {"temporal_trace": "/tmp/run/temporal/trace.jsonl"}
            }])),
            false,
            &artifacts,
        )
        .unwrap();
        assert_eq!(failures.len(), 1);
        assert_eq!(
            failures[0].expected,
            Some(json!({"max": 32.0, "window_frames": 12}))
        );
        assert_eq!(
            failures[0].actual,
            Some(json!({"max": 81.25, "sample_frame": 17}))
        );
    }

    #[test]
    fn temporal_observe_until_failure_contract() {
        let artifacts = json!({"run_root":"/tmp/run","result_path":"/tmp/run/result.json"})
            .as_object()
            .cloned()
            .unwrap();
        let failures = validate_runtime_failures(
            "scenario run",
            Path::new("/tmp/run/result.json"),
            Some(&json!([{
                "type": "observe_until_mismatch",
                "message": "condition did not become true within the bounded window",
                "target": "qa:demo_actor",
                "resolved": "Main/DemoActor",
                "condition": {"type":"property","property":"state","operator":"equals","expected":"pending"},
                "window": {"frames":4,"sample_every_frames":1,"cadence":{"sample_every_frames":1}},
                "sample_every_frames": 1,
                "observed_samples": [
                    {"sample":1,"frame":10,"time_msec":100,"target":"qa:demo_actor","resolved":"Main/DemoActor","matched":false,"actual":{"path":"state","operator":"equals","expected":"pending","actual":"idle","present":true}},
                    {"sample":2,"frame":11,"time_msec":116,"target":"qa:demo_actor","resolved":"Main/DemoActor","matched":false,"actual":{"path":"state","operator":"equals","expected":"pending","actual":"idle","present":true}}
                ],
                "first_late_sample": {"sample":3,"frame":12,"time_msec":132,"target":"qa:demo_actor","resolved":"Main/DemoActor","matched":true,"actual":{"path":"state","operator":"equals","expected":"pending","actual":"pending","present":true}},
                "why_no_match": "matched_after_window",
                "expected": {"property":"state","operator":"equals","value":"pending"},
                "actual": {"path":"state","operator":"equals","expected":"pending","actual":"idle","present":true},
                "artifacts": {
                    "temporal_evidence": "/tmp/run/temporal/observe_until_qa_demo_actor.json",
                    "temporal_dir": "/tmp/run/temporal"
                }
            }])),
            false,
            &artifacts,
        )
        .unwrap();
        let result = godot_qa_core::results::CommandResult {
            ok: false,
            command: "scenario run".to_string(),
            message: "scenario run failed".to_string(),
            data: serde_json::Map::new(),
            failures,
            exit_code: 1,
        };

        let body = serde_json::to_value(&result).unwrap();
        let failure = &body["failures"][0];
        assert_eq!(failure["type"], json!("observe_until_mismatch"));
        assert_eq!(failure["target"], json!("qa:demo_actor"));
        assert_eq!(
            failure["expected"],
            json!({"property":"state","operator":"equals","value":"pending"})
        );
        assert_eq!(
            failure["actual"],
            json!({"path":"state","operator":"equals","expected":"pending","actual":"idle","present":true})
        );
        assert_eq!(failure["condition"]["property"], json!("state"));
        assert_eq!(failure["window"]["frames"], json!(4));
        assert_eq!(failure["sample_every_frames"], json!(1));
        assert_eq!(failure["observed_samples"].as_array().unwrap().len(), 2);
        assert_eq!(failure["first_late_sample"]["matched"], json!(true));
        assert_eq!(failure["why_no_match"], json!("matched_after_window"));
        assert_eq!(
            failure["artifacts"]["temporal_evidence"],
            json!("/tmp/run/temporal/observe_until_qa_demo_actor.json")
        );
        assert_eq!(
            failure["artifacts"]["temporal_dir"],
            json!("/tmp/run/temporal")
        );
    }

    #[test]
    fn proximity_runtime_failure_fields_survive_public_json() {
        let artifacts = json!({"run_root":"/tmp/run","result_path":"/tmp/run/result.json"})
            .as_object()
            .cloned()
            .unwrap();
        let failures = validate_runtime_failures(
            "scenario run",
            Path::new("/tmp/run/result.json"),
            Some(&json!([{
                "type": "distance_too_far",
                "message": "distance exceeded max threshold",
                "a": "qa:actor",
                "b": "qa:target",
                "a_resolved": "Main/QAProximityActor",
                "b_resolved": "Main/QAProximityTarget",
                "a_position": {"x": 120.0, "y": 520.0},
                "b_position": {"x": 280.0, "y": 520.0},
                "coordinate_space": "world_2d",
                "distance": 160.0,
                "threshold": {"max": 96.0},
                "mismatch_type": "distance_too_far",
                "expected": {"max": 96.0},
                "actual": {"distance": 160.0}
            }])),
            false,
            &artifacts,
        )
        .unwrap();
        let result = godot_qa_core::results::CommandResult {
            ok: false,
            command: "scenario run".to_string(),
            message: "scenario run failed".to_string(),
            data: serde_json::Map::new(),
            failures,
            exit_code: 1,
        };

        let body = serde_json::to_value(&result).unwrap();
        let failure = &body["failures"][0];
        assert_eq!(failure["type"], json!("distance_too_far"));
        assert_eq!(failure["a"], json!("qa:actor"));
        assert_eq!(failure["b"], json!("qa:target"));
        assert_eq!(failure["a_position"], json!({"x": 120.0, "y": 520.0}));
        assert_eq!(failure["b_position"], json!({"x": 280.0, "y": 520.0}));
        assert_eq!(failure["coordinate_space"], json!("world_2d"));
        assert_eq!(failure["distance"], json!(160.0));
        assert_eq!(failure["threshold"], json!({"max": 96.0}));
        assert_eq!(failure["mismatch_type"], json!("distance_too_far"));
        assert!(failure.get("details").is_none());
    }

    #[test]
    fn temporal_failure_artifacts_reject_non_string_values() {
        let artifacts = json!({"run_root":"/tmp/run","result_path":"/tmp/run/result.json"})
            .as_object()
            .cloned()
            .unwrap();
        let err = validate_runtime_failures(
            "scenario run",
            Path::new("/tmp/run/result.json"),
            Some(&json!([{
                "type": "assertion_failed",
                "message": "bad artifacts",
                "artifacts": {"temporal_trace": {"path":"/tmp/run/temporal/trace.jsonl"}}
            }])),
            false,
            &artifacts,
        )
        .unwrap_err();
        assert_eq!(err.kind, "scenario_result_invalid");
        assert!(
            err.message
                .as_deref()
                .unwrap_or_default()
                .contains("failure field 'artifacts' must contain only string values")
        );
    }

    #[test]
    fn runtime_payload_keeps_temporal_assertions_while_stripping_snapshot_layout_only() {
        let payload = runtime_scenario_payload(&json!({
            "name":"temporal_payload",
            "scene":"res://main.tscn",
            "viewports":[],
            "assert":[
                {"screenshot_match":{"snapshot":"after","baseline":"qa/baselines/a.png"}},
                {"layout_audit":{"snapshot":"after"}},
                {"max_delta_position":{"target":"qa:demo_actor","window_frames":12,"max":32}, "temporal": true}
            ]
        }));
        let assertions = payload["assert"].as_array().cloned().unwrap();
        assert_eq!(assertions.len(), 1);
        assert!(assertions[0].get("max_delta_position").is_some());
    }

    fn write_layout_snapshot(snapshot_root: &Path, ui: &Value, focus_owner: &str) {
        write(&snapshot_root.join("tree.json"), "{}");
        write(
            &snapshot_root.join("ui.json"),
            &serde_json::to_string(ui).unwrap(),
        );
        write(
            &snapshot_root.join("focus.json"),
            &serde_json::to_string(&json!({ "focus_owner": focus_owner })).unwrap(),
        );
        write(&snapshot_root.join("errors.json"), "[]");
    }

    #[test]
    fn apply_layout_audit_attaches_failure_for_bad_layout() {
        let root = temp_dir("apply-layout-audit-bad");
        let artifacts = test_run_artifacts(&root);
        write_layout_snapshot(
            &artifacts.snapshots_dir.join("audit"),
            // A visible interactive Button pushed off the right of the viewport.
            &json!({
                "viewport": [1280, 720],
                "controls": [{
                    "type": "Button",
                    "path": "/root/Menu/OffscreenButton",
                    "visible": true,
                    "focus_mode": "all",
                    "global_rect": [2000.0, 10.0, 200.0, 48.0],
                }],
            }),
            "/root/Menu/OffscreenButton",
        );

        let scenario = json!({
            "name": "layout_single", "scene": "res://x.tscn",
            "viewports": [[1280, 720]],
            "assert": [{"layout_audit": "strict"}],
        });
        let mut data = serde_json::Map::new();
        data.insert("snapshots".to_string(), json!(["audit"]));
        let result = CommandResult {
            ok: true,
            command: "scenario run".to_string(),
            message: "scenario run passed".to_string(),
            data,
            failures: Vec::new(),
            exit_code: 0,
        };
        let audited = super::apply_layout_audit("scenario run", &scenario, &artifacts, result);
        assert!(!audited.ok);
        assert_eq!(audited.exit_code, 1);
        assert!(
            audited
                .failures
                .iter()
                .any(|failure| failure.kind == "outside_viewport")
        );
    }

    #[test]
    fn apply_layout_audit_passes_clean_snapshot() {
        let root = temp_dir("apply-layout-audit-clean");
        let artifacts = test_run_artifacts(&root);
        write_layout_snapshot(
            &artifacts.snapshots_dir.join("audit"),
            &json!({
                "viewport": [1280, 720],
                "controls": [{
                    "type": "Button",
                    "path": "/root/Menu/StartButton",
                    "visible": true,
                    "focus_mode": "all",
                    "global_rect": [10.0, 10.0, 200.0, 48.0],
                }],
            }),
            "/root/Menu/StartButton",
        );

        let scenario = json!({
            "name": "layout_single", "scene": "res://x.tscn",
            "viewports": [[1280, 720]],
            "assert": [{"layout_audit": "strict"}],
        });
        let mut data = serde_json::Map::new();
        data.insert("snapshots".to_string(), json!(["audit"]));
        let result = CommandResult {
            ok: true,
            command: "scenario run".to_string(),
            message: "scenario run passed".to_string(),
            data,
            failures: Vec::new(),
            exit_code: 0,
        };
        let audited = super::apply_layout_audit("scenario run", &scenario, &artifacts, result);
        assert!(audited.ok);
        assert!(audited.failures.is_empty());
    }

    #[test]
    fn apply_layout_audit_missing_snapshot_when_run_recorded_none() {
        let root = temp_dir("apply-layout-audit-missing");
        let artifacts = test_run_artifacts(&root);
        let scenario = json!({
            "name": "layout_single", "scene": "res://x.tscn",
            "assert": [{"layout_audit": "strict"}],
        });
        let mut data = serde_json::Map::new();
        data.insert("snapshots".to_string(), json!([]));
        let result = CommandResult {
            ok: true,
            command: "scenario run".to_string(),
            message: "scenario run passed".to_string(),
            data,
            failures: Vec::new(),
            exit_code: 0,
        };
        let audited = super::apply_layout_audit("scenario run", &scenario, &artifacts, result);
        assert!(!audited.ok);
        assert_eq!(audited.failures.len(), 1);
        assert_eq!(audited.failures[0].kind, "layout_audit_missing_snapshot");
    }

    #[test]
    fn run_viewport_matrix_with_no_viewports_fails_loudly() {
        // Defense in depth: an empty viewport list would otherwise vacuously pass
        // (`runs.iter().all(..)` over zero runs is true). The guard fails instead.
        let root = temp_dir("viewport-matrix-empty");
        let config = test_config(&root);
        let artifacts = test_run_artifacts(&root);
        let scenario = json!({"name": "empty_matrix", "scene": "res://x.tscn"});
        let scenario_path = root.join("qa/scenarios/empty_matrix.yaml");
        let result = super::run_viewport_matrix(
            "scenario run",
            "scenario run failed",
            &config,
            &scenario,
            &scenario_path,
            &artifacts,
            &[],
            Some("strict"),
            "godot-qa scenario run empty_matrix",
            None,
            None,
        );
        assert!(!result.ok);
        assert_eq!(result.exit_code, 1);
        assert_eq!(result.failures.len(), 1);
        assert_eq!(result.failures[0].kind, "viewport_matrix_empty");
        assert_eq!(result.data.get("runs"), Some(&json!([])));
    }

    #[test]
    fn apply_layout_audit_is_noop_without_layout_assertion() {
        let root = temp_dir("apply-layout-audit-noop");
        let artifacts = test_run_artifacts(&root);
        let scenario = json!({
            "name": "plain", "scene": "res://x.tscn",
            "assert": [{"no_errors": true}],
        });
        let mut data = serde_json::Map::new();
        data.insert("snapshots".to_string(), json!([]));
        let result = CommandResult {
            ok: true,
            command: "scenario run".to_string(),
            message: "scenario run passed".to_string(),
            data,
            failures: Vec::new(),
            exit_code: 0,
        };
        let audited = super::apply_layout_audit("scenario run", &scenario, &artifacts, result);
        assert!(audited.ok);
        assert!(audited.failures.is_empty());
    }

    #[test]
    fn temporal_sampling_runtime_payload_contract() {
        let payload = runtime_scenario_payload(&json!({
            "name":"temporal_sampling_payload",
            "scene":"res://main.tscn",
            "determinism":{
                "temporal_sampling":{
                    "mode":"fixed_delta",
                    "fixed_delta_seconds":0.016666667
                }
            },
            "assert":[
                {"screenshot_match":{"snapshot":"after","baseline":"qa/baselines/a.png"}},
                {"max_delta_position":{"target":"qa:demo_actor","window_frames":12,"sample_every_frames":2,"max":32}, "temporal": true},
                {"never":{"target":"qa:demo_actor","property":"state","forbidden":["done"],"window_seconds":0.25,"sample_every_frames":1}, "temporal": true}
            ]
        }));

        assert_eq!(
            payload["determinism"]["temporal_sampling"],
            json!({"mode":"fixed_delta","fixed_delta_seconds":0.016666667})
        );
        let assertions = payload["assert"].as_array().cloned().unwrap();
        assert_eq!(assertions.len(), 2);
        assert!(assertions[0].get("max_delta_position").is_some());
        assert!(assertions[1].get("never").is_some());
    }

    #[test]
    fn runtime_payload_strips_screenshot_match_but_preserves_visual_assertions() {
        let payload = runtime_scenario_payload(&json!({
            "name":"visual_payload",
            "scene":"res://main.tscn",
            "assert":[
                {"screenshot_match":{"snapshot":"after","baseline":"qa/baselines/a.png"}},
                {"visible_pixels":{"target":"qa:important_actor","snapshot":"after","min_ratio":0.25}},
                {"not_occluded":{"target":"qa:important_actor","occluder":"qa:overlay","snapshot":"after"}},
                {"renders_above":{"target":"qa:important_actor","other":"qa:background","snapshot":"after"}},
                {"renders_below":{"target":"qa:background","other":"qa:important_actor","snapshot":"after"}}
            ]
        }));
        let assertions = payload["assert"].as_array().cloned().unwrap();
        assert_eq!(assertions.len(), 4);
        assert!(assertions[0].get("visible_pixels").is_some());
        assert!(assertions[1].get("not_occluded").is_some());
        assert!(assertions[2].get("renders_above").is_some());
        assert!(assertions[3].get("renders_below").is_some());
        assert!(
            assertions
                .iter()
                .all(|assertion| assertion.get("screenshot_match").is_none())
        );
    }

    #[test]
    fn runtime_payload_and_viewport_variants_preserve_snapshot_filters() {
        let scenario = json!({
            "name":"filtered_payload",
            "scene":"res://main.tscn",
            "snapshots":{
                "save_tree": true,
                "target":"qa:start_button",
                "max_depth":1,
                "qa_only":true,
                "include_metadata":false,
                "metadata_keys":["qa_id"]
            },
            "assert":[{"layout_audit":{"snapshot":"after"}}]
        });
        let payload = runtime_scenario_payload(&scenario);
        assert_eq!(payload["snapshots"]["target"], "qa:start_button");
        assert_eq!(payload["snapshots"]["metadata_keys"], json!(["qa_id"]));

        let variant = variant_scenario(&scenario, &[800, 600]);
        assert_eq!(variant["snapshots"]["target"], "qa:start_button");
        assert_eq!(variant["viewport"], json!({"width":800,"height":600}));
    }

    #[test]
    fn surface_runtime_payload_preserves_surface_assertions() {
        let payload = runtime_scenario_payload(&json!({
            "name":"surface_payload",
            "scene":"res://main.tscn",
            "assert":[
                {"surface_probe":{"target":"qa:actor","provider":"qa:provider","points":[{"point":[0,0],"expected_surface":"walkable"}]}},
                {"surface_matches":{"target":"qa:actor","surface":"walkable"}},
                {"layout_audit":{"snapshot":"after"}}
            ]
        }));
        let assertions = payload["assert"].as_array().cloned().unwrap();
        assert_eq!(assertions.len(), 2);
        assert!(assertions[0].get("surface_probe").is_some());
        assert!(assertions[1].get("surface_matches").is_some());
    }

    #[test]
    fn surface_failure_fields_survive_public_json() {
        let artifacts = json!({"run_root":"/tmp/run","result_path":"/tmp/run/result.json"})
            .as_object()
            .cloned()
            .unwrap();
        let failures = validate_runtime_failures(
            "scenario run",
            Path::new("/tmp/run/result.json"),
            Some(&json!([{
                "type":"surface_mismatch",
                "message":"surface mismatch at sampled points",
                "target":"qa:actor",
                "coordinate_space":"world_2d",
                "provider":"qa:surface_provider",
                "provider_method":"probe_surface",
                "point":{"x":1.0,"y":2.0},
                "expected_surface":"walkable",
                "actual_surface":"blocked",
                "mismatch_type":"label_mismatch",
                "probe_evidence":{"sample_count":2},
                "points":[
                    {"point":{"x":1.0,"y":2.0},"expected_surface":"walkable","actual_surface":"blocked"},
                    {"point":{"x":3.0,"y":4.0},"expected_surface":"walkable","actual_surface":"walkable"}
                ]
            }])),
            false,
            &artifacts,
        )
        .unwrap();
        let failure = serde_json::to_value(&failures[0]).unwrap();
        assert_eq!(failure["coordinate_space"], json!("world_2d"));
        assert_eq!(failure["provider"], json!("qa:surface_provider"));
        assert_eq!(failure["provider_method"], json!("probe_surface"));
        assert_eq!(failure["point"]["x"], json!(1.0));
        assert_eq!(failure["point"]["y"], json!(2.0));
        assert_eq!(failure["expected_surface"], json!("walkable"));
        assert_eq!(failure["actual_surface"], json!("blocked"));
        assert_eq!(failure["mismatch_type"], json!("label_mismatch"));
        assert_eq!(failure["probe_evidence"], json!({"sample_count":2}));
        assert!(failure["points"].is_array());
        assert!(failure.get("details").is_none());
    }

    #[test]
    fn visual_diagnostic_artifacts_are_under_run_artifacts_not_baselines() {
        let root = temp_dir("visual-artifact-path");
        let artifact_root = root.join(".qa/artifacts");
        let artifacts = crate::artifacts::prepare_run_artifacts(
            &artifact_root,
            &json!({"name":"visual_occlusion_demo","scene":"res://main.tscn"}),
            &root,
            &root.join("qa/scenarios/visual_occlusion_demo.json"),
            "scenario run visual_occlusion_demo",
            None,
            &[],
            &[],
            Some("run-001"),
            Some("0Z"),
        )
        .unwrap();
        let map = artifacts.to_json().as_object().cloned().unwrap();
        let run_root = map.get("run_root").and_then(Value::as_str).unwrap();
        let run_root = Path::new(run_root);
        let visual_dir = run_root.join("visual");
        assert!(run_root.starts_with(&artifact_root));
        assert!(!run_root.starts_with(root.join("qa/baselines")));
        assert!(visual_dir.starts_with(&artifact_root));
        assert!(!visual_dir.starts_with(root.join("qa/baselines")));
    }
