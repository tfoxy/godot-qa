use serde_json::Value;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Instant;

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

fn run_godot_qa(args: &[&str]) -> (std::process::ExitStatus, Value, u128) {
    let started = Instant::now();
    let output = Command::new(env!("CARGO_BIN_EXE_godot-qa"))
        .args(args)
        .output()
        .unwrap();
    let elapsed_ms = started.elapsed().as_millis();
    let body = serde_json::from_slice(&output.stdout).unwrap_or_else(|error| {
        panic!(
            "invalid stdout JSON: {error}\nstdout: {}\nstderr: {}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        )
    });
    (output.status, body, elapsed_ms)
}

fn assert_restored_time_control(time_control: &Value, expected_time_scale: f64) {
    assert_eq!(time_control["requested"]["time_scale"], expected_time_scale);
    assert_eq!(time_control["effective"]["time_scale"], expected_time_scale);
    assert_eq!(time_control["restoration_status"], "restored");
    assert_eq!(time_control["restored_matches_original"], true);
    assert_eq!(time_control["restored"], time_control["original"]);
}

#[test]
fn fastest_frame_pacing_demo_cli_contract() {
    let project = sample_project_root();
    let (status, body, elapsed_ms) = run_godot_qa(&[
        "scenario",
        "run",
        "fastest_frame_pacing_demo",
        "--project",
        project.to_str().unwrap(),
        "--json",
    ]);

    assert!(
        status.success(),
        "scenario failed with body: {}",
        serde_json::to_string_pretty(&body).unwrap()
    );
    assert_eq!(body["command"], "scenario run");
    assert_eq!(body["ok"], true);
    assert_eq!(body["data"]["scenario"], "fastest_frame_pacing_demo");
    assert!(
        elapsed_ms < 12_000,
        "fastest frame pacing scenario took {elapsed_ms}ms wall-clock"
    );
    assert_eq!(body["data"]["frame_pacing"]["mode"], "fastest");
    assert_eq!(body["data"]["frame_pacing"]["fixed_fps"], 60);
    assert_eq!(body["data"]["frame_pacing"]["source"], "godot_default");
    assert!(body["data"].get("time_control").is_none());

    let command = body["data"]["process"]["command"].as_array().unwrap();
    assert!(command.iter().any(|arg| arg == "--fixed-fps"));
    assert!(command.iter().any(|arg| arg == "--disable-vsync"));
}

#[test]
fn accelerated_timer_demo_cli_contract() {
    let project = sample_project_root();
    let (status, body, elapsed_ms) = run_godot_qa(&[
        "scenario",
        "run",
        "accelerated_timer_demo",
        "--project",
        project.to_str().unwrap(),
        "--json",
    ]);

    assert!(
        status.success(),
        "scenario failed with body: {}",
        serde_json::to_string_pretty(&body).unwrap()
    );
    assert_eq!(body["command"], "scenario run");
    assert_eq!(body["ok"], true);
    assert_eq!(body["data"]["scenario"], "accelerated_timer_demo");
    assert!(
        elapsed_ms < 12_000,
        "accelerated scenario took {elapsed_ms}ms wall-clock"
    );
    assert_restored_time_control(&body["data"]["time_control"], 8.0);

    let temporal_path = Path::new(body["data"]["artifacts"]["run_root"].as_str().unwrap())
        .join("temporal/observe_until_qa_main_menu.json");
    assert!(
        temporal_path.is_file(),
        "missing temporal evidence at {}",
        temporal_path.display()
    );
    let temporal: Value = serde_json::from_str(&fs::read_to_string(&temporal_path).unwrap())
        .expect("temporal evidence JSON");
    assert_eq!(
        temporal["first_matching_sample"]["actual"]["matched"][0]["actual"],
        "complete"
    );
    assert_eq!(temporal["first_matching_sample"]["matched"], true);
}

#[test]
fn check_tests_time_control_cli_contract() {
    let project = sample_project_root();
    let (status, body, _elapsed_ms) = run_godot_qa(&[
        "check",
        "--tests",
        "--test",
        "accelerated_main_menu_startup",
        "--project",
        project.to_str().unwrap(),
        "--json",
    ]);

    assert!(
        status.success(),
        "check --tests failed with body: {}",
        serde_json::to_string_pretty(&body).unwrap()
    );
    assert_eq!(body["command"], "check");
    assert_eq!(body["ok"], true);
    assert_eq!(body["data"]["selected_test_count"], 1);
    assert_eq!(body["data"]["skipped_test_count"], 5);
    assert_eq!(body["data"]["total_test_count"], 6);
    assert_eq!(body["data"]["selection"]["mode"], "filtered");
    let tests = body["data"]["tests"].as_array().unwrap();
    let accelerated = tests
        .iter()
        .find(|test| test["id"] == "accelerated_main_menu_startup")
        .expect("accelerated manifest test");
    assert_eq!(accelerated["ok"], true);
    assert_eq!(
        accelerated["tags"],
        serde_json::json!(["physics", "time-control", "ui"])
    );
    assert_restored_time_control(&accelerated["time_control"], 4.0);
}
