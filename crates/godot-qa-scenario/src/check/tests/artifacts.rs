use super::*;

#[test]
fn check_tests_progress_evidence_contract() {
    let project_root = project_with_manifest(
        "progress-evidence",
        r#"{
                "version": 1,
                "tests": [
                    {
                        "id": "script_a",
                        "kind": "script",
                        "target": "res://tests/test_main.gd",
                        "tags": ["smoke"]
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
    let test_artifacts = prepare_test_artifacts(&check_artifacts, &manifest.entries[0]).unwrap();
    assert_eq!(
        check_artifacts.to_json()["progress"],
        json!(check_artifacts.progress_path)
    );
    assert_eq!(
        check_artifacts.batch_json()["progress"],
        json!(check_artifacts.progress_path)
    );
    assert_eq!(
        test_artifacts.to_json()["progress"],
        json!(test_artifacts.progress_path)
    );

    append_check_progress_event(
        &check_artifacts.progress_path,
        Some(&test_artifacts.progress_path),
        &manifest.entries[0],
        "start",
        "123Z",
        None,
        None,
        None,
        &test_artifacts,
    )
    .unwrap();
    append_check_progress_event(
        &check_artifacts.progress_path,
        Some(&test_artifacts.progress_path),
        &manifest.entries[0],
        "finish",
        "123Z",
        Some(7),
        Some(true),
        Some(0),
        &test_artifacts,
    )
    .unwrap();
    let events = fs::read_to_string(&test_artifacts.progress_path)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str::<Value>(line).unwrap())
        .collect::<Vec<_>>();
    assert_eq!(events.len(), 2);
    assert_eq!(events[0]["event"], "start");
    assert_eq!(events[0]["id"], "script_a");
    assert_eq!(events[0]["kind"], "script");
    assert_eq!(events[0]["target"], "res://tests/test_main.gd");
    assert_eq!(events[0]["started_at"], "123Z");
    assert_eq!(events[0]["elapsed_ms"], Value::Null);
    assert_eq!(events[1]["event"], "finish");
    assert_eq!(events[1]["elapsed_ms"], 7);
    assert_eq!(events[1]["ok"], true);
    assert_eq!(events[1]["failure_count"], 0);
    assert_eq!(
        events[1]["artifacts"]["progress"],
        json!(test_artifacts.progress_path)
    );

    let request = super::super::execution::batch_check_request(
        &[(
            &manifest.entries[0],
            test_artifacts.clone(),
            test_check_request(
                &manifest.entries[0],
                &test_artifacts,
                "run-1",
                "check",
                CheckTestRunOptions::default(),
            ),
        )],
        &check_artifacts,
        "check",
        CheckTestRunOptions::default(),
    );
    assert_eq!(
        request["artifacts"]["progress"],
        json!(check_artifacts.progress_path)
    );
    assert_eq!(
        request["check"]["tests"][0]["artifacts"]["progress"],
        json!(test_artifacts.progress_path)
    );

    fs::remove_dir_all(project_root).unwrap();
}

#[test]
fn slow_summary_contract_check_tests_are_bounded_sorted_artifact_backed_and_non_failing() {
    let root = unique_temp_dir("check-slow-summary-contract");
    fs::create_dir_all(&root).unwrap();
    let artifact_path = root.join("slow-summary.json");
    let payloads = vec![
        json!({"id":"fast","kind":"script","target":"res://fast.gd","ok":true,"duration_ms":5,"artifacts":{"result":"fast.json"}}),
        json!({"id":"slow_a","kind":"script","target":"res://slow_a.gd","ok":true,"duration_ms":75,"artifacts":{"result":"slow_a.json"}}),
        json!({"id":"slow_b","kind":"script","target":"res://slow_b.gd","ok":true,"duration_ms":125,"artifacts":{"result":"slow_b.json"}}),
    ];
    let summary = slow_check_summary_json(&payloads, 50, None, &artifact_path);
    write_json(&artifact_path, &summary).unwrap();

    assert_eq!(summary["slow_count"], json!(2));
    assert_eq!(summary["slow_tests"][0]["id"], json!("slow_b"));
    assert_eq!(summary["slow_tests"][1]["id"], json!("slow_a"));
    assert_eq!(summary["slow_tests"][0]["duration_ms"], json!(125));
    assert_eq!(summary["artifacts"]["slow_summary"], json!(artifact_path));
    assert!(artifact_path.is_file());

    fs::remove_dir_all(root).unwrap();
}

#[test]
fn check_tests_isolated_colliding_ids_keep_distinct_artifacts() {
    check_tests_colliding_ids_keep_distinct_artifacts(CheckTestExecutionMode::Isolated);
}

#[test]
fn check_tests_batch_colliding_ids_keep_distinct_artifacts() {
    check_tests_colliding_ids_keep_distinct_artifacts(CheckTestExecutionMode::Batch);
}

fn check_tests_colliding_ids_keep_distinct_artifacts(mode: CheckTestExecutionMode) {
    let ids = ["Foo", "foo", "a_b", "a-b", "foo-0", "plain"];
    let manifest = json!({"version": 1, "tests": ids.iter().map(|id| json!({
        "id": id, "kind": "script", "target": "res://tests/test_main.gd"
    })).collect::<Vec<_>>()});
    let project_root = project_with_manifest("colliding-ids", &manifest.to_string());
    let mut config = test_config(&project_root);
    let (_, manifest, failure) = load_test_manifest(&config, "check");
    assert!(failure.is_none(), "{failure:?}");
    let manifest = manifest.unwrap();
    let layout = prepare_check_artifacts(&config.artifacts_dir, &project_root, "check").unwrap();
    let mut fixture_options = vec![
        ("version".to_string(), "4.5.2.test".to_string()),
        (
            "request_prefix".to_string(),
            "--qa-check-request=".to_string(),
        ),
    ];
    for entry in &manifest.entries {
        let artifacts = prepare_test_artifacts(&layout, entry).unwrap();
        let directory = artifacts.root_dir.file_name().unwrap().to_str().unwrap();
        let rule = match mode {
            CheckTestExecutionMode::Isolated => format!("write_match:{directory}:result.json"),
            CheckTestExecutionMode::Batch => format!("write:../tests/{directory}/result.json"),
        };
        fixture_options.push((
            rule,
            json!({
                "id": entry.id, "kind": entry.kind, "target": entry.target,
                "ok": true, "duration_ms": 1, "runtime_errors": [], "failures": []
            })
            .to_string(),
        ));
    }
    let fixture_options = fixture_options
        .iter()
        .map(|(key, value)| (key.as_str(), value.as_str()))
        .collect::<Vec<_>>();
    config.godot_binary = Some(
        godot_qa_test_support::fixture(&project_root.join("fake-godot"), &fixture_options)
            .to_string_lossy()
            .into_owned(),
    );
    let result = run_check_tests(
        &config,
        "godot-qa check --tests --json",
        CheckTestSelectionRequest::default(),
        mode,
        CheckTestRunOptions::default(),
    );
    let tests = result.data["tests"].as_array().unwrap();
    let roots = tests
        .iter()
        .map(|test| test["artifacts"]["root"].as_str().unwrap())
        .collect::<std::collections::HashSet<_>>();
    assert_eq!(
        roots.len(),
        ids.len(),
        "distinct IDs must have distinct paths"
    );
    assert!(result.ok, "{:?}", result.failures);
    for (test, id) in tests.iter().zip(ids) {
        assert_eq!(test["id"], id);
        assert_eq!(test["ok"], true);
        for artifact in ["meta", "request", "result"] {
            let payload: Value = serde_json::from_slice(
                &fs::read(test["artifacts"][artifact].as_str().unwrap()).unwrap(),
            )
            .unwrap();
            let identity = if artifact == "request" {
                &payload["check"]
            } else {
                &payload
            };
            assert_eq!(identity["id"], id, "{artifact} identity");
        }
    }
    assert!(
        tests[4]["artifacts"]["root"]
            .as_str()
            .unwrap()
            .ends_with("foo-0")
    );
    assert!(
        tests[5]["artifacts"]["root"]
            .as_str()
            .unwrap()
            .ends_with("plain")
    );
    fs::remove_dir_all(project_root).unwrap();
}

#[test]
fn check_tests_required_result_write_failure_fails_command() {
    check_tests_required_artifact_write_failure(CheckTestExecutionMode::Isolated, "result.json");
}

#[test]
fn check_tests_required_slow_summary_write_failure_fails_command() {
    check_tests_required_artifact_write_failure(
        CheckTestExecutionMode::Isolated,
        "slow-summary.json",
    );
}

#[test]
fn check_tests_required_batch_summary_write_failure_fails_command() {
    check_tests_required_artifact_write_failure(
        CheckTestExecutionMode::Batch,
        "batch/summary.json",
    );
}

fn check_tests_required_artifact_write_failure(mode: CheckTestExecutionMode, blocked: &str) {
    let root = project_with_manifest(
        "required-artifact-write",
        r#"{
            "version": 1, "tests": [{"id":"first","kind":"script","target":"res://tests/test_main.gd"}]
        }"#,
    );
    let passing = json!({
        "id": "first", "kind": "script", "target": "res://tests/test_main.gd",
        "ok": true, "duration_ms": 1, "runtime_errors": [], "failures": []
    })
    .to_string();
    let (result_rule, block_rule) = match mode {
        CheckTestExecutionMode::Isolated => (
            "write:result.json",
            format!("write:../../{blocked}/blocker"),
        ),
        CheckTestExecutionMode::Batch => (
            "write:../tests/first/result.json",
            format!("write:../{blocked}/blocker"),
        ),
    };
    let mut config = test_config(&root);
    config.godot_binary = Some(
        godot_qa_test_support::fixture(
            &root.join("fake-godot"),
            &[
                ("version", "4.5.2.test"),
                ("request_prefix", "--qa-check-request="),
                (result_rule, passing.as_str()),
                (block_rule.as_str(), "block required artifact write"),
            ],
        )
        .to_string_lossy()
        .into_owned(),
    );
    let result = run_check_tests(
        &config,
        "godot-qa check --tests --json",
        CheckTestSelectionRequest::default(),
        mode,
        CheckTestRunOptions::default(),
    );
    assert_eq!(
        result.data["tests"][0]["ok"], true,
        "execution passed before persistence"
    );
    assert!(!result.ok, "failed required artifact write must fail check");
    assert_eq!(result.exit_code, 1);
    assert_eq!(result.failures.len(), 1, "{:?}", result.failures);
    let failure = &result.failures[0];
    assert_eq!(failure.kind, "artifact_write_failed");
    assert_eq!(failure.command.as_deref(), Some("check"));
    let path = Path::new(failure.path.as_deref().unwrap());
    assert!(path.ends_with(blocked));
    assert!(
        path.is_dir(),
        "injected blocker occupies the required artifact path"
    );
    assert!(failure.target.is_some());
    assert!(failure.expected.is_some());
    assert!(failure.actual.is_some());
    assert!(failure.artifacts.as_ref().unwrap()["root"].is_string());
    if blocked != "result.json" {
        let persisted: Value = serde_json::from_slice(
            &fs::read(result.data["artifacts"]["result"].as_str().unwrap()).unwrap(),
        )
        .unwrap();
        assert_eq!(persisted["ok"], false);
        assert_eq!(persisted["failures"][0]["type"], "artifact_write_failed");
    }
    fs::remove_dir_all(root).unwrap();
}
