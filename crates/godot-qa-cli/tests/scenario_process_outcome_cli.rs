use serde_json::Value;
use std::fs;
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

fn copy_project(source: &Path, target: &Path) {
    fs::create_dir_all(target).unwrap();
    for entry in fs::read_dir(source).unwrap() {
        let entry = entry.unwrap();
        let name = entry.file_name();
        if name == ".qa" || name == ".godot" || name == "config.local.yaml" {
            continue;
        }
        if entry.file_type().unwrap().is_dir() {
            copy_project(&entry.path(), &target.join(name));
        } else {
            fs::copy(entry.path(), target.join(name)).unwrap();
        }
    }
}

struct TestProject(PathBuf);
impl Drop for TestProject {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn run_sample_scenario(name: &str) -> (std::process::ExitStatus, Value, TestProject) {
    let unique = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let project = TestProject(std::env::temp_dir().join(format!(
        "godot qa process outcome {} {unique}",
        std::process::id()
    )));
    copy_project(&sample_project_root(), &project.0);
    let fixture = sample_project_root().join(format!(
        ".qa/tools/qa-process-fixture{}",
        std::env::consts::EXE_SUFFIX
    ));
    assert!(
        fixture.is_file(),
        "run cargo xtask prepare-sample before runtime tests"
    );
    fs::write(
        project.0.join("qa/config.local.yaml"),
        serde_json::to_vec(
            &serde_json::json!({"godot": {"launch_prefix": [fixture, "process-wrapper"]}}),
        )
        .unwrap(),
    )
    .unwrap();
    let godot = std::env::var("GODOT_BIN").unwrap_or_else(|_| "godot".to_string());
    let import = Command::new(godot)
        .args(["--headless", "--path"])
        .arg(&project.0)
        .arg("--import")
        .output()
        .unwrap();
    assert!(
        import.status.success(),
        "sample import failed: {}",
        String::from_utf8_lossy(&import.stderr)
    );
    let output = Command::new(env!("CARGO_BIN_EXE_godot-qa"))
        .arg("scenario")
        .arg("run")
        .arg(name)
        .arg("--project")
        .arg(&project.0)
        .arg("--json")
        .output()
        .unwrap();
    let body = serde_json::from_slice(&output.stdout).unwrap_or_else(|error| {
        panic!(
            "invalid stdout JSON: {error}\nstdout: {}\nstderr: {}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        )
    });
    (output.status, body, project)
}

#[test]
fn scenario_process_outcome_cli_strict_wrapper_exit_fails_with_stable_json() {
    let (status, body, _project) = run_sample_scenario("process_exit_strict_failure_demo");

    assert_eq!(status.code(), Some(1));
    assert_eq!(body["ok"], false);
    assert_eq!(body["command"], "scenario run");
    assert_eq!(body["message"], "scenario run failed");
    assert_eq!(body["failures"][0]["type"], "godot_run_failed");
    assert_eq!(body["data"]["process"]["status"], "failed");
    assert_eq!(body["data"]["process"]["exit_code"], 7);
    assert_eq!(body["data"]["runtime_result"]["status"], "passed");
    assert_eq!(body["data"]["runtime_result"]["ok"], true);
    assert_eq!(body["data"]["runtime_result"]["failure_count"], 0);
    assert_eq!(body["data"]["logs"]["status"], "clean");
    assert_eq!(
        body["data"]["outcome"],
        serde_json::json!({
            "status": "fail",
            "reason": "godot_run_failed",
            "failure_category": "process",
            "retryable": false
        })
    );
}

#[test]
fn scenario_process_outcome_cli_known_wrapper_exit_passes_with_warning_json() {
    let (status, body, _project) = run_sample_scenario("process_exit_warning_policy_demo");

    assert!(status.success());
    assert_eq!(body["ok"], true);
    assert_eq!(body["command"], "scenario run");
    assert_eq!(body["message"], "scenario run passed with warning");
    assert_eq!(body["failures"], serde_json::json!([]));
    assert_eq!(body["data"]["process"]["status"], "failed");
    assert_eq!(body["data"]["process"]["exit_code"], 7);
    assert_eq!(body["data"]["runtime_result"]["status"], "passed");
    assert_eq!(body["data"]["runtime_result"]["ok"], true);
    assert_eq!(body["data"]["runtime_result"]["failure_count"], 0);
    assert_eq!(body["data"]["logs"]["status"], "clean");
    assert_eq!(body["data"]["outcome"]["status"], "pass_with_warning");
    assert_eq!(body["data"]["outcome"]["reason"], "godot_run_failed");
    assert_eq!(body["data"]["outcome"]["ok"], true);
    assert_eq!(body["data"]["outcome"]["exit_code"], 0);

    let warnings = body["data"]["process"]["warnings"].as_array().unwrap();
    assert_eq!(warnings.len(), 1);
    assert_eq!(warnings[0]["type"], "godot_run_failed");
    assert_eq!(warnings[0]["severity"], "warning");
    assert_eq!(warnings[0]["actual"], 7);
}

#[test]
fn scenario_process_outcome_cli_wrapper_is_transparent_for_ordinary_scenarios() {
    let (status, body, _project) = run_sample_scenario("sample_setup_steps");

    assert!(status.success());
    assert_eq!(body["ok"], true);
    assert_eq!(body["command"], "scenario run");
    assert_eq!(body["message"], "scenario run passed");
    assert_eq!(body["failures"], serde_json::json!([]));
    assert_eq!(body["data"]["process"]["status"], "passed");
    assert_eq!(body["data"]["process"]["exit_code"], 0);
    assert_eq!(body["data"]["runtime_result"]["status"], "passed");
    assert_eq!(body["data"]["logs"]["status"], "clean");
    assert_eq!(body["data"]["progress"]["exists"], true);
    assert!(body["data"]["progress"]["record_count"].as_u64().unwrap() > 0);
    let progress_path = body["data"]["progress"]["path"].as_str().unwrap();
    assert_eq!(body["data"]["artifacts"]["progress"], progress_path);
    let progress = fs::read_to_string(progress_path).unwrap();
    assert!(progress.lines().any(|line| {
        let record: Value = serde_json::from_str(line).unwrap();
        record["event"] == "start" && record["operation"] == "step"
    }));
    assert!(progress.lines().any(|line| {
        let record: Value = serde_json::from_str(line).unwrap();
        record["event"] == "finish" && record["operation"] == "assertion"
    }));
    assert_eq!(
        body["data"]["outcome"],
        serde_json::json!({
            "status": "pass",
            "reason": null
        })
    );
}
