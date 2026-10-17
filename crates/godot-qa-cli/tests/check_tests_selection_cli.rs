use serde_json::{Value, json};
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus};

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

fn run_sample_check_tests(extra_args: &[&str]) -> (ExitStatus, Value) {
    let project = sample_project_root();
    let mut args = vec!["check", "--tests"];
    args.extend_from_slice(extra_args);
    args.extend_from_slice(["--project", project.to_str().unwrap(), "--json"].as_slice());
    run_godot_qa(&args)
}

fn selected_ids(body: &Value) -> Vec<String> {
    body["data"]["tests"]
        .as_array()
        .unwrap()
        .iter()
        .map(|test| test["id"].as_str().unwrap().to_string())
        .collect()
}

fn assert_successful_selection(
    body: &Value,
    expected_ids: &[&str],
    expected_mode: &str,
    expected_skipped: u64,
) {
    assert_eq!(body["command"], "check");
    assert_eq!(body["ok"], true);
    assert_eq!(body["data"]["mode"], "tests");
    assert_eq!(
        body["data"]["selected_test_count"],
        expected_ids.len() as u64
    );
    assert_eq!(
        body["data"]["checked_test_count"],
        expected_ids.len() as u64
    );
    assert_eq!(body["data"]["skipped_test_count"], expected_skipped);
    assert_eq!(body["data"]["total_test_count"], 6);
    assert_eq!(body["data"]["selection"]["mode"], expected_mode);
    assert_eq!(selected_ids(body), expected_ids);
    assert!(
        body["failures"].as_array().unwrap().is_empty(),
        "unexpected flattened failures: {}",
        serde_json::to_string_pretty(&body["failures"]).unwrap()
    );
    for test in body["data"]["tests"].as_array().unwrap() {
        assert_eq!(test["ok"], true, "selected test failed: {test:#}");
        assert!(
            test["failures"].as_array().unwrap().is_empty(),
            "per-test failures should remain empty for passing tests"
        );
    }
}

fn read_jsonl(path: &Path) -> Vec<Value> {
    fs::read_to_string(path)
        .unwrap_or_else(|error| panic!("could not read {}: {error}", path.display()))
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
}

fn profile_artifact(body: &Value) -> Value {
    let profile_path = PathBuf::from(body["data"]["artifacts"]["profile"].as_str().unwrap());
    assert!(profile_path.is_file(), "missing {}", profile_path.display());
    serde_json::from_str(&fs::read_to_string(profile_path).unwrap()).unwrap()
}

fn assert_profile_contract(body: &Value, expected_ids: &[&str]) -> Value {
    assert!(
        body["data"]["profile"].is_object(),
        "missing profile summary"
    );
    assert_eq!(
        body["data"]["profile"]["artifacts"]["profile"],
        body["data"]["artifacts"]["profile"]
    );
    assert!(
        body["data"]["profile"]["frame_time_ms"]["sample_count"]
            .as_u64()
            .is_some()
    );
    assert!(
        body["data"]["profile"]["per_test_timing_count"]
            .as_u64()
            .unwrap()
            >= expected_ids.len() as u64
    );
    assert!(
        body["data"]["profile"]["per_test_timings"]
            .as_array()
            .unwrap()
            .len()
            <= body["data"]["profile"]["sample_limits"]["max_frame_samples"]
                .as_u64()
                .unwrap() as usize
    );

    let artifact = profile_artifact(body);
    assert_eq!(artifact["selection"]["selected_ids"], json!(expected_ids));
    assert_eq!(
        artifact["tests"].as_array().unwrap().len(),
        expected_ids.len()
    );
    assert_eq!(
        artifact["tests"]
            .as_array()
            .unwrap()
            .iter()
            .map(|test| test["id"].as_str().unwrap())
            .collect::<Vec<_>>(),
        expected_ids
    );
    assert_eq!(
        artifact["sample_limits"]["max_frame_samples"],
        body["data"]["profile"]["sample_limits"]["max_frame_samples"]
    );
    assert!(
        artifact["frame_times_ms"].as_array().unwrap().len()
            <= artifact["sample_limits"]["max_frame_samples"]
                .as_u64()
                .unwrap() as usize
    );
    assert!(
        artifact["per_test_timings"]
            .as_array()
            .unwrap()
            .iter()
            .any(|timing| timing["label"] == "check_test_total")
    );
    artifact
}

