use super::*;

#[test]
fn check_manifest_accepts_time_control() {
    let project_root = project_with_manifest(
        "manifest-time-control",
        r#"{
                "version": 1,
                "tests": [
                    {
                        "id": "accelerated_scene",
                        "kind": "scene",
                        "target": "res://scenes/main.tscn",
                        "startup_frames": 2,
                        "time_control": {
                            "time_scale": 4,
                            "physics_ticks_per_second": 120,
                            "max_physics_steps_per_frame": 16
                        }
                    },
                    {
                        "id": "normal_script",
                        "kind": "script",
                        "target": "res://tests/test_main.gd"
                    }
                ]
            }"#,
    );
    let config = test_config(&project_root);

    let (_, manifest, failure) = load_test_manifest(&config, "check");

    assert!(failure.is_none(), "{failure:?}");
    let entries = manifest.unwrap().entries;
    assert_eq!(entries.len(), 2);
    assert_eq!(
        entries[0].time_control,
        Some(json!({
            "time_scale": 4.0,
            "physics_ticks_per_second": 120,
            "max_physics_steps_per_frame": 16
        }))
    );
    assert_eq!(entries[1].time_control, None);

    let artifacts = test_artifacts(&project_root);
    let request = test_check_request(
        &entries[0],
        &artifacts,
        "run-1",
        "check",
        CheckTestRunOptions::default(),
    );
    assert_eq!(
        request["check"]["time_control"],
        json!({
            "time_scale": 4.0,
            "physics_ticks_per_second": 120,
            "max_physics_steps_per_frame": 16
        })
    );
    let normal_request = test_check_request(
        &entries[1],
        &artifacts,
        "run-1",
        "check",
        CheckTestRunOptions::default(),
    );
    assert!(normal_request["check"].get("time_control").is_none());
    assert_eq!(normal_request["check"]["frame_pacing"]["fixed_fps"], 60);

    let process_result = godot_qa_process::GodotRunResult {
        command: vec!["godot".to_string()],
        returncode: 0,
        termination: None,
        process_timing: None,
        stdout: String::new(),
        stderr: String::new(),
        display: godot_qa_process::resolve_project_display_launch_metadata(
            &project_root,
            &[],
            true,
        ),
        pid: None,
        process_group: None,
        cleanup: None,
        infrastructure_failure_kind: None,
        timed_out: false,
        timeout_ms: None,
        output_truncation: None,
    };
    let runtime_result = json!({
        "id": "accelerated_scene",
        "kind": "scene",
        "target": "res://scenes/main.tscn",
        "ok": true,
        "duration_ms": 7,
        "runtime_errors": [],
        "failures": [],
        "time_control": {
            "requested": {"time_scale": 4.0},
            "effective": {"time_scale": 4.0},
            "restored": true
        }
    });
    let (payload, failures) =
        validate_runtime_test_result(&entries[0], runtime_result, &artifacts, &process_result);
    assert!(failures.is_empty());
    assert_eq!(
        payload["time_control"],
        json!({
            "requested": {"time_scale": 4.0},
            "effective": {"time_scale": 4.0},
            "restored": true
        })
    );

    fs::remove_dir_all(project_root).unwrap();
}

#[test]
fn check_manifest_accepts_fixed_fps_and_derives_project_default() {
    let project_root = project_with_manifest(
        "manifest-fixed-fps",
        r#"{
                "version": 1,
                "tests": [
                    {
                        "id": "custom_fps_scene",
                        "kind": "scene",
                        "target": "res://scenes/main.tscn",
                        "fixed_fps": 144
                    },
                    {
                        "id": "project_fps_script",
                        "kind": "script",
                        "target": "res://tests/test_main.gd"
                    }
                ]
            }"#,
    );
    fs::write(
        project_root.join("project.godot"),
        "[application]\nrun/max_fps=0\n[physics]\ncommon/physics_ticks_per_second=120\n",
    )
    .unwrap();
    let config = test_config(&project_root);

    let (_, manifest, failure) = load_test_manifest(&config, "check");

    assert!(failure.is_none(), "{failure:?}");
    let entries = manifest.unwrap().entries;
    assert_eq!(entries[0].frame_pacing.as_ref().unwrap().fixed_fps, 144);
    assert_eq!(entries[0].frame_pacing.as_ref().unwrap().source, "manifest");
    assert_eq!(entries[1].frame_pacing.as_ref().unwrap().fixed_fps, 120);
    assert_eq!(entries[1].frame_pacing.as_ref().unwrap().source, "project");
    assert_eq!(
        entries[1]
            .frame_pacing
            .as_ref()
            .unwrap()
            .project_setting
            .as_deref(),
        Some("physics/common/physics_ticks_per_second")
    );

    fs::remove_dir_all(project_root).unwrap();
}

