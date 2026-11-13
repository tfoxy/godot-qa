use super::*;

#[test]
fn check_tests_profile_request_contract() {
    let project_root = project_with_manifest(
        "profile-request-contract",
        r#"{
                "version": 1,
                "tests": [
                    {
                        "id": "sample_script_test",
                        "kind": "script",
                        "target": "res://tests/test_main.gd"
                    },
                    {
                        "id": "sample_scene_test",
                        "kind": "scene",
                        "target": "res://scenes/main.tscn"
                    }
                ]
            }"#,
    );
    let config = test_config(&project_root);
    let (_, manifest, failure) = load_test_manifest(&config, "check");
    assert!(failure.is_none(), "{failure:?}");
    let manifest = manifest.unwrap();
    let check_artifacts =
        prepare_check_artifacts(&config.artifacts_dir, &config.project_root, "check").unwrap();
    let script_artifacts = prepare_test_artifacts(&check_artifacts, &manifest.entries[0]).unwrap();
    let scene_artifacts = prepare_test_artifacts(&check_artifacts, &manifest.entries[1]).unwrap();
    let profiled_options = CheckTestRunOptions {
        timeout_override: None,
        profile_enabled: true,
    };

    let unprofiled_request = test_check_request(
        &manifest.entries[0],
        &script_artifacts,
        "run-1",
        "check",
        CheckTestRunOptions::default(),
    );
    assert!(unprofiled_request.get("profiling").is_none());

    let profiled_request = test_check_request(
        &manifest.entries[0],
        &script_artifacts,
        "run-1",
        "check",
        profiled_options,
    );
    assert_eq!(
        profiled_request["profiling"],
        json!({"enabled": true, "max_frame_samples": super::super::CHECK_PROFILE_MAX_FRAME_SAMPLES})
    );

    let scene_request = test_check_request(
        &manifest.entries[1],
        &scene_artifacts,
        "run-1",
        "check",
        profiled_options,
    );
    let batch_profiled = super::super::execution::batch_check_request(
        &[
            (
                &manifest.entries[0],
                script_artifacts.clone(),
                profiled_request,
            ),
            (&manifest.entries[1], scene_artifacts, scene_request),
        ],
        &check_artifacts,
        "check",
        profiled_options,
    );
    assert_eq!(
        batch_profiled["check"]["tests"][0]["profiling"],
        json!({"enabled": true, "max_frame_samples": super::super::CHECK_PROFILE_MAX_FRAME_SAMPLES})
    );
    assert_eq!(
        batch_profiled["check"]["tests"][1]["profiling"],
        json!({"enabled": true, "max_frame_samples": super::super::CHECK_PROFILE_MAX_FRAME_SAMPLES})
    );

    let batch_unprofiled = super::super::execution::batch_check_request(
        &[(&manifest.entries[0], script_artifacts, unprofiled_request)],
        &check_artifacts,
        "check",
        CheckTestRunOptions::default(),
    );
    assert!(
        batch_unprofiled["check"]["tests"][0]
            .get("profiling")
            .is_none()
    );

    fs::remove_dir_all(project_root).unwrap();
}