#[test]
fn check_tests_selection_cli_contract() {
    let (status, body) = run_sample_check_tests(&["--test", "sample_script_test"]);
    assert!(
        status.success(),
        "explicit test selection failed: {}",
        serde_json::to_string_pretty(&body).unwrap()
    );
    assert_successful_selection(&body, &["sample_script_test"], "filtered", 5);
    assert_eq!(
        body["data"]["selection"]["tests"],
        json!(["sample_script_test"])
    );
    assert_eq!(body["data"]["selection"]["tags"], json!([]));
    assert_eq!(body["data"]["selection"]["profile"], Value::Null);
    assert_eq!(
        body["data"]["selection"]["selected_ids"],
        json!(["sample_script_test"])
    );
    assert_eq!(body["data"]["tests"][0]["tags"], json!(["smoke", "script"]));
    let run_progress = PathBuf::from(body["data"]["artifacts"]["progress"].as_str().unwrap());
    let test_progress = PathBuf::from(
        body["data"]["tests"][0]["artifacts"]["progress"]
            .as_str()
            .unwrap(),
    );
    assert!(run_progress.is_file(), "missing {}", run_progress.display());
    assert!(
        test_progress.is_file(),
        "missing {}",
        test_progress.display()
    );
    let run_events = read_jsonl(&run_progress);
    let test_events = read_jsonl(&test_progress);
    assert_eq!(test_events.len(), 2);
    assert_eq!(test_events[0]["event"], "start");
    assert_eq!(test_events[1]["event"], "finish");
    assert_eq!(test_events[0]["id"], "sample_script_test");
    assert_eq!(test_events[0]["kind"], "script");
    assert_eq!(test_events[0]["target"], body["data"]["tests"][0]["target"]);
    assert!(test_events[0]["started_at"].is_string());
    assert_eq!(test_events[0]["elapsed_ms"], Value::Null);
    assert!(test_events[1]["elapsed_ms"].as_u64().is_some());
    assert_eq!(test_events[1]["ok"], true);
    assert_eq!(test_events[1]["failure_count"], 0);
    assert_eq!(
        test_events[1]["artifacts"]["progress"],
        json!(test_progress)
    );
    assert_eq!(run_events[0]["id"], "sample_script_test");

    let (status, body) = run_sample_check_tests(&["--tag", "smoke"]);
    assert!(
        status.success(),
        "tag selection failed: {}",
        serde_json::to_string_pretty(&body).unwrap()
    );
    assert_successful_selection(
        &body,
        &[
            "main_menu_startup",
            "sample_script_test",
            "sample_script_state_test",
        ],
        "filtered",
        3,
    );
    assert_eq!(body["data"]["selection"]["tests"], json!([]));
    assert_eq!(body["data"]["selection"]["tags"], json!(["smoke"]));
    assert_eq!(body["data"]["selection"]["profile"], Value::Null);
    assert_eq!(
        body["data"]["selection"]["selected_ids"],
        json!([
            "main_menu_startup",
            "sample_script_test",
            "sample_script_state_test"
        ])
    );

    let (status, body) = run_sample_check_tests(&["--profile", "smoke"]);
    assert!(
        status.success(),
        "profile selection failed: {}",
        serde_json::to_string_pretty(&body).unwrap()
    );
    assert_successful_selection(
        &body,
        &[
            "main_menu_startup",
            "sample_script_test",
            "sample_script_state_test",
        ],
        "filtered",
        3,
    );
    assert_eq!(body["data"]["selection"]["profile"], json!("smoke"));
    assert_eq!(body["data"]["selection"]["profile_tags"], json!(["smoke"]));
    assert_eq!(body["data"]["selection"]["profile_tests"], json!([]));
    assert_eq!(
        body["data"]["selection"]["selected_ids"],
        json!([
            "main_menu_startup",
            "sample_script_test",
            "sample_script_state_test"
        ])
    );

    let (status, body) = run_sample_check_tests(&[]);
    assert!(
        status.success(),
        "unfiltered check failed: {}",
        serde_json::to_string_pretty(&body).unwrap()
    );
    assert_eq!(body["command"], "check");
    assert_eq!(body["ok"], true);
    assert_eq!(body["data"]["mode"], "tests");
    assert_eq!(body["data"]["selected_test_count"], 6);
    assert_eq!(body["data"]["checked_test_count"], 6);
    assert_eq!(body["data"]["skipped_test_count"], 0);
    assert_eq!(body["data"]["total_test_count"], 6);
    assert_eq!(body["data"]["selection"]["mode"], "all");
    assert_eq!(
        body["data"]["selection"]["selected_ids"],
        json!([
            "main_menu_startup",
            "accelerated_main_menu_startup",
            "sample_script_test",
            "sample_script_state_test",
            "slow_manifest_test",
            "engine_drift_manifest_test"
        ])
    );
    assert_eq!(body["failures"], json!([]));
}