#[test]
fn check_manifest_rejects_invalid_time_control() {
    let cases = [
        (
            "too-large",
            r#"{
                    "version": 1,
                    "tests": [{
                        "id": "bad_scale",
                        "kind": "scene",
                        "target": "res://scenes/main.tscn",
                        "time_control": {"time_scale": 65}
                    }]
                }"#,
            "bad_scale",
            "time_control.time_scale must be greater than 0.0 and <= 64",
        ),
        (
            "wrong-type",
            r#"{
                    "version": 1,
                    "tests": [{
                        "id": "bad_type",
                        "kind": "script",
                        "target": "res://tests/test_main.gd",
                        "time_control": {"time_scale": "fast"}
                    }]
                }"#,
            "bad_type",
            "time_control.time_scale must be a positive finite number",
        ),
    ];

    for (name, manifest, expected_target, expected_message) in cases {
        let project_root = project_with_manifest(name, manifest);
        let config = test_config(&project_root);

        let (_, manifest, failure) = load_test_manifest(&config, "check");

        assert!(manifest.is_none());
        let failure = failure.expect("invalid manifest failure");
        assert_eq!(failure.kind, "test_manifest_invalid");
        assert_eq!(failure.command.as_deref(), Some("check"));
        assert_eq!(failure.target.as_deref(), Some(expected_target));
        assert_eq!(failure.message.as_deref(), Some(expected_message));

        fs::remove_dir_all(project_root).unwrap();
    }
}

#[test]
fn check_manifest_rejects_invalid_fixed_fps_and_time_control_conflict() {
    let cases = [
        (
            "bad-fixed-fps",
            r#"{
                    "version": 1,
                    "tests": [{
                        "id": "bad_fps",
                        "kind": "scene",
                        "target": "res://scenes/main.tscn",
                        "fixed_fps": 0
                    }]
                }"#,
            "bad_fps",
            "fixed_fps must be a positive integer",
        ),
        (
            "conflicting-fixed-fps",
            r#"{
                    "version": 1,
                    "tests": [{
                        "id": "ambiguous_pacing",
                        "kind": "scene",
                        "target": "res://scenes/main.tscn",
                        "fixed_fps": 60,
                        "time_control": {"time_scale": 1.0}
                    }]
                }"#,
            "ambiguous_pacing",
            "fixed_fps cannot be combined with time_control",
        ),
    ];

    for (name, manifest, expected_target, expected_message) in cases {
        let project_root = project_with_manifest(name, manifest);
        let config = test_config(&project_root);

        let (_, manifest, failure) = load_test_manifest(&config, "check");

        assert!(manifest.is_none());
        let failure = failure.expect("invalid manifest failure");
        assert_eq!(failure.kind, "test_manifest_invalid");
        assert_eq!(failure.command.as_deref(), Some("check"));
        assert_eq!(failure.target.as_deref(), Some(expected_target));
        assert_eq!(failure.message.as_deref(), Some(expected_message));

        fs::remove_dir_all(project_root).unwrap();
    }
}

