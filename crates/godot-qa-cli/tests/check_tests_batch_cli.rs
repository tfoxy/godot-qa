use serde_json::{Value, json};
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus};
use std::time::{SystemTime, UNIX_EPOCH};

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("repo root")
        .to_path_buf()
}

fn sample_project_root() -> PathBuf {
    repo_root().join("examples/sample_project")
}

fn unique_suffix() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or(0);
    format!("{nanos}-{}", std::process::id())
}

fn temp_project_root(prefix: &str) -> PathBuf {
    let root = env::temp_dir().join(format!("godot-qa-{prefix}-{}", unique_suffix()));
    fs::create_dir_all(root.join("qa/tests")).unwrap();
    fs::create_dir_all(root.join("qa/scenarios")).unwrap();
    fs::create_dir_all(root.join("addons/godot_qa")).unwrap();
    fs::write(root.join("project.godot"), "[application]\n").unwrap();
    fs::write(root.join("addons/godot_qa/plugin.cfg"), "[plugin]\n").unwrap();
    root
}

fn write(path: &Path, contents: &str) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(path, contents).unwrap();
}

fn run_godot_qa(args: &[&str]) -> (ExitStatus, Value) {
    let output = Command::new(env!("CARGO_BIN_EXE_godot-qa"))
        .args(args)
        .output()
        .unwrap();
    let body = serde_json::from_slice(&output.stdout).unwrap_or_else(|error| {
        panic!(
            "invalid stdout JSON: {error}\nstdout: {}\nstderr: {}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        )
    });
    (output.status, body)
}

fn artifact_root(test: &Value) -> PathBuf {
    PathBuf::from(test["artifacts"]["root"].as_str().expect("artifact root"))
}

fn read_jsonl(path: &Path) -> Vec<Value> {
    fs::read_to_string(path)
        .unwrap_or_else(|error| panic!("could not read {}: {error}", path.display()))
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
}

#[test]
fn check_tests_batch_cli_contract() {
    let project = sample_project_root();
    let (status, body) = run_godot_qa(&[
        "check",
        "--tests",
        "--batch",
        "--tag",
        "smoke",
        "--project",
        project.to_str().unwrap(),
        "--json",
    ]);

    assert!(
        status.success(),
        "batch smoke check failed: {}",
        serde_json::to_string_pretty(&body).unwrap()
    );
    assert_eq!(body["command"], "check");
    assert_eq!(body["ok"], true);
    assert_eq!(body["data"]["mode"], "tests");
    assert_eq!(body["data"]["execution_mode"], "batch");
    assert_eq!(body["data"]["selected_test_count"], 3);
    assert_eq!(body["data"]["checked_test_count"], 3);
    assert_eq!(body["data"]["skipped_test_count"], 3);
    assert_eq!(body["data"]["total_test_count"], 6);
    assert_eq!(
        body["data"]["selection"]["selected_ids"],
        json!([
            "main_menu_startup",
            "sample_script_test",
            "sample_script_state_test"
        ])
    );
    assert!(
        body["failures"].as_array().unwrap().is_empty(),
        "passing batch run should not flatten failures"
    );

    let tests = body["data"]["tests"].as_array().unwrap();
    let script_tests = tests
        .iter()
        .filter(|test| test["kind"] == "script")
        .collect::<Vec<_>>();
    assert!(
        script_tests.len() >= 2,
        "smoke batch should include at least two script tests: {tests:#?}"
    );
    for test in tests {
        assert_eq!(test["ok"], true, "selected test failed: {test:#}");
        assert!(test["failures"].as_array().unwrap().is_empty());
        assert!(test["artifacts"]["result"].is_string());
        assert!(test["artifacts"]["progress"].is_string());
        assert!(artifact_root(test).is_dir());
    }
    assert_ne!(
        artifact_root(tests.first().unwrap()),
        artifact_root(tests.get(1).unwrap())
    );

    let summary_path = PathBuf::from(
        body["data"]["artifacts"]["batch_summary"]
            .as_str()
            .expect("batch summary"),
    );
    assert!(summary_path.is_file(), "missing {}", summary_path.display());
    assert_eq!(
        body["data"]["batch"]["artifacts"]["summary"],
        body["data"]["artifacts"]["batch_summary"]
    );
    let summary: Value = serde_json::from_str(&fs::read_to_string(summary_path).unwrap()).unwrap();
    assert_eq!(summary["ok"], true);
    assert_eq!(summary["execution_mode"], "batch");
    assert_eq!(summary["test_count"], 3);
    assert_eq!(summary["process"]["returncode"], 0);
    assert!(
        summary["process"]["command"]
            .as_array()
            .unwrap()
            .iter()
            .any(|arg| arg
                .as_str()
                .unwrap_or_default()
                .starts_with("--qa-check-request=")),
        "batch summary should retain one-process request evidence"
    );
    assert!(summary["artifacts"]["progress"].is_string());
    let batch_progress = PathBuf::from(body["data"]["artifacts"]["progress"].as_str().unwrap());
    assert_eq!(summary["artifacts"]["progress"], json!(batch_progress));
    let batch_events = read_jsonl(&batch_progress);
    assert_eq!(batch_events.len(), tests.len() * 2);
    for test in tests {
        let test_progress = PathBuf::from(test["artifacts"]["progress"].as_str().unwrap());
        let test_events = read_jsonl(&test_progress);
        assert_eq!(test_events.len(), 2);
        assert_eq!(test_events[0]["event"], "start");
        assert_eq!(test_events[1]["event"], "finish");
        assert_eq!(test_events[0]["id"], test["id"]);
        assert_eq!(test_events[0]["kind"], test["kind"]);
        assert_eq!(test_events[0]["target"], test["target"]);
        assert!(test_events[0]["started_at"].is_string());
        assert!(test_events[1]["elapsed_ms"].as_u64().is_some());
        assert_eq!(test_events[1]["ok"], true);
        assert_eq!(test_events[1]["failure_count"], 0);
        assert_eq!(
            test_events[1]["artifacts"]["progress"],
            json!(test_progress)
        );
    }
}

#[test]
fn check_tests_batch_failure_policy_contract() {
    let project = temp_project_root("batch-failure-policy");
    write(
        &project.join("qa/tests/manifest.json"),
        r#"{
  "version": 1,
  "tests": [
    {
      "id": "script_fails",
      "kind": "script",
      "target": "res://qa/tests/script_fails.gd",
      "tags": ["smoke", "script"]
    },
    {
      "id": "script_later_passes",
      "kind": "script",
      "target": "res://qa/tests/script_later_passes.gd",
      "tags": ["smoke", "script"]
    }
  ]
}"#,
    );
    write(
        &project.join("qa/tests/script_fails.gd"),
        "extends RefCounted\n",
    );
    write(
        &project.join("qa/tests/script_later_passes.gd"),
        "extends RefCounted\n",
    );

    let failed_result = json!({
        "id": "script_fails",
        "kind": "script",
        "target": "res://qa/tests/script_fails.gd",
        "ok": false,
        "duration_ms": 4,
        "runtime_errors": [],
        "failures": [{
            "type": "assertion_failed",
            "message": "intentional fixture failure"
        }]
    })
    .to_string();
    let later_result = json!({
        "id": "script_later_passes",
        "kind": "script",
        "target": "res://qa/tests/script_later_passes.gd",
        "ok": true,
        "duration_ms": 5,
        "runtime_errors": [],
        "failures": [],
        "data": {"fixture": "later pass retained"}
    })
    .to_string();
    // A batch request lives in batch/request.json; each selected test retains
    // its own result beneath the sibling tests directory.
    let fake_godot = godot_qa_test_support::fixture(
        &project.join("fake-godot"),
        &[
            ("version", "4.3.test"),
            ("request_prefix", "--qa-check-request="),
            ("write:../tests/script-fails/result.json", &failed_result),
            (
                "write:../tests/script-later-passes/result.json",
                &later_result,
            ),
        ],
    );

    let (status, body) = run_godot_qa(&[
        "check",
        "--tests",
        "--batch",
        "--tag",
        "smoke",
        "--project",
        project.to_str().unwrap(),
        "--godot-bin",
        fake_godot.to_str().unwrap(),
        "--json",
    ]);

    assert!(!status.success(), "expected aggregate batch failure");
    assert_eq!(body["ok"], false);
    assert_eq!(body["data"]["execution_mode"], "batch");
    assert_eq!(body["data"]["checked_test_count"], 2);
    let tests = body["data"]["tests"].as_array().unwrap();
    assert_eq!(tests.len(), 2);
    assert_eq!(tests[0]["id"], "script_fails");
    assert_eq!(tests[0]["ok"], false);
    assert_eq!(tests[0]["failures"][0]["type"], "assertion_failed");
    assert_eq!(tests[1]["id"], "script_later_passes");
    assert_eq!(tests[1]["ok"], true);
    assert_eq!(tests[1]["data"]["fixture"], "later pass retained");

    let failures = body["failures"].as_array().unwrap();
    assert_eq!(failures.len(), 1);
    assert_eq!(failures[0]["target"], "script_fails");
    assert_eq!(failures[0]["type"], "assertion_failed");
    assert!(
        failures
            .iter()
            .all(|failure| failure["type"] != "godot_run_failed"),
        "per-test fixture failure should not collapse into a generic batch failure"
    );

    let summary_path = PathBuf::from(body["data"]["artifacts"]["batch_summary"].as_str().unwrap());
    let summary: Value = serde_json::from_str(&fs::read_to_string(summary_path).unwrap()).unwrap();
    assert_eq!(summary["ok"], false);
    assert_eq!(summary["test_count"], 2);
    assert_eq!(summary["process"]["returncode"], 0);
    assert_eq!(summary["tests"][1]["id"], "script_later_passes");
    assert_eq!(summary["tests"][1]["ok"], true);

    fs::remove_dir_all(project).unwrap();
}