#[test]
fn check_tests_profile_cli_contract() {
    let (status, body) = run_sample_check_tests(&["--profile", "--test", "sample_script_test"]);
    assert!(
        status.success(),
        "profiled explicit test selection failed: {}",
        serde_json::to_string_pretty(&body).unwrap()
    );
    assert_successful_selection(&body, &["sample_script_test"], "filtered", 5);
    assert_eq!(body["data"]["tests"][0]["ok"], true);
    assert_eq!(
        body["data"]["selection"]["selected_ids"],
        json!(["sample_script_test"])
    );
    let artifact = assert_profile_contract(&body, &["sample_script_test"]);
    assert!(
        artifact["per_test_timings"]
            .as_array()
            .unwrap()
            .iter()
            .any(|timing| timing["test_id"] == "sample_script_test")
    );

    let (status, body) = run_sample_check_tests(&["--profile", "--tag", "smoke"]);
    assert!(
        status.success(),
        "profiled smoke tag selection failed: {}",
        serde_json::to_string_pretty(&body).unwrap()
    );
    assert_successful_selection(
        &body,
        &[
            "main_menu_startup",
            "sample_script_test",
            "sample_script_state_test",
        ],
        "filtered",
        3,
    );
    assert_eq!(body["data"]["selection"]["tags"], json!(["smoke"]));
    assert_eq!(
        body["data"]["selection"]["selected_ids"],
        json!([
            "main_menu_startup",
            "sample_script_test",
            "sample_script_state_test"
        ])
    );
    assert_profile_contract(
        &body,
        &[
            "main_menu_startup",
            "sample_script_test",
            "sample_script_state_test",
        ],
    );

    let (status, body) = run_sample_check_tests(&[
        "--profile",
        "--test",
        "slow_manifest_test",
        "--timeout-seconds",
        "5",
    ]);
    assert!(
        status.success(),
        "profiled slow test failed: {}",
        serde_json::to_string_pretty(&body).unwrap()
    );
    assert_successful_selection(&body, &["slow_manifest_test"], "filtered", 5);
    let slow_tests = body["data"]["profile"]["slow_tests"].as_array().unwrap();
    assert_eq!(slow_tests[0]["id"], "slow_manifest_test");
    assert_eq!(slow_tests[0]["ok"], true);
    assert!(
        body["data"]["profile"]["per_test_timings"]
            .as_array()
            .unwrap()
            .iter()
            .any(|timing| timing["test_id"] == "slow_manifest_test")
    );
    let artifact = assert_profile_contract(&body, &["slow_manifest_test"]);
    assert!(
        artifact["per_test_timings"]
            .as_array()
            .unwrap()
            .iter()
            .any(|timing| timing["test_id"] == "slow_manifest_test")
    );
}

#[test]
fn check_tests_selection_negative_filter_contract() {
    for (name, args, expected_target, expected_message) in [
        (
            "unknown test id",
            vec!["--test", "missing_test"],
            "missing_test",
            "check --tests --test value must match a manifest test id",
        ),
        (
            "unknown tag",
            vec!["--tag", "missing"],
            "missing",
            "check --tests --tag value must match at least one manifest test tag",
        ),
        (
            "unknown profile",
            vec!["--profile", "missing"],
            "missing",
            "check --tests profile must exist in qa/tests/manifest.json",
        ),
        (
            "duplicate tag request",
            vec!["--tag", "smoke", "--tag", " smoke "],
            "smoke",
            "check --tests --tag values must not contain duplicates",
        ),
        (
            "empty tag request",
            vec!["--tag", " "],
            "",
            "check --tests --tag values must be non-empty strings",
        ),
    ] {
        let (status, body) = run_sample_check_tests(&args);
        assert!(!status.success(), "{name} unexpectedly passed");
        assert_eq!(body["command"], "check", "{name}");
        assert_eq!(body["ok"], false, "{name}");
        assert_eq!(body["data"]["selected_test_count"], 0, "{name}");
        assert_eq!(body["data"]["skipped_test_count"], 0, "{name}");
        assert_eq!(body["data"]["total_test_count"], 6, "{name}");
        assert_eq!(body["data"]["selection"]["mode"], "invalid", "{name}");
        assert_eq!(body["data"]["tests"], json!([]), "{name}");
        assert_eq!(
            body["failures"][0]["type"], "test_selection_invalid",
            "{name}"
        );
        if expected_target.is_empty() {
            assert_eq!(body["failures"][0]["target"], Value::Null, "{name}");
        } else {
            assert_eq!(body["failures"][0]["target"], expected_target, "{name}");
        }
        assert_eq!(body["failures"][0]["message"], expected_message, "{name}");
    }
}