#[test]
fn check_tests_timeout_validation_contract() {
    let invalid_cases = [
        (
            "zero-number",
            r#"{"version":1,"timeout_seconds":0,"tests":[{"id":"script_test","kind":"script","target":"res://tests/test_main.gd"}]}"#,
        ),
        (
            "negative-number",
            r#"{"version":1,"tests":[{"id":"script_test","kind":"script","target":"res://tests/test_main.gd","timeout_seconds":-1}]}"#,
        ),
        (
            "non-number",
            r#"{"version":1,"tests":[{"id":"script_test","kind":"script","target":"res://tests/test_main.gd","timeout_seconds":"fast"}]}"#,
        ),
        (
            "unsupported-string",
            r#"{"version":1,"tests":[{"id":"script_test","kind":"script","target":"res://tests/test_main.gd","timeout":"5frames"}]}"#,
        ),
    ];
    for (name, manifest_json) in invalid_cases {
        let project_root = project_with_manifest(name, manifest_json);
        let config = test_config(&project_root);
        let (_, manifest, failure) = load_test_manifest(&config, "check");
        assert!(manifest.is_none(), "{name}");
        let failure = failure.expect("invalid timeout failure");
        assert_eq!(failure.kind, "test_manifest_invalid", "{name}");
        assert_eq!(
            failure.message.as_deref(),
            Some("manifest timeout must be a positive finite duration"),
            "{name}"
        );
        fs::remove_dir_all(project_root).unwrap();
    }
}

