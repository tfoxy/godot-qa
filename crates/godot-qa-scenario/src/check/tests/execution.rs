use super::*;

#[test]
fn check_tests_accepts_empty_manifest_as_noop() {
    let project_root = unique_temp_dir("empty-manifest");
    fs::create_dir_all(project_root.join("addons/godot_qa")).unwrap();
    fs::create_dir_all(project_root.join("qa/tests")).unwrap();
    fs::write(project_root.join("project.godot"), "[application]\n").unwrap();
    fs::write(
        project_root.join("addons/godot_qa/plugin.cfg"),
        "[plugin]\n",
    )
    .unwrap();
    fs::write(
        project_root.join("qa/tests/manifest.json"),
        r#"{"version":1,"tests":[]}"#,
    )
    .unwrap();

    let config = ProjectConfig {
        project_root: project_root.clone(),
        repo_root: project_root.clone(),
        verbose: false,
        godot_binary: Some(
            godot_qa_test_support::fixture(
                &project_root.join("fake-godot"),
                &[("version", "4.5.2.test")],
            )
            .to_string_lossy()
            .into_owned(),
        ),
        qa_dir: project_root.join("qa"),
        artifacts_dir: project_root.join(".qa/artifacts"),
        scenarios_dir: project_root.join("qa/scenarios"),
        addon_dir: project_root.join("addons/godot_qa"),
        repo_addon_dir: project_root.join("addons/godot_qa"),
        addon_identity_cache: Default::default(),
        godot_launch_prefix: Vec::new(),
        godot_joypad_isolation: false,
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
    };

    let result = run_check_tests(
        &config,
        "godot-qa check --tests --json",
        CheckTestSelectionRequest::default(),
        CheckTestExecutionMode::Isolated,
        CheckTestRunOptions::default(),
    );

    assert!(result.ok);
    assert_eq!(result.exit_code, 0);
    assert_eq!(result.data["checked_test_count"], 0);
    assert_eq!(result.data["selected_test_count"], 0);
    assert_eq!(result.data["skipped_test_count"], 0);
    assert_eq!(result.data["total_test_count"], 0);
    assert_eq!(result.data["selection"]["mode"], "all");
    assert_eq!(result.data["tests"].as_array().unwrap().len(), 0);
    assert_eq!(result.data["import"]["skipped"], true);

    fs::remove_dir_all(project_root).unwrap();
}

#[test]
fn check_tests_reports_missing_manifest_without_scenarios_dir() {
    let project_root = unique_temp_dir("missing-manifest-no-scenarios");
    fs::create_dir_all(project_root.join("addons/godot_qa")).unwrap();
    fs::write(project_root.join("project.godot"), "[application]\n").unwrap();
    fs::write(
        project_root.join("addons/godot_qa/plugin.cfg"),
        "[plugin]\n",
    )
    .unwrap();

    let config = ProjectConfig {
        project_root: project_root.clone(),
        repo_root: project_root.clone(),
        verbose: false,
        godot_binary: Some(
            godot_qa_test_support::fixture(
                &project_root.join("fake-godot"),
                &[("version", "4.5.2.test")],
            )
            .to_string_lossy()
            .into_owned(),
        ),
        qa_dir: project_root.join("qa"),
        artifacts_dir: project_root.join(".qa/artifacts"),
        scenarios_dir: project_root.join("qa/scenarios"),
        addon_dir: project_root.join("addons/godot_qa"),
        repo_addon_dir: project_root.join("addons/godot_qa"),
        addon_identity_cache: Default::default(),
        godot_launch_prefix: Vec::new(),
        godot_joypad_isolation: false,
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
    };

    let result = run_check_tests(
        &config,
        "godot-qa check --tests --json",
        CheckTestSelectionRequest::default(),
        CheckTestExecutionMode::Isolated,
        CheckTestRunOptions::default(),
    );

    assert!(!result.ok);
    assert_eq!(result.exit_code, 1);
    assert_eq!(result.failures[0].kind, "test_manifest_missing");
    assert!(
        result
            .failures
            .iter()
            .all(|failure| failure.kind != "missing_scenarios_dir")
    );

    fs::remove_dir_all(project_root).unwrap();
}

