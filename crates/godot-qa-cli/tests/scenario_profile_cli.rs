use serde_json::{Value, json};
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

// These tests share one imported Godot project and shader cache on CI runners.
static SAMPLE_PROJECT_RUN_LOCK: Mutex<()> = Mutex::new(());

fn unique_suffix() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or(0);
    format!("{nanos}-{}", std::process::id())
}

fn make_project_root(prefix: &str) -> PathBuf {
    let root = env::temp_dir().join(format!("godot-qa-{prefix}-{}", unique_suffix()));
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

fn run_scenario_profile(
    project_root: &Path,
    name: &str,
    extra_args: &[&str],
) -> (std::process::ExitStatus, Value) {
    let output = Command::new(env!("CARGO_BIN_EXE_godot-qa"))
        .arg("scenario")
        .arg("profile")
        .arg(name)
        .args(extra_args)
        .arg("--project")
        .arg(project_root)
        .arg("--godot-bin")
        .arg("fake-godot")
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
    (output.status, body)
}

fn run_sample_scenario_profile(name: &str) -> (std::process::ExitStatus, Value) {
    let _guard = SAMPLE_PROJECT_RUN_LOCK.lock().unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_godot-qa"))
        .arg("scenario")
        .arg("profile")
        .arg(name)
        .arg("--project")
        .arg(sample_project_root())
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
    (output.status, body)
}

fn run_sample_scenario_run(name: &str) -> (std::process::ExitStatus, Value) {
    let _guard = SAMPLE_PROJECT_RUN_LOCK.lock().unwrap();
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
            "invalid stdout JSON: {error}\nstdout: {}\nstderr: {}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        )
    });
    (output.status, body)
}