#[test]
fn check_manifest_tags_and_profiles_contract() {
    let valid_cases = [
        (
            "omitted-tags",
            r#"{
                    "version": 1,
                    "tests": [{
                        "id": "script_test",
                        "kind": "script",
                        "target": "res://tests/test_main.gd"
                    }]
                }"#,
            Vec::<(&str, Vec<&str>, Vec<&str>)>::new(),
        ),
        (
            "valid-tags",
            r#"{
                    "version": 1,
                    "tests": [{
                        "id": "script_test",
                        "kind": "script",
                        "target": "res://tests/test_main.gd",
                        "tags": [" smoke ", "ui"]
                    }]
                }"#,
            Vec::<(&str, Vec<&str>, Vec<&str>)>::new(),
        ),
        (
            "valid-profile-mixed",
            r#"{
                    "version": 1,
                    "tests": [
                        {
                            "id": "script_test",
                            "kind": "script",
                            "target": "res://tests/test_main.gd",
                            "tags": ["smoke"]
                        },
                        {
                            "id": "scene_test",
                            "kind": "scene",
                            "target": "res://scenes/main.tscn",
                            "tags": ["physics"]
                        }
                    ],
                    "profiles": {
                        "full": {
                            "tags": ["smoke"],
                            "tests": ["scene_test"]
                        }
                    }
                }"#,
            vec![("full", vec!["smoke"], vec!["scene_test"])],
        ),
        (
            "valid-empty-profile",
            r#"{
                    "version": 1,
                    "tests": [{
                        "id": "script_test",
                        "kind": "script",
                        "target": "res://tests/test_main.gd"
                    }],
                    "profiles": {
                        "smoke": {}
                    }
                }"#,
            vec![("smoke", vec![], vec![])],
        ),
    ];

    for (name, manifest_json, expected_profiles) in valid_cases {
        let project_root = project_with_manifest(name, manifest_json);
        let config = test_config(&project_root);

        let (_, manifest, failure) = load_test_manifest(&config, "check");

        assert!(failure.is_none(), "{name}: {failure:?}");
        let manifest = manifest.expect("valid manifest");
        assert!(!manifest.entries.is_empty());
        if name == "omitted-tags" {
            assert_eq!(manifest.entries[0].tags, Vec::<String>::new());
        }
        if name == "valid-tags" {
            assert_eq!(manifest.entries[0].tags, vec!["smoke", "ui"]);
        }
        for (profile_name, tags, tests) in expected_profiles {
            let profile = manifest.profiles.get(profile_name).unwrap();
            assert_eq!(profile.tags, tags);
            assert_eq!(profile.tests, tests);
        }

        fs::remove_dir_all(project_root).unwrap();
    }

    let invalid_cases = [
        (
            "empty-tag",
            r#"{
                    "version": 1,
                    "tests": [{
                        "id": "script_test",
                        "kind": "script",
                        "target": "res://tests/test_main.gd",
                        "tags": [" "]
                    }]
                }"#,
            "script_test",
            "manifest test tags must be an array of non-empty strings",
        ),
        (
            "non-string-tag",
            r#"{
                    "version": 1,
                    "tests": [{
                        "id": "script_test",
                        "kind": "script",
                        "target": "res://tests/test_main.gd",
                        "tags": [7]
                    }]
                }"#,
            "script_test",
            "manifest test tags must be an array of non-empty strings",
        ),
        (
            "duplicate-entry-tag",
            r#"{
                    "version": 1,
                    "tests": [{
                        "id": "script_test",
                        "kind": "script",
                        "target": "res://tests/test_main.gd",
                        "tags": ["smoke", " smoke "]
                    }]
                }"#,
            "script_test",
            "manifest string arrays must not contain duplicate values",
        ),
        (
            "reserved-entry-tag",
            r#"{
                    "version": 1,
                    "tests": [{
                        "id": "script_test",
                        "kind": "script",
                        "target": "res://tests/test_main.gd",
                        "tags": ["all"]
                    }]
                }"#,
            "script_test",
            "manifest test tags must not use reserved names",
        ),
        (
            "malformed-profile-root",
            r#"{
                    "version": 1,
                    "tests": [{
                        "id": "script_test",
                        "kind": "script",
                        "target": "res://tests/test_main.gd"
                    }],
                    "profiles": []
                }"#,
            "",
            "manifest profiles must be an object",
        ),
        (
            "unknown-profile-test",
            r#"{
                    "version": 1,
                    "tests": [{
                        "id": "script_test",
                        "kind": "script",
                        "target": "res://tests/test_main.gd"
                    }],
                    "profiles": {
                        "smoke": {"tests": ["missing_test"]}
                    }
                }"#,
            "smoke",
            "manifest profile tests must reference known test ids",
        ),
        (
            "duplicate-profile-value",
            r#"{
                    "version": 1,
                    "tests": [{
                        "id": "script_test",
                        "kind": "script",
                        "target": "res://tests/test_main.gd",
                        "tags": ["smoke"]
                    }],
                    "profiles": {
                        "smoke": {"tags": ["smoke", " smoke "]}
                    }
                }"#,
            "smoke",
            "manifest string arrays must not contain duplicate values",
        ),
    ];

    for (name, manifest_json, expected_target, expected_message) in invalid_cases {
        let project_root = project_with_manifest(name, manifest_json);
        let config = test_config(&project_root);

        let (_, manifest, failure) = load_test_manifest(&config, "check");

        assert!(manifest.is_none(), "{name}");
        let failure = failure.expect("invalid manifest failure");
        assert_eq!(failure.kind, "test_manifest_invalid", "{name}");
        assert_eq!(failure.command.as_deref(), Some("check"), "{name}");
        if expected_target.is_empty() {
            assert!(failure.target.is_none(), "{name}: {:?}", failure.target);
        } else {
            assert_eq!(failure.target.as_deref(), Some(expected_target), "{name}");
        }
        assert_eq!(failure.message.as_deref(), Some(expected_message), "{name}");
        assert!(failure.path.is_some(), "{name}");
        assert!(failure.expected.is_some(), "{name}");
        assert!(failure.actual.is_some(), "{name}");

        fs::remove_dir_all(project_root).unwrap();
    }
}

