use serde_json::Value;
use std::path::{Path, PathBuf};
use std::process::Command;

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

fn run_cli(args: &[&str]) -> (std::process::ExitStatus, Value) {
    let output = Command::new(env!("CARGO_BIN_EXE_godot-qa"))
        .arg("--json")
        .arg("--project")
        .arg(sample_project_root())
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

fn failure_kinds(failures: &Value) -> Vec<String> {
    failures
        .as_array()
        .unwrap_or(&Vec::new())
        .iter()
        .filter_map(|failure| failure["type"].as_str().map(str::to_string))
        .collect()
}

#[test]
fn scenario_frame_budget_exceeded_cold_is_a_clean_deterministic_failure() {
    let (status, body) = run_cli(&["scenario", "run", "frame_budget_demo"]);

    assert_eq!(status.code(), Some(1));
    assert_eq!(body["ok"], false);
    assert_eq!(body["command"], "scenario run");
    let kinds = failure_kinds(&body["failures"]);
    assert!(
        kinds.contains(&"scenario_frame_budget_exceeded".to_string()),
        "expected scenario_frame_budget_exceeded, got {kinds:?}"
    );
    // The blown budget is a clean engine exit, not a process kill: the wall
    // clock stayed a backstop and result.json was written normally.
    assert!(
        !kinds.contains(&"scenario_run_timeout".to_string()),
        "frame budget must not surface as a wall-clock timeout: {kinds:?}"
    );
    assert_eq!(body["data"]["process"]["timed_out"], false);
    assert_eq!(body["data"]["runtime_result"]["ok"], false);
    let budget_failure = body["failures"]
        .as_array()
        .unwrap()
        .iter()
        .find(|failure| failure["type"] == "scenario_frame_budget_exceeded")
        .unwrap();
    assert_eq!(budget_failure["actual"]["budget_frames"], 60);
    assert!(
        budget_failure["actual"]["consumed_frames"]
            .as_i64()
            .unwrap()
            >= 60
    );
}

#[test]
fn scenario_frame_budget_exceeded_warm_bundle_reports_the_same_kind() {
    // --jobs 1 with two scenarios: a warm session is only reused when the
    // scenario count exceeds parallelism, and reuse is the point of the test.
    let (status, body) = run_cli(&[
        "scenario",
        "bundle",
        "run",
        "frame_budget_demo_gate",
        "--warm",
        "--jobs",
        "1",
    ]);

    assert_eq!(status.code(), Some(1));
    assert_eq!(body["ok"], false);
    let kinds = failure_kinds(&body["failures"]);
    assert!(
        kinds.contains(&"scenario_frame_budget_exceeded".to_string()),
        "expected scenario_frame_budget_exceeded, got {kinds:?}"
    );
    assert!(
        !kinds.contains(&"scenario_run_timeout".to_string()),
        "frame budget must not surface as a wall-clock timeout: {kinds:?}"
    );
    let run = &body["data"]["runs"][0];
    assert_eq!(run["scenario"], "frame_budget_demo");
    assert_eq!(run["ok"], false);
    // Warm and cold share the runner, so the warm lane reports the identical
    // deterministic kind instead of tripping the transport wall clock.
    assert_eq!(
        run["warm"]["ran_warm"], true,
        "expected a warm run, got {:?}",
        run["warm"]
    );
    let run_kinds = failure_kinds(&run["failures"]);
    assert!(
        run_kinds.contains(&"scenario_frame_budget_exceeded".to_string()),
        "expected scenario_frame_budget_exceeded in the child run, got {run_kinds:?}"
    );
}