fn assert_successful_profile_contract(name: &str, body: &Value) {
    assert_eq!(body["command"], "scenario profile");
    assert_eq!(body["ok"], true);
    assert_eq!(body["data"]["scenario"], name);
    assert!(
        body["data"]["profile"]["frame_time_ms"]["sample_count"]
            .as_u64()
            .unwrap()
            > 0
    );
    assert!(body["data"]["profile"]["frame_time_ms"]["p50"].is_number());
    assert!(
        !body["data"]["profile"]["step_timings"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    assert!(body["data"]["profile"]["monitor_counters"].is_object());
    assert!(body["data"]["launch_display"].is_object());

    let profile_path = body["data"]["artifacts"]["profile"].as_str().unwrap();
    assert!(Path::new(profile_path).ends_with("profile.json"));
    assert!(Path::new(profile_path).is_file());

    let artifact: Value = serde_json::from_str(&fs::read_to_string(profile_path).unwrap()).unwrap();
    let max_frame_samples = artifact["sample_limits"]["max_frame_samples"]
        .as_u64()
        .unwrap();
    assert!(artifact["frame_times_ms"].as_array().unwrap().len() as u64 <= max_frame_samples);
    assert!(artifact["monitor_samples"].is_array());
    assert!(artifact["step_timings"].is_array());
    assert_eq!(
        artifact["step_timings"],
        body["data"]["profile"]["step_timings"]
    );
    assert_eq!(body["data"]["progress"]["exists"], true);
    assert!(body["data"]["progress"]["record_count"].as_u64().unwrap() > 0);
    let progress_path = body["data"]["progress"]["path"].as_str().unwrap();
    assert_eq!(body["data"]["artifacts"]["progress"], progress_path);
    assert!(Path::new(progress_path).is_file());
}

#[test]
fn scenario_profile_sample_setup_steps_writes_bounded_artifact() {
    let (status, body) = run_sample_scenario_profile("sample_setup_steps");

    assert!(status.success());
    assert_successful_profile_contract("sample_setup_steps", &body);
}

#[test]
fn scenario_profile_sample_main_menu_start_writes_bounded_artifact() {
    let (status, body) = run_sample_scenario_profile("sample_main_menu_start");

    assert!(status.success());
    assert_successful_profile_contract("sample_main_menu_start", &body);
}

#[test]
fn scenario_profile_accelerated_timer_includes_time_control_evidence() {
    let (status, body) = run_sample_scenario_profile("accelerated_timer_demo");

    assert!(status.success());
    assert_successful_profile_contract("accelerated_timer_demo", &body);
    assert_eq!(body["data"]["time_control"]["requested"]["time_scale"], 8.0);
    assert_eq!(body["data"]["time_control"]["effective"]["time_scale"], 8.0);
    assert_eq!(
        body["data"]["time_control"]["restoration_status"],
        "restored"
    );
    assert_eq!(
        body["data"]["time_control"]["restored"],
        body["data"]["time_control"]["original"]
    );

    let profile_path = body["data"]["artifacts"]["profile"].as_str().unwrap();
    let artifact: Value = serde_json::from_str(&fs::read_to_string(profile_path).unwrap()).unwrap();
    assert_eq!(artifact["time_control"], body["data"]["time_control"]);
}

#[test]
fn scenario_profile_assertion_failure_remains_failure() {
    let (status, body) = run_sample_scenario_profile("profile_assertion_failure_demo");

    assert_eq!(status.code(), Some(1));
    assert_eq!(body["ok"], false);
    assert_eq!(body["command"], "scenario profile");
    assert_eq!(body["data"]["outcome"]["status"], "fail");
    assert!(matches!(
        body["failures"][0]["type"].as_str(),
        Some("assertion_failed" | "text_contains_mismatch")
    ));
}

#[test]
fn scenario_profile_applies_scenario_frame_pacing_like_run() {
    // `sample_main_menu_start` declares `determinism.fixed_fps: 60`. Profiling must honor the
    // same fixed timestep as `scenario run`, otherwise time-integrated state advances at a
    // different rate and asserts that pass under `run` can fail under `profile`.
    let (run_status, run_body) = run_sample_scenario_run("sample_main_menu_start");
    let (profile_status, profile_body) = run_sample_scenario_profile("sample_main_menu_start");

    assert!(run_status.success(), "{run_body:#?}");
    assert!(profile_status.success(), "{profile_body:#?}");

    let run_pacing = &run_body["data"]["frame_pacing"];
    let profile_pacing = &profile_body["data"]["frame_pacing"];

    assert_eq!(run_pacing["fixed_fps"], 60);
    assert_eq!(run_pacing["source"], "scenario");
    assert_eq!(
        profile_pacing, run_pacing,
        "scenario profile must report the same frame pacing as scenario run"
    );
}

#[test]
fn scenario_profile_normal_run_output_excludes_profile_sample_history() {
    let (status, body) = run_sample_scenario_run("sample_main_menu_start");

    assert!(status.success(), "{body:#?}");
    assert_eq!(body["command"], "scenario run");
    assert!(body["data"].get("profile").is_none());
    assert!(body["data"]["artifacts"].get("profile").is_none());
}

#[test]
fn scenario_profile_missing_scenario_uses_profile_failure_envelope() {
    let project_root = make_project_root("scenario-profile-cli-missing");

    let (status, body) = run_scenario_profile(&project_root, "missing", &["--headless"]);

    assert_eq!(status.code(), Some(1));
    assert_eq!(body["ok"], false);
    assert_eq!(body["command"], "scenario profile");
    assert_eq!(body["message"], "scenario profile failed");
    assert_eq!(body["failures"][0]["type"], "missing_scenario");
    assert_eq!(body["failures"][0]["command"], "scenario profile");
    assert_eq!(body["failures"][0]["actual"], "missing");

    fs::remove_dir_all(project_root).unwrap();
}

#[test]
fn scenario_profile_ambiguous_scenario_uses_profile_command_name() {
    let project_root = make_project_root("scenario-profile-cli-ambiguous");
    write(
        &project_root.join("qa/scenarios/one.json"),
        &json!({"name":"same","scene":"res://scene.tscn","steps":[]}).to_string(),
    );
    write(
        &project_root.join("qa/scenarios/two.json"),
        &json!({"name":"same","scene":"res://scene.tscn","steps":[]}).to_string(),
    );

    let (status, body) = run_scenario_profile(&project_root, "same", &["--windowed"]);

    assert_eq!(status.code(), Some(1));
    assert_eq!(body["ok"], false);
    assert_eq!(body["command"], "scenario profile");
    assert_eq!(body["message"], "scenario profile failed");
    assert_eq!(body["failures"][0]["type"], "scenario_name_ambiguous");
    assert_eq!(body["failures"][0]["command"], "scenario profile");

    fs::remove_dir_all(project_root).unwrap();
}

#[test]
fn scenario_profile_invalid_scenario_payload_uses_profile_command_name() {
    let project_root = make_project_root("scenario-profile-cli-invalid");
    write(
        &project_root.join("qa/scenarios/broken.json"),
        &json!({"name":"broken","steps":[]}).to_string(),
    );

    let (status, body) = run_scenario_profile(&project_root, "broken", &[]);

    assert_eq!(status.code(), Some(1));
    assert_eq!(body["ok"], false);
    assert_eq!(body["command"], "scenario profile");
    assert_eq!(body["message"], "scenario profile failed");
    assert_eq!(body["failures"][0]["command"], "scenario profile");
    assert_ne!(
        body["failures"][0]["type"],
        "scenario_profile_not_implemented"
    );

    fs::remove_dir_all(project_root).unwrap();
}

#[test]
fn scenario_profile_valid_cli_shape_reaches_profile_entry_point() {
    let project_root = make_project_root("scenario-profile-cli-valid");
    write(
        &project_root.join("qa/scenarios/valid.json"),
        &json!({"name":"valid","scene":"res://scene.tscn","steps":[]}).to_string(),
    );

    let (status, body) = run_scenario_profile(&project_root, "valid", &["--headless"]);

    assert_eq!(status.code(), Some(1));
    assert_eq!(body["ok"], false);
    assert_eq!(body["command"], "scenario profile");
    assert_eq!(body["message"], "scenario profile failed");
    assert_ne!(
        body["failures"][0]["type"],
        "scenario_profile_not_implemented"
    );
    assert_eq!(body["data"]["scenario"], "valid");
    assert_eq!(body["data"]["scene"], "res://scene.tscn");
    assert_eq!(body["data"]["headless"], true);
    assert!(body["data"]["artifacts"]["request_path"].is_string());

    fs::remove_dir_all(project_root).unwrap();
}