#[test]
fn check_manifest_batch_opt_out_contract() {
    let project_root = project_with_manifest(
        "manifest-batch-opt-out",
        r#"{
                "version": 1,
                "tests": [
                    {
                        "id": "batch_default",
                        "kind": "script",
                        "target": "res://tests/test_main.gd",
                        "tags": ["smoke"]
                    },
                    {
                        "id": "batch_true",
                        "kind": "script",
                        "target": "res://tests/test_main.gd",
                        "batch": true,
                        "tags": ["smoke"]
                    },
                    {
                        "id": "batch_false",
                        "kind": "script",
                        "target": "res://tests/test_main.gd",
                        "batch": false,
                        "tags": ["slow"]
                    }
                ]
            }"#,
    );
    let config = test_config(&project_root);
    let (manifest_path, manifest, failure) = load_test_manifest(&config, "check");
    assert!(failure.is_none(), "{failure:?}");
    let manifest = manifest.unwrap();
    assert!(manifest.entries[0].batchable);
    assert!(manifest.entries[1].batchable);
    assert!(!manifest.entries[2].batchable);

    let isolated = run_check_tests(
        &config,
        "godot-qa check --tests --test batch_false --json",
        CheckTestSelectionRequest {
            test_ids: vec!["batch_false".to_string()],
            tags: vec![],
            profile: None,
        },
        CheckTestExecutionMode::Isolated,
        CheckTestRunOptions::default(),
    );
    assert_ne!(
        isolated
            .failures
            .first()
            .map(|failure| failure.kind.as_str()),
        Some("test_batch_opt_out")
    );

    let batch = run_check_tests(
        &config,
        "godot-qa check --tests --batch --test batch_false --json",
        CheckTestSelectionRequest {
            test_ids: vec!["batch_false".to_string()],
            tags: vec![],
            profile: None,
        },
        CheckTestExecutionMode::Batch,
        CheckTestRunOptions::default(),
    );
    assert!(!batch.ok);
    assert_eq!(batch.exit_code, 1);
    assert_eq!(batch.data["checked_test_count"], 0);
    assert_eq!(batch.data["selected_test_count"], 1);
    assert_eq!(batch.data["tests"], json!([]));
    assert_eq!(batch.failures.len(), 1);
    assert_eq!(batch.failures[0].kind, "test_batch_opt_out");
    assert_eq!(batch.failures[0].target.as_deref(), Some("batch_false"));

    let smoke = resolve_check_test_selection(
        "check",
        &manifest_path,
        &manifest,
        CheckTestSelectionRequest {
            test_ids: vec![],
            tags: vec!["smoke".to_string()],
            profile: None,
        },
    )
    .unwrap();
    assert!(
        super::super::selected_batch_opt_out_failures("check", &manifest, &smoke.selected_indices)
            .is_empty()
    );

    fs::remove_dir_all(project_root).unwrap();

    let malformed_project = project_with_manifest(
        "manifest-batch-malformed",
        r#"{
                "version": 1,
                "tests": [
                    {
                        "id": "bad_batch",
                        "kind": "script",
                        "target": "res://tests/test_main.gd",
                        "batch": "false"
                    }
                ]
            }"#,
    );
    let malformed_config = test_config(&malformed_project);
    let (_, manifest, failure) = load_test_manifest(&malformed_config, "check");
    assert!(manifest.is_none());
    let failure = failure.expect("malformed batch failure");
    assert_eq!(failure.kind, "test_manifest_invalid");
    assert_eq!(
        failure.message.as_deref(),
        Some("manifest test batch must be a boolean")
    );
    assert_eq!(failure.target.as_deref(), Some("bad_batch"));
    fs::remove_dir_all(malformed_project).unwrap();
}