#[test]
fn check_tests_timeout_contract() {
    let project_root = project_with_manifest(
        "check-timeout-contract",
        r#"{
                "version": 1,
                "timeout_seconds": 3,
                "tests": [
                    {
                        "id": "hung_script",
                        "kind": "script",
                        "target": "res://tests/test_main.gd",
                        "timeout": "2s"
                    }
                ]
            }"#,
    );
    let config = test_config(&project_root);
    let (_, manifest, failure) = load_test_manifest(&config, "check");
    assert!(failure.is_none(), "{failure:?}");
    let manifest = manifest.unwrap();
    assert_eq!(
        manifest.entries[0].timeout,
        Some(std::time::Duration::from_secs(2))
    );

    let result_payload = json!({
        "id": "hung_script", "kind": "script", "target": "res://tests/test_main.gd",
        "ok": true, "duration_ms": 1, "runtime_errors": [], "failures": []
    })
    .to_string();
    let fake_godot = godot_qa_test_support::fixture(
        &project_root.join("fake-godot-timeout"),
        &[
            ("version", "4.5.2.test"),
            ("request_prefix", "--qa-check-request="),
            ("write:result.json", &result_payload),
            ("sleep_ms", "5000"),
        ],
    );
    let mut config = test_config(&project_root);
    config.godot_binary = Some(fake_godot.to_string_lossy().into_owned());

    let result = run_check_tests(
        &config,
        "godot-qa check --tests --test hung_script --timeout-seconds 0.05 --json",
        CheckTestSelectionRequest {
            test_ids: vec!["hung_script".to_string()],
            tags: vec![],
            profile: None,
        },
        CheckTestExecutionMode::Isolated,
        CheckTestRunOptions {
            timeout_override: Some(std::time::Duration::from_millis(50)),
            profile_enabled: false,
        },
    );

    assert!(!result.ok);
    assert_eq!(result.exit_code, 1);
    assert_eq!(result.failures.len(), 1);
    let failure = &result.failures[0];
    assert_eq!(failure.kind, "check_test_timeout");
    assert_eq!(failure.target.as_deref(), Some("hung_script"));
    assert!(
        failure
            .command
            .as_deref()
            .unwrap()
            .contains("fake-godot-timeout")
    );
    assert_eq!(
        failure.expected,
        Some(json!("Godot check exits with status 0 before timeout"))
    );
    let actual = failure.actual.as_ref().unwrap();
    assert_eq!(actual["state"], "timed_out");
    assert!(actual["elapsed_ms"].as_u64().unwrap() >= 40);
    assert_eq!(actual["timeout_ms"], 50);
    assert_eq!(actual["active_test"]["id"], "hung_script");
    assert!(actual["progress"].as_str().is_some());
    assert!(actual["cleanup"]["owned"].as_bool().unwrap());
    assert_ne!(failure.kind, "assertion_failed");
    assert_eq!(result.data["tests"][0]["ok"], false);
    assert_eq!(
        result.data["tests"][0]["failures"][0]["type"],
        "check_test_timeout"
    );

    fs::remove_dir_all(project_root).unwrap();
}

