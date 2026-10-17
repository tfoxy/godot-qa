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

fn run_sample_scenario(name: &str) -> (std::process::ExitStatus, Value) {
    let output = Command::new(env!("CARGO_BIN_EXE_godot-qa"))
        .arg("scenario")
        .arg("run")
        .arg(name)
        .arg("--project")
        .arg(sample_project_root())
        .arg("--json")
        .output()
        .unwrap();
    let body = serde_json::from_slice(&output.stdout).unwrap_or_else(|error| {
        panic!(
            "invalid stdout JSON for {name}: {error}\nstdout: {}\nstderr: {}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        )
    });
    (output.status, body)
}

#[test]
fn temporal_observe_until_failure_contract() {
    let (status, body) = run_sample_scenario("temporal_observe_until_late_failure_demo");

    assert!(
        !status.success(),
        "late observe-until scenario should fail: {}",
        serde_json::to_string_pretty(&body).unwrap()
    );
    assert_eq!(body["ok"], false);
    assert_eq!(body["command"], "scenario run");

    let failure = body["failures"]
        .as_array()
        .and_then(|failures| {
            failures
                .iter()
                .find(|failure| failure["type"] == "observe_until_mismatch")
        })
        .unwrap_or_else(|| {
            panic!(
                "missing observe_until_mismatch failure: {}",
                serde_json::to_string_pretty(&body["failures"]).unwrap()
            )
        });

    assert_eq!(failure["target"], "qa:main_menu");
    assert_eq!(failure["condition"]["type"], "state_matches");
    assert_eq!(failure["condition"]["matches"][0]["path"], "status");
    assert_eq!(failure["condition"]["matches"][0]["expected"], "pending");
    assert_eq!(failure["window"]["frames"], 1);
    assert_eq!(failure["sample_every_frames"], 1);
    assert!(
        failure["observed_samples"]
            .as_array()
            .is_some_and(|samples| !samples.is_empty()),
        "failure should include observed samples: {}",
        serde_json::to_string_pretty(failure).unwrap()
    );
    assert_eq!(failure["first_late_sample"]["matched"], true);
    assert_eq!(
        failure["first_late_sample"]["actual"]["matched"][0]["actual"],
        "pending"
    );
    assert_eq!(failure["why_no_match"], "matched_after_window");
    assert!(
        failure["artifacts"]["temporal_evidence"]
            .as_str()
            .is_some_and(|path| path.ends_with(".json")),
        "missing temporal evidence artifact path: {}",
        serde_json::to_string_pretty(failure).unwrap()
    );
    assert!(
        failure["artifacts"]["temporal_dir"]
            .as_str()
            .is_some_and(|path| !path.is_empty()),
        "missing temporal artifact directory: {}",
        serde_json::to_string_pretty(failure).unwrap()
    );
}