#[test]
fn check_tests_profile_failure_contract() {
    let project = temp_project_root("profile-failure-policy");
    write(
        &project.join("qa/tests/manifest.json"),
        r#"{
  "version": 1,
  "tests": [
    {
      "id": "profiled_script_fails",
      "kind": "script",
      "target": "res://qa/tests/profiled_script_fails.gd",
      "tags": ["smoke", "script"]
    },
    {
      "id": "profiled_malformed_profile",
      "kind": "script",
      "target": "res://qa/tests/profiled_malformed_profile.gd",
      "tags": ["smoke", "script"]
    }
  ]
}"#,
    );
    write(
        &project.join("qa/tests/profiled_script_fails.gd"),
        "extends RefCounted\n",
    );
    write(
        &project.join("qa/tests/profiled_malformed_profile.gd"),
        "extends RefCounted\n",
    );

    let failed_result = json!({
        "id": "profiled_script_fails",
        "kind": "script",
        "target": "res://qa/tests/profiled_script_fails.gd",
        "ok": false,
        "duration_ms": 7,
        "runtime_errors": [],
        "failures": [{
            "type": "assertion_failed",
            "message": "intentional profiled fixture failure"
        }],
        "profile": {
            "frame_times_ms": [16.0, 17.0],
            "monitor_samples": [{"name": "time/fps", "value": 60.0}],
            "step_timings": [{"label": "run_qa_test", "duration_ms": 3}]
        }
    })
    .to_string();
    let malformed_result = json!({
        "id": "profiled_malformed_profile",
        "kind": "script",
        "target": "res://qa/tests/profiled_malformed_profile.gd",
        "ok": true,
        "duration_ms": 5,
        "runtime_errors": [],
        "failures": [],
        "profile": {"frame_times_ms": ["slow"]}
    })
    .to_string();
    // Each isolated subprocess emits only the result for its requested test.
    let fake_godot = godot_qa_test_support::fixture(
        &project.join("fake-godot"),
        &[
            ("version", "4.3.test"),
            ("request_prefix", "--qa-check-request="),
            (
                "write_match:profiled-script-fails:result.json",
                &failed_result,
            ),
            (
                "write_match:profiled-malformed-profile:result.json",
                &malformed_result,
            ),
        ],
    );

    let (status, body) = run_godot_qa(&[
        "check",
        "--tests",
        "--profile",
        "--tag",
        "smoke",
        "--project",
        project.to_str().unwrap(),
        "--godot-bin",
        fake_godot.to_str().unwrap(),
        "--json",
    ]);

    assert!(
        !status.success(),
        "profiled expected-failure run unexpectedly passed: {}",
        serde_json::to_string_pretty(&body).unwrap()
    );
    assert_eq!(body["ok"], false);
    assert_eq!(body["data"]["execution_mode"], "isolated");
    assert_eq!(body["data"]["checked_test_count"], 2);
    assert!(body["data"]["artifacts"]["profile"].is_string());
    assert!(PathBuf::from(body["data"]["artifacts"]["profile"].as_str().unwrap()).is_file());

    let tests = body["data"]["tests"].as_array().unwrap();
    assert_eq!(tests.len(), 2);
    assert_eq!(tests[0]["id"], "profiled_script_fails");
    assert_eq!(tests[0]["ok"], false);
    assert_eq!(tests[0]["failures"][0]["type"], "assertion_failed");
    assert_eq!(tests[1]["id"], "profiled_malformed_profile");
    assert_eq!(tests[1]["ok"], false);
    assert_eq!(tests[1]["failures"][0]["type"], "profile_result_invalid");

    let failures = body["failures"].as_array().unwrap();
    assert!(
        failures
            .iter()
            .any(|failure| failure["target"] == "profiled_script_fails"
                && failure["type"] == "assertion_failed"),
        "flattened assertion failure was not preserved: {failures:#?}"
    );
    let malformed = failures
        .iter()
        .find(|failure| failure["target"] == "profiled_malformed_profile")
        .expect("missing malformed profile failure");
    assert_eq!(malformed["type"], "profile_result_invalid");
    assert_eq!(
        malformed["message"],
        "profile.frame_times_ms must be an array of non-negative numbers"
    );
    assert!(malformed["artifacts"]["profile"].is_string());

    let profile_path = PathBuf::from(body["data"]["artifacts"]["profile"].as_str().unwrap());
    let profile: Value = serde_json::from_str(&fs::read_to_string(profile_path).unwrap()).unwrap();
    assert_eq!(profile["tests"][0]["id"], "profiled_script_fails");
    assert_eq!(profile["tests"][0]["ok"], false);
    assert_eq!(profile["tests"][1]["id"], "profiled_malformed_profile");
    assert_eq!(profile["tests"][1]["ok"], false);

    fs::remove_dir_all(project).unwrap();
}