#[test]
fn check_tests_profile_result_contract() {
    let project_root = project_with_manifest(
        "profile-result-contract",
        r#"{"version":1,"tests":[{"id":"sample_script_test","kind":"script","target":"res://tests/test_main.gd"}]}"#,
    );
    let config = test_config(&project_root);
    let (_, manifest, failure) = load_test_manifest(&config, "check");
    assert!(failure.is_none(), "{failure:?}");
    let manifest = manifest.unwrap();
    let check_artifacts =
        prepare_check_artifacts(&config.artifacts_dir, &config.project_root, "check").unwrap();
    let artifacts = prepare_test_artifacts(&check_artifacts, &manifest.entries[0]).unwrap();
    let process_result = successful_process_result(&project_root);
    let runtime_result = json!({
        "id": "sample_script_test",
        "kind": "script",
        "target": "res://tests/test_main.gd",
        "ok": true,
        "duration_ms": 25,
        "runtime_errors": [],
        "failures": [],
        "profile": {
            "frame_times_ms": [16.0, 17.5, 14.0],
            "monitor_samples": [
                {"name": "time/fps", "value": 60.0},
                {"name": "time/fps", "value": 58.0},
                {"name": "object/node_count", "value": 42.0}
            ],
            "unsupported_monitors": [{"name": "render/unsupported", "reason": "unavailable"}],
            "step_timings": [{"label": "run_qa_test", "duration_ms": 9}],
            "top_script_methods": {
                "status": "available",
                "instrumentation": "opt_in",
                "method_count": 1,
                "sample_count": 3,
                "methods": [{
                    "label": "MainMenu._process",
                    "call_count": 3,
                    "total_ms": 1.5,
                    "self_ms": 1.2,
                    "avg_ms": 0.5,
                    "max_ms": 0.75,
                    "last_ms": 0.25
                }]
            }
        }
    });
    let (payload, failures) = validate_runtime_test_result_with_evidence(
        &manifest.entries[0],
        runtime_result,
        &artifacts,
        &process_result,
        None,
        CheckTestRunOptions {
            timeout_override: None,
            profile_enabled: true,
        },
    );
    assert!(failures.is_empty(), "{failures:?}");
    let payloads = vec![payload];
    let summary = check_profile_summary_json(&payloads, &check_artifacts);
    assert_eq!(summary["frame_time_percentiles_ms"]["sample_count"], 3);
    assert_eq!(summary["monitor_counters"]["time/fps"]["sample_count"], 2);
    assert_eq!(summary["monitor_sample_count"], 3);
    assert_eq!(
        summary["unsupported_monitors"][0]["name"],
        "render/unsupported"
    );
    assert_eq!(
        summary["sample_limits"]["max_frame_samples"],
        super::super::CHECK_PROFILE_MAX_FRAME_SAMPLES
    );
    assert_eq!(
        summary["top_script_methods"]["methods"][0]["label"],
        "MainMenu._process"
    );
    assert_eq!(summary["slow_tests"][0]["id"], "sample_script_test");
    assert_eq!(summary["per_test_timing_count"], 2);
    assert!(
        summary["per_test_timings"].as_array().unwrap().len()
            <= super::super::CHECK_PROFILE_MAX_FRAME_SAMPLES
    );

    let artifact = check_profile_artifact_json(
        &config,
        &project_root.join("qa/tests/manifest.json"),
        &json!({"mode": "all", "selected_ids": ["sample_script_test"]}),
        CheckTestExecutionMode::Isolated,
        &check_artifacts,
        &payloads,
    );
    super::super::write_check_profile_artifact(&check_artifacts.profile_path, &artifact).unwrap();
    let written: Value =
        serde_json::from_str(&fs::read_to_string(&check_artifacts.profile_path).unwrap()).unwrap();
    assert_eq!(written["frame_time_percentiles_ms"]["p90"], 17.5);
    assert_eq!(
        written["monitor_counters"]["object/node_count"]["latest"],
        42.0
    );
    assert_eq!(
        written["unsupported_monitors"][0]["name"],
        "render/unsupported"
    );
    assert_eq!(
        written["top_script_methods"]["methods"][0]["label"],
        "MainMenu._process"
    );
    assert_eq!(written["per_test_timings"].as_array().unwrap().len(), 2);
    assert_eq!(written["tests"][0]["ok"], true);

    fs::remove_dir_all(project_root).unwrap();
}

#[test]
fn check_tests_profile_malformed_payload_contract() {
    let project_root = project_with_manifest(
        "profile-malformed-payload-contract",
        r#"{"version":1,"tests":[{"id":"sample_script_test","kind":"script","target":"res://tests/test_main.gd"}]}"#,
    );
    let config = test_config(&project_root);
    let (_, manifest, failure) = load_test_manifest(&config, "check");
    assert!(failure.is_none(), "{failure:?}");
    let manifest = manifest.unwrap();
    let check_artifacts =
        prepare_check_artifacts(&config.artifacts_dir, &config.project_root, "check").unwrap();
    let artifacts = prepare_test_artifacts(&check_artifacts, &manifest.entries[0]).unwrap();
    let process_result = successful_process_result(&project_root);
    let runtime_result = json!({
        "id": "sample_script_test",
        "kind": "script",
        "target": "res://tests/test_main.gd",
        "ok": true,
        "duration_ms": 25,
        "runtime_errors": [],
        "failures": [],
        "profile": {"frame_times_ms": ["slow"]}
    });
    let (payload, failures) = validate_runtime_test_result_with_evidence(
        &manifest.entries[0],
        runtime_result,
        &artifacts,
        &process_result,
        None,
        CheckTestRunOptions {
            timeout_override: None,
            profile_enabled: true,
        },
    );
    assert_eq!(payload["ok"], false);
    assert_eq!(failures.len(), 1);
    assert_eq!(failures[0].kind, "profile_result_invalid");
    assert_eq!(
        failures[0].path,
        Some(artifacts.result_path.to_string_lossy().into_owned())
    );
    assert_eq!(
        failures[0].artifacts.as_ref().unwrap()["profile"],
        json!(artifacts.result_path)
    );

    fs::remove_dir_all(project_root).unwrap();
}