#[test]
fn check_tests_selection_resolution_contract() {
    let project_root = project_with_manifest(
        "selection-resolution",
        r#"{
                "version": 1,
                "tests": [
                    {
                        "id": "script_a",
                        "kind": "script",
                        "target": "res://tests/test_main.gd",
                        "tags": ["smoke", "script"]
                    },
                    {
                        "id": "scene_b",
                        "kind": "scene",
                        "target": "res://scenes/main.tscn",
                        "tags": ["smoke", "ui"]
                    },
                    {
                        "id": "script_c",
                        "kind": "script",
                        "target": "res://tests/test_other.gd",
                        "tags": ["physics"]
                    }
                ],
                "profiles": {
                    "smoke": {"tags": ["smoke"]},
                    "mixed": {"tags": ["physics"], "tests": ["script_a"]}
                }
            }"#,
    );
    fs::write(project_root.join("tests/test_other.gd"), "extends Node\n").unwrap();
    let config = test_config(&project_root);
    let (manifest_path, manifest, failure) = load_test_manifest(&config, "check");
    assert!(failure.is_none(), "{failure:?}");
    let manifest = manifest.unwrap();

    let all = resolve_check_test_selection(
        "check",
        &manifest_path,
        &manifest,
        CheckTestSelectionRequest::default(),
    )
    .unwrap();
    assert_eq!(all.selected_test_count(), 3);
    assert_eq!(all.skipped_test_count(), 0);
    assert_eq!(all.selection["mode"], "all");

    let explicit = resolve_check_test_selection(
        "check",
        &manifest_path,
        &manifest,
        CheckTestSelectionRequest {
            test_ids: vec!["script_c".to_string()],
            tags: vec![],
            profile: None,
        },
    )
    .unwrap();
    assert_eq!(selected_ids(&manifest, &explicit), vec!["script_c"]);
    assert_eq!(explicit.selected_test_count(), 1);
    assert_eq!(explicit.skipped_test_count(), 2);
    assert_eq!(explicit.total_test_count, 3);
    assert_eq!(explicit.selection["selected_ids"], json!(["script_c"]));

    let repeated_id = resolve_check_test_selection(
        "check",
        &manifest_path,
        &manifest,
        CheckTestSelectionRequest {
            test_ids: vec!["script_a".to_string(), " script_a ".to_string()],
            tags: vec![],
            profile: None,
        },
    )
    .unwrap();
    assert_eq!(selected_ids(&manifest, &repeated_id), vec!["script_a"]);

    let tag = resolve_check_test_selection(
        "check",
        &manifest_path,
        &manifest,
        CheckTestSelectionRequest {
            test_ids: vec![],
            tags: vec!["smoke".to_string()],
            profile: None,
        },
    )
    .unwrap();
    assert_eq!(selected_ids(&manifest, &tag), vec!["script_a", "scene_b"]);

    let tag_union = resolve_check_test_selection(
        "check",
        &manifest_path,
        &manifest,
        CheckTestSelectionRequest {
            test_ids: vec![],
            tags: vec!["ui".to_string(), "physics".to_string()],
            profile: None,
        },
    )
    .unwrap();
    assert_eq!(
        selected_ids(&manifest, &tag_union),
        vec!["scene_b", "script_c"]
    );

    let profile = resolve_check_test_selection(
        "check",
        &manifest_path,
        &manifest,
        CheckTestSelectionRequest {
            test_ids: vec![],
            tags: vec![],
            profile: Some("mixed".to_string()),
        },
    )
    .unwrap();
    assert_eq!(
        selected_ids(&manifest, &profile),
        vec!["script_a", "script_c"]
    );
    assert_eq!(profile.selection["profile"], "mixed");
    assert_eq!(profile.selection["profile_tags"], json!(["physics"]));
    assert_eq!(profile.selection["profile_tests"], json!(["script_a"]));

    let selected_failure_runtime = json!({
        "id": "script_c",
        "kind": "script",
        "target": "res://tests/test_other.gd",
        "ok": false,
        "duration_ms": 5,
        "runtime_errors": [],
        "failures": [{"type": "assertion_failed", "message": "selected failed"}]
    });
    let artifacts = test_artifacts(&project_root);
    let process_result = godot_qa_process::GodotRunResult {
        command: vec!["godot".to_string()],
        returncode: 0,
        termination: None,
        process_timing: None,
        stdout: String::new(),
        stderr: String::new(),
        display: godot_qa_process::resolve_project_display_launch_metadata(
            &project_root,
            &[],
            true,
        ),
        pid: None,
        process_group: None,
        cleanup: None,
        infrastructure_failure_kind: None,
        timed_out: false,
        timeout_ms: None,
        output_truncation: None,
    };
    let (payload, failures) = validate_runtime_test_result(
        &manifest.entries[2],
        selected_failure_runtime,
        &artifacts,
        &process_result,
    );
    assert!(!failures.is_empty());
    assert_eq!(payload["id"], "script_c");
    assert_eq!(payload["tags"], json!(["physics"]));
    assert_eq!(payload["ok"], false);

    for (name, request, expected_message) in [
        (
            "unknown-test",
            CheckTestSelectionRequest {
                test_ids: vec!["missing".to_string()],
                tags: vec![],
                profile: None,
            },
            "check --tests --test value must match a manifest test id",
        ),
        (
            "unknown-tag",
            CheckTestSelectionRequest {
                test_ids: vec![],
                tags: vec!["missing".to_string()],
                profile: None,
            },
            "check --tests --tag value must match at least one manifest test tag",
        ),
        (
            "duplicate-tag",
            CheckTestSelectionRequest {
                test_ids: vec![],
                tags: vec!["smoke".to_string(), " smoke ".to_string()],
                profile: None,
            },
            "check --tests --tag values must not contain duplicates",
        ),
        (
            "unknown-profile",
            CheckTestSelectionRequest {
                test_ids: vec![],
                tags: vec![],
                profile: Some("missing".to_string()),
            },
            "check --tests profile must exist in qa/tests/manifest.json",
        ),
    ] {
        let failure = resolve_check_test_selection("check", &manifest_path, &manifest, request)
            .expect_err(name);
        assert_eq!(failure.kind, "test_selection_invalid", "{name}");
        assert_eq!(failure.message.as_deref(), Some(expected_message), "{name}");
    }

    let result = run_check_tests(
        &config,
        "godot-qa check --tests --tag missing --json",
        CheckTestSelectionRequest {
            test_ids: vec![],
            tags: vec!["missing".to_string()],
            profile: None,
        },
        CheckTestExecutionMode::Isolated,
        CheckTestRunOptions::default(),
    );
    assert!(!result.ok);
    assert_eq!(result.exit_code, 1);
    assert_eq!(result.data["checked_test_count"], 0);
    assert_eq!(result.data["selected_test_count"], 0);
    assert_eq!(result.data["skipped_test_count"], 0);
    assert_eq!(result.data["total_test_count"], 3);
    assert_eq!(result.data["selection"]["mode"], "invalid");
    assert_eq!(result.data["tests"], json!([]));
    assert_eq!(result.failures[0].kind, "test_selection_invalid");

    let empty_profile_project = project_with_manifest(
        "selection-empty-profile",
        r#"{
                "version": 1,
                "tests": [{
                    "id": "script_test",
                    "kind": "script",
                    "target": "res://tests/test_main.gd",
                    "tags": ["ui"]
                }],
                "profiles": {
                    "smoke": {}
                }
            }"#,
    );
    let empty_profile_config = test_config(&empty_profile_project);
    let (empty_profile_manifest_path, empty_profile_manifest, empty_profile_failure) =
        load_test_manifest(&empty_profile_config, "check");
    assert!(empty_profile_failure.is_none(), "{empty_profile_failure:?}");
    let empty_profile_manifest = empty_profile_manifest.unwrap();
    let failure = resolve_check_test_selection(
        "check",
        &empty_profile_manifest_path,
        &empty_profile_manifest,
        CheckTestSelectionRequest {
            test_ids: vec![],
            tags: vec![],
            profile: Some("smoke".to_string()),
        },
    )
    .unwrap_err();
    assert_eq!(failure.kind, "test_selection_invalid");
    assert_eq!(failure.actual, Some(json!("empty_selection")));

    let empty_project = project_with_manifest(
        "selection-empty",
        r#"{
                "version": 1,
                "tests": [],
                "profiles": {}
            }"#,
    );
    let empty_config = test_config(&empty_project);
    let (empty_manifest_path, empty_manifest, empty_failure) =
        load_test_manifest(&empty_config, "check");
    assert!(empty_failure.is_none(), "{empty_failure:?}");
    let empty_manifest = empty_manifest.unwrap();
    let empty_selection = resolve_check_test_selection(
        "check",
        &empty_manifest_path,
        &empty_manifest,
        CheckTestSelectionRequest::default(),
    )
    .unwrap();
    assert_eq!(empty_selection.selected_test_count(), 0);
    assert_eq!(empty_selection.total_test_count, 0);
    assert_eq!(empty_selection.selection["mode"], "all");

    fs::remove_dir_all(project_root).unwrap();
    fs::remove_dir_all(empty_profile_project).unwrap();
    fs::remove_dir_all(empty_project).unwrap();
}