#[test]
fn raw_runtime_diagnostics_cannot_hide_behind_successful_manifest_results() {
    let project = temp_project_root("raw-log-verdict");
    write(
        &project.join("qa/tests/manifest.json"),
        r#"{"version":1,"tests":[{"id":"a","kind":"script","target":"res://qa/tests/a.gd"},{"id":"b","kind":"script","target":"res://qa/tests/b.gd"}]}"#,
    );
    for id in ["a", "b"] {
        write(
            &project.join(format!("qa/tests/{id}.gd")),
            "extends RefCounted\n",
        );
    }
    for batch in [false, true] {
        for prefix in ["ERROR:", "SCRIPT ERROR:", "WARNING:", "SCRIPT WARNING:"] {
            let mut options = vec![
                ("version".to_string(), "4.3.test".to_string()),
                (
                    "request_prefix".to_string(),
                    "--qa-check-request=".to_string(),
                ),
                ("stdout".to_string(), "unterminated stdout".to_string()),
                (
                    "stderr".to_string(),
                    format!("{prefix} fixture raw diagnostic\n"),
                ),
            ];
            for id in ["a", "b"] {
                let result = json!({"id":id,"kind":"script","target":format!("res://qa/tests/{id}.gd"),"ok":true,"duration_ms":1,"runtime_errors":[],"failures":[]}).to_string();
                options.push((
                    if batch {
                        format!("write:../tests/{id}/result.json")
                    } else {
                        format!("write_match:{id}:result.json")
                    },
                    result,
                ));
            }
            let refs = options
                .iter()
                .map(|(key, value)| (key.as_str(), value.as_str()))
                .collect::<Vec<_>>();
            let fake = godot_qa_test_support::fixture(&project.join("fake-godot"), &refs);
            let mut args = vec![
                "check",
                "--tests",
                "--project",
                project.to_str().unwrap(),
                "--godot-bin",
                fake.to_str().unwrap(),
                "--json",
            ];
            if batch {
                args.push("--batch");
            }
            let (status, body) = run_godot_qa(&args);
            let warning = prefix.contains("WARNING");
            assert_eq!(
                status.success(),
                warning,
                "batch={batch}, {prefix}: {body:#}"
            );
            let tests = body["data"]["tests"].as_array().unwrap();
            assert_eq!(tests.len(), 2);
            if batch {
                assert!(
                    tests.iter().all(|test| test["ok"] == true),
                    "batch diagnostics must not fabricate child attribution: {body:#}"
                );
                let failures = body["failures"].as_array().unwrap();
                assert_eq!(failures.len(), usize::from(!warning));
                assert!(
                    failures
                        .iter()
                        .all(|failure| failure.get("target").is_none())
                );
                let summary: Value = serde_json::from_slice(
                    &fs::read(body["data"]["artifacts"]["batch_summary"].as_str().unwrap())
                        .unwrap(),
                )
                .unwrap();
                assert_eq!(
                    summary["logs"]["warnings"].as_array().unwrap().len(),
                    usize::from(warning)
                );
            } else {
                assert!(tests.iter().all(|test| test["ok"] == warning));
                assert!(
                    tests
                        .iter()
                        .all(|test| test["warnings"].as_array().unwrap().len()
                            == usize::from(warning))
                );
            }
            write(
                &project.join("qa/config.yaml"),
                "logs:\n  ignore: [fixture raw diagnostic]\n",
            );
            let (status, ignored) = run_godot_qa(&args);
            assert!(
                status.success(),
                "ignored diagnostic changed verdict: {ignored:#}"
            );
            fs::remove_file(project.join("qa/config.yaml")).unwrap();
        }
    }
    fs::remove_dir_all(project).unwrap();
}