#[test]
fn check_tests_batch_result_contract() {
    let project_root = project_with_manifest(
        "batch-result",
        r#"{
                "version": 1,
                "tests": [
                    {
                        "id": "script_bad",
                        "kind": "script",
                        "target": "res://tests/test_main.gd",
                        "tags": ["smoke"]
                    },
                    {
                        "id": "script_late",
                        "kind": "script",
                        "target": "res://tests/test_late.gd",
                        "tags": ["smoke"]
                    }
                ]
            }"#,
    );
    fs::write(project_root.join("tests/test_late.gd"), "extends Node\n").unwrap();
    let invalid_payload = json!({
        "id": "script_bad", "kind": "script", "target": "res://tests/test_main.gd",
        "duration_ms": 3, "runtime_errors": [], "failures": []
    })
    .to_string();
    let failed_payload = json!({
        "id": "script_late", "kind": "script", "target": "res://tests/test_late.gd",
        "ok": false, "duration_ms": 5,
        "runtime_errors": [{"message": "late runtime"}],
        "failures": [{"type": "assertion_failed", "message": "late failure"}]
    })
    .to_string();
    let fake_godot = godot_qa_test_support::fixture(
        &project_root.join("fake-godot-batch"),
        &[
            ("version", "4.5.2.test"),
            ("request_prefix", "--qa-check-request="),
            ("write:../tests/script-bad/result.json", &invalid_payload),
            ("write:../tests/script-late/result.json", &failed_payload),
            ("exit", "1"),
        ],
    );
    let mut config = test_config(&project_root);
    config.godot_binary = Some(fake_godot.to_string_lossy().into_owned());

    let result = run_check_tests(
        &config,
        "godot-qa check --tests --batch --tag smoke --json",
        CheckTestSelectionRequest {
            test_ids: vec![],
            tags: vec!["smoke".to_string()],
            profile: None,
        },
        CheckTestExecutionMode::Batch,
        CheckTestRunOptions::default(),
    );

    assert!(!result.ok);
    assert_eq!(result.exit_code, 1);
    assert_eq!(result.data["execution_mode"], "batch");
    assert!(result.data["artifacts"]["batch_summary"].is_string());
    assert_eq!(
        result.data["batch"]["artifacts"]["summary"],
        result.data["artifacts"]["batch_summary"]
    );
    let tests = result.data["tests"].as_array().unwrap();
    assert_eq!(tests.len(), 2);
    assert_eq!(tests[0]["id"], "script_bad");
    assert_eq!(tests[0]["ok"], false);
    assert_eq!(tests[0]["failures"][0]["type"], "check_result_invalid");
    assert!(tests[0]["failures"][0]["artifacts"]["batch_logs"].is_string());
    assert!(tests[0]["failures"][0]["artifacts"]["batch_summary"].is_string());
    assert_eq!(tests[1]["id"], "script_late");
    assert_eq!(tests[1]["kind"], "script");
    assert_eq!(tests[1]["target"], "res://tests/test_late.gd");
    assert_eq!(tests[1]["ok"], false);
    assert_eq!(tests[1]["duration_ms"], 5);
    assert_eq!(tests[1]["runtime_errors"][0]["message"], "late runtime");
    assert!(tests[1]["artifacts"]["result"].is_string());
    assert_eq!(tests[1]["failures"][0]["type"], "assertion_failed");
    assert_eq!(result.failures.len(), 2);
    assert_eq!(result.failures[0].target.as_deref(), Some("script_bad"));
    assert_eq!(result.failures[1].target.as_deref(), Some("script_late"));

    let summary_path = PathBuf::from(result.data["artifacts"]["batch_summary"].as_str().unwrap());
    let summary: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(summary_path).unwrap()).unwrap();
    assert_eq!(summary["execution_mode"], "batch");
    assert_eq!(summary["test_count"], 2);

    fs::remove_dir_all(project_root).unwrap();
}
