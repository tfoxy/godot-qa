use godot_qa_core::config::ProjectConfig;
use godot_qa_core::results::CommandResult;
use godot_qa_scenario::bundle::{
    BundlePostRun, ScenarioBundleRunOptions, list_bundles, load_bundle_manifest,
    resolve_bundle_parallelism_with_available, run_scenario_bundle_with_runner,
    validate_bundle_manifest,
};
use godot_qa_scenario::bundle_schedule::BundleSchedulePolicy;
use godot_qa_scenario::bundle_selection::BundleSelection;
use serde_json::json;
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, AtomicUsize, Ordering},
};
use std::thread;
use std::time::Duration;
use std::time::{SystemTime, UNIX_EPOCH};

fn temp_dir(name: &str) -> PathBuf {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let path = std::env::temp_dir().join(format!("godot-qa-{name}-{unique}"));
    fs::create_dir_all(&path).unwrap();
    path
}

fn write(path: &Path, contents: &str) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(path, contents).unwrap();
}

fn make_config(root: &Path) -> ProjectConfig {
    ProjectConfig {
        project_root: root.to_path_buf(),
        repo_root: root.to_path_buf(),
        verbose: false,
        godot_binary: None,
        qa_dir: root.join("qa"),
        artifacts_dir: root.join(".qa/artifacts"),
        scenarios_dir: root.join("qa/scenarios"),
        addon_dir: root.join("addons/godot_qa"),
        repo_addon_dir: root.join("addons/godot_qa"),
        addon_identity_cache: Default::default(),
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

fn retry_options(attempts: usize) -> ScenarioBundleRunOptions {
    ScenarioBundleRunOptions {
        strict: false,
        warm_strict: false,
        headless_override: None,
        audio_override: None,
        command_line: "godot-qa scenario bundle run gate --json".to_string(),
        retry_policy: godot_qa_scenario::bundle::BundleRetryPolicy {
            max_infrastructure_startup_attempts: attempts,
            backoff_ms: 0,
        },
        jobs: 1,
        jobs_origin: "default".to_string(),
        ..Default::default()
    }
}

fn write_retry_fixture(root: &Path) {
    write(
        &root.join("qa/gates/gate.json"),
        r#"{"name":"gate","required_scenarios":["child"]}"#,
    );
    write(
        &root.join("qa/scenarios/child.json"),
        r#"{"name":"child","scene":"res://Child.tscn","steps":[]}"#,
    );
}

fn child_result(
    root: &Path,
    attempt: usize,
    ok: bool,
    failures: Vec<godot_qa_core::results::FailureDetail>,
) -> CommandResult {
    let run_root = root.join(format!("attempt-{attempt}"));
    let mut result = if ok {
        CommandResult::success("scenario run", "ok")
    } else {
        CommandResult {
            ok: false,
            command: "scenario run".to_string(),
            message: "failed".to_string(),
            data: serde_json::Map::new(),
            failures,
            exit_code: 1,
        }
    };
    result
        .data
        .insert("result".to_string(), json!(run_root.join("result.json")));
    result.data.insert(
        "artifacts".to_string(),
        json!({
            "run_root": run_root,
            "result_path": run_root.join("result.json"),
            "logs_path": run_root.join("logs.txt")
        }),
    );
    result.data.insert(
        "launch_display".to_string(),
        json!({"selected_mode":"windowed"}),
    );
    result
}

fn child_result_with_time_control(root: &Path, time_control: serde_json::Value) -> CommandResult {
    let mut result = child_result(root, 1, true, vec![]);
    result.data.insert("time_control".to_string(), time_control);
    result
}

fn update_max(max_active: &AtomicUsize, value: usize) {
    let mut current = max_active.load(Ordering::SeqCst);
    while value > current {
        match max_active.compare_exchange(current, value, Ordering::SeqCst, Ordering::SeqCst) {
            Ok(_) => break,
            Err(next) => current = next,
        }
    }
}

fn startup_failure() -> godot_qa_core::results::FailureDetail {
    godot_qa_core::results::FailureDetail {
        kind: "scenario_result_missing".to_string(),
        message: Some("Godot run did not produce result.json".to_string()),
        details: serde_json::Map::from_iter([
            (
                "failure_category".to_string(),
                json!("display_infrastructure"),
            ),
            ("retryable".to_string(), json!(true)),
            (
                "infrastructure_failure_kind".to_string(),
                json!("display_startup_failed"),
            ),
            (
                "retry_guidance".to_string(),
                json!("Retry after display resources recover."),
            ),
        ]),
        ..Default::default()
    }
}

fn assertion_failure() -> godot_qa_core::results::FailureDetail {
    godot_qa_core::results::FailureDetail {
        kind: "assertion_failed".to_string(),
        details: serde_json::Map::from_iter([
            ("failure_category".to_string(), json!("gameplay_assertion")),
            ("retryable".to_string(), json!(false)),
        ]),
        ..Default::default()
    }
}

fn artifact_failure(kind: &str) -> godot_qa_core::results::FailureDetail {
    godot_qa_core::results::FailureDetail {
        kind: kind.to_string(),
        details: serde_json::Map::from_iter([
            ("failure_category".to_string(), json!("artifact")),
            ("retryable".to_string(), json!(false)),
        ]),
        ..Default::default()
    }
}

#[test]
fn load_bundle_manifest_supports_json_and_yaml_subset() {
    let root = temp_dir("bundle-load");
    let json_path = root.join("qa/gates/a.json");
    let yaml_path = root.join("qa/gates/b.yaml");
    write(
        &json_path,
        r#"{"name":"alpha","required_bundles":["base_gate"],"required_scenarios":["main"]}"#,
    );
    write(
        &yaml_path,
        "name: beta\nrequired_bundles:\n  - smoke_gate\nrequired_scenarios:\n  - settings\n",
    );

    let json_manifest = load_bundle_manifest(&json_path).unwrap();
    let yaml_manifest = load_bundle_manifest(&yaml_path).unwrap();
    assert_eq!(json_manifest["name"], "alpha");
    assert_eq!(json_manifest["required_bundles"], json!(["base_gate"]));
    assert_eq!(yaml_manifest["name"], "beta");
    assert_eq!(yaml_manifest["required_bundles"], json!(["smoke_gate"]));
}

#[test]
fn list_bundles_is_sorted_by_name_then_path_and_reports_duplicates() {
    let root = temp_dir("bundle-list");
    write(&root.join("qa/gates/z.yaml"), "name: zeta\n");
    write(&root.join("qa/gates/a/first.json"), r#"{"name":"alpha"}"#);
    write(&root.join("qa/gates/b/dup.json"), r#"{"name":"alpha"}"#);

    let listed = list_bundles(&make_config(&root));
    assert_eq!(listed.bundles.len(), 3);
    assert_eq!(listed.bundles[0].name, "alpha");
    assert_eq!(listed.bundles[1].name, "alpha");
    assert_eq!(listed.bundles[2].name, "zeta");

    assert!(
        listed
            .failures
            .iter()
            .any(|f| f.kind == "duplicate_bundle_name")
    );
}

#[test]
fn validate_bundle_manifest_accepts_unicode_artifact_names_and_references() {
    let document = json!({
        "name": "門_hyvä",
        "required_scenarios": ["playable_1hyvä_two_player"],
        "optional_scenarios": ["1HYVÄ"],
        "required_bundles": ["玩家_二人"]
    });
    validate_bundle_manifest(&document, Path::new("gate.json")).unwrap();
}

#[test]
fn validate_bundle_manifest_rejects_malformed_types_and_unsafe_refs() {
    let root = temp_dir("bundle-invalid");
    let path = root.join("qa/gates/bad.json");

    let err = validate_bundle_manifest(&json!({"name":"ok","required_scenarios":"bad"}), &path)
        .unwrap_err();
    assert_eq!(err.kind, "bundle_validation_failed");
    assert_eq!(err.expected, Some(json!("list")));

    let err = validate_bundle_manifest(
        &json!({"name":"ok","required_scenarios":["../escape"]}),
        &path,
    )
    .unwrap_err();
    assert_eq!(err.kind, "unsafe_scenario_reference");

    let err = validate_bundle_manifest(&json!({"name":"ok","required_bundles":"bad"}), &path)
        .unwrap_err();
    assert_eq!(err.kind, "bundle_validation_failed");
    assert_eq!(err.expected, Some(json!("list")));

    let err = validate_bundle_manifest(
        &json!({"name":"ok","required_bundles":["../escape"]}),
        &path,
    )
    .unwrap_err();
    assert_eq!(err.kind, "unsafe_bundle_reference");
}

#[test]
fn validate_bundle_manifest_rejects_unsupported_orchestration_and_recursive_references() {
    let root = temp_dir("bundle-unsupported");
    let path = root.join("qa/gates/bad.json");

    let err =
        validate_bundle_manifest(&json!({"name":"ok","script":"echo hi"}), &path).unwrap_err();
    assert_eq!(err.kind, "bundle_unsupported_key");

    let err = validate_bundle_manifest(
        &json!({"name":"ok","optional_bundles":["regression"]}),
        &path,
    )
    .unwrap_err();
    assert_eq!(err.kind, "bundle_recursive_reference_unsupported");
}

#[test]
fn bundle_manifest_required_bundles_parses_lists_and_artifacts() {
    let root = temp_dir("bundle-required-bundles");
    write(
        &root.join("qa/gates/base.json"),
        r#"{"name":"base","required_scenarios":[]}"#,
    );
    write(
        &root.join("qa/gates/gate.json"),
        &json!({
            "name": "gate",
            "purpose": "Composed gate metadata",
            "required_bundles": ["base"],
            "required_scenarios": [],
            "optional_scenarios": [],
            "recommended_manual_checks": ["Inspect included gate"]
        })
        .to_string(),
    );
    let cfg = make_config(&root);

    let listed = list_bundles(&cfg);
    assert!(listed.failures.is_empty());
    let gate = listed
        .bundles
        .iter()
        .find(|bundle| bundle.name == "gate")
        .unwrap();
    assert_eq!(gate.required_bundles, vec!["base"]);

    let result = run_scenario_bundle_with_runner(
        &cfg,
        "gate",
        &ScenarioBundleRunOptions {
            strict: false,
            warm_strict: false,
            headless_override: None,
            audio_override: None,
            command_line: "godot-qa scenario bundle run gate --json".to_string(),
            retry_policy: Default::default(),
            jobs: 1,
            jobs_origin: "default".to_string(),
            ..Default::default()
        },
        |_c, _p, _s, _cmd, _h, _a| CommandResult::success("scenario run", "ok"),
        BundlePostRun::default,
    );

    assert!(result.ok, "{:#?}", result.failures);
    assert_eq!(result.data["required_bundles"], json!(["base"]));
    let meta_path = result.data["artifacts"]["meta_path"].as_str().unwrap();
    let summary_path = result.data["artifacts"]["summary_path"].as_str().unwrap();
    let meta: serde_json::Value = serde_json::from_slice(&fs::read(meta_path).unwrap()).unwrap();
    let summary: serde_json::Value =
        serde_json::from_slice(&fs::read(summary_path).unwrap()).unwrap();
    assert_eq!(meta["required_bundles"], json!(["base"]));
    assert_eq!(summary["required_bundles"], json!(["base"]));
}

#[test]
fn load_bundle_manifest_reports_non_object_root() {
    let root = temp_dir("bundle-root");
    let path = root.join("qa/gates/list.json");
    write(&path, "[]");

    let err = load_bundle_manifest(&path).unwrap_err();
    assert_eq!(
        serde_json::to_value(err).unwrap(),
        json!({
            "type":"bundle_parse_failed",
            "path": path,
            "message":"bundle manifest root must be an object",
            "expected":"object",
            "actual":"list"
        })
    );
}

#[test]
fn run_bundle_optional_missing_warns_and_strict_fails() {
    let root = temp_dir("bundle-run");
    write(
        &root.join("qa/gates/gate.json"),
        r#"{"name":"gate","required_scenarios":[],"optional_scenarios":["missing"]}"#,
    );
    let cfg = make_config(&root);
    let ok = run_scenario_bundle_with_runner(
        &cfg,
        "gate",
        &ScenarioBundleRunOptions {
            strict: false,
            warm_strict: false,
            headless_override: None,
            audio_override: None,
            command_line: "godot-qa scenario bundle run gate --json".to_string(),
            retry_policy: Default::default(),
            jobs: 1,
            jobs_origin: "default".to_string(),
            ..Default::default()
        },
        |_c, _p, _s, _cmd, _h, _a| CommandResult::success("scenario run", "ok"),
        BundlePostRun::default,
    );
    assert!(ok.ok);
    assert!(ok.data.get("warnings").is_some());
    let strict = run_scenario_bundle_with_runner(
        &cfg,
        "gate",
        &ScenarioBundleRunOptions {
            strict: true,
            warm_strict: false,
            headless_override: None,
            audio_override: None,
            command_line: "godot-qa scenario bundle run gate --strict --json".to_string(),
            retry_policy: Default::default(),
            jobs: 1,
            jobs_origin: "default".to_string(),
            ..Default::default()
        },
        |_c, _p, _s, _cmd, _h, _a| CommandResult::success("scenario run", "ok"),
        BundlePostRun::default,
    );
    assert!(!strict.ok);
    assert!(strict.failures.iter().any(|f| f.kind == "missing_scenario"));
}

#[test]
fn scenario_bundle_parallelism_resolution_contract() {
    let defaulted = resolve_bundle_parallelism_with_available(0, Some(8), 10);
    assert_eq!(defaulted.raw_jobs, 0);
    assert_eq!(defaulted.resolved_jobs, 8);
    assert_eq!(defaulted.source, "available_parallelism");
    assert_eq!(defaulted.available_parallelism, 8);
    assert_eq!(defaulted.reserved_count, 0);
    assert_eq!(defaulted.runnable_count, 10);
    assert!(defaulted.complete_queued);

    let positive = resolve_bundle_parallelism_with_available(3, Some(8), 10);
    assert_eq!(positive.resolved_jobs, 3);
    assert_eq!(positive.source, "explicit");

    let capped = resolve_bundle_parallelism_with_available(6, Some(8), 2);
    assert_eq!(capped.resolved_jobs, 2);
    assert_eq!(capped.runnable_count, 2);

    let reserved = resolve_bundle_parallelism_with_available(-2, Some(8), 10);
    assert_eq!(reserved.resolved_jobs, 6);
    assert_eq!(reserved.source, "reserved_parallelism");
    assert_eq!(reserved.reserved_count, 2);

    let lower_bound = resolve_bundle_parallelism_with_available(-16, Some(8), 10);
    assert_eq!(lower_bound.resolved_jobs, 1);

    let fallback = resolve_bundle_parallelism_with_available(0, None, 10);
    assert_eq!(fallback.resolved_jobs, 1);
    assert_eq!(fallback.available_parallelism, 1);
    assert_eq!(fallback.source, "available_parallelism_fallback");
}

#[test]
fn scenario_bundle_parallel_children_overlap_and_preserve_order() {
    let root = temp_dir("bundle-parallel-overlap");
    write(
        &root.join("qa/gates/gate.json"),
        r#"{"name":"gate","required_scenarios":["slow","fast"]}"#,
    );
    write(
        &root.join("qa/scenarios/slow.json"),
        r#"{"name":"slow","scene":"res://Slow.tscn","steps":[]}"#,
    );
    write(
        &root.join("qa/scenarios/fast.json"),
        r#"{"name":"fast","scene":"res://Fast.tscn","steps":[]}"#,
    );
    let cfg = make_config(&root);
    let active = AtomicUsize::new(0);
    let max_active = AtomicUsize::new(0);
    let completion_order = Mutex::new(Vec::new());

    let result = run_scenario_bundle_with_runner(
        &cfg,
        "gate",
        &ScenarioBundleRunOptions {
            strict: false,
            warm_strict: false,
            headless_override: None,
            audio_override: None,
            command_line: "godot-qa scenario bundle run gate --jobs 2 --json".to_string(),
            retry_policy: Default::default(),
            jobs: 2,
            jobs_origin: "default".to_string(),
            ..Default::default()
        },
        |c, _p, s, _cmd, _h, _a| {
            let name = s["name"].as_str().unwrap();
            let now_active = active.fetch_add(1, Ordering::SeqCst) + 1;
            update_max(&max_active, now_active);
            if name == "slow" {
                thread::sleep(Duration::from_millis(120));
            } else {
                thread::sleep(Duration::from_millis(10));
            }
            active.fetch_sub(1, Ordering::SeqCst);
            completion_order.lock().unwrap().push(name.to_string());
            child_result(&c.artifacts_dir.join(name), 1, true, vec![])
        },
        BundlePostRun::default,
    );

    assert!(result.ok, "{:#?}", result.failures);
    assert!(
        max_active.load(Ordering::SeqCst) > 1,
        "children did not overlap"
    );
    assert_eq!(
        completion_order.lock().unwrap().as_slice(),
        ["fast".to_string(), "slow".to_string()]
    );
    let runs = result.data["runs"].as_array().unwrap();
    assert_eq!(runs[0]["index"], 0);
    assert_eq!(runs[0]["scenario"], "slow");
    assert_eq!(runs[1]["index"], 1);
    assert_eq!(runs[1]["scenario"], "fast");
    assert_eq!(result.data["parallelism"]["raw_jobs"], 2);
    assert_eq!(result.data["parallelism"]["resolved_jobs"], 2);
    assert_eq!(result.data["parallelism"]["complete_queued"], true);
}

#[test]
fn scenario_bundle_jobs_one_remains_serial() {
    let root = temp_dir("bundle-serial-jobs-one");
    write(
        &root.join("qa/gates/gate.json"),
        r#"{"name":"gate","required_scenarios":["first","second"]}"#,
    );
    write(
        &root.join("qa/scenarios/first.json"),
        r#"{"name":"first","scene":"res://First.tscn","steps":[]}"#,
    );
    write(
        &root.join("qa/scenarios/second.json"),
        r#"{"name":"second","scene":"res://Second.tscn","steps":[]}"#,
    );
    let cfg = make_config(&root);
    let active = AtomicUsize::new(0);
    let max_active = AtomicUsize::new(0);
    let call_order = Mutex::new(Vec::new());

    let result = run_scenario_bundle_with_runner(
        &cfg,
        "gate",
        &ScenarioBundleRunOptions {
            strict: false,
            warm_strict: false,
            headless_override: None,
            audio_override: None,
            command_line: "godot-qa scenario bundle run gate --jobs 1 --json".to_string(),
            retry_policy: Default::default(),
            jobs: 1,
            jobs_origin: "default".to_string(),
            ..Default::default()
        },
        |c, _p, s, _cmd, _h, _a| {
            let name = s["name"].as_str().unwrap();
            let now_active = active.fetch_add(1, Ordering::SeqCst) + 1;
            update_max(&max_active, now_active);
            thread::sleep(Duration::from_millis(10));
            active.fetch_sub(1, Ordering::SeqCst);
            call_order.lock().unwrap().push(name.to_string());
            child_result(&c.artifacts_dir.join(name), 1, true, vec![])
        },
        BundlePostRun::default,
    );

    assert!(result.ok, "{:#?}", result.failures);
    assert_eq!(max_active.load(Ordering::SeqCst), 1);
    assert_eq!(
        call_order.lock().unwrap().as_slice(),
        ["first".to_string(), "second".to_string()]
    );
    assert_eq!(result.data["parallelism"]["resolved_jobs"], 1);
}

#[test]
fn scenario_bundle_required_child_failure_does_not_stop_runnable_siblings() {
    let root = temp_dir("bundle-required-failure-continues");
    write(
        &root.join("qa/gates/gate.json"),
        r#"{"name":"gate","required_scenarios":["failing","passing"]}"#,
    );
    write(
        &root.join("qa/scenarios/failing.json"),
        r#"{"name":"failing","scene":"res://Failing.tscn","steps":[]}"#,
    );
    write(
        &root.join("qa/scenarios/passing.json"),
        r#"{"name":"passing","scene":"res://Passing.tscn","steps":[]}"#,
    );
    let cfg = make_config(&root);

    let result = run_scenario_bundle_with_runner(
        &cfg,
        "gate",
        &ScenarioBundleRunOptions {
            strict: false,
            warm_strict: false,
            headless_override: None,
            audio_override: None,
            command_line: "godot-qa scenario bundle run gate --jobs 1 --json".to_string(),
            retry_policy: Default::default(),
            jobs: 1,
            jobs_origin: "default".to_string(),
            ..Default::default()
        },
        |c, _p, s, _cmd, _h, _a| {
            let name = s["name"].as_str().unwrap();
            if name == "failing" {
                child_result(
                    &c.artifacts_dir.join(name),
                    1,
                    false,
                    vec![assertion_failure()],
                )
            } else {
                child_result(&c.artifacts_dir.join(name), 1, true, vec![])
            }
        },
        BundlePostRun::default,
    );

    assert!(!result.ok);
    let runs = result.data["runs"].as_array().unwrap();
    assert_eq!(runs.len(), 2);
    assert_eq!(runs[0]["scenario"], "failing");
    assert_eq!(runs[0]["status"], "failed");
    assert_eq!(runs[1]["scenario"], "passing");
    assert_eq!(runs[1]["status"], "passed");
    assert!(
        result
            .failures
            .iter()
            .any(|failure| failure.kind == "assertion_failed")
    );
}

#[test]
fn scenario_bundle_parallel_retry_attempts_remain_per_child() {
    let root = temp_dir("bundle-parallel-retry-per-child");
    write(
        &root.join("qa/gates/gate.json"),
        r#"{"name":"gate","required_scenarios":["flaky","steady"]}"#,
    );
    write(
        &root.join("qa/scenarios/flaky.json"),
        r#"{"name":"flaky","scene":"res://Flaky.tscn","steps":[]}"#,
    );
    write(
        &root.join("qa/scenarios/steady.json"),
        r#"{"name":"steady","scene":"res://Steady.tscn","steps":[]}"#,
    );
    let cfg = make_config(&root);
    let calls = Arc::new(Mutex::new(HashMap::<String, usize>::new()));

    let result = run_scenario_bundle_with_runner(
        &cfg,
        "gate",
        &ScenarioBundleRunOptions {
            strict: false,
            warm_strict: false,
            headless_override: None,
            audio_override: None,
            command_line: "godot-qa scenario bundle run gate --jobs 2 --retry-startup 3 --json"
                .to_string(),
            retry_policy: godot_qa_scenario::bundle::BundleRetryPolicy {
                max_infrastructure_startup_attempts: 3,
                backoff_ms: 0,
            },
            jobs: 2,
            jobs_origin: "default".to_string(),
            ..Default::default()
        },
        |c, _p, s, _cmd, _h, _a| {
            let name = s["name"].as_str().unwrap().to_string();
            let attempt = {
                let mut calls = calls.lock().unwrap();
                let entry = calls.entry(name.clone()).or_insert(0);
                *entry += 1;
                *entry
            };
            if name == "flaky" && attempt == 1 {
                child_result(
                    &c.artifacts_dir.join(&name),
                    attempt,
                    false,
                    vec![startup_failure()],
                )
            } else {
                child_result(&c.artifacts_dir.join(&name), attempt, true, vec![])
            }
        },
        BundlePostRun::default,
    );

    assert!(result.ok, "{:#?}", result.failures);
    let runs = result.data["runs"].as_array().unwrap();
    let flaky = runs.iter().find(|run| run["scenario"] == "flaky").unwrap();
    let steady = runs.iter().find(|run| run["scenario"] == "steady").unwrap();
    assert_eq!(flaky["attempts"].as_array().unwrap().len(), 2);
    assert_eq!(flaky["final_attempt"], 2);
    assert_eq!(steady["attempts"].as_array().unwrap().len(), 1);
    assert_eq!(*calls.lock().unwrap().get("flaky").unwrap(), 2);
    assert_eq!(*calls.lock().unwrap().get("steady").unwrap(), 1);
}

#[test]
fn bundle_artifact_summary_includes_child_paths_and_optional_warnings() {
    let root = temp_dir("bundle-artifact-summary");
    write(
        &root.join("qa/gates/base.json"),
        r#"{"name":"base","required_scenarios":["passing_child"]}"#,
    );
    write(
        &root.join("qa/gates/gate.json"),
        r#"{"name":"gate","required_bundles":["base"],"optional_scenarios":["missing_optional"]}"#,
    );
    write(
        &root.join("qa/scenarios/passing_child.json"),
        r#"{"name":"passing_child","scene":"res://Passing.tscn","steps":[]}"#,
    );
    let cfg = make_config(&root);

    let result = run_scenario_bundle_with_runner(
        &cfg,
        "gate",
        &ScenarioBundleRunOptions {
            strict: false,
            warm_strict: false,
            headless_override: None,
            audio_override: None,
            command_line: "godot-qa scenario bundle run gate --json".to_string(),
            retry_policy: Default::default(),
            jobs: 1,
            jobs_origin: "default".to_string(),
            ..Default::default()
        },
        |c, _p, _s, _cmd, _h, _a| {
            let run_root = c.artifacts_dir.join("child-runs/passing_child");
            let mut result = CommandResult::success("scenario run", "ok");
            result.data.insert(
                "artifacts".to_string(),
                json!({
                    "run_root": run_root,
                    "result_path": run_root.join("result.json"),
                    "logs_path": run_root.join("logs.txt"),
                    "diagnostic_path": run_root.join("diagnostics/report.json")
                }),
            );
            result.data.insert(
                "snapshot_artifacts".to_string(),
                json!({
                    "screenshots": [{"screenshot_path": run_root.join("screens/main.png")}]
                }),
            );
            result
        },
        BundlePostRun::default,
    );

    assert!(result.ok, "{:#?}", result.failures);
    let artifact_index = &result.data["artifact_index"];
    let children = artifact_index["children"].as_array().unwrap();
    assert_eq!(children.len(), 2);

    let child = &children[0];
    assert_eq!(child["scenario"], "passing_child");
    assert_eq!(child["required"], true);
    assert_eq!(child["status"], "passed");
    assert_eq!(child["source_bundle"], "base");
    assert_eq!(child["bundle_chain"], json!(["gate", "base"]));
    assert!(
        child["run_root"]
            .as_str()
            .unwrap()
            .replace('\\', "/")
            .ends_with(".qa/artifacts/child-runs/passing_child")
    );
    assert!(
        child["result_path"]
            .as_str()
            .unwrap()
            .ends_with("result.json")
    );
    assert!(child["logs_path"].as_str().unwrap().ends_with("logs.txt"));
    assert_eq!(child["screenshot_paths"].as_array().unwrap().len(), 1);
    assert!(
        child["screenshot_paths"][0]
            .as_str()
            .unwrap()
            .replace('\\', "/")
            .ends_with("screens/main.png")
    );
    assert_eq!(child["diagnostic_paths"].as_array().unwrap().len(), 1);
    assert!(
        child["diagnostic_paths"][0]
            .as_str()
            .unwrap()
            .replace('\\', "/")
            .ends_with("diagnostics/report.json")
    );
    assert!(
        child["failure_artifact_paths"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    assert!(child.get("child_run_root").is_none());
    assert!(child.get("launch_args").is_none());

    let warning = &children[1];
    assert_eq!(warning["scenario"], "missing_optional");
    assert_eq!(warning["required"], false);
    assert_eq!(warning["status"], "missing_optional");
    assert_eq!(warning["source_bundle"], "gate");
    assert_eq!(warning["bundle_chain"], json!(["gate"]));
    assert_eq!(warning["warning"]["type"], "missing_scenario");
    assert_eq!(warning["warning"]["actual"], "missing_optional");
    assert!(warning["run_root"].is_null());
    assert!(warning.get("child_run_root").is_none());
    assert!(warning.get("launch_args").is_none());

    let summary_path = result.data["artifacts"]["summary_path"].as_str().unwrap();
    let summary: serde_json::Value =
        serde_json::from_slice(&fs::read(summary_path).unwrap()).unwrap();
    assert_eq!(summary["artifact_index"], result.data["artifact_index"]);
    assert!(
        summary["artifact_index"]["children"][0]
            .get("child_run_root")
            .is_none()
    );
    assert!(
        summary["artifact_index"]["children"][0]
            .get("launch_args")
            .is_none()
    );
}

#[test]
fn bundle_artifact_summary_preserves_nested_lineage_and_failure_paths() {
    let root = temp_dir("bundle-artifact-summary-nested-failure");
    write(
        &root.join("qa/gates/base.json"),
        r#"{"name":"base","required_scenarios":["failing_child"]}"#,
    );
    write(
        &root.join("qa/gates/mid.json"),
        r#"{"name":"mid","required_bundles":["base"]}"#,
    );
    write(
        &root.join("qa/gates/gate.json"),
        r#"{"name":"gate","required_bundles":["mid"]}"#,
    );
    write(
        &root.join("qa/scenarios/failing_child.json"),
        r#"{"name":"failing_child","scene":"res://Failing.tscn","steps":[]}"#,
    );
    let cfg = make_config(&root);

    let result = run_scenario_bundle_with_runner(
        &cfg,
        "gate",
        &ScenarioBundleRunOptions {
            strict: false,
            warm_strict: false,
            headless_override: None,
            audio_override: None,
            command_line: "godot-qa scenario bundle run gate --json".to_string(),
            retry_policy: Default::default(),
            jobs: 1,
            jobs_origin: "default".to_string(),
            ..Default::default()
        },
        |c, _p, _s, _cmd, _h, _a| {
            let run_root = c.artifacts_dir.join("child-runs/failing_child");
            CommandResult {
                ok: false,
                command: "scenario run".to_string(),
                message: "scenario failed".to_string(),
                data: serde_json::Map::from_iter([
                    (
                        "artifacts".to_string(),
                        json!({
                            "run_root": run_root,
                            "result_path": run_root.join("result.json"),
                            "logs_path": run_root.join("logs.txt"),
                            "diagnostic_path": run_root.join("diagnostics/runtime.json"),
                            "metrics_path": run_root.join("metrics/layout.json")
                        }),
                    ),
                    (
                        "snapshot_artifacts".to_string(),
                        json!({
                            "screenshots": [{
                                "screenshot_path": run_root.join("screens/failure.png"),
                                "diff_path": run_root.join("screens/failure.diff.png"),
                                "temporal_trace": run_root.join("temporal/frames.json")
                            }]
                        }),
                    ),
                ]),
                failures: vec![godot_qa_core::results::FailureDetail {
                    kind: "assertion_failed".to_string(),
                    message: Some("button was not visible".to_string()),
                    path: Some(
                        run_root
                            .join("assertions/visibility.json")
                            .to_string_lossy()
                            .into_owned(),
                    ),
                    artifacts: Some(serde_json::Map::from_iter([
                        (
                            "screenshot_path".to_string(),
                            json!(run_root.join("screens/failure-assertion.png")),
                        ),
                        (
                            "diff_path".to_string(),
                            json!(run_root.join("screens/failure-assertion.diff.png")),
                        ),
                        (
                            "metrics_path".to_string(),
                            json!(run_root.join("metrics/failure.json")),
                        ),
                    ])),
                    details: serde_json::Map::from_iter([
                        (
                            "diagnostics_path".to_string(),
                            json!(run_root.join("diagnostics/failure.json")),
                        ),
                        (
                            "temporal_evidence_path".to_string(),
                            json!(run_root.join("temporal/failure.json")),
                        ),
                        (
                            "artifact_paths".to_string(),
                            json!([run_root.join("artifacts/failure-extra.json")]),
                        ),
                    ]),
                    ..Default::default()
                }],
                exit_code: 1,
            }
        },
        BundlePostRun::default,
    );

    assert!(!result.ok);
    assert!(
        result
            .failures
            .iter()
            .any(|failure| failure.kind == "assertion_failed")
    );

    let runs = result.data["runs"].as_array().unwrap();
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0]["scenario"], "failing_child");
    assert_eq!(runs[0]["status"], "failed");
    assert_eq!(runs[0]["source_bundle"], "base");
    assert_eq!(runs[0]["bundle_chain"], json!(["gate", "mid", "base"]));
    assert_eq!(runs[0]["failures"][0]["type"], "assertion_failed");
    assert!(
        runs[0]["failures"][0]["artifacts"]["screenshot_path"]
            .as_str()
            .unwrap()
            .replace('\\', "/")
            .ends_with("screens/failure-assertion.png")
    );

    let artifact_index = &result.data["artifact_index"];
    let children = artifact_index["children"].as_array().unwrap();
    assert_eq!(children.len(), 1);
    let child = &children[0];
    assert_eq!(child["scenario"], "failing_child");
    assert_eq!(child["required"], true);
    assert_eq!(child["status"], "failed");
    assert_eq!(child["source_bundle"], "base");
    assert_eq!(child["bundle_chain"], json!(["gate", "mid", "base"]));
    assert!(
        child["run_root"]
            .as_str()
            .unwrap()
            .replace('\\', "/")
            .ends_with("child-runs/failing_child")
    );
    assert!(
        child["result_path"]
            .as_str()
            .unwrap()
            .ends_with("result.json")
    );
    assert!(child["logs_path"].as_str().unwrap().ends_with("logs.txt"));
    assert!(
        child["screenshot_paths"]
            .as_array()
            .unwrap()
            .iter()
            .any(|path| path
                .as_str()
                .unwrap()
                .replace('\\', "/")
                .ends_with("screens/failure.png"))
    );
    assert!(
        child["screenshot_paths"]
            .as_array()
            .unwrap()
            .iter()
            .any(|path| path
                .as_str()
                .unwrap()
                .replace('\\', "/")
                .ends_with("screens/failure-assertion.png"))
    );
    let diagnostic_paths = child["diagnostic_paths"].as_array().unwrap();
    for suffix in [
        "diagnostics/runtime.json",
        "screens/failure-assertion.diff.png",
        "diagnostics/failure.json",
        "temporal/failure.json",
    ] {
        assert!(
            diagnostic_paths
                .iter()
                .any(|path| path.as_str().unwrap().ends_with(suffix)),
            "missing diagnostic path suffix {suffix}: {diagnostic_paths:#?}"
        );
    }
    let failure_artifact_paths = child["failure_artifact_paths"].as_array().unwrap();
    for suffix in [
        "assertions/visibility.json",
        "screens/failure-assertion.png",
        "screens/failure-assertion.diff.png",
        "metrics/failure.json",
        "diagnostics/failure.json",
        "temporal/failure.json",
        "artifacts/failure-extra.json",
    ] {
        assert!(
            failure_artifact_paths
                .iter()
                .any(|path| path.as_str().unwrap().ends_with(suffix)),
            "missing failure artifact path suffix {suffix}: {failure_artifact_paths:#?}"
        );
    }
    assert!(child.get("child_run_root").is_none());
    assert!(child.get("launch_args").is_none());

    let summary_path = result.data["artifacts"]["summary_path"].as_str().unwrap();
    let summary: serde_json::Value =
        serde_json::from_slice(&fs::read(summary_path).unwrap()).unwrap();
    assert_eq!(summary["artifact_index"], result.data["artifact_index"]);
    assert_eq!(
        summary["runs"][0]["failures"][0]["type"],
        "assertion_failed"
    );
}

#[test]
fn bundle_artifact_summary_is_written_and_matches_output() {
    let root = temp_dir("bundle-artifact-summary-file");
    write(
        &root.join("qa/gates/gate.json"),
        r#"{"name":"gate","required_scenarios":["child"]}"#,
    );
    write(
        &root.join("qa/scenarios/child.json"),
        r#"{"name":"child","scene":"res://Child.tscn","steps":[]}"#,
    );
    let cfg = make_config(&root);

    let result = run_scenario_bundle_with_runner(
        &cfg,
        "gate",
        &ScenarioBundleRunOptions {
            strict: false,
            warm_strict: false,
            headless_override: None,
            audio_override: None,
            command_line: "godot-qa scenario bundle run gate --json".to_string(),
            retry_policy: Default::default(),
            jobs: 1,
            jobs_origin: "default".to_string(),
            ..Default::default()
        },
        |c, _p, _s, _cmd, _h, _a| {
            let run_root = c.artifacts_dir.join("child-runs/child");
            let mut result = CommandResult::success("scenario run", "ok");
            result.data.insert(
                "artifacts".to_string(),
                json!({
                    "run_root": run_root,
                    "result_path": run_root.join("result.json"),
                    "logs_path": run_root.join("logs.txt")
                }),
            );
            result
        },
        BundlePostRun::default,
    );

    assert!(result.ok, "{:#?}", result.failures);
    assert!(result.data.get("artifact_summary").is_some());
    assert_eq!(
        result.data["artifact_summary"],
        result.data["artifact_index"]
    );

    let artifacts = &result.data["artifacts"];
    let summary_path = artifacts["summary_path"].as_str().unwrap();
    assert!(summary_path.ends_with("summary.json"));
    assert!(Path::new(summary_path).exists());

    let artifact_index_path = artifacts["artifact_index_path"].as_str().unwrap();
    assert!(artifact_index_path.ends_with("artifact-index.json"));
    let artifact_index: serde_json::Value =
        serde_json::from_slice(&fs::read(artifact_index_path).unwrap()).unwrap();
    assert_eq!(
        artifact_index["children"],
        result.data["artifact_summary"]["children"]
    );

    let child = &artifact_index["children"][0];
    assert_eq!(child["scenario"], "child");
    assert!(
        child["result_path"]
            .as_str()
            .unwrap()
            .ends_with("result.json")
    );
}

#[test]
fn run_bundle_named_invalid_manifest_returns_manifest_failure() {
    let root = temp_dir("bundle-run-invalid");
    let manifest_path = root.join("qa/gates/malformed_bundle_demo.yaml");
    write(
        &manifest_path,
        "name: malformed_bundle_demo\nrequired_scenarios:\n  - sample_main_menu_start\ncommands:\n  - echo unsafe\n",
    );
    let cfg = make_config(&root);

    let result = run_scenario_bundle_with_runner(
        &cfg,
        "malformed_bundle_demo",
        &ScenarioBundleRunOptions {
            strict: false,
            warm_strict: false,
            headless_override: None,
            audio_override: None,
            command_line: "godot-qa scenario bundle run malformed_bundle_demo --json".to_string(),
            retry_policy: Default::default(),
            jobs: 1,
            jobs_origin: "default".to_string(),
            ..Default::default()
        },
        |_c, _p, _s, _cmd, _h, _a| CommandResult::success("scenario run", "ok"),
        BundlePostRun::default,
    );

    assert!(!result.ok);
    assert_eq!(result.failures[0].kind, "bundle_unsupported_key");
    assert_eq!(
        Path::new(result.failures[0].path.as_deref().unwrap()),
        manifest_path
    );
    assert_eq!(result.failures[0].actual, Some(json!("commands")));
}

#[test]
fn run_bundle_summary_records_manifest_metadata() {
    let root = temp_dir("bundle-summary");
    write(
        &root.join("qa/gates/base_gate.json"),
        r#"{"name":"base_gate","required_scenarios":["base_child"]}"#,
    );
    write(
        &root.join("qa/gates/gate.json"),
        &json!({
            "name": "gate",
            "purpose": "Feature regression gate",
            "required_bundles": ["base_gate"],
            "required_scenarios": ["direct_child"],
            "optional_scenarios": ["missing_optional"],
            "recommended_manual_checks": ["Inspect child run roots"]
        })
        .to_string(),
    );
    write(
        &root.join("qa/scenarios/base_child.json"),
        r#"{"name":"base_child","scene":"res://Base.tscn","steps":[],"godot_args":["--base-flag"]}"#,
    );
    write(
        &root.join("qa/scenarios/direct_child.json"),
        r#"{"name":"direct_child","scene":"res://Direct.tscn","steps":[],"godot_args":["--direct-flag"]}"#,
    );
    let cfg = make_config(&root);

    let result = run_scenario_bundle_with_runner(
        &cfg,
        "gate",
        &ScenarioBundleRunOptions {
            strict: false,
            warm_strict: false,
            headless_override: None,
            audio_override: None,
            command_line: "godot-qa scenario bundle run gate --json".to_string(),
            retry_policy: Default::default(),
            jobs: 1,
            jobs_origin: "default".to_string(),
            ..Default::default()
        },
        |c, p, s, _cmd, _h, _a| {
            let name = s["name"].as_str().unwrap();
            let run_root = c.artifacts_dir.join("child-runs").join(name);
            let mut result = CommandResult::success("scenario run", "ok");
            result.data.insert(
                "artifacts".to_string(),
                json!({
                    "run_root": run_root,
                    "result_path": run_root.join("result.json"),
                    "logs_path": run_root.join("logs.txt"),
                    "diagnostic_path": run_root.join("diagnostics/report.json")
                }),
            );
            result
                .data
                .insert("godot_args".to_string(), s["godot_args"].clone());
            result.data.insert(
                "launch_display".to_string(),
                json!({"headless": true, "selected_mode": "headless"}),
            );
            result.data.insert("scenario_path".to_string(), json!(p));
            result
        },
        BundlePostRun::default,
    );

    assert!(result.ok, "{:#?}", result.failures);
    let runs = result.data["runs"].as_array().unwrap();
    assert_eq!(runs.len(), 2);
    assert_eq!(runs[0]["index"], 0);
    assert_eq!(runs[0]["scenario"], "base_child");
    assert_eq!(runs[0]["source_bundle"], "base_gate");
    assert_eq!(runs[0]["source_kind"], "included");
    assert_eq!(runs[0]["status"], "passed");
    assert_eq!(runs[0]["godot_args"], json!(["--base-flag"]));
    assert_eq!(runs[0]["bundle_chain"], json!(["gate", "base_gate"]));
    assert_eq!(runs[0]["launch_display"]["headless"], true);
    assert!(
        runs[0]["run_root"]
            .as_str()
            .unwrap()
            .replace('\\', "/")
            .ends_with(".qa/artifacts/child-runs/base_child")
    );
    assert!(runs[0].get("launch_args").is_none());
    assert!(runs[0].get("child_run_root").is_none());
    assert!(runs[0].get("failure_category").is_none());
    assert!(
        runs[0]["result_path"]
            .as_str()
            .unwrap()
            .ends_with("result.json")
    );
    assert!(runs[0]["logs_path"].as_str().unwrap().ends_with("logs.txt"));
    assert_eq!(runs[0]["diagnostic_paths"].as_array().unwrap().len(), 1);
    assert!(runs[0]["screenshot_paths"].as_array().unwrap().is_empty());
    assert_eq!(runs[1]["index"], 1);
    assert_eq!(runs[1]["scenario"], "direct_child");
    assert_eq!(runs[1]["source_bundle"], "gate");
    assert_eq!(runs[1]["source_kind"], "direct");
    assert_eq!(runs[1]["godot_args"], json!(["--direct-flag"]));
    assert_eq!(runs[1]["bundle_chain"], json!(["gate"]));
    let summary_path = result.data["artifacts"]["summary_path"].as_str().unwrap();
    let summary: serde_json::Value =
        serde_json::from_slice(&fs::read(summary_path).unwrap()).unwrap();
    assert_eq!(summary["bundle"], "gate");
    assert_eq!(summary["purpose"], "Feature regression gate");
    assert_eq!(summary["required_bundles"], json!(["base_gate"]));
    assert_eq!(summary["required_scenarios"], json!(["direct_child"]));
    assert_eq!(summary["optional_scenarios"], json!(["missing_optional"]));
    assert_eq!(
        summary["recommended_manual_checks"],
        json!(["Inspect child run roots"])
    );
    assert_eq!(summary["runs"], result.data["runs"]);
    assert_eq!(summary["runs"][0]["source_bundle"], "base_gate");
    assert_eq!(summary["runs"][1]["run_root"], runs[1]["run_root"]);
    assert!(summary["runs"][0].get("launch_args").is_none());
    assert!(summary["runs"][0].get("child_run_root").is_none());
    assert!(summary["runs"][0].get("failure_category").is_none());
}

#[test]
fn bundle_expanded_run_summary() {
    let root = temp_dir("bundle-expanded-run-summary");
    write(
        &root.join("qa/gates/base.json"),
        r#"{"name":"base","required_scenarios":["included_required"]}"#,
    );
    write(
        &root.join("qa/gates/gate.json"),
        r#"{"name":"gate","required_bundles":["base"],"required_scenarios":["direct_required"]}"#,
    );
    write(
        &root.join("qa/scenarios/included_required.json"),
        r#"{"name":"included_required","scene":"res://Included.tscn","steps":[],"godot_args":["--included"]}"#,
    );
    write(
        &root.join("qa/scenarios/direct_required.json"),
        r#"{"name":"direct_required","scene":"res://Direct.tscn","steps":[],"godot_args":["--direct"]}"#,
    );
    let cfg = make_config(&root);

    let result = run_scenario_bundle_with_runner(
        &cfg,
        "gate",
        &ScenarioBundleRunOptions {
            strict: false,
            warm_strict: false,
            headless_override: Some(true),
            audio_override: None,
            command_line: "godot-qa scenario bundle run gate --json --headless".to_string(),
            retry_policy: Default::default(),
            jobs: 1,
            jobs_origin: "default".to_string(),
            ..Default::default()
        },
        |c, p, s, cmd, h, _a| {
            assert_eq!(cmd, "godot-qa scenario bundle run gate --json --headless");
            assert_eq!(h, Some(true));
            let name = s["name"].as_str().unwrap();
            let root = c.artifacts_dir.join("runs").join(name);
            let mut result = CommandResult::success("scenario run", "ok");
            result
                .data
                .insert("result".to_string(), json!(root.join("result.json")));
            result.data.insert(
                "artifacts".to_string(),
                json!({
                    "run_root": root,
                    "result_path": root.join("result.json"),
                    "logs_path": root.join("logs.txt")
                }),
            );
            result
                .data
                .insert("godot_args".to_string(), s["godot_args"].clone());
            result.data.insert(
                "launch_display".to_string(),
                json!({"headless": true, "selected_mode": "headless", "scenario_path": p}),
            );
            result
        },
        BundlePostRun::default,
    );

    assert!(result.ok, "{:#?}", result.failures);
    let summary_path = result.data["artifacts"]["summary_path"].as_str().unwrap();
    let summary: serde_json::Value =
        serde_json::from_slice(&fs::read(summary_path).unwrap()).unwrap();
    let runs = summary["runs"].as_array().unwrap();
    assert_eq!(runs.len(), 2);
    assert_eq!(runs[0]["index"], 0);
    assert_eq!(runs[0]["scenario"], "included_required");
    assert_eq!(
        Path::new(runs[0]["scenario_path"].as_str().unwrap()),
        root.join("qa/scenarios/included_required.json")
    );
    assert_eq!(runs[0]["source_bundle"], "base");
    assert_eq!(runs[0]["source_kind"], "included");
    assert_eq!(runs[0]["godot_args"], json!(["--included"]));
    assert_eq!(runs[0]["bundle_chain"], json!(["gate", "base"]));
    assert_eq!(runs[0]["launch_display"]["headless"], true);
    assert_eq!(runs[0]["status"], "passed");
    assert_eq!(runs[0]["ok"], true);
    assert!(
        runs[0]["run_root"]
            .as_str()
            .unwrap()
            .replace('\\', "/")
            .ends_with(".qa/artifacts/runs/included_required")
    );
    assert_eq!(runs[1]["index"], 1);
    assert_eq!(runs[1]["scenario"], "direct_required");
    assert_eq!(runs[1]["source_bundle"], "gate");
    assert_eq!(runs[1]["source_kind"], "direct");
    assert_eq!(runs[1]["godot_args"], json!(["--direct"]));
    assert_eq!(runs[1]["bundle_chain"], json!(["gate"]));
    assert_eq!(result.data["runs"], summary["runs"]);
}

#[test]
fn bundle_project_args_summary_direct() {
    let root = temp_dir("bundle-project-args-direct");
    write(
        &root.join("qa/gates/gate.json"),
        r#"{"name":"gate","required_scenarios":["direct_child"]}"#,
    );
    write(
        &root.join("qa/scenarios/direct_child.json"),
        r#"{"name":"direct_child","scene":"res://Direct.tscn","steps":[],"godot_args":["--direct"],"project_args":["--slot","qa","--token","abcdef","API_KEY=supersecret"]}"#,
    );
    let cfg = make_config(&root);

    let result = run_scenario_bundle_with_runner(
        &cfg,
        "gate",
        &ScenarioBundleRunOptions {
            strict: false,
            warm_strict: false,
            headless_override: None,
            audio_override: None,
            command_line: "godot-qa scenario bundle run gate --json".to_string(),
            retry_policy: Default::default(),
            jobs: 1,
            jobs_origin: "default".to_string(),
            ..Default::default()
        },
        |c, _p, s, _cmd, _h, _a| {
            let run_root = c.artifacts_dir.join("child-runs/direct_child");
            let mut result = CommandResult::success("scenario run", "ok");
            result.data.insert(
                "artifacts".to_string(),
                json!({
                    "run_root": run_root,
                    "result_path": run_root.join("result.json"),
                    "logs_path": run_root.join("logs.txt")
                }),
            );
            result
                .data
                .insert("godot_args".to_string(), s["godot_args"].clone());
            result
                .data
                .insert("project_args".to_string(), s["project_args"].clone());
            result
        },
        BundlePostRun::default,
    );

    assert!(result.ok, "{:#?}", result.failures);
    let run = &result.data["runs"][0];
    assert_eq!(run["godot_args"], json!(["--direct"]));
    assert_eq!(
        run["project_args"],
        json!(["--slot", "qa", "--token", "abcdef", "API_KEY=supersecret"])
    );
    assert_eq!(
        run["attempts"][0]["project_args"],
        json!(["--slot", "qa", "--token", "abcdef", "API_KEY=supersecret"])
    );
    assert!(run.get("launch_args").is_none());

    let summary_path = result.data["artifacts"]["summary_path"].as_str().unwrap();
    let summary: serde_json::Value =
        serde_json::from_slice(&fs::read(summary_path).unwrap()).unwrap();
    assert_eq!(
        summary["runs"][0]["project_args"],
        json!([
            "--slot",
            "qa",
            "--token",
            "<redacted:6>",
            "API_KEY=<redacted:11>"
        ])
    );
    assert_eq!(
        summary["runs"][0]["attempts"][0]["project_args"],
        json!([
            "--slot",
            "qa",
            "--token",
            "<redacted:6>",
            "API_KEY=<redacted:11>"
        ])
    );
    assert!(summary["runs"][0].get("launch_args").is_none());
}

#[test]
fn bundle_project_args_summary_included() {
    let root = temp_dir("bundle-project-args-included");
    write(
        &root.join("qa/gates/base.json"),
        r#"{"name":"base","required_scenarios":["included_child"]}"#,
    );
    write(
        &root.join("qa/gates/gate.json"),
        r#"{"name":"gate","required_bundles":["base"]}"#,
    );
    write(
        &root.join("qa/scenarios/included_child.json"),
        r#"{"name":"included_child","scene":"res://Included.tscn","steps":[],"godot_args":["--included"],"project_args":["--password","hunter2","SAVE_TOKEN=abc123"]}"#,
    );
    let cfg = make_config(&root);

    let result = run_scenario_bundle_with_runner(
        &cfg,
        "gate",
        &ScenarioBundleRunOptions {
            strict: false,
            warm_strict: false,
            headless_override: None,
            audio_override: None,
            command_line: "godot-qa scenario bundle run gate --json".to_string(),
            retry_policy: Default::default(),
            jobs: 1,
            jobs_origin: "default".to_string(),
            ..Default::default()
        },
        |c, _p, s, _cmd, _h, _a| {
            let run_root = c.artifacts_dir.join("child-runs/included_child");
            let mut result = CommandResult::success("scenario run", "ok");
            result.data.insert(
                "artifacts".to_string(),
                json!({
                    "run_root": run_root,
                    "result_path": run_root.join("result.json"),
                    "logs_path": run_root.join("logs.txt")
                }),
            );
            result
                .data
                .insert("godot_args".to_string(), s["godot_args"].clone());
            result
                .data
                .insert("project_args".to_string(), s["project_args"].clone());
            result
        },
        BundlePostRun::default,
    );

    assert!(result.ok, "{:#?}", result.failures);
    let run = &result.data["runs"][0];
    assert_eq!(run["source_kind"], "included");
    assert_eq!(run["godot_args"], json!(["--included"]));
    assert_eq!(
        run["project_args"],
        json!(["--password", "hunter2", "SAVE_TOKEN=abc123"])
    );

    let summary_path = result.data["artifacts"]["summary_path"].as_str().unwrap();
    let summary: serde_json::Value =
        serde_json::from_slice(&fs::read(summary_path).unwrap()).unwrap();
    assert_eq!(
        summary["runs"][0]["project_args"],
        json!(["--password", "<redacted:7>", "SAVE_TOKEN=<redacted:6>"])
    );
    assert!(summary["runs"][0].get("launch_args").is_none());
}

#[test]
fn bundle_retry_infrastructure_startup_contract() {
    let root = temp_dir("bundle-retry-startup");
    write_retry_fixture(&root);
    let cfg = make_config(&root);
    let calls = AtomicUsize::new(0);

    let result = run_scenario_bundle_with_runner(
        &cfg,
        "gate",
        &retry_options(3),
        |c, _p, _s, _cmd, _h, _a| {
            let attempt = calls.fetch_add(1, Ordering::SeqCst) + 1;
            if attempt == 1 {
                child_result(&c.artifacts_dir, attempt, false, vec![startup_failure()])
            } else {
                child_result(&c.artifacts_dir, attempt, true, vec![])
            }
        },
        BundlePostRun::default,
    );

    assert!(result.ok, "{:#?}", result.failures);
    assert_eq!(calls.load(Ordering::SeqCst), 2);
    let run = &result.data["runs"][0];
    assert_eq!(run["status"], "passed");
    assert_eq!(run["final_attempt"], 2);
    assert_eq!(
        run["retry_policy"]["max_infrastructure_startup_attempts"],
        3
    );
    assert_eq!(run["attempts"].as_array().unwrap().len(), 2);
    assert_eq!(run["attempts"][0]["retry_decision"], "retrying");
    assert_eq!(run["attempts"][0]["retryable"], true);
    assert_eq!(run["attempts"][0]["category"], "display_infrastructure");
    assert!(run["attempts"][0].get("failure_category").is_none());
    assert!(run["attempts"][0].get("child_run_root").is_none());
    assert_eq!(run["attempts"][1]["retry_decision"], "not_retryable");
    assert_eq!(run["attempt_artifacts"].as_array().unwrap().len(), 2);
    assert!(
        run["attempt_artifacts"][0]["run_root"]
            .as_str()
            .unwrap()
            .ends_with("attempt-1")
    );
    assert!(
        run["attempt_artifacts"][1]["run_root"]
            .as_str()
            .unwrap()
            .ends_with("attempt-2")
    );
    assert!(run["retry_guidance"].is_null());
    assert_eq!(run["display_lifecycle"]["status"], "completed");
}

#[test]
fn bundle_does_not_retry_gameplay_or_artifact_failures() {
    for (case, failure) in [
        ("assertion", assertion_failure()),
        (
            "malformed-result",
            artifact_failure("scenario_result_invalid"),
        ),
        ("script-error", artifact_failure("godot_run_failed")),
        ("screenshot", artifact_failure("screenshot_low_information")),
    ] {
        let root = temp_dir(&format!("bundle-no-retry-{case}"));
        write_retry_fixture(&root);
        let cfg = make_config(&root);
        let calls = AtomicUsize::new(0);
        let result = run_scenario_bundle_with_runner(
            &cfg,
            "gate",
            &retry_options(3),
            |c, _p, _s, _cmd, _h, _a| {
                let attempt = calls.fetch_add(1, Ordering::SeqCst) + 1;
                child_result(&c.artifacts_dir, attempt, false, vec![failure.clone()])
            },
            BundlePostRun::default,
        );

        assert!(!result.ok, "case {case}");
        assert_eq!(calls.load(Ordering::SeqCst), 1, "case {case}");
        let run = &result.data["runs"][0];
        assert_eq!(run["attempts"].as_array().unwrap().len(), 1, "case {case}");
        assert_eq!(
            run["attempts"][0]["retry_decision"], "not_retryable",
            "case {case}"
        );
        assert_eq!(run["attempts"][0]["retryable"], false, "case {case}");
        assert_eq!(
            run["attempt_artifacts"].as_array().unwrap().len(),
            1,
            "case {case}"
        );
    }
}

#[test]
fn bundle_summary_preserves_time_control_evidence() {
    let root = temp_dir("bundle-time-control-evidence");
    write_retry_fixture(&root);
    let cfg = make_config(&root);
    let time_control = json!({
        "target": "scenario",
        "requested": {"time_scale": 4.0},
        "effective": {"time_scale": 4.0},
        "restoration_status": "restored",
        "warnings": []
    });

    let result = run_scenario_bundle_with_runner(
        &cfg,
        "gate",
        &retry_options(1),
        |c, _p, _s, _cmd, _h, _a| {
            child_result_with_time_control(&c.artifacts_dir, time_control.clone())
        },
        BundlePostRun::default,
    );

    assert!(result.ok, "{:#?}", result.failures);
    let run = &result.data["runs"][0];
    assert_eq!(run["time_control"], time_control);
    assert_eq!(run["attempts"][0]["time_control"], time_control);
}

#[test]
fn recursive_bundle_contract() {
    fn run_failure(root: &Path, name: &str) -> CommandResult {
        let cfg = make_config(root);
        let called = AtomicBool::new(false);
        let result = run_scenario_bundle_with_runner(
            &cfg,
            name,
            &ScenarioBundleRunOptions {
                strict: false,
                warm_strict: false,
                headless_override: None,
                audio_override: None,
                command_line: format!("godot-qa scenario bundle run {name} --json"),
                retry_policy: Default::default(),
                jobs: 1,
                jobs_origin: "default".to_string(),
                ..Default::default()
            },
            |_c, _p, _s, _cmd, _h, _a| {
                called.store(true, Ordering::SeqCst);
                CommandResult::success("scenario run", "ok")
            },
            BundlePostRun::default,
        );
        assert!(
            !called.load(Ordering::SeqCst),
            "child runner must not run on preflight failure"
        );
        result
    }

    let self_root = temp_dir("bundle-self-cycle");
    write(
        &self_root.join("qa/gates/self.json"),
        r#"{"name":"self","required_bundles":["self"],"required_scenarios":["a"]}"#,
    );
    let result = run_failure(&self_root, "self");
    assert_eq!(result.failures[0].kind, "bundle_cycle_detected");
    assert_eq!(
        result.failures[0].actual,
        Some(json!({"chain":["self","self"]}))
    );

    let cycle_root = temp_dir("bundle-multi-cycle");
    write(
        &cycle_root.join("qa/gates/a.json"),
        r#"{"name":"a","required_bundles":["b"],"required_scenarios":[]}"#,
    );
    write(
        &cycle_root.join("qa/gates/b.json"),
        r#"{"name":"b","required_bundles":["a"],"required_scenarios":[]}"#,
    );
    let result = run_failure(&cycle_root, "a");
    assert_eq!(result.failures[0].kind, "bundle_cycle_detected");
    assert_eq!(
        result.failures[0].actual,
        Some(json!({"chain":["a","b","a"]}))
    );

    let missing_root = temp_dir("bundle-missing-required");
    write(
        &missing_root.join("qa/gates/gate.json"),
        r#"{"name":"gate","required_bundles":["base"],"required_scenarios":[]}"#,
    );
    let result = run_failure(&missing_root, "gate");
    assert_eq!(result.failures[0].kind, "missing_required_bundle");
    assert_eq!(result.failures[0].target.as_deref(), Some("base"));

    let duplicate_root = temp_dir("bundle-duplicate-required-scenario");
    write(
        &duplicate_root.join("qa/gates/base.json"),
        r#"{"name":"base","required_scenarios":["shared"]}"#,
    );
    write(
        &duplicate_root.join("qa/gates/gate.json"),
        r#"{"name":"gate","required_bundles":["base"],"required_scenarios":["shared"]}"#,
    );
    let result = run_failure(&duplicate_root, "gate");
    assert_eq!(result.failures[0].kind, "duplicate_required_scenario");
    assert_eq!(result.failures[0].target.as_deref(), Some("gate"));

    let unsafe_root = temp_dir("bundle-unsafe-reference");
    write(
        &unsafe_root.join("qa/gates/gate.json"),
        r#"{"name":"gate","required_bundles":["../base"],"required_scenarios":[]}"#,
    );
    let result = run_failure(&unsafe_root, "gate");
    assert_eq!(result.failures[0].kind, "unsafe_bundle_reference");
}

/// A four-scenario gate plus its scenario files, used by the selection and scheduling tests.
fn write_four_scenario_gate(root: &Path) {
    write(
        &root.join("qa/gates/gate.json"),
        r#"{"name":"gate","required_scenarios":["alpha","bravo","charlie","delta"]}"#,
    );
    for name in ["alpha", "bravo", "charlie", "delta"] {
        write(
            &root.join(format!("qa/scenarios/{name}.json")),
            &format!(r#"{{"name":"{name}","scene":"res://{name}.tscn","steps":[]}}"#),
        );
    }
}

/// Write one previous bundle run whose `summary.json` records the given durations, so
/// longest-first dispatch has a history to read.
fn write_duration_history(root: &Path, epoch: u64, durations: &[(&str, f64)]) {
    let runs: Vec<serde_json::Value> = durations
        .iter()
        .map(|(scenario, duration_ms)| {
            json!({"scenario": scenario, "status": "passed", "duration_ms": duration_ms})
        })
        .collect();
    write(
        &root.join(format!(".qa/artifacts/bundles/gate/{epoch}/summary.json")),
        &serde_json::to_string_pretty(&json!({
            "bundle": "gate",
            "runs": runs,
            "failures": [],
        }))
        .unwrap(),
    );
}

fn bundle_options(jobs: i64) -> ScenarioBundleRunOptions {
    ScenarioBundleRunOptions {
        command_line: "godot-qa scenario bundle run gate --json".to_string(),
        jobs,
        ..Default::default()
    }
}

#[test]
fn scenario_bundle_only_selection_runs_the_named_children_at_their_expansion_index() {
    let root = temp_dir("bundle-only-selection");
    write_four_scenario_gate(&root);
    let cfg = make_config(&root);
    let ran = Mutex::new(Vec::new());

    let result = run_scenario_bundle_with_runner(
        &cfg,
        "gate",
        &ScenarioBundleRunOptions {
            selection: BundleSelection {
                // Deliberately out of manifest order: selection never reorders anything.
                only: vec!["delta".to_string(), "bravo".to_string()],
                failed_only: false,
            },
            ..bundle_options(1)
        },
        |c, _p, s, _cmd, _h, _a| {
            let name = s["name"].as_str().unwrap().to_string();
            ran.lock().unwrap().push(name.clone());
            child_result(&c.artifacts_dir.join(name), 1, true, vec![])
        },
        BundlePostRun::default,
    );

    assert!(result.ok, "{:#?}", result.failures);
    assert_eq!(result.exit_code, 0);
    assert_eq!(
        ran.lock().unwrap().as_slice(),
        ["bravo".to_string(), "delta".to_string()]
    );
    // The indices are the EXPANSION indices, not 0..selected_count: a filtered run must
    // stay addressable against the manifest it narrowed.
    let runs = result.data["runs"].as_array().unwrap();
    assert_eq!(runs.len(), 2);
    assert_eq!(runs[0]["index"], 1);
    assert_eq!(runs[0]["scenario"], "bravo");
    assert_eq!(runs[1]["index"], 3);
    assert_eq!(runs[1]["scenario"], "delta");
    assert_eq!(result.data["selection"]["mode"], "filtered");
    assert_eq!(result.data["selection"]["member_count"], 4);
    assert_eq!(result.data["selection"]["only"], json!(["delta", "bravo"]));
    assert_eq!(
        result.data["selection"]["selected_ids"],
        json!(["bravo", "delta"])
    );
    assert_eq!(
        result.message,
        "scenario bundle run completed (filtered: 2 of 4)"
    );
    // Parallelism is capped to what actually runs, which is what makes the warm skip
    // decision see the filtered count too.
    assert_eq!(result.data["parallelism"]["runnable_count"], 2);
}

#[test]
fn scenario_bundle_unfiltered_run_reports_selection_without_changing_the_message() {
    let root = temp_dir("bundle-selection-all");
    write_four_scenario_gate(&root);
    let cfg = make_config(&root);

    let result = run_scenario_bundle_with_runner(
        &cfg,
        "gate",
        &bundle_options(1),
        |c, _p, s, _cmd, _h, _a| {
            child_result(
                &c.artifacts_dir.join(s["name"].as_str().unwrap()),
                1,
                true,
                vec![],
            )
        },
        BundlePostRun::default,
    );

    assert!(result.ok, "{:#?}", result.failures);
    assert_eq!(result.message, "scenario bundle run completed");
    assert_eq!(result.data["selection"]["mode"], "all");
    assert_eq!(result.data["selection"]["failed_only"], false);
    assert_eq!(result.data["selection"]["source_run_root"], json!(null));
}

#[test]
fn scenario_bundle_unresolvable_selection_exits_two_and_writes_no_run_directory() {
    let root = temp_dir("bundle-selection-unresolvable");
    write_four_scenario_gate(&root);
    let cfg = make_config(&root);

    let unknown = run_scenario_bundle_with_runner(
        &cfg,
        "gate",
        &ScenarioBundleRunOptions {
            selection: BundleSelection {
                only: vec!["typo".to_string()],
                failed_only: false,
            },
            ..bundle_options(1)
        },
        |_c, _p, _s, _cmd, _h, _a| {
            unreachable!("no child may run when the selection cannot be resolved")
        },
        BundlePostRun::default,
    );

    assert!(!unknown.ok);
    assert_eq!(unknown.exit_code, 2);
    assert!(unknown.data.is_empty(), "{:#?}", unknown.data);
    assert_eq!(unknown.failures[0].kind, "selection_unknown_scenario");
    assert_eq!(
        unknown.failures[0].command.as_deref(),
        Some("scenario bundle run")
    );
    assert!(!root.join(".qa/artifacts/bundles/gate").exists());

    // Same shape when --failed-only has no baseline to read.
    let no_baseline = run_scenario_bundle_with_runner(
        &cfg,
        "gate",
        &ScenarioBundleRunOptions {
            selection: BundleSelection {
                only: Vec::new(),
                failed_only: true,
            },
            ..bundle_options(1)
        },
        |_c, _p, _s, _cmd, _h, _a| unreachable!("no child may run without a baseline"),
        BundlePostRun::default,
    );
    assert_eq!(no_baseline.exit_code, 2);
    assert_eq!(no_baseline.failures[0].kind, "selection_no_previous_run");
    assert!(!root.join(".qa/artifacts/bundles/gate").exists());
}

#[test]
fn scenario_bundle_empty_selection_warns_inside_the_artifacts_and_still_passes() {
    let root = temp_dir("bundle-selection-empty");
    write_four_scenario_gate(&root);
    // A green previous run: --failed-only therefore selects nothing.
    write_duration_history(
        &root,
        1000,
        &[
            ("alpha", 10.0),
            ("bravo", 10.0),
            ("charlie", 10.0),
            ("delta", 10.0),
        ],
    );
    let cfg = make_config(&root);

    let result = run_scenario_bundle_with_runner(
        &cfg,
        "gate",
        &ScenarioBundleRunOptions {
            selection: BundleSelection {
                only: Vec::new(),
                failed_only: true,
            },
            ..bundle_options(1)
        },
        |_c, _p, _s, _cmd, _h, _a| unreachable!("nothing was selected"),
        BundlePostRun::default,
    );

    assert!(result.ok);
    assert_eq!(result.exit_code, 0);
    assert_eq!(
        result.message,
        "scenario bundle run completed (filtered: 0 of 4)"
    );
    assert_eq!(result.data["runs"].as_array().unwrap().len(), 0);
    let warnings = result.data["warnings"].as_array().unwrap();
    assert!(
        warnings.iter().any(|w| w["type"] == "selection_empty"),
        "{warnings:#?}"
    );
    // The warning has to be in the persisted summary too: a run that executed nothing
    // must say so in its own artifacts, not only in the command JSON.
    let summary_path = result.data["artifacts"]["summary_path"].as_str().unwrap();
    let summary: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(summary_path).unwrap()).unwrap();
    assert!(
        summary["warnings"]
            .as_array()
            .unwrap()
            .iter()
            .any(|w| w["type"] == "selection_empty")
    );
    assert_eq!(summary["selection"]["mode"], "filtered");
    assert_eq!(summary["selection"]["failed_only"], true);
    let meta_path = result.data["artifacts"]["meta_path"].as_str().unwrap();
    let meta: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(meta_path).unwrap()).unwrap();
    assert_eq!(meta["selection"]["mode"], "filtered");
    assert_eq!(meta["selection"]["selected_ids"], json!([]));
}

/// Record dispatch order in pairs: with two lanes and a child that sleeps, the first two
/// recorded names are the first two dispatched (their order between each other races), and
/// the next two are the rest. Comparing sorted pairs keeps the assertion deterministic.
fn dispatch_pairs(order: &[String]) -> (Vec<String>, Vec<String>) {
    let mut first: Vec<String> = order[0..2].to_vec();
    let mut second: Vec<String> = order[2..4].to_vec();
    first.sort();
    second.sort();
    (first, second)
}

#[test]
fn scenario_bundle_longest_first_dispatches_the_history_slowest_children_first() {
    let root = temp_dir("bundle-schedule-longest-first");
    write_four_scenario_gate(&root);
    write_duration_history(
        &root,
        1000,
        &[
            ("alpha", 10.0),
            ("bravo", 400.0),
            ("charlie", 200.0),
            ("delta", 50.0),
        ],
    );
    let cfg = make_config(&root);
    let order = Mutex::new(Vec::new());

    let result = run_scenario_bundle_with_runner(
        &cfg,
        "gate",
        &bundle_options(2),
        |c, _p, s, _cmd, _h, _a| {
            let name = s["name"].as_str().unwrap().to_string();
            order.lock().unwrap().push(name.clone());
            // Long enough that both lanes pick up their first task before either
            // finishes, so the first two recorded names are the first two dispatched.
            thread::sleep(Duration::from_millis(60));
            child_result(&c.artifacts_dir.join(name), 1, true, vec![])
        },
        BundlePostRun::default,
    );

    assert!(result.ok, "{:#?}", result.failures);
    let (first, second) = dispatch_pairs(&order.lock().unwrap());
    assert_eq!(first, vec!["bravo".to_string(), "charlie".to_string()]);
    assert_eq!(second, vec!["alpha".to_string(), "delta".to_string()]);
    assert_eq!(
        result.data["scheduling"],
        json!({
            "policy": "longest_first",
            "origin": "default",
            "applied": true,
            "history_runs": 1,
            "predicted": 4,
            "unknown": 0,
        })
    );
    // Dispatch order is invisible in the result: runs stay in expansion index order.
    let runs = result.data["runs"].as_array().unwrap();
    let names: Vec<&str> = runs
        .iter()
        .map(|r| r["scenario"].as_str().unwrap())
        .collect();
    assert_eq!(names, ["alpha", "bravo", "charlie", "delta"]);
    let summary_path = result.data["artifacts"]["summary_path"].as_str().unwrap();
    let summary: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(summary_path).unwrap()).unwrap();
    assert_eq!(summary["scheduling"], result.data["scheduling"]);
}

#[test]
fn scenario_bundle_serial_and_manifest_runs_keep_plain_expansion_order() {
    let root = temp_dir("bundle-schedule-not-applied");
    write_four_scenario_gate(&root);
    write_duration_history(
        &root,
        1000,
        &[
            ("alpha", 10.0),
            ("bravo", 400.0),
            ("charlie", 200.0),
            ("delta", 50.0),
        ],
    );
    let cfg = make_config(&root);

    // --jobs 1: nothing to gain from reordering, so the history is never even read.
    let serial_order = Mutex::new(Vec::new());
    let serial = run_scenario_bundle_with_runner(
        &cfg,
        "gate",
        &bundle_options(1),
        |c, _p, s, _cmd, _h, _a| {
            let name = s["name"].as_str().unwrap().to_string();
            serial_order.lock().unwrap().push(name.clone());
            child_result(&c.artifacts_dir.join(name), 1, true, vec![])
        },
        BundlePostRun::default,
    );
    assert!(serial.ok, "{:#?}", serial.failures);
    assert_eq!(
        serial_order.lock().unwrap().as_slice(),
        [
            "alpha".to_string(),
            "bravo".to_string(),
            "charlie".to_string(),
            "delta".to_string()
        ]
    );
    assert_eq!(serial.data["scheduling"]["applied"], false);
    assert_eq!(serial.data["scheduling"]["history_runs"], 0);
    assert_eq!(serial.data["scheduling"]["policy"], "longest_first");

    // The manifest policy keeps expansion order even with lanes to spare.
    let manifest_order = Mutex::new(Vec::new());
    let manifest = run_scenario_bundle_with_runner(
        &cfg,
        "gate",
        &ScenarioBundleRunOptions {
            schedule: BundleSchedulePolicy::Manifest,
            schedule_origin: "flag".to_string(),
            ..bundle_options(2)
        },
        |c, _p, s, _cmd, _h, _a| {
            let name = s["name"].as_str().unwrap().to_string();
            manifest_order.lock().unwrap().push(name.clone());
            thread::sleep(Duration::from_millis(60));
            child_result(&c.artifacts_dir.join(name), 1, true, vec![])
        },
        BundlePostRun::default,
    );
    assert!(manifest.ok, "{:#?}", manifest.failures);
    let (first, second) = dispatch_pairs(&manifest_order.lock().unwrap());
    assert_eq!(first, vec!["alpha".to_string(), "bravo".to_string()]);
    assert_eq!(second, vec!["charlie".to_string(), "delta".to_string()]);
    assert_eq!(
        manifest.data["scheduling"],
        json!({
            "policy": "manifest",
            "origin": "flag",
            "applied": false,
            "history_runs": 0,
            "predicted": 0,
            "unknown": 0,
        })
    );
}

#[test]
#[cfg(unix)]
fn scenario_bundle_latest_link_follows_the_newest_run() {
    let root = temp_dir("bundle-latest-link");
    write_four_scenario_gate(&root);
    let cfg = make_config(&root);
    let run = || {
        run_scenario_bundle_with_runner(
            &cfg,
            "gate",
            &bundle_options(1),
            |c, _p, s, _cmd, _h, _a| {
                child_result(
                    &c.artifacts_dir.join(s["name"].as_str().unwrap()),
                    1,
                    true,
                    vec![],
                )
            },
            BundlePostRun::default,
        )
    };

    let first = run();
    let link = root.join(".qa/artifacts/bundles/gate/latest");
    assert_eq!(
        first.data["artifacts"]["latest_path"],
        json!(link.to_string_lossy())
    );
    let first_run_root = PathBuf::from(first.data["artifacts"]["run_root"].as_str().unwrap());
    // Relative, so the artifacts tree survives being copied or mounted elsewhere.
    assert_eq!(
        fs::read_link(&link).unwrap(),
        PathBuf::from(first_run_root.file_name().unwrap())
    );
    assert!(link.join("summary.json").exists());

    // A second run rotates the link rather than leaving it on the older directory.
    thread::sleep(Duration::from_millis(1100));
    let second = run();
    let second_run_root = PathBuf::from(second.data["artifacts"]["run_root"].as_str().unwrap());
    assert_ne!(first_run_root, second_run_root);
    assert_eq!(
        fs::read_link(&link).unwrap(),
        PathBuf::from(second_run_root.file_name().unwrap())
    );
}

/// Every abandoning return of `run_scenario_bundle_with_runner` must still run the runner's
/// post-run hook. The hook is how a runner tears down process-level resources it created
/// BEFORE the bundle was resolved — the warm pool is seeded before the call — so a return
/// that skips it leaks Godot processes that outlive the CLI. The published result is
/// unchanged: these shapes carry no `data`, so the hook's own evidence is still discarded.
#[test]
fn bundle_runner_post_run_hook_runs_on_every_abandoning_return() {
    // A listing failure (two manifests claiming one name), a name no manifest declares, and
    // an expansion failure (a bundle requiring itself) — the three returns that used to
    // abandon the runner's resources. Each gets its own project root, because a poisoned
    // listing is reported for every name and would mask the other two.
    let listing_failure = temp_dir("bundle-post-run-listing");
    write(
        &listing_failure.join("qa/gates/first_twin.json"),
        r#"{"name":"twin","required_scenarios":[]}"#,
    );
    write(
        &listing_failure.join("qa/gates/second_twin.json"),
        r#"{"name":"twin","required_scenarios":[]}"#,
    );
    let resolvable = temp_dir("bundle-post-run-resolvable");
    write(
        &resolvable.join("qa/gates/self_referential.json"),
        r#"{"name":"self_referential","required_bundles":["self_referential"],"required_scenarios":[]}"#,
    );

    for (root, name, expected_kind) in [
        (&listing_failure, "twin", "duplicate_bundle_name"),
        (&resolvable, "nonexistent_gate", "missing_bundle"),
        (&resolvable, "self_referential", "bundle_cycle_detected"),
    ] {
        let cfg = make_config(root);
        let torn_down = Arc::new(AtomicUsize::new(0));
        let hook_counter = Arc::clone(&torn_down);
        let result = run_scenario_bundle_with_runner(
            &cfg,
            name,
            &bundle_options(1),
            |_c, _p, _s, _cmd, _h, _a| panic!("no child may run for {name}"),
            move || {
                hook_counter.fetch_add(1, Ordering::SeqCst);
                BundlePostRun::default()
            },
        );

        assert!(!result.ok, "{name} must fail");
        assert_eq!(result.failures[0].kind, expected_kind, "{name}");
        assert_eq!(
            torn_down.load(Ordering::SeqCst),
            1,
            "{name} must run the post-run hook exactly once"
        );
        // The hook's evidence is still not published from these shapes.
        assert!(result.data.is_empty(), "{name}");
    }
}