#[test]
fn godot_script_error_fails_even_when_script_test_returns_success() {
    fn copy_tree(source: &Path, destination: &Path) {
        fs::create_dir_all(destination).unwrap();
        for entry in fs::read_dir(source).unwrap() {
            let entry = entry.unwrap();
            let target = destination.join(entry.file_name());
            if entry.file_type().unwrap().is_dir() {
                copy_tree(&entry.path(), &target);
            } else {
                fs::copy(entry.path(), target).unwrap();
            }
        }
    }
    let project = temp_project_root("real-script-error");
    copy_tree(
        &repo_root().join("addons/godot_qa"),
        &project.join("addons/godot_qa"),
    );
    write(
        &project.join("project.godot"),
        "config_version=5\n[application]\nconfig/name=\"Raw script error regression\"\n[rendering]\nrenderer/rendering_method=\"gl_compatibility\"\n",
    );
    write(
        &project.join("qa/tests/manifest.json"),
        r#"{"version":1,"tests":[{"id":"raw_error","kind":"script","target":"res://qa/tests/raw_error.gd"}]}"#,
    );
    write(
        &project.join("qa/tests/raw_error.gd"),
        "extends RefCounted\n\nfunc run_qa_test() -> Dictionary:\n\t_emit_error()\n\treturn {\"ok\": true, \"failures\": [], \"data\": {\"returned_success\": true}}\n\nfunc _emit_error() -> void:\n\tvar missing: Variant = null\n\tmissing.call_a_method_on_null()\n",
    );
    for batch in [false, true] {
        let mut args = vec![
            "check",
            "--tests",
            "--project",
            project.to_str().unwrap(),
            "--json",
        ];
        if batch {
            args.push("--batch");
        }
        let (status, body) = run_godot_qa(&args);
        assert!(
            !status.success(),
            "raw Godot script error escaped check: {body:#}"
        );
        let test = &body["data"]["tests"][0];
        assert_eq!(
            test["data"]["returned_success"], true,
            "fixture must reach its successful return: {body:#}"
        );
        assert!(
            body["failures"]
                .as_array()
                .unwrap()
                .iter()
                .any(|failure| failure["type"] == "godot_log_error"
                    && failure["actual"]
                        .as_str()
                        .is_some_and(|line| line.starts_with("SCRIPT ERROR:"))),
            "raw script diagnostic must be retained: {body:#}"
        );
    }
    fs::remove_dir_all(project).unwrap();
}