#[test]
fn check_tests_batch_request_contract() {
    let project_root = project_with_manifest(
        "batch-request",
        r#"{
                "version": 1,
                "tests": [
                    {
                        "id": "script_a",
                        "kind": "script",
                        "target": "res://tests/test_main.gd",
                        "tags": ["smoke"]
                    },
                    {
                        "id": "scene_b",
                        "kind": "scene",
                        "target": "res://scenes/main.tscn",
                        "startup_frames": 2,
                        "tags": ["smoke", "ui"]
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
    let script_request = test_check_request(
        &manifest.entries[0],
        &script_artifacts,
        "run-1",
        "check",
        CheckTestRunOptions::default(),
    );
    let scene_request = test_check_request(
        &manifest.entries[1],
        &scene_artifacts,
        "run-1",
        "check",
        CheckTestRunOptions::default(),
    );
    let request = super::super::execution::batch_check_request(
        &[
            (
                &manifest.entries[0],
                script_artifacts.clone(),
                script_request,
            ),
            (&manifest.entries[1], scene_artifacts.clone(), scene_request),
        ],
        &check_artifacts,
        "check",
        CheckTestRunOptions::default(),
    );

    assert_eq!(request["check"]["batch"], true);
    assert_eq!(request["run"]["execution_mode"], "batch");
    assert_eq!(
        request["artifacts"]["summary"],
        json!(check_artifacts.batch_summary_path)
    );
    let tests = request["check"]["tests"].as_array().unwrap();
    assert_eq!(tests.len(), 2);
    assert_eq!(tests[0]["id"], "script_a");
    assert_eq!(
        tests[0]["artifacts"]["result"],
        json!(script_artifacts.result_path)
    );
    assert_eq!(tests[0]["request"], json!(script_artifacts.request_path));
    assert_eq!(tests[1]["id"], "scene_b");
    assert_eq!(tests[1]["check"]["startup_frames"], 2);
    assert_eq!(
        tests[1]["artifacts"]["logs"],
        json!(scene_artifacts.logs_path)
    );
    assert!(
        check_artifacts
            .batch_request_path
            .ends_with("batch/request.json")
    );
    assert!(check_artifacts.batch_logs_path.ends_with("batch/logs.txt"));
    assert!(
        check_artifacts
            .batch_summary_path
            .ends_with("batch/summary.json")
    );

    fs::remove_dir_all(project_root).unwrap();
}

#[test]
fn check_tests_batch_runner_contract() {
    let project_root = project_with_manifest(
        "batch-runner-contract",
        r#"{
                "version": 1,
                "tests": [
                    {
                        "id": "script_a",
                        "kind": "script",
                        "target": "res://tests/test_main.gd",
                        "time_control": {"time_scale": 2.0},
                        "tags": ["smoke"]
                    },
                    {
                        "id": "scene_b",
                        "kind": "scene",
                        "target": "res://scenes/main.tscn",
                        "startup_frames": 2,
                        "tags": ["smoke", "ui"]
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
    let request = super::super::execution::batch_check_request(
        &[
            (
                &manifest.entries[0],
                script_artifacts.clone(),
                test_check_request(
                    &manifest.entries[0],
                    &script_artifacts,
                    "run-1",
                    "check",
                    CheckTestRunOptions::default(),
                ),
            ),
            (
                &manifest.entries[1],
                scene_artifacts.clone(),
                test_check_request(
                    &manifest.entries[1],
                    &scene_artifacts,
                    "run-1",
                    "check",
                    CheckTestRunOptions::default(),
                ),
            ),
        ],
        &check_artifacts,
        "check",
        CheckTestRunOptions::default(),
    );

    assert_eq!(request["check"]["batch"], true);
    assert_eq!(request["run"]["execution_mode"], "batch");
    let tests = request["check"]["tests"].as_array().unwrap();
    assert_eq!(tests.len(), 2);
    for test in tests {
        assert!(test["id"].is_string());
        assert!(test["kind"].is_string());
        assert!(test["target"].is_string());
        assert!(test["check"].is_object());
        assert!(test["artifacts"].is_object());
        assert!(test["artifacts"]["result"].is_string());
    }
    assert_eq!(tests[0]["check"]["time_control"]["time_scale"], 2.0);
    assert_eq!(tests[1]["check"]["startup_frames"], 2);
    assert_eq!(
        request["artifacts"]["summary"],
        json!(check_artifacts.batch_summary_path)
    );

    fs::remove_dir_all(project_root).unwrap();
}

#[test]
fn check_tests_batch_deadlines_allow_individually_valid_tests() {
    let (root, payloads, failures) = batch_deadline_fixture([1.0, 1.0], [600, 600]);
    assert!(failures.is_empty(), "{failures:?}");
    assert_eq!(payloads.len(), 2);
    assert!(payloads.iter().all(|test| test["ok"] == true));
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn check_tests_batch_deadlines_enforce_short_budget_beside_long_budget() {
    let (root, _, failures) = batch_deadline_fixture([0.1, 2.0], [500, 100]);
    assert_eq!(failures.len(), 1, "{failures:?}");
    assert_eq!(failures[0].kind, "check_test_timeout");
    assert_eq!(failures[0].target.as_deref(), Some("first"));
    assert_eq!(failures[0].actual.as_ref().unwrap()["timeout_ms"], 100);
    fs::remove_dir_all(root).unwrap();
}

fn batch_deadline_fixture(
    budgets: [f64; 2],
    durations: [u64; 2],
) -> (
    PathBuf,
    Vec<Value>,
    Vec<godot_qa_core::results::FailureDetail>,
) {
    let root = project_with_manifest("batch-deadlines", &json!({
        "version": 1, "tests": (["first", "second"].iter().zip(budgets).map(|(id, timeout)| json!({
            "id": id, "kind": "script", "target": "res://tests/test_main.gd", "timeout_seconds": timeout
        })).collect::<Vec<_>>())
    }).to_string());
    let mut config = test_config(&root);
    config.godot_binary = Some(
        godot_qa_test_support::fixture(
            &root.join("fake-godot"),
            &[
                ("request_prefix", "--qa-check-request="),
                ("write:ready", "ready"),
                ("sleep_ms", "1500"),
            ],
        )
        .to_string_lossy()
        .into_owned(),
    );
    let (_, manifest, failure) = load_test_manifest(&config, "check");
    assert!(failure.is_none(), "{failure:?}");
    let manifest = manifest.unwrap();
    let artifacts = prepare_check_artifacts(&config.artifacts_dir, &root, "check").unwrap();
    let runtime_artifacts = artifacts.clone();
    let entries = manifest.entries.clone();
    let writer = std::thread::spawn(move || {
        let ready = runtime_artifacts.batch_request_path.with_file_name("ready");
        let started = std::time::Instant::now();
        while !ready.exists() {
            assert!(started.elapsed() < std::time::Duration::from_secs(5));
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        for (entry, duration) in entries.iter().zip(durations) {
            let test_artifacts = prepare_test_artifacts(&runtime_artifacts, entry).unwrap();
            append_check_progress_event(
                &runtime_artifacts.progress_path,
                Some(&test_artifacts.progress_path),
                entry,
                "start",
                "fixture",
                None,
                None,
                None,
                &test_artifacts,
            )
            .unwrap();
            std::thread::sleep(std::time::Duration::from_millis(duration));
            write_json(
                &test_artifacts.result_path,
                &json!({
                    "id": entry.id, "kind": entry.kind, "target": entry.target,
                    "ok": true, "duration_ms": duration, "runtime_errors": [], "failures": []
                }),
            )
            .unwrap();
            append_check_progress_event(
                &runtime_artifacts.progress_path,
                Some(&test_artifacts.progress_path),
                entry,
                "finish",
                "fixture",
                Some(duration),
                Some(true),
                Some(0),
                &test_artifacts,
            )
            .unwrap();
        }
    });
    let (payloads, failures) = super::super::run_manifest_tests_batch(
        &config,
        &manifest,
        &[0, 1],
        &artifacts,
        "godot-qa check --tests --batch --json",
        CheckTestRunOptions::default(),
    );
    writer.join().unwrap();
    (root, payloads, failures)
}

#[test]
fn check_tests_batch_timeout_identifies_active_test_from_progress() {
    let (project_root, result) = batch_timeout_after_completed_test(true);
    assert!(!result.ok);
    assert_eq!(result.exit_code, 1);
    assert_eq!(result.failures.len(), 1);
    let failure = &result.failures[0];
    assert_eq!(failure.kind, "check_test_timeout");
    assert_eq!(failure.target.as_deref(), Some("hung"));
    let actual = failure.actual.as_ref().unwrap();
    assert_eq!(actual["active_test"]["id"], "hung");
    assert_eq!(actual["active_test"]["kind"], "script");
    assert_eq!(actual["active_test"]["target"], "res://tests/test_main.gd");
    let progress = actual["progress"].as_str().unwrap();
    assert!(Path::new(progress).ends_with("tests/hung/progress.jsonl"));
    assert!(failure.artifacts.as_ref().unwrap()["batch_logs"].is_string());
    assert!(failure.artifacts.as_ref().unwrap()["batch_summary"].is_string());
    fs::remove_dir_all(project_root).unwrap();
}

#[test]
fn check_tests_batch_timeout_after_all_tests_finish_retains_results() {
    let project_root = project_with_manifest(
        "batch-timeout-after-finish",
        &json!({
            "version": 1,
            "tests": (["first", "second"].iter().map(|id| json!({
                "id": id, "kind": "script", "target": "res://tests/test_main.gd"
            })).collect::<Vec<_>>())
        })
        .to_string(),
    );
    let results = ["first", "second"].map(|id| {
        json!({
            "id": id, "kind": "script", "target": "res://tests/test_main.gd",
            "ok": true, "duration_ms": 7, "runtime_errors": [], "failures": []
        })
        .to_string()
    });
    let progress = ["first", "second"]
        .iter()
        .flat_map(|id| {
            ["start", "finish"].map(|event| {
                json!({
                    "id": id, "kind": "script", "target": "res://tests/test_main.gd",
                    "event": event, "elapsed_ms": 7
                })
                .to_string()
            })
        })
        .collect::<Vec<_>>()
        .join("\n");
    let mut config = test_config(&project_root);
    config.godot_binary = Some(
        godot_qa_test_support::fixture(
            &project_root.join("fake-godot"),
            &[
                ("version", "4.5.2.test"),
                ("request_prefix", "--qa-check-request="),
                ("write:../tests/first/result.json", &results[0]),
                ("write:../tests/second/result.json", &results[1]),
                ("write:../progress.jsonl", &progress),
                ("sleep_ms", "5000"),
            ],
        )
        .to_string_lossy()
        .into_owned(),
    );
    let result = run_check_tests(
        &config,
        "godot-qa check --tests --batch --json",
        CheckTestSelectionRequest::default(),
        CheckTestExecutionMode::Batch,
        CheckTestRunOptions {
            timeout_override: Some(std::time::Duration::from_millis(50)),
            profile_enabled: false,
        },
    );
    assert!(!result.ok);
    assert_eq!(result.exit_code, 1);
    let tests = result.data["tests"].as_array().unwrap();
    assert_eq!(tests.len(), 2);
    for (test, id) in tests.iter().zip(["first", "second"]) {
        assert_eq!(test["id"], id);
        assert_eq!(test["ok"], true);
        assert_eq!(test["duration_ms"], 7);
        assert_eq!(test["failures"], json!([]));
    }
    assert_eq!(result.failures.len(), 1);
    let failure = &result.failures[0];
    assert_eq!(failure.kind, "check_test_timeout");
    assert!(failure.command.is_some());
    let actual = failure.actual.as_ref().unwrap();
    assert_eq!(actual["active_test"]["id"], "second");
    assert_eq!(actual["timeout_ms"], 3100);
    assert!(actual["elapsed_ms"].as_u64().unwrap() >= 3100);
    assert!(actual["cleanup"].is_object());
    let artifacts = failure.artifacts.as_ref().unwrap();
    for key in ["batch_logs", "progress", "batch_summary"] {
        assert!(Path::new(artifacts[key].as_str().unwrap()).is_file());
    }
    let summary: Value = serde_json::from_slice(
        &fs::read(result.data["artifacts"]["batch_summary"].as_str().unwrap()).unwrap(),
    )
    .unwrap();
    assert_eq!(summary["tests"], result.data["tests"]);
    assert_eq!(summary["ok"], false);
    fs::remove_dir_all(project_root).unwrap();
}

#[test]
fn check_tests_batch_timeout_retains_completed_results() {
    for with_progress in [true, false] {
        let (project_root, result) = batch_timeout_after_completed_test(with_progress);
        assert!(!result.ok);
        assert_eq!(result.exit_code, 1);
        assert_eq!(result.failures.len(), 1, "{:?}", result.failures);
        assert_eq!(result.failures[0].kind, "check_test_timeout");
        assert_eq!(result.failures[0].target.as_deref(), Some("hung"));
        let tests = result.data["tests"].as_array().unwrap();
        assert_eq!(
            tests.len(),
            2,
            "retain completed results alongside the timeout"
        );
        assert_eq!(tests[0]["id"], "first");
        assert_eq!(tests[0]["ok"], true);
        assert_eq!(tests[0]["duration_ms"], 7);
        assert_eq!(tests[0]["failures"], json!([]));
        assert_eq!(tests[1]["id"], "hung");
        assert_eq!(tests[1]["ok"], false);
        assert_eq!(tests[1]["failures"][0]["type"], "check_test_timeout");
        let summary: Value = serde_json::from_slice(
            &fs::read(result.data["artifacts"]["batch_summary"].as_str().unwrap()).unwrap(),
        )
        .unwrap();
        assert_eq!(summary["tests"], result.data["tests"]);
        assert_eq!(summary["ok"], false);
        fs::remove_dir_all(project_root).unwrap();
    }
}

fn batch_timeout_after_completed_test(
    with_progress: bool,
) -> (PathBuf, godot_qa_core::results::CommandResult) {
    let project_root = project_with_manifest(
        "batch-timeout-after-completed",
        &json!({
            "version": 1,
            "tests": (["first", "hung", "not_started"].iter().map(|id| json!({
                "id": id, "kind": "script", "target": "res://tests/test_main.gd"
            })).collect::<Vec<_>>())
        })
        .to_string(),
    );
    let completed = json!({
        "id": "first", "kind": "script", "target": "res://tests/test_main.gd",
        "ok": true, "duration_ms": 7, "runtime_errors": [], "failures": []
    })
    .to_string();
    let mut progress = [("first", "start"), ("first", "finish"), ("hung", "start")]
        .iter()
        .map(|(id, event)| {
            json!({
                "id": id, "kind": "script", "target": "res://tests/test_main.gd", "event": event
            })
            .to_string()
        })
        .collect::<Vec<_>>()
        .join("\n");
    // A process kill can leave a partially written final journal line.
    progress.push_str("\n{\"event\":");
    let mut fixture_options = vec![
        ("version", "4.5.2.test"),
        ("request_prefix", "--qa-check-request="),
        ("write:../tests/first/result.json", completed.as_str()),
        // An active test may have begun writing its result when the process is killed.
        ("write:../tests/hung/result.json", "{"),
        ("sleep_ms", "5000"),
    ];
    if with_progress {
        fixture_options.push(("write:../progress.jsonl", progress.as_str()));
        fixture_options.push(("write:../tests/hung/progress.jsonl", progress.as_str()));
    }
    let mut config = test_config(&project_root);
    config.godot_binary = Some(
        godot_qa_test_support::fixture(&project_root.join("fake-godot"), &fixture_options)
            .to_string_lossy()
            .into_owned(),
    );
    let result = run_check_tests(
        &config,
        "godot-qa check --tests --batch --json",
        CheckTestSelectionRequest::default(),
        CheckTestExecutionMode::Batch,
        CheckTestRunOptions {
            timeout_override: Some(std::time::Duration::from_millis(100)),
            profile_enabled: false,
        },
    );
    (project_root, result)
}
