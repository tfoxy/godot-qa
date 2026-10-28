use godot_qa_test_support::fixture;
use serde_json::{Value, json};
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

fn unique_suffix() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or(0);
    format!("{nanos}-{}", std::process::id())
}

fn make_project_root(prefix: &str) -> PathBuf {
    let root = env::temp_dir().join(format!("godot-qa-{prefix}-{}", unique_suffix()));
    fs::create_dir_all(&root).unwrap();
    fs::write(root.join("project.godot"), "[application]\n").unwrap();
    root
}

fn write(path: &Path, contents: &str) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(path, contents).unwrap();
}

fn run_scenario_bundle_list(project_root: &Path) -> (std::process::ExitStatus, Value) {
    let output = Command::new(env!("CARGO_BIN_EXE_godot-qa"))
        .arg("--json")
        .arg("--project")
        .arg(project_root)
        .arg("scenario")
        .arg("bundle")
        .arg("list")
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

fn run_scenario_bundle_run(
    project_root: &Path,
    name: &str,
    strict: bool,
) -> (std::process::ExitStatus, Value) {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_godot-qa"));
    cmd.arg("--json")
        .arg("--project")
        .arg(project_root)
        .arg("scenario")
        .arg("bundle")
        .arg("run")
        .arg(name);
    if strict {
        cmd.arg("--strict");
    }
    let output = cmd.output().unwrap();
    let body = serde_json::from_slice(&output.stdout).unwrap();
    (output.status, body)
}

fn run_scenario_bundle_run_with_godot(
    project_root: &Path,
    name: &str,
    godot_bin: &Path,
) -> (std::process::ExitStatus, Value) {
    let output = Command::new(env!("CARGO_BIN_EXE_godot-qa"))
        .arg("--json")
        .arg("--project")
        .arg(project_root)
        .arg("--godot-bin")
        .arg(godot_bin)
        .arg("scenario")
        .arg("bundle")
        .arg("run")
        .arg(name)
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

fn run_scenario_bundle_run_with_args(
    project_root: &Path,
    name: &str,
    extra_args: &[&str],
) -> (std::process::ExitStatus, Value) {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_godot-qa"));
    cmd.arg("--json")
        .arg("--project")
        .arg(project_root)
        .arg("scenario")
        .arg("bundle")
        .arg("run")
        .arg(name);
    cmd.args(extra_args);
    let output = cmd.output().unwrap();
    let body = serde_json::from_slice(&output.stdout).unwrap_or_else(|error| {
        panic!(
            "invalid stdout JSON: {error}\nstdout: {}\nstderr: {}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        )
    });
    (output.status, body)
}

fn run_scenario_bundle_run_with_godot_args(
    project_root: &Path,
    name: &str,
    godot_bin: &Path,
    extra_args: &[&str],
) -> (std::process::ExitStatus, Value) {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_godot-qa"));
    cmd.arg("--json")
        .arg("--project")
        .arg(project_root)
        .arg("--godot-bin")
        .arg(godot_bin)
        .arg("scenario")
        .arg("bundle")
        .arg("run")
        .arg(name);
    cmd.args(extra_args);
    let output = cmd.output().unwrap();
    let body = serde_json::from_slice(&output.stdout).unwrap_or_else(|error| {
        panic!(
            "invalid stdout JSON: {error}\nstdout: {}\nstderr: {}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        )
    });
    (output.status, body)
}

fn write_fake_godot(path: &Path) {
    fixture(
        path,
        &[
            ("request_prefix", "--qa-run-request="),
            ("scenario_proof", "passed"),
            (
                "write:result.json",
                r#"{"ok":true,"command":"scenario run","message":"scenario run completed","data":{"runtime":"fake"},"failures":[]}"#,
            ),
        ],
    );
}

#[test]
fn scenario_bundle_list_outputs_json_envelope_and_bundle_metadata() {
    let project_root = make_project_root("scenario-bundle-cli-list");
    write(
        &project_root.join("qa/gates/sample_regression_gate.json"),
        &json!({
            "name": "sample_regression_gate",
            "purpose": "Smoke gate",
            "required_bundles": ["base_gate"],
            "required_scenarios": ["sample_main_menu_start"],
            "optional_scenarios": ["sample_settings_layout"],
            "recommended_manual_checks": ["Verify menu animation smoothness"]
        })
        .to_string(),
    );

    let (status, body) = run_scenario_bundle_list(&project_root);

    assert!(status.success());
    assert_eq!(body["ok"], true);
    assert_eq!(body["command"], "scenario bundle list");
    assert!(body["message"].is_string());
    assert!(body["data"].is_object());
    assert!(body["failures"].is_array());

    let bundles = body["data"]["bundles"].as_array().unwrap();
    assert_eq!(bundles.len(), 1);
    let bundle = &bundles[0];
    assert_eq!(bundle["name"], "sample_regression_gate");
    assert!(bundle["path"].is_string());
    assert_eq!(bundle["purpose"], "Smoke gate");
    assert_eq!(bundle["required_bundles"], json!(["base_gate"]));
    assert_eq!(
        bundle["required_scenarios"],
        json!(["sample_main_menu_start"])
    );
    assert_eq!(
        bundle["optional_scenarios"],
        json!(["sample_settings_layout"])
    );
    assert_eq!(
        bundle["recommended_manual_checks"],
        json!(["Verify menu animation smoothness"])
    );

    write(
        &project_root.join("qa/gates/without_required_bundles.json"),
        &json!({
            "name": "without_required_bundles",
            "required_scenarios": []
        })
        .to_string(),
    );
    let (status, body) = run_scenario_bundle_list(&project_root);
    assert!(status.success());
    let bundles = body["data"]["bundles"].as_array().unwrap();
    let omitted = bundles
        .iter()
        .find(|bundle| bundle["name"] == "without_required_bundles")
        .unwrap();
    assert_eq!(omitted["required_bundles"], json!([]));

    fs::remove_dir_all(project_root).unwrap();
}

#[test]
fn scenario_bundle_list_missing_gates_dir_exits_nonzero_with_structured_failure() {
    let project_root = make_project_root("scenario-bundle-cli-missing-gates");

    let (status, body) = run_scenario_bundle_list(&project_root);

    assert_eq!(status.code(), Some(1));
    assert_eq!(body["ok"], false);
    assert_eq!(body["command"], "scenario bundle list");
    assert_eq!(body["failures"][0]["type"], "missing_gates_dir");

    fs::remove_dir_all(project_root).unwrap();
}

#[test]
fn scenario_bundle_list_duplicate_names_exits_nonzero_with_structured_failure() {
    let project_root = make_project_root("scenario-bundle-cli-duplicate");
    write(
        &project_root.join("qa/gates/first.json"),
        r#"{"name":"duplicate_gate","required_scenarios":[]}"#,
    );
    write(
        &project_root.join("qa/gates/nested/second.json"),
        r#"{"name":"duplicate_gate","required_scenarios":[]}"#,
    );

    let (status, body) = run_scenario_bundle_list(&project_root);

    assert_eq!(status.code(), Some(1));
    assert_eq!(body["ok"], false);
    assert_eq!(body["command"], "scenario bundle list");
    assert_eq!(body["failures"][0]["type"], "duplicate_bundle_name");
    assert_eq!(body["failures"][0]["command"], "scenario bundle list");

    fs::remove_dir_all(project_root).unwrap();
}

#[test]
fn scenario_bundle_run_missing_bundle_exits_nonzero() {
    let project_root = make_project_root("scenario-bundle-cli-run-missing");
    fs::create_dir_all(project_root.join("qa/gates")).unwrap();
    let (status, body) = run_scenario_bundle_run(&project_root, "missing", false);
    assert_eq!(status.code(), Some(1));
    assert_eq!(body["command"], "scenario bundle run");
    assert_eq!(body["failures"][0]["type"], "missing_bundle");
    fs::remove_dir_all(project_root).unwrap();
}

#[test]
fn scenario_bundle_run_named_malformed_manifest_reports_manifest_failure() {
    let project_root = make_project_root("scenario-bundle-cli-run-malformed");
    write(
        &project_root.join("qa/gates/malformed_bundle_demo.yaml"),
        "name: malformed_bundle_demo\nrequired_scenarios:\n  - sample_main_menu_start\ncommands:\n  - echo unsafe\n",
    );

    let (status, body) = run_scenario_bundle_run(&project_root, "malformed_bundle_demo", false);

    assert_eq!(status.code(), Some(1));
    assert_eq!(body["command"], "scenario bundle run");
    assert_eq!(body["failures"][0]["type"], "bundle_unsupported_key");
    assert_eq!(body["failures"][0]["command"], "scenario bundle run");
    assert_eq!(body["failures"][0]["actual"], "commands");
    assert!(
        Path::new(body["failures"][0]["path"].as_str().unwrap())
            .ends_with("qa/gates/malformed_bundle_demo.yaml")
    );

    fs::remove_dir_all(project_root).unwrap();
}

#[test]
fn scenario_bundle_run_composed_happy_path_outputs_expanded_summary() {
    let project_root = make_project_root("scenario-bundle-cli-run-composed");
    write_fake_godot(&project_root.join(format!("fake-godot{}", env::consts::EXE_SUFFIX)));
    write(
        &project_root.join("qa/gates/base.json"),
        r#"{"name":"base","required_scenarios":["included_child"]}"#,
    );
    write(
        &project_root.join("qa/gates/gate.json"),
        r#"{"name":"gate","required_bundles":["base"],"required_scenarios":["direct_child"]}"#,
    );
    write(
        &project_root.join("qa/gates/screenshot_heavy_regression_gate.json"),
        r#"{"name":"screenshot_heavy_regression_gate","required_scenarios":["visual_occlusion_demo","saved_snapshot_pixels_demo","visual_metadata_rect_demo","visual_line_indicator_demo"],"recommended_manual_checks":["Inspect bundle summary artifacts"]}"#,
    );
    write(
        &project_root.join("qa/scenarios/included_child.json"),
        r#"{"name":"included_child","scene":"res://Included.tscn","steps":[],"godot_args":["--included-cli"]}"#,
    );
    write(
        &project_root.join("qa/scenarios/direct_child.json"),
        r#"{"name":"direct_child","scene":"res://Direct.tscn","steps":[],"godot_args":["--direct-cli"]}"#,
    );
    for scenario_name in [
        "visual_occlusion_demo",
        "saved_snapshot_pixels_demo",
        "visual_metadata_rect_demo",
        "visual_line_indicator_demo",
    ] {
        write(
            &project_root.join(format!("qa/scenarios/{scenario_name}.json")),
            &json!({
                "name": scenario_name,
                "scene": format!("res://{scenario_name}.tscn"),
                "steps": []
            })
            .to_string(),
        );
    }

    let (status, body) = run_scenario_bundle_run_with_godot(
        &project_root,
        "gate",
        &project_root.join(format!("fake-godot{}", env::consts::EXE_SUFFIX)),
    );

    assert!(status.success());
    assert_eq!(body["ok"], true);
    assert_eq!(body["command"], "scenario bundle run");
    let runs = body["data"]["runs"].as_array().unwrap();
    assert_eq!(runs.len(), 2);
    assert_eq!(runs[0]["index"], 0);
    assert_eq!(runs[0]["scenario"], "included_child");
    assert_eq!(runs[0]["source_bundle"], "base");
    assert_eq!(runs[0]["source_kind"], "included");
    assert_eq!(runs[0]["bundle_chain"], json!(["gate", "base"]));
    assert_eq!(runs[0]["godot_args"], json!(["--included-cli"]));
    assert_eq!(runs[0]["status"], "passed");
    assert!(
        runs[0]["run_root"]
            .as_str()
            .unwrap()
            .contains("included_child")
    );
    assert!(
        runs[0]["result_path"]
            .as_str()
            .unwrap()
            .contains("result.json")
    );
    assert!(runs[0]["logs_path"].as_str().unwrap().contains("logs.txt"));
    assert!(runs[0]["screenshot_paths"].as_array().unwrap().is_empty());
    assert!(runs[0]["diagnostic_paths"].as_array().unwrap().is_empty());
    assert!(runs[0].get("launch_args").is_none());
    assert!(runs[0].get("child_run_root").is_none());
    assert!(runs[0].get("failure_category").is_none());
    assert_eq!(runs[1]["index"], 1);
    assert_eq!(runs[1]["scenario"], "direct_child");
    assert_eq!(runs[1]["source_bundle"], "gate");
    assert_eq!(runs[1]["source_kind"], "direct");
    assert_eq!(runs[1]["bundle_chain"], json!(["gate"]));
    assert_eq!(runs[1]["godot_args"], json!(["--direct-cli"]));
    assert_eq!(runs[1]["status"], "passed");

    let summary_path = body["data"]["artifacts"]["summary_path"].as_str().unwrap();
    let summary: Value = serde_json::from_slice(&fs::read(summary_path).unwrap()).unwrap();
    assert_eq!(summary["runs"], body["data"]["runs"]);
    assert!(body["data"]["artifact_summary"].is_object());
    let artifact_summary = &body["data"]["artifact_summary"];
    let artifact_children = artifact_summary["children"].as_array().unwrap();
    assert_eq!(artifact_children.len(), 2);
    assert_eq!(artifact_children[0]["scenario"], "included_child");
    assert_eq!(artifact_children[0]["source_bundle"], "base");
    assert_eq!(
        artifact_children[0]["bundle_chain"],
        json!(["gate", "base"])
    );
    assert_eq!(artifact_children[0]["status"], "passed");
    assert_eq!(artifact_children[1]["scenario"], "direct_child");
    assert_eq!(artifact_children[1]["source_bundle"], "gate");
    assert_eq!(artifact_children[1]["bundle_chain"], json!(["gate"]));

    let artifact_index_path = body["data"]["artifacts"]["artifact_index_path"]
        .as_str()
        .unwrap();
    assert!(Path::new(artifact_index_path).exists());
    let artifact_index: Value =
        serde_json::from_slice(&fs::read(artifact_index_path).unwrap()).unwrap();
    assert_eq!(artifact_index, *artifact_summary);
    let artifact_summary_text = serde_json::to_string(artifact_summary).unwrap();
    assert!(!artifact_summary_text.contains("child_run_root"));
    assert!(!artifact_summary_text.contains("launch_args"));
    let artifact_index_text = fs::read_to_string(artifact_index_path).unwrap();
    assert!(!artifact_index_text.contains("child_run_root"));
    assert!(!artifact_index_text.contains("launch_args"));

    let (list_status, list_body) = run_scenario_bundle_list(&project_root);
    assert!(list_status.success());
    let listed_names: Vec<&str> = list_body["data"]["bundles"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|bundle| bundle["name"].as_str())
        .collect();
    assert!(listed_names.contains(&"screenshot_heavy_regression_gate"));

    let (screenshot_status, screenshot_body) = run_scenario_bundle_run_with_godot(
        &project_root,
        "screenshot_heavy_regression_gate",
        &project_root.join(format!("fake-godot{}", env::consts::EXE_SUFFIX)),
    );
    assert!(screenshot_status.success());
    assert_eq!(screenshot_body["ok"], true);
    let screenshot_runs = screenshot_body["data"]["runs"].as_array().unwrap();
    assert_eq!(screenshot_runs.len(), 4);
    for run in screenshot_runs {
        let scenario_name = run["scenario"].as_str().unwrap();
        assert_eq!(run["source_bundle"], "screenshot_heavy_regression_gate");
        assert_eq!(run["source_kind"], "direct");
        assert_eq!(
            run["bundle_chain"],
            json!(["screenshot_heavy_regression_gate"])
        );
        assert_eq!(run["status"], "passed");
        assert!(run["launch_display"].is_object());
        assert!(run["run_root"].as_str().unwrap().contains(scenario_name));
        assert!(run.get("launch_args").is_none());
        assert!(run.get("child_run_root").is_none());
        assert!(run.get("failure_category").is_none());
    }

    fs::remove_dir_all(project_root).unwrap();
}

#[test]
fn scenario_bundle_retry_cli_contract() {
    let project_root = make_project_root("scenario-bundle-cli-retry");
    write_fake_godot(&project_root.join(format!("fake-godot{}", env::consts::EXE_SUFFIX)));
    write(
        &project_root.join("qa/gates/gate.json"),
        r#"{"name":"gate","required_scenarios":["child"]}"#,
    );
    write(
        &project_root.join("qa/scenarios/child.json"),
        r#"{"name":"child","scene":"res://Child.tscn","steps":[]}"#,
    );

    let (default_status, default_body) = run_scenario_bundle_run_with_godot(
        &project_root,
        "gate",
        &project_root.join(format!("fake-godot{}", env::consts::EXE_SUFFIX)),
    );
    assert!(default_status.success());
    let default_run = &default_body["data"]["runs"][0];
    assert_eq!(default_run["attempts"].as_array().unwrap().len(), 1);
    assert_eq!(
        default_run["retry_policy"]["max_infrastructure_startup_attempts"],
        1
    );
    assert_eq!(default_run["retry_policy"]["backoff_ms"], 0);

    let (retry_status, retry_body) = run_scenario_bundle_run_with_godot_args(
        &project_root,
        "gate",
        &project_root.join(format!("fake-godot{}", env::consts::EXE_SUFFIX)),
        &["--retry-startup", "3", "--retry-startup-backoff-ms", "1"],
    );
    assert!(retry_status.success());
    let retry_run = &retry_body["data"]["runs"][0];
    assert_eq!(retry_run["attempts"].as_array().unwrap().len(), 1);
    assert_eq!(
        retry_run["retry_policy"]["max_infrastructure_startup_attempts"],
        3
    );
    assert_eq!(retry_run["retry_policy"]["backoff_ms"], 1);
    assert!(
        retry_body["data"]["artifacts"]["summary_path"]
            .as_str()
            .is_some_and(|path| path.contains("gate"))
    );

    let (alias_status, alias_body) = run_scenario_bundle_run_with_godot_args(
        &project_root,
        "gate",
        &project_root.join(format!("fake-godot{}", env::consts::EXE_SUFFIX)),
        &[
            "--retry-infrastructure-startup",
            "2",
            "--retry-backoff-ms",
            "1",
        ],
    );
    assert!(alias_status.success());
    let alias_run = &alias_body["data"]["runs"][0];
    assert_eq!(
        alias_run["retry_policy"]["max_infrastructure_startup_attempts"],
        2
    );
    assert_eq!(alias_run["retry_policy"]["backoff_ms"], 1);

    let (invalid_status, invalid_body) = run_scenario_bundle_run_with_godot_args(
        &project_root,
        "gate",
        &project_root.join(format!("fake-godot{}", env::consts::EXE_SUFFIX)),
        &["--retry-startup", "0"],
    );
    assert_eq!(invalid_status.code(), Some(2));
    assert_eq!(invalid_body["failures"][0]["type"], "invalid_retry_policy");

    fs::remove_dir_all(project_root).unwrap();
}

fn run_dir_names(dir: &Path) -> Vec<String> {
    let Ok(entries) = fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut names: Vec<String> = entries
        .flatten()
        .filter(|entry| {
            fs::symlink_metadata(entry.path())
                .map(|meta| meta.is_dir() && !godot_qa_process::directory_link::is_link(&meta))
                .unwrap_or(false)
        })
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

fn write_retention_gate_project(prefix: &str, config_yaml: Option<&str>) -> PathBuf {
    let project_root = make_project_root(prefix);
    write_fake_godot(&project_root.join(format!("fake-godot{}", env::consts::EXE_SUFFIX)));
    if let Some(config_yaml) = config_yaml {
        write(&project_root.join("qa/config.yaml"), config_yaml);
    }
    write(
        &project_root.join("qa/gates/gate.json"),
        r#"{"name":"gate","required_scenarios":["first","second"]}"#,
    );
    write(
        &project_root.join("qa/scenarios/first.json"),
        r#"{"name":"first","scene":"res://First.tscn","steps":[]}"#,
    );
    write(
        &project_root.join("qa/scenarios/second.json"),
        r#"{"name":"second","scene":"res://Second.tscn","steps":[]}"#,
    );
    project_root
}

#[test]
fn scenario_bundle_run_prunes_run_dirs_when_keep_runs_is_set() {
    let project_root = write_retention_gate_project(
        "scenario-bundle-cli-retention",
        Some("artifacts:\n  keep_runs: 1\n"),
    );
    let godot = project_root.join(format!("fake-godot{}", env::consts::EXE_SUFFIX));

    let (first_status, first_body) =
        run_scenario_bundle_run_with_godot(&project_root, "gate", &godot);
    assert!(first_status.success());
    assert_eq!(first_body["ok"], true);
    // The contract is reported on every opted-in run, even when there is nothing to prune.
    assert_eq!(first_body["data"]["retention"]["keep_runs"], 1);
    assert_eq!(
        first_body["data"]["retention"]["pruned"]["scenario_runs"],
        0
    );
    assert_eq!(first_body["data"]["retention"]["pruned"]["bundle_runs"], 0);
    assert_eq!(first_body["data"]["retention"]["errors"], json!([]));

    // Bundle run ids are second-resolution, so separate the runs enough to claim two dirs.
    std::thread::sleep(std::time::Duration::from_millis(1100));

    let (second_status, second_body) =
        run_scenario_bundle_run_with_godot(&project_root, "gate", &godot);
    assert!(second_status.success());
    assert_eq!(second_body["ok"], true);
    assert_eq!(second_body["data"]["retention"]["keep_runs"], 1);
    assert_eq!(
        second_body["data"]["retention"]["pruned"]["scenario_runs"],
        2
    );
    assert_eq!(second_body["data"]["retention"]["pruned"]["bundle_runs"], 1);
    assert_eq!(second_body["data"]["retention"]["errors"], json!([]));

    let artifacts_dir = project_root.join(".qa/artifacts");
    for scenario in ["first", "second"] {
        assert_eq!(
            run_dir_names(&artifacts_dir.join(scenario)).len(),
            1,
            "expected one retained run dir for {scenario}"
        );
    }
    assert_eq!(run_dir_names(&artifacts_dir.join("bundles/gate")).len(), 1);
    // The run the second invocation just wrote is the one that survived.
    let run_root = second_body["data"]["artifacts"]["run_root"]
        .as_str()
        .unwrap();
    assert!(Path::new(run_root).is_dir());

    fs::remove_dir_all(project_root).unwrap();
}

#[test]
fn scenario_bundle_run_keeps_every_run_without_keep_runs() {
    let project_root = write_retention_gate_project("scenario-bundle-cli-no-retention", None);
    let godot = project_root.join(format!("fake-godot{}", env::consts::EXE_SUFFIX));

    let (first_status, first_body) =
        run_scenario_bundle_run_with_godot(&project_root, "gate", &godot);
    assert!(first_status.success());
    // Unset `artifacts.keep_runs` means no retention key at all and nothing removed.
    assert!(first_body["data"]["retention"].is_null());

    std::thread::sleep(std::time::Duration::from_millis(1100));

    let (second_status, second_body) =
        run_scenario_bundle_run_with_godot(&project_root, "gate", &godot);
    assert!(second_status.success());
    assert!(second_body["data"]["retention"].is_null());

    let artifacts_dir = project_root.join(".qa/artifacts");
    for scenario in ["first", "second"] {
        assert_eq!(
            run_dir_names(&artifacts_dir.join(scenario)).len(),
            2,
            "expected both run dirs kept for {scenario}"
        );
    }
    assert_eq!(run_dir_names(&artifacts_dir.join("bundles/gate")).len(), 2);

    fs::remove_dir_all(project_root).unwrap();
}

#[test]
fn scenario_bundle_run_keeps_pool_sessions_belonging_to_the_retained_run() {
    let project_root = write_retention_gate_project(
        "scenario-bundle-cli-session-margin",
        Some("artifacts:\n  keep_runs: 1\n"),
    );
    let live_dir = project_root.join(".qa/artifacts/live");
    fs::create_dir_all(&live_dir).unwrap();
    let now_millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis();
    // A bundle run id is the epoch second the run *finished*, and its pooled sessions start
    // when the run begins and are shut down before the artifacts are written. A session from
    // a minute ago therefore belongs to the run about to be retained: with a bare
    // "older than the oldest retained bundle run" cutoff it would be deleted immediately.
    let from_this_run = live_dir.join(format!("session-{}-aaaa", now_millis - 60_000));
    // Two days old: genuinely abandoned, well past the one-hour margin.
    let abandoned = live_dir.join(format!("session-{}-bbbb", now_millis - 172_800_000));
    fs::create_dir_all(&from_this_run).unwrap();
    fs::create_dir_all(&abandoned).unwrap();

    let (status, body) = run_scenario_bundle_run_with_godot(
        &project_root,
        "gate",
        &project_root.join(format!("fake-godot{}", env::consts::EXE_SUFFIX)),
    );

    assert!(status.success());
    assert_eq!(body["data"]["retention"]["pruned"]["sessions"], 1);
    assert!(
        from_this_run.is_dir(),
        "a session belonging to the retained bundle run must survive"
    );
    assert!(
        !abandoned.exists(),
        "an abandoned session past the margin must be pruned"
    );

    fs::remove_dir_all(project_root).unwrap();
}

#[test]
fn scenario_bundle_meta_json_is_identity_only_and_summary_keeps_the_runs() {
    let project_root = write_retention_gate_project("scenario-bundle-cli-meta-split", None);
    let godot = project_root.join(format!("fake-godot{}", env::consts::EXE_SUFFIX));

    let (status, body) = run_scenario_bundle_run_with_godot(&project_root, "gate", &godot);
    assert!(status.success());

    let meta_path = body["data"]["artifacts"]["meta_path"].as_str().unwrap();
    let meta: Value = serde_json::from_slice(&fs::read(meta_path).unwrap()).unwrap();
    // Identity, invocation, and scenario lists stay in meta.json...
    assert_eq!(meta["bundle"], "gate");
    assert!(meta["command"].is_string());
    assert!(meta["parallelism"].is_object());
    assert_eq!(meta["required_scenarios"], json!(["first", "second"]));
    // ...but the bulky duplicated payloads do not.
    assert!(meta.get("runs").is_none());
    assert!(meta.get("artifact_index").is_none());

    let summary_path = body["data"]["artifacts"]["summary_path"].as_str().unwrap();
    let summary: Value = serde_json::from_slice(&fs::read(summary_path).unwrap()).unwrap();
    assert_eq!(summary["runs"].as_array().unwrap().len(), 2);
    assert!(summary["artifact_index"].is_object());
    let index_path = body["data"]["artifacts"]["artifact_index_path"]
        .as_str()
        .unwrap();
    let index: Value = serde_json::from_slice(&fs::read(index_path).unwrap()).unwrap();
    assert_eq!(index["children"].as_array().unwrap().len(), 2);

    fs::remove_dir_all(project_root).unwrap();
}

#[test]
fn scenario_bundle_cli_jobs_contract() {
    let project_root = make_project_root("scenario-bundle-cli-jobs");
    write_fake_godot(&project_root.join(format!("fake-godot{}", env::consts::EXE_SUFFIX)));
    write(
        &project_root.join("qa/gates/gate.json"),
        r#"{"name":"gate","required_scenarios":["first","second"]}"#,
    );
    write(
        &project_root.join("qa/scenarios/first.json"),
        r#"{"name":"first","scene":"res://First.tscn","steps":[]}"#,
    );
    write(
        &project_root.join("qa/scenarios/second.json"),
        r#"{"name":"second","scene":"res://Second.tscn","steps":[]}"#,
    );

    let (zero_status, zero_body) = run_scenario_bundle_run_with_godot_args(
        &project_root,
        "gate",
        &project_root.join(format!("fake-godot{}", env::consts::EXE_SUFFIX)),
        &["--jobs", "0"],
    );
    assert!(zero_status.success());
    assert_eq!(zero_body["data"]["parallelism"]["raw_jobs"], 0);
    assert_eq!(zero_body["data"]["parallelism"]["runnable_count"], 2);
    assert_eq!(zero_body["data"]["parallelism"]["complete_queued"], true);

    let (two_status, two_body) = run_scenario_bundle_run_with_godot_args(
        &project_root,
        "gate",
        &project_root.join(format!("fake-godot{}", env::consts::EXE_SUFFIX)),
        &["--jobs", "2"],
    );
    assert!(two_status.success());
    assert_eq!(two_body["data"]["parallelism"]["raw_jobs"], 2);
    assert_eq!(two_body["data"]["parallelism"]["resolved_jobs"], 2);
    assert_eq!(two_body["data"]["runs"][0]["scenario"], "first");
    assert_eq!(two_body["data"]["runs"][1]["scenario"], "second");

    let summary_path = two_body["data"]["artifacts"]["summary_path"]
        .as_str()
        .unwrap();
    let summary: Value = serde_json::from_slice(&fs::read(summary_path).unwrap()).unwrap();
    assert_eq!(summary["parallelism"], two_body["data"]["parallelism"]);
    let meta_path = two_body["data"]["artifacts"]["meta_path"].as_str().unwrap();
    let meta: Value = serde_json::from_slice(&fs::read(meta_path).unwrap()).unwrap();
    assert_eq!(meta["parallelism"], two_body["data"]["parallelism"]);

    let (negative_status, negative_body) = run_scenario_bundle_run_with_godot_args(
        &project_root,
        "gate",
        &project_root.join(format!("fake-godot{}", env::consts::EXE_SUFFIX)),
        &["--jobs", "-1"],
    );
    assert!(negative_status.success());
    assert_eq!(negative_body["data"]["parallelism"]["raw_jobs"], -1);
    assert_eq!(negative_body["data"]["parallelism"]["reserved_count"], 1);
    assert!(
        negative_body["data"]["parallelism"]["resolved_jobs"]
            .as_u64()
            .is_some_and(|jobs| (1..=2).contains(&jobs))
    );

    fs::remove_dir_all(project_root).unwrap();
}

#[test]
fn scenario_bundle_cli_config_jobs_default_contract() {
    let project_root = make_project_root("scenario-bundle-cli-config-jobs");
    write_fake_godot(&project_root.join(format!("fake-godot{}", env::consts::EXE_SUFFIX)));
    write(
        &project_root.join("qa/gates/gate.json"),
        r#"{"name":"gate","required_scenarios":["first","second"]}"#,
    );
    write(
        &project_root.join("qa/scenarios/first.json"),
        r#"{"name":"first","scene":"res://First.tscn","steps":[]}"#,
    );
    write(
        &project_root.join("qa/scenarios/second.json"),
        r#"{"name":"second","scene":"res://Second.tscn","steps":[]}"#,
    );

    // Neither flag nor config: available parallelism, origin "default".
    let (default_status, default_body) = run_scenario_bundle_run_with_godot(
        &project_root,
        "gate",
        &project_root.join(format!("fake-godot{}", env::consts::EXE_SUFFIX)),
    );
    assert!(default_status.success());
    assert_eq!(default_body["data"]["parallelism"]["raw_jobs"], 0);
    assert_eq!(default_body["data"]["jobs_origin"], "default");

    // qa/config.yaml scenario.jobs pins the project default.
    write(
        &project_root.join("qa/config.yaml"),
        "scenario:\n  jobs: 1\n",
    );
    let (config_status, config_body) = run_scenario_bundle_run_with_godot(
        &project_root,
        "gate",
        &project_root.join(format!("fake-godot{}", env::consts::EXE_SUFFIX)),
    );
    assert!(config_status.success());
    assert_eq!(config_body["data"]["parallelism"]["raw_jobs"], 1);
    assert_eq!(config_body["data"]["parallelism"]["resolved_jobs"], 1);
    assert_eq!(config_body["data"]["parallelism"]["source"], "explicit");
    assert_eq!(config_body["data"]["jobs_origin"], "config");

    // An explicit --jobs flag overrides the config default.
    let (flag_status, flag_body) = run_scenario_bundle_run_with_godot_args(
        &project_root,
        "gate",
        &project_root.join(format!("fake-godot{}", env::consts::EXE_SUFFIX)),
        &["--jobs", "2"],
    );
    assert!(flag_status.success());
    assert_eq!(flag_body["data"]["parallelism"]["raw_jobs"], 2);
    assert_eq!(flag_body["data"]["parallelism"]["resolved_jobs"], 2);
    assert_eq!(flag_body["data"]["jobs_origin"], "flag");

    fs::remove_dir_all(project_root).unwrap();
}

#[test]
fn scenario_bundle_run_warm_strict_without_warm_fails_fast() {
    let project_root = make_project_root("scenario-bundle-cli-warm-strict-no-warm");

    let (status, body) =
        run_scenario_bundle_run_with_args(&project_root, "gate", &["--warm-strict"]);

    assert_eq!(status.code(), Some(2));
    assert_eq!(body["ok"], false);
    assert_eq!(body["command"], "scenario bundle run");
    assert_eq!(body["failures"][0]["type"], "warm_strict_requires_warm");

    fs::remove_dir_all(project_root).unwrap();
}

#[test]
fn scenario_bundle_run_config_warm_strict_default_without_warm_fails_fast() {
    let project_root = make_project_root("scenario-bundle-cli-warm-strict-config-only");
    write(
        &project_root.join("qa/config.yaml"),
        "scenario:\n  warm_strict: true\n",
    );

    let (status, body) = run_scenario_bundle_run_with_args(&project_root, "gate", &[]);

    assert_eq!(status.code(), Some(2));
    assert_eq!(body["failures"][0]["type"], "warm_strict_requires_warm");

    fs::remove_dir_all(project_root).unwrap();
}

#[test]
fn scenario_bundle_run_cold_flag_overrides_config_warm_strict_default_without_error() {
    let project_root = make_project_root("scenario-bundle-cli-cold-overrides-config");
    fs::create_dir_all(project_root.join("qa/gates")).unwrap();
    write(
        &project_root.join("qa/config.yaml"),
        "scenario:\n  warm: true\n  warm_strict: true\n",
    );

    // --cold alone (no --no-warm-strict) is enough: strict is moot without
    // warm, so this must reach ordinary bundle-lookup failure, not the
    // warm_strict_requires_warm validation error.
    let (status, body) = run_scenario_bundle_run_with_args(&project_root, "missing", &["--cold"]);

    assert_eq!(status.code(), Some(1));
    assert_eq!(body["failures"][0]["type"], "missing_bundle");

    fs::remove_dir_all(project_root).unwrap();
}

#[test]
fn scenario_bundle_run_cold_flag_wins_outright_even_with_explicit_warm_strict() {
    let project_root = make_project_root("scenario-bundle-cli-cold-wins-outright");
    fs::create_dir_all(project_root.join("qa/gates")).unwrap();

    // --cold is an unconditional "run cold, ignore warm entirely" escape
    // hatch: it wins even over an explicit --warm-strict on the same
    // invocation, reaching ordinary bundle-lookup failure rather than the
    // warm_strict_requires_warm validation error.
    let (status, body) =
        run_scenario_bundle_run_with_args(&project_root, "missing", &["--cold", "--warm-strict"]);

    assert_eq!(status.code(), Some(1));
    assert_eq!(body["failures"][0]["type"], "missing_bundle");

    fs::remove_dir_all(project_root).unwrap();
}

#[test]
fn scenario_bundle_run_no_warm_strict_flag_disables_config_strict_default_without_error() {
    let project_root = make_project_root("scenario-bundle-cli-no-warm-strict-override");
    fs::create_dir_all(project_root.join("qa/gates")).unwrap();
    write(
        &project_root.join("qa/config.yaml"),
        "scenario:\n  warm_strict: true\n",
    );

    // --no-warm-strict alone disables the config's strict default even
    // though warm was never requested either, so this must reach ordinary
    // bundle-lookup failure, not the warm_strict_requires_warm validation.
    let (status, body) =
        run_scenario_bundle_run_with_args(&project_root, "missing", &["--no-warm-strict"]);

    assert_eq!(status.code(), Some(1));
    assert_eq!(body["failures"][0]["type"], "missing_bundle");

    fs::remove_dir_all(project_root).unwrap();
}

#[test]
fn scenario_bundle_run_warm_strict_fails_fast_when_session_cannot_start() {
    let project_root = make_project_root("scenario-bundle-cli-warm-strict-dead-session");
    fs::create_dir_all(project_root.join("qa/gates")).unwrap();

    let mut cmd = Command::new(env!("CARGO_BIN_EXE_godot-qa"));
    cmd.arg("--json")
        .arg("--project")
        .arg(&project_root)
        .arg("--godot-bin")
        .arg("/nonexistent/godot-binary")
        .arg("scenario")
        .arg("bundle")
        .arg("run")
        .arg("gate")
        .arg("--warm")
        .arg("--warm-strict");
    let output = cmd.output().unwrap();
    let body: Value = serde_json::from_slice(&output.stdout).unwrap();

    assert_eq!(output.status.code(), Some(1));
    assert_eq!(body["ok"], false);
    assert_eq!(body["failures"][0]["type"], "warm_session_start_failed");
    assert_eq!(body["data"]["warm"]["session_started"], false);
    assert_eq!(body["data"]["warm"]["mode"], "session_start_failed");
    // The underlying startup failure is carried, not discarded: strict mode has to
    // say WHY the session could not start, not just that it could not.
    let cause = &body["failures"][0]["cause"];
    assert!(
        cause["type"].as_str().is_some_and(|kind| !kind.is_empty()),
        "expected a cause on the strict failure: {}",
        serde_json::to_string_pretty(&body["failures"][0]).unwrap()
    );
    assert_eq!(body["data"]["warm"]["start_failure"]["type"], cause["type"]);
    // Fail-fast: no cold bundle ran, so there is no runs array.
    assert!(body["data"].get("runs").is_none());

    fs::remove_dir_all(project_root).unwrap();
}

#[test]
fn scenario_bundle_run_warm_seed_start_failure_runs_cold_and_reports_cause() {
    // WITHOUT --warm-strict a dead seed session degrades to an all-cold bundle —
    // that part is deliberate. What must not happen is degrading SILENTLY: the
    // bundle now names the failure that made it pay cold boots.
    let project_root = make_project_root("scenario-bundle-cli-warm-seed-failure-cold");
    // The fake Godot exits immediately, so a session start gets no live bridge.
    write_fake_godot(&project_root.join(format!("fake-godot{}", env::consts::EXE_SUFFIX)));
    write(
        &project_root.join("qa/gates/gate.json"),
        r#"{"name":"gate","required_scenarios":["first","second","third"]}"#,
    );
    for scenario in ["first", "second", "third"] {
        write(
            &project_root.join(format!("qa/scenarios/{scenario}.json")),
            &json!({"name": scenario, "scene": "res://Scene.tscn", "steps": []}).to_string(),
        );
    }

    // --jobs 1 with three scenarios keeps warm on (the parallelism skip needs
    // jobs >= count), so the seed session really is attempted.
    let (status, body) = run_scenario_bundle_run_with_godot_args(
        &project_root,
        "gate",
        &project_root.join(format!("fake-godot{}", env::consts::EXE_SUFFIX)),
        &["--warm", "--jobs", "1"],
    );

    assert!(
        status.success(),
        "{}",
        serde_json::to_string_pretty(&body).unwrap()
    );
    assert_eq!(body["ok"], true);
    assert_eq!(body["data"]["warm"]["session_started"], false);
    assert_eq!(body["data"]["warm"]["mode"], "session_start_failed");
    assert!(
        body["data"]["warm"]["start_failure"]["type"]
            .as_str()
            .is_some_and(|kind| !kind.is_empty()),
        "the cold fallback must name its cause: {}",
        serde_json::to_string_pretty(&body["data"]["warm"]).unwrap()
    );
    // Every scenario ran, and ran cold (no warm annotation on any child).
    let runs = body["data"]["runs"].as_array().unwrap();
    assert_eq!(runs.len(), 3);
    for run in runs {
        assert_eq!(run["ok"], true);
        assert!(
            run.get("warm").is_none_or(Value::is_null),
            "expected a plain cold run: {run}"
        );
    }

    fs::remove_dir_all(project_root).unwrap();
}

#[test]
fn scenario_bundle_run_warm_strict_skip_enforces_structural_violation() {
    // A many-core host skips warm (jobs >= scenario count) and runs the parallel
    // cold path, but --warm-strict must STILL fail a structurally-incompatible
    // scenario (godot_args) so the verdict matches a 1-job CI. Same bundle, same
    // verdict, any host core count.
    let project_root = make_project_root("scenario-bundle-cli-warm-strict-skip-violation");
    write_fake_godot(&project_root.join(format!("fake-godot{}", env::consts::EXE_SUFFIX)));
    write(
        &project_root.join("qa/gates/gate.json"),
        r#"{"name":"gate","required_scenarios":["clean","violator"]}"#,
    );
    write(
        &project_root.join("qa/scenarios/clean.json"),
        r#"{"name":"clean","scene":"res://Clean.tscn","steps":[]}"#,
    );
    write(
        &project_root.join("qa/scenarios/violator.json"),
        r#"{"name":"violator","scene":"res://Violator.tscn","steps":[],"godot_args":["--single-window"]}"#,
    );

    // jobs 4 > count 2 -> warm is skipped; strict is still enforced structurally.
    let (status, body) = run_scenario_bundle_run_with_godot_args(
        &project_root,
        "gate",
        &project_root.join(format!("fake-godot{}", env::consts::EXE_SUFFIX)),
        &["--warm", "--warm-strict", "--jobs", "4"],
    );

    assert_eq!(status.code(), Some(1));
    assert_eq!(body["ok"], false);
    // Warm was skipped (no session reuse below parallelism) but strict was still
    // enforced during the skip.
    assert_eq!(body["data"]["warm"]["reason"], "no_reuse_below_parallelism");
    assert_eq!(body["data"]["warm"]["ran_warm"], false);
    assert_eq!(body["data"]["warm"]["strict_structural_check"], true);
    // The structural violator carries warm_fallback_forced_cold; the clean
    // scenario passes.
    let runs = body["data"]["runs"].as_array().unwrap();
    let violator = runs
        .iter()
        .find(|run| run["scenario"] == "violator")
        .unwrap();
    assert_eq!(violator["ok"], false);
    assert_eq!(violator["failures"][0]["type"], "warm_fallback_forced_cold");
    assert_eq!(violator["warm"]["reason"], "godot_args");
    assert_eq!(violator["warm"]["ran_warm"], false);
    let clean = runs.iter().find(|run| run["scenario"] == "clean").unwrap();
    assert_eq!(clean["ok"], true);
    // The bundle-level failure list picks up the appended per-scenario failure.
    assert!(
        body["failures"]
            .as_array()
            .unwrap()
            .iter()
            .any(|failure| failure["type"] == "warm_fallback_forced_cold")
    );

    fs::remove_dir_all(project_root).unwrap();
}

#[test]
fn scenario_bundle_run_warm_strict_skip_all_clean_passes() {
    // Same skip regime, but every scenario is warm-eligible: strict enforcement
    // finds no structural violation, so the bundle passes.
    let project_root = make_project_root("scenario-bundle-cli-warm-strict-skip-clean");
    write_fake_godot(&project_root.join(format!("fake-godot{}", env::consts::EXE_SUFFIX)));
    write(
        &project_root.join("qa/gates/gate.json"),
        r#"{"name":"gate","required_scenarios":["first","second"]}"#,
    );
    write(
        &project_root.join("qa/scenarios/first.json"),
        r#"{"name":"first","scene":"res://First.tscn","steps":[]}"#,
    );
    write(
        &project_root.join("qa/scenarios/second.json"),
        r#"{"name":"second","scene":"res://Second.tscn","steps":[]}"#,
    );

    let (status, body) = run_scenario_bundle_run_with_godot_args(
        &project_root,
        "gate",
        &project_root.join(format!("fake-godot{}", env::consts::EXE_SUFFIX)),
        &["--warm", "--warm-strict", "--jobs", "4"],
    );

    assert!(status.success());
    assert_eq!(body["ok"], true);
    assert_eq!(body["data"]["warm"]["reason"], "no_reuse_below_parallelism");
    assert_eq!(body["data"]["warm"]["strict_structural_check"], true);
    for run in body["data"]["runs"].as_array().unwrap() {
        assert_eq!(run["ok"], true);
    }

    fs::remove_dir_all(project_root).unwrap();
}

#[test]
fn scenario_bundle_run_warm_skip_without_strict_allows_cold_fallback() {
    // The SAME structurally-violating bundle under --warm WITHOUT strict: the
    // skip is a plain cold run, the structural scenario is allowed to fall back
    // to cold, and there is no strict_structural_check marker.
    let project_root = make_project_root("scenario-bundle-cli-warm-skip-no-strict");
    write_fake_godot(&project_root.join(format!("fake-godot{}", env::consts::EXE_SUFFIX)));
    write(
        &project_root.join("qa/gates/gate.json"),
        r#"{"name":"gate","required_scenarios":["clean","violator"]}"#,
    );
    write(
        &project_root.join("qa/scenarios/clean.json"),
        r#"{"name":"clean","scene":"res://Clean.tscn","steps":[]}"#,
    );
    write(
        &project_root.join("qa/scenarios/violator.json"),
        r#"{"name":"violator","scene":"res://Violator.tscn","steps":[],"godot_args":["--single-window"]}"#,
    );

    let (status, body) = run_scenario_bundle_run_with_godot_args(
        &project_root,
        "gate",
        &project_root.join(format!("fake-godot{}", env::consts::EXE_SUFFIX)),
        &["--warm", "--jobs", "4"],
    );

    assert!(status.success());
    assert_eq!(body["ok"], true);
    assert_eq!(body["data"]["warm"]["reason"], "no_reuse_below_parallelism");
    assert_eq!(body["data"]["warm"]["ran_warm"], false);
    // Without --warm-strict the skip does not enforce structural strictness.
    assert!(
        body["data"]["warm"]
            .get("strict_structural_check")
            .is_none()
    );
    for run in body["data"]["runs"].as_array().unwrap() {
        assert_eq!(run["ok"], true);
    }
    // It does still say why each run was cold: the structural scenario names its
    // real reason, the eligible one names the skip.
    let runs = body["data"]["runs"].as_array().unwrap();
    let violator = runs
        .iter()
        .find(|run| run["scenario"] == "violator")
        .unwrap();
    assert_eq!(violator["warm"]["reason"], "godot_args");
    let clean = runs.iter().find(|run| run["scenario"] == "clean").unwrap();
    assert_eq!(clean["warm"]["reason"], "warm_skipped_below_parallelism");

    fs::remove_dir_all(project_root).unwrap();
}

#[test]
fn scenario_bundle_cli_composition_failures() {
    let cases = [
        (
            "missing-required",
            vec![(
                "qa/gates/gate.json",
                r#"{"name":"gate","required_bundles":["base"],"required_scenarios":[]}"#,
            )],
            "missing_required_bundle",
        ),
        (
            "self-cycle",
            vec![(
                "qa/gates/gate.json",
                r#"{"name":"gate","required_bundles":["gate"],"required_scenarios":[]}"#,
            )],
            "bundle_cycle_detected",
        ),
        (
            "multi-cycle",
            vec![
                (
                    "qa/gates/gate.json",
                    r#"{"name":"gate","required_bundles":["base"],"required_scenarios":[]}"#,
                ),
                (
                    "qa/gates/base.json",
                    r#"{"name":"base","required_bundles":["gate"],"required_scenarios":[]}"#,
                ),
            ],
            "bundle_cycle_detected",
        ),
        (
            "duplicate-required-scenario",
            vec![
                (
                    "qa/gates/gate.json",
                    r#"{"name":"gate","required_bundles":["base"],"required_scenarios":["shared"]}"#,
                ),
                (
                    "qa/gates/base.json",
                    r#"{"name":"base","required_scenarios":["shared"]}"#,
                ),
            ],
            "duplicate_required_scenario",
        ),
        (
            "unsafe-reference",
            vec![(
                "qa/gates/gate.json",
                r#"{"name":"gate","required_bundles":["../base"],"required_scenarios":[]}"#,
            )],
            "unsafe_bundle_reference",
        ),
        (
            "duplicate-name",
            vec![
                (
                    "qa/gates/gate.json",
                    r#"{"name":"gate","required_scenarios":[]}"#,
                ),
                (
                    "qa/gates/nested/gate.json",
                    r#"{"name":"gate","required_scenarios":[]}"#,
                ),
            ],
            "duplicate_bundle_name",
        ),
    ];

    for (name, files, expected_type) in cases {
        let project_root = make_project_root(&format!("scenario-bundle-cli-{name}"));
        for (path, contents) in files {
            write(&project_root.join(path), contents);
        }

        let (status, body) = run_scenario_bundle_run(&project_root, "gate", false);

        assert_eq!(status.code(), Some(1), "case {name}");
        assert_eq!(body["ok"], false, "case {name}");
        assert_eq!(body["command"], "scenario bundle run", "case {name}");
        assert_eq!(body["failures"][0]["type"], expected_type, "case {name}");
        assert_eq!(
            body["failures"][0]["command"], "scenario bundle run",
            "case {name}"
        );

        fs::remove_dir_all(project_root).unwrap();
    }
}

/// A fake Godot that reports engine timing, so the bundle has something real to
/// aggregate. The per-scenario numbers are keyed off the result path (which
/// contains the scenario name) so `slowest[]` has a known order to assert.
fn write_timed_fake_godot(path: &Path) {
    let result = |duration: u64, scene_load: u64, steps: u64| {
        json!({
            "ok": true,
            "command": "scenario run",
            "message": "scenario run completed",
            "duration_ms": duration,
            "timing": {
                "scene_load_ms": scene_load, "first_frame_ms": 5,
                "steps_ms": steps, "asserts_ms": 1, "teardown_ms": 2,
                "engine_total_ms": duration
            },
            "data": {"runtime": "fake"},
            "failures": []
        })
        .to_string()
    };
    fixture(
        path,
        &[
            ("request_prefix", "--qa-run-request="),
            ("scenario_proof", "passed"),
            ("write:result.json", &result(100, 50, 10)),
            ("write_match:slow_child:result.json", &result(300, 250, 30)),
            (
                "write_match:medium_child:result.json",
                &result(200, 150, 20),
            ),
        ],
    );
}

/// A fake Godot where every scenario fails the same way for the same reason — the
/// shape one missing provider registration produces across a whole gate.
fn write_provider_failure_fake_godot(path: &Path) {
    fixture(
        path,
        &[
            ("request_prefix", "--qa-run-request="),
            ("scenario_proof", "passed"),
            (
                "write:result.json",
                r#"{"ok":false,"command":"scenario run","message":"scenario run failed","duration_ms":10,"data":{"runtime":"fake"},"failures":[{"type":"provider_not_registered","message":"surface provider is not registered","provider":"layout_probe","target":"res://Main.tscn"}]}"#,
            ),
        ],
    );
}

#[test]
fn scenario_bundle_run_reports_per_run_and_bundle_timing() {
    // "Which scenario is slow" must be answerable from the bundle JSON alone,
    // without opening one result.json per child.
    let project_root = make_project_root("scenario-bundle-cli-timing");
    write_timed_fake_godot(&project_root.join(format!("fake-godot{}", env::consts::EXE_SUFFIX)));
    write(
        &project_root.join("qa/gates/gate.json"),
        r#"{"name":"gate","required_scenarios":["slow_child","medium_child","fast_child"]}"#,
    );
    for scenario_name in ["slow_child", "medium_child", "fast_child"] {
        write(
            &project_root.join(format!("qa/scenarios/{scenario_name}.json")),
            &json!({
                "name": scenario_name,
                "scene": format!("res://{scenario_name}.tscn"),
                "steps": []
            })
            .to_string(),
        );
    }

    let (status, body) = run_scenario_bundle_run_with_godot_args(
        &project_root,
        "gate",
        &project_root.join(format!("fake-godot{}", env::consts::EXE_SUFFIX)),
        &["--jobs", "2"],
    );
    assert!(status.success());
    assert_eq!(body["ok"], true);

    let runs = body["data"]["runs"].as_array().unwrap();
    assert_eq!(runs.len(), 3);
    for run in runs {
        assert!(run["duration_ms"].is_number(), "run: {run}");
        assert!(run["timing"]["scene_load_ms"].is_number());
        assert!(run["queue_wait_ms"].is_number());
        // Epoch milliseconds, not a monotonic counter: any plausible run is well
        // past 2023 and ends no earlier than it started.
        let started_at_ms = run["started_at_ms"].as_u64().unwrap();
        let finished_at_ms = run["finished_at_ms"].as_u64().unwrap();
        assert!(started_at_ms > 1_700_000_000_000);
        assert!(finished_at_ms >= started_at_ms);
        let lane = run["lane"].as_u64().unwrap();
        assert!(lane < 2, "lane {lane} outside the resolved worker count");
        assert!(run["attempts"][0]["duration_ms"].is_number());
        assert!(run["attempts"][0]["started_at_ms"].is_number());
    }

    let timing = &body["data"]["timing"];
    for key in [
        "started_at_ms",
        "finished_at_ms",
        "wall_ms",
        "prepare_ms",
        "execute_ms",
        "artifacts_ms",
        "sum_child_duration_ms",
        "parallel_efficiency",
        "slowest",
        "pool_seed_ms",
        "pool_shutdown_ms",
    ] {
        assert!(timing.get(key).is_some(), "data.timing is missing {key}");
    }
    assert_eq!(timing["sum_child_duration_ms"], 600.0);
    assert!(timing["parallel_efficiency"].is_number());
    assert!(timing["wall_ms"].as_f64().unwrap() > 0.0);
    // A cold bundle has no warm pool, so the pool spans are explicitly null.
    assert!(timing["pool_seed_ms"].is_null());
    assert!(timing["pool_shutdown_ms"].is_null());

    let slowest = timing["slowest"].as_array().unwrap();
    assert_eq!(slowest.len(), 3);
    assert!(slowest.len() <= 10);
    assert_eq!(slowest[0]["scenario"], "slow_child");
    assert_eq!(slowest[0]["duration_ms"], 300.0);
    assert_eq!(slowest[0]["scene_load_ms"], 250);
    assert_eq!(slowest[0]["steps_ms"], 30);
    assert!(
        slowest[0]["run_root"]
            .as_str()
            .unwrap()
            .contains("slow_child")
    );
    let durations: Vec<f64> = slowest
        .iter()
        .map(|entry| entry["duration_ms"].as_f64().unwrap())
        .collect();
    assert!(
        durations.windows(2).all(|pair| pair[0] >= pair[1]),
        "slowest[] is not sorted descending: {durations:?}"
    );

    // summary.json carries a snapshot taken before it was written: same shape,
    // artifacts_ms not yet spent.
    let summary_path = body["data"]["artifacts"]["summary_path"].as_str().unwrap();
    let summary: Value = serde_json::from_slice(&fs::read(summary_path).unwrap()).unwrap();
    assert_eq!(summary["timing"]["sum_child_duration_ms"], 600.0);
    assert_eq!(summary["timing"]["artifacts_ms"], 0.0);
    assert_eq!(summary["timing"]["slowest"][0]["scenario"], "slow_child");
    assert!(summary["failure_clusters"].is_array());
    assert_eq!(body["data"]["failure_clusters"], json!([]));

    fs::remove_dir_all(project_root).unwrap();
}

#[test]
fn scenario_bundle_run_attributes_and_clusters_child_failures() {
    // One shared cause across many scenarios must read as one cause: every child
    // failure names its scenario, and the repeat collapses into one cluster.
    let project_root = make_project_root("scenario-bundle-cli-clusters");
    write_provider_failure_fake_godot(
        &project_root.join(format!("fake-godot{}", env::consts::EXE_SUFFIX)),
    );
    write(
        &project_root.join("qa/gates/gate.json"),
        r#"{"name":"gate","required_scenarios":["first","second","third"]}"#,
    );
    for scenario_name in ["first", "second", "third"] {
        write(
            &project_root.join(format!("qa/scenarios/{scenario_name}.json")),
            &json!({
                "name": scenario_name,
                "scene": format!("res://{scenario_name}.tscn"),
                "steps": []
            })
            .to_string(),
        );
    }

    let (status, body) = run_scenario_bundle_run_with_godot(
        &project_root,
        "gate",
        &project_root.join(format!("fake-godot{}", env::consts::EXE_SUFFIX)),
    );
    assert_eq!(status.code(), Some(1));
    assert_eq!(body["ok"], false);

    // Failures are unchanged in number and order; they only gained attribution.
    let failures = body["failures"].as_array().unwrap();
    assert_eq!(failures.len(), 3);
    assert_eq!(failures[0]["type"], "provider_not_registered");
    assert_eq!(failures[0]["scenario"], "first");
    assert_eq!(failures[1]["scenario"], "second");
    assert_eq!(failures[2]["scenario"], "third");
    assert!(failures[0]["run_root"].as_str().unwrap().contains("first"));

    let clusters = body["data"]["failure_clusters"].as_array().unwrap();
    assert_eq!(clusters.len(), 1);
    let cluster = &clusters[0];
    assert_eq!(cluster["kind"], "provider_not_registered");
    assert_eq!(cluster["key"], "layout_probe");
    assert_eq!(cluster["scenario_count"], 3);
    assert_eq!(cluster["failure_count"], 3);
    assert_eq!(cluster["scenarios"], json!(["first", "second", "third"]));
    let hint = cluster["hint"].as_str().unwrap();
    assert!(
        hint.contains("provider 'layout_probe' is not registered in 3 of 3 scenarios"),
        "unexpected hint: {hint}"
    );

    let summary_path = body["data"]["artifacts"]["summary_path"].as_str().unwrap();
    let summary: Value = serde_json::from_slice(&fs::read(summary_path).unwrap()).unwrap();
    assert_eq!(
        summary["failure_clusters"],
        body["data"]["failure_clusters"]
    );
    assert_eq!(summary["failures"][0]["scenario"], "first");

    fs::remove_dir_all(project_root).unwrap();
}

#[test]
fn scenario_bundle_run_warm_skip_annotates_every_run_with_the_skip_reason() {
    // A skipped-warm bundle used to leave per-run `warm` null, which reads exactly
    // like "warm was never requested". Every run now says why it ran cold, on both
    // the strict and non-strict paths, and the skip reason is not a strict failure.
    let project_root = make_project_root("scenario-bundle-cli-warm-skip-reason");
    write_fake_godot(&project_root.join(format!("fake-godot{}", env::consts::EXE_SUFFIX)));
    write(
        &project_root.join("qa/gates/gate.json"),
        r#"{"name":"gate","required_scenarios":["first","second"]}"#,
    );
    for scenario_name in ["first", "second"] {
        write(
            &project_root.join(format!("qa/scenarios/{scenario_name}.json")),
            &json!({
                "name": scenario_name,
                "scene": format!("res://{scenario_name}.tscn"),
                "steps": []
            })
            .to_string(),
        );
    }

    for extra_args in [
        vec!["--warm", "--jobs", "4"],
        vec!["--warm", "--warm-strict", "--jobs", "4"],
    ] {
        let (status, body) = run_scenario_bundle_run_with_godot_args(
            &project_root,
            "gate",
            &project_root.join(format!("fake-godot{}", env::consts::EXE_SUFFIX)),
            &extra_args,
        );
        assert!(status.success(), "args {extra_args:?} body {body}");
        assert_eq!(body["ok"], true, "args {extra_args:?}");
        assert_eq!(body["data"]["warm"]["reason"], "no_reuse_below_parallelism");
        for run in body["data"]["runs"].as_array().unwrap() {
            assert_eq!(run["ok"], true, "args {extra_args:?}");
            assert_eq!(run["warm"]["reason"], "warm_skipped_below_parallelism");
            assert_eq!(run["warm"]["ran_warm"], false);
            assert_eq!(run["warm"]["declared"], false);
            assert!(run["warm"]["mode"].is_null());
            assert!(run["warm"]["audio"].is_null());
            // The persisted child artifact matches the summary, as for every other
            // warm annotation.
            let result_path = run["result_path"].as_str().unwrap();
            let result: Value = serde_json::from_slice(&fs::read(result_path).unwrap()).unwrap();
            assert_eq!(
                result["data"]["warm"]["reason"],
                "warm_skipped_below_parallelism"
            );
        }
    }

    fs::remove_dir_all(project_root).unwrap();
}

/// A fake Godot that fails exactly the scenarios named `bravo` and `delta`, so a bundle
/// run leaves a mixed pass/fail baseline for `--failed-only` to narrow to.
fn write_selective_failure_fake_godot(path: &Path) {
    let failure = r#"{"ok":false,"command":"scenario run","message":"scenario run failed","duration_ms":10,"data":{"runtime":"fake"},"failures":[{"type":"assertion_failed","message":"expected state not reached"}]}"#;
    fixture(
        path,
        &[
            ("request_prefix", "--qa-run-request="),
            ("scenario_proof", "passed"),
            (
                "write:result.json",
                r#"{"ok":true,"command":"scenario run","message":"scenario run completed","duration_ms":10,"data":{"runtime":"fake"},"failures":[]}"#,
            ),
            ("write_match:bravo:result.json", failure),
            ("write_match:delta:result.json", failure),
        ],
    );
}

/// A four-scenario gate with an all-passing `fake-godot`, a `failing-godot` that fails
/// `bravo`/`delta`, and a `timed-godot` that reports per-child durations.
fn write_selection_gate_project(prefix: &str, config_yaml: Option<&str>) -> PathBuf {
    let project_root = make_project_root(prefix);
    write_fake_godot(&project_root.join(format!("fake-godot{}", env::consts::EXE_SUFFIX)));
    write_selective_failure_fake_godot(
        &project_root.join(format!("failing-godot{}", env::consts::EXE_SUFFIX)),
    );
    write_timed_fake_godot(&project_root.join(format!("timed-godot{}", env::consts::EXE_SUFFIX)));
    if let Some(config_yaml) = config_yaml {
        write(&project_root.join("qa/config.yaml"), config_yaml);
    }
    write(
        &project_root.join("qa/gates/gate.json"),
        r#"{"name":"gate","required_scenarios":["alpha","bravo","charlie","delta"]}"#,
    );
    for scenario_name in ["alpha", "bravo", "charlie", "delta"] {
        write(
            &project_root.join(format!("qa/scenarios/{scenario_name}.json")),
            &json!({
                "name": scenario_name,
                "scene": format!("res://{scenario_name}.tscn"),
                "steps": []
            })
            .to_string(),
        );
    }
    project_root
}

fn run_names(body: &Value) -> Vec<String> {
    body["data"]["runs"]
        .as_array()
        .unwrap()
        .iter()
        .map(|run| run["scenario"].as_str().unwrap().to_string())
        .collect()
}

fn read_json(path: &Path) -> Value {
    serde_json::from_str(&fs::read_to_string(path).unwrap()).unwrap()
}

#[test]
fn scenario_bundle_run_only_filters_children_and_records_the_selection() {
    let project_root = write_selection_gate_project("scenario-bundle-cli-only", None);

    let (status, body) = run_scenario_bundle_run_with_godot_args(
        &project_root,
        "gate",
        &project_root.join(format!("fake-godot{}", env::consts::EXE_SUFFIX)),
        &["--only", "bravo", "--only", "delta"],
    );

    assert!(status.success());
    assert_eq!(body["ok"], true);
    assert_eq!(run_names(&body), ["bravo", "delta"]);
    assert_eq!(
        body["message"],
        "scenario bundle run completed (filtered: 2 of 4)"
    );
    let selection = &body["data"]["selection"];
    assert_eq!(selection["mode"], "filtered");
    assert_eq!(selection["only"], json!(["bravo", "delta"]));
    assert_eq!(selection["selected_ids"], json!(["bravo", "delta"]));
    assert_eq!(selection["member_count"], 4);
    assert_eq!(selection["failed_only"], false);
    assert_eq!(selection["source_run_root"], json!(null));

    // The persisted artifacts record the same narrowing, and the reproducer command
    // carries the flags that produced it.
    let run_root = PathBuf::from(body["data"]["artifacts"]["run_root"].as_str().unwrap());
    let summary = read_json(&run_root.join("summary.json"));
    assert_eq!(summary["selection"], *selection);
    let meta = read_json(&run_root.join("meta.json"));
    assert_eq!(meta["selection"], *selection);
    let command = meta["command"].as_str().unwrap();
    assert!(
        command.contains("--only bravo") && command.contains("--only delta"),
        "command: {command}"
    );

    fs::remove_dir_all(project_root).unwrap();
}

#[test]
fn scenario_bundle_run_unknown_only_name_exits_two_and_writes_nothing() {
    let project_root = write_selection_gate_project("scenario-bundle-cli-only-unknown", None);

    let (status, body) = run_scenario_bundle_run_with_godot_args(
        &project_root,
        "gate",
        &project_root.join(format!("fake-godot{}", env::consts::EXE_SUFFIX)),
        &["--only", "bravo", "--only", "typo"],
    );

    // Exit 2, not 1: the bundle never ran, so this is a bad invocation rather than a
    // failed gate.
    assert_eq!(status.code(), Some(2));
    assert_eq!(body["ok"], false);
    assert_eq!(body["failures"][0]["type"], "selection_unknown_scenario");
    assert_eq!(body["failures"][0]["actual"], json!(["typo"]));
    assert_eq!(
        body["failures"][0]["expected"],
        json!(["alpha", "bravo", "charlie", "delta"])
    );
    assert!(
        !project_root.join(".qa/artifacts/bundles/gate").exists(),
        "a rejected selection must not leave a run directory behind"
    );

    fs::remove_dir_all(project_root).unwrap();
}

#[test]
fn scenario_bundle_run_failed_only_without_a_baseline_exits_two() {
    let project_root = write_selection_gate_project("scenario-bundle-cli-failed-only-cold", None);

    let (status, body) = run_scenario_bundle_run_with_godot_args(
        &project_root,
        "gate",
        &project_root.join(format!("fake-godot{}", env::consts::EXE_SUFFIX)),
        &["--failed-only"],
    );

    assert_eq!(status.code(), Some(2));
    assert_eq!(body["failures"][0]["type"], "selection_no_previous_run");
    assert_eq!(body["failures"][0]["actual"], "no previous run");
    assert!(!project_root.join(".qa/artifacts/bundles/gate").exists());

    fs::remove_dir_all(project_root).unwrap();
}

#[test]
fn scenario_bundle_run_failed_only_reruns_the_previous_failures_then_settles() {
    let project_root = write_selection_gate_project("scenario-bundle-cli-failed-only", None);

    // Baseline: bravo and delta fail, alpha and charlie pass.
    let (baseline_status, baseline_body) = run_scenario_bundle_run_with_godot_args(
        &project_root,
        "gate",
        &project_root.join(format!("failing-godot{}", env::consts::EXE_SUFFIX)),
        &[],
    );
    assert_eq!(baseline_status.code(), Some(1));
    assert_eq!(
        run_names(&baseline_body),
        ["alpha", "bravo", "charlie", "delta"]
    );
    let baseline_run_root = baseline_body["data"]["artifacts"]["run_root"].clone();

    // Bundle run ids are second-resolution, so separate the runs enough to claim two dirs.
    std::thread::sleep(std::time::Duration::from_millis(1100));

    // The rerun narrows to exactly those two, and names the run it took the baseline from.
    let (rerun_status, rerun_body) = run_scenario_bundle_run_with_godot_args(
        &project_root,
        "gate",
        &project_root.join(format!("fake-godot{}", env::consts::EXE_SUFFIX)),
        &["--failed-only"],
    );
    assert!(rerun_status.success());
    assert_eq!(run_names(&rerun_body), ["bravo", "delta"]);
    assert_eq!(rerun_body["data"]["selection"]["mode"], "filtered");
    assert_eq!(rerun_body["data"]["selection"]["failed_only"], true);
    assert_eq!(
        rerun_body["data"]["selection"]["source_run_root"],
        baseline_run_root
    );

    std::thread::sleep(std::time::Duration::from_millis(1100));

    // Now that the newest run is green, --failed-only selects nothing: a warning and a
    // pass, not a failure. Chaining therefore narrows monotonically.
    let (settled_status, settled_body) = run_scenario_bundle_run_with_godot_args(
        &project_root,
        "gate",
        &project_root.join(format!("fake-godot{}", env::consts::EXE_SUFFIX)),
        &["--failed-only"],
    );
    assert!(settled_status.success());
    assert_eq!(settled_body["ok"], true);
    assert!(run_names(&settled_body).is_empty());
    assert_eq!(
        settled_body["message"],
        "scenario bundle run completed (filtered: 0 of 4)"
    );
    let warnings = settled_body["data"]["warnings"].as_array().unwrap();
    assert!(
        warnings
            .iter()
            .any(|warning| warning["type"] == "selection_empty"),
        "{warnings:#?}"
    );

    fs::remove_dir_all(project_root).unwrap();
}

#[test]
fn scenario_bundle_run_only_and_failed_only_intersect() {
    let project_root =
        write_selection_gate_project("scenario-bundle-cli-selection-intersect", None);

    let (baseline_status, _) = run_scenario_bundle_run_with_godot_args(
        &project_root,
        "gate",
        &project_root.join(format!("failing-godot{}", env::consts::EXE_SUFFIX)),
        &[],
    );
    assert_eq!(baseline_status.code(), Some(1));

    std::thread::sleep(std::time::Duration::from_millis(1100));

    // failed {bravo, delta} ∩ only {alpha, delta} = {delta}.
    let (status, body) = run_scenario_bundle_run_with_godot_args(
        &project_root,
        "gate",
        &project_root.join(format!("fake-godot{}", env::consts::EXE_SUFFIX)),
        &["--failed-only", "--only", "alpha", "--only", "delta"],
    );
    assert!(status.success());
    assert_eq!(run_names(&body), ["delta"]);
    assert_eq!(body["data"]["selection"]["only"], json!(["alpha", "delta"]));
    assert_eq!(body["data"]["selection"]["selected_ids"], json!(["delta"]));

    fs::remove_dir_all(project_root).unwrap();
}

#[test]
fn scenario_bundle_run_warm_skip_uses_the_filtered_scenario_count() {
    let project_root = write_selection_gate_project("scenario-bundle-cli-selection-warm", None);

    // Four scenarios over two lanes would run warm; narrowed to two it is at parallelism,
    // so no session would ever be reused and warm is skipped. The skip decision therefore
    // has to see the FILTERED count, not the manifest's.
    let (status, body) = run_scenario_bundle_run_with_godot_args(
        &project_root,
        "gate",
        &project_root.join(format!("fake-godot{}", env::consts::EXE_SUFFIX)),
        &[
            "--warm", "--jobs", "2", "--only", "alpha", "--only", "bravo",
        ],
    );

    assert!(status.success());
    assert_eq!(body["data"]["warm"]["reason"], "no_reuse_below_parallelism");
    assert_eq!(body["data"]["warm"]["ran_warm"], false);
    assert_eq!(run_names(&body), ["alpha", "bravo"]);

    fs::remove_dir_all(project_root).unwrap();
}

#[test]
fn scenario_bundle_run_reports_the_dispatch_schedule_and_its_origin() {
    let project_root = write_selection_gate_project("scenario-bundle-cli-schedule", None);
    let timed = project_root.join(format!("timed-godot{}", env::consts::EXE_SUFFIX));

    // First run: longest-first by default, but there is no history yet, so the order is
    // left alone and nothing was read.
    let (first_status, first_body) =
        run_scenario_bundle_run_with_godot_args(&project_root, "gate", &timed, &["--jobs", "2"]);
    assert!(first_status.success());
    assert_eq!(
        first_body["data"]["scheduling"],
        json!({
            "policy": "longest_first",
            "origin": "default",
            "applied": false,
            "history_runs": 0,
            "predicted": 0,
            "unknown": 0,
        })
    );

    std::thread::sleep(std::time::Duration::from_millis(1100));

    // Second run: the first run's summary.json is the history, so the policy applies.
    let (second_status, second_body) =
        run_scenario_bundle_run_with_godot_args(&project_root, "gate", &timed, &["--jobs", "2"]);
    assert!(second_status.success());
    assert_eq!(second_body["data"]["scheduling"]["applied"], true);
    assert_eq!(second_body["data"]["scheduling"]["history_runs"], 1);
    assert_eq!(second_body["data"]["scheduling"]["predicted"], 4);
    assert_eq!(second_body["data"]["scheduling"]["unknown"], 0);
    // Reordering is a dispatch detail only: the reported runs stay in expansion order.
    assert_eq!(
        run_names(&second_body),
        ["alpha", "bravo", "charlie", "delta"]
    );
    let run_root = PathBuf::from(
        second_body["data"]["artifacts"]["run_root"]
            .as_str()
            .unwrap(),
    );
    assert_eq!(
        read_json(&run_root.join("summary.json"))["scheduling"],
        second_body["data"]["scheduling"]
    );

    // The flag pins the manifest order and is recorded as such.
    let (flag_status, flag_body) = run_scenario_bundle_run_with_godot_args(
        &project_root,
        "gate",
        &timed,
        &["--jobs", "2", "--schedule", "manifest"],
    );
    assert!(flag_status.success());
    assert_eq!(flag_body["data"]["scheduling"]["policy"], "manifest");
    assert_eq!(flag_body["data"]["scheduling"]["origin"], "flag");
    assert_eq!(flag_body["data"]["scheduling"]["applied"], false);

    fs::remove_dir_all(project_root).unwrap();
}

#[test]
fn scenario_bundle_run_config_schedule_default_contract() {
    let project_root = write_selection_gate_project(
        "scenario-bundle-cli-schedule-config",
        Some("scenario:\n  schedule: manifest\n"),
    );
    let godot = project_root.join(format!("fake-godot{}", env::consts::EXE_SUFFIX));

    let (status, body) =
        run_scenario_bundle_run_with_godot_args(&project_root, "gate", &godot, &["--jobs", "2"]);
    assert!(status.success());
    assert_eq!(body["data"]["scheduling"]["policy"], "manifest");
    assert_eq!(body["data"]["scheduling"]["origin"], "config");

    // The flag still wins over the project default.
    let (flag_status, flag_body) = run_scenario_bundle_run_with_godot_args(
        &project_root,
        "gate",
        &godot,
        &["--jobs", "2", "--schedule", "longest-first"],
    );
    assert!(flag_status.success());
    assert_eq!(flag_body["data"]["scheduling"]["policy"], "longest_first");
    assert_eq!(flag_body["data"]["scheduling"]["origin"], "flag");

    fs::remove_dir_all(project_root).unwrap();
}

#[test]
fn scenario_bundle_run_maintains_latest_link_through_retention() {
    let project_root = write_selection_gate_project(
        "scenario-bundle-cli-latest-link",
        Some("artifacts:\n  keep_runs: 1\n"),
    );
    let godot = project_root.join(format!("fake-godot{}", env::consts::EXE_SUFFIX));
    let link = project_root.join(".qa/artifacts/bundles/gate/latest");

    let (first_status, first_body) =
        run_scenario_bundle_run_with_godot_args(&project_root, "gate", &godot, &[]);
    assert!(first_status.success());
    assert_eq!(
        Path::new(
            first_body["data"]["artifacts"]["latest_path"]
                .as_str()
                .unwrap()
        ),
        link
    );
    let first_run_root = PathBuf::from(
        first_body["data"]["artifacts"]["run_root"]
            .as_str()
            .unwrap(),
    );
    // Unix links stay relative for movable artifacts; Windows uses a directory junction.
    #[cfg(unix)]
    assert_eq!(
        fs::read_link(&link).unwrap(),
        PathBuf::from(first_run_root.file_name().unwrap())
    );
    #[cfg(windows)]
    assert_eq!(
        fs::canonicalize(&link).unwrap(),
        fs::canonicalize(&first_run_root).unwrap()
    );

    std::thread::sleep(std::time::Duration::from_millis(1100));

    let (second_status, second_body) =
        run_scenario_bundle_run_with_godot_args(&project_root, "gate", &godot, &[]);
    assert!(second_status.success());
    let second_run_root = PathBuf::from(
        second_body["data"]["artifacts"]["run_root"]
            .as_str()
            .unwrap(),
    );
    assert_ne!(first_run_root, second_run_root);
    // `keep_runs: 1` pruned the first run; the link followed the newest one rather than
    // being pruned itself or left dangling.
    assert!(!first_run_root.exists());
    #[cfg(unix)]
    assert_eq!(
        fs::read_link(&link).unwrap(),
        PathBuf::from(second_run_root.file_name().unwrap())
    );
    #[cfg(windows)]
    assert_eq!(
        fs::canonicalize(&link).unwrap(),
        fs::canonicalize(&second_run_root).unwrap()
    );
    assert!(link.join("summary.json").is_file());
    assert_eq!(
        run_dir_names(&project_root.join(".qa/artifacts/bundles/gate")),
        vec![
            second_run_root
                .file_name()
                .unwrap()
                .to_string_lossy()
                .to_string()
        ]
    );

    fs::remove_dir_all(project_root).unwrap();
}

// ---------------------------------------------------------------------------
// Several bundles in one invocation.
// ---------------------------------------------------------------------------

/// Two independent gates, `gate_a` (`alpha`, `charlie`) and `gate_b` (`bravo`, `delta`),
/// with an all-passing `fake-godot` and a `failing-godot` that fails exactly `bravo` and
/// `delta` — so a multi-bundle run can have exactly one red bundle.
fn write_two_gate_project(prefix: &str, config_yaml: Option<&str>) -> PathBuf {
    let project_root = make_project_root(prefix);
    write_fake_godot(&project_root.join(format!("fake-godot{}", env::consts::EXE_SUFFIX)));
    write_selective_failure_fake_godot(
        &project_root.join(format!("failing-godot{}", env::consts::EXE_SUFFIX)),
    );
    if let Some(config_yaml) = config_yaml {
        write(&project_root.join("qa/config.yaml"), config_yaml);
    }
    write(
        &project_root.join("qa/gates/gate_a.json"),
        r#"{"name":"gate_a","required_scenarios":["alpha","charlie"]}"#,
    );
    write(
        &project_root.join("qa/gates/gate_b.json"),
        r#"{"name":"gate_b","required_scenarios":["bravo","delta"]}"#,
    );
    for scenario_name in ["alpha", "bravo", "charlie", "delta"] {
        write(
            &project_root.join(format!("qa/scenarios/{scenario_name}.json")),
            &json!({
                "name": scenario_name,
                "scene": format!("res://{scenario_name}.tscn"),
                "steps": []
            })
            .to_string(),
        );
    }
    project_root
}

fn run_scenario_bundles(
    project_root: &Path,
    names: &[&str],
    godot_bin: Option<&Path>,
    extra_args: &[&str],
) -> (std::process::ExitStatus, Value) {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_godot-qa"));
    cmd.arg("--json").arg("--project").arg(project_root);
    if let Some(godot_bin) = godot_bin {
        cmd.arg("--godot-bin").arg(godot_bin);
    }
    cmd.arg("scenario").arg("bundle").arg("run").args(names);
    cmd.args(extra_args);
    let output = cmd.output().unwrap();
    let body = serde_json::from_slice(&output.stdout).unwrap_or_else(|error| {
        panic!(
            "invalid stdout JSON: {error}\nstdout: {}\nstderr: {}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        )
    });
    (output.status, body)
}

#[test]
fn scenario_bundle_run_aggregates_several_bundles_into_one_envelope() {
    let project_root = write_two_gate_project("scenario-bundle-cli-multi-envelope", None);
    let godot = project_root.join(format!("fake-godot{}", env::consts::EXE_SUFFIX));

    let (status, body) = run_scenario_bundles(
        &project_root,
        &["gate_a", "gate_b"],
        Some(&godot),
        &["--jobs", "1"],
    );

    assert!(status.success(), "body {body}");
    assert_eq!(body["ok"], true);
    assert_eq!(body["command"], "scenario bundle run");
    assert_eq!(body["message"], "scenario bundle run completed (2 bundles)");
    assert_eq!(
        body["data"]["summary"],
        json!({"total": 2, "passed": 2, "failed": 0})
    );

    let bundles = body["data"]["bundles"].as_array().unwrap();
    assert_eq!(bundles.len(), 2);
    assert_eq!(bundles[0]["name"], "gate_a");
    assert_eq!(bundles[1]["name"], "gate_b");
    for (index, expected) in [(0usize, ["alpha", "charlie"]), (1, ["bravo", "delta"])] {
        let names: Vec<&str> = bundles[index]["runs"]
            .as_array()
            .unwrap()
            .iter()
            .map(|run| run["scenario"].as_str().unwrap())
            .collect();
        assert_eq!(names, expected.to_vec());
        // No pool was started at all here, so certainly none is claimed per bundle.
        assert!(bundles[index].get("warm_pool").is_none());
        // Each bundle wrote its own run directory exactly as a solo invocation would.
        let summary_path = bundles[index]["artifacts"]["summary_path"]
            .as_str()
            .unwrap();
        assert!(Path::new(summary_path).is_file());
    }
    // Both `bundles/<name>/` trees exist, each with one run.
    for name in ["gate_a", "gate_b"] {
        let runs = run_dir_names(&project_root.join(format!(".qa/artifacts/bundles/{name}")));
        assert_eq!(runs.len(), 1, "{name} run dirs: {runs:?}");
    }
    // The invocation owns the wall clock; a cold run has no pool spans.
    assert!(body["data"]["timing"]["wall_ms"].is_u64());
    assert!(body["data"]["timing"]["pool_seed_ms"].is_null());
    assert!(body["data"]["timing"]["pool_shutdown_ms"].is_null());
    // Each child's meta.json records the WHOLE invocation, siblings included.
    let meta_path = bundles[0]["artifacts"]["meta_path"].as_str().unwrap();
    let meta = read_json(Path::new(meta_path));
    let command = meta["command"].as_str().unwrap();
    assert!(command.contains("gate_a"), "{command}");
    assert!(command.contains("gate_b"), "{command}");

    fs::remove_dir_all(project_root).unwrap();
}

#[test]
fn scenario_bundle_run_multi_reports_one_flat_failure_list_naming_the_bundle() {
    let project_root = write_two_gate_project("scenario-bundle-cli-multi-failure", None);
    let godot = project_root.join(format!("failing-godot{}", env::consts::EXE_SUFFIX));

    let (status, body) = run_scenario_bundles(
        &project_root,
        &["gate_a", "gate_b"],
        Some(&godot),
        &["--jobs", "1"],
    );

    // One red bundle fails the invocation, exactly as one red child fails a bundle.
    assert_eq!(status.code(), Some(1), "body {body}");
    assert_eq!(body["ok"], false);
    assert_eq!(
        body["message"],
        "scenario bundle run failed (1 of 2 bundles failed)"
    );
    assert_eq!(
        body["data"]["summary"],
        json!({"total": 2, "passed": 1, "failed": 1})
    );
    // Every failure is gate_b's, stamped so the usual "read failures[0]" reflex still
    // identifies which gate broke without cross-referencing `data.bundles[]`.
    let failures = body["failures"].as_array().unwrap();
    assert_eq!(failures.len(), 2);
    for failure in failures {
        assert_eq!(failure["bundle"], "gate_b", "{failure}");
        assert_eq!(failure["type"], "assertion_failed", "{failure}");
        assert!(failure["scenario"].is_string(), "{failure}");
    }
    let bundles = body["data"]["bundles"].as_array().unwrap();
    assert_eq!(bundles[0]["ok"], true);
    assert_eq!(bundles[1]["ok"], false);
    assert_eq!(bundles[1]["failures"].as_array().unwrap().len(), 2);

    fs::remove_dir_all(project_root).unwrap();
}

#[test]
fn scenario_bundle_run_multi_rejects_arguments_it_cannot_honour() {
    let project_root = write_two_gate_project("scenario-bundle-cli-multi-reject", None);
    let godot = project_root.join(format!("fake-godot{}", env::consts::EXE_SUFFIX));

    for (names, extra_args) in [
        // The same gate twice would write two `bundles[]` entries with one name and rotate
        // its `latest` link over its own first run.
        (vec!["gate_a", "gate_a"], vec![]),
        // These three are per-bundle by definition and v1 has no way to say which bundle.
        (vec!["gate_a", "gate_b"], vec!["--equivalence-check"]),
        (vec!["gate_a", "gate_b"], vec!["--only", "alpha"]),
        (vec!["gate_a", "gate_b"], vec!["--failed-only"]),
    ] {
        let (status, body) = run_scenario_bundles(&project_root, &names, Some(&godot), &extra_args);

        assert_eq!(status.code(), Some(2), "{names:?} {extra_args:?} {body}");
        assert_eq!(body["ok"], false, "{names:?} {extra_args:?}");
        assert_eq!(body["command"], "scenario bundle run");
        assert_eq!(body["failures"][0]["type"], "invalid_arguments");
        assert_eq!(body["failures"][0]["command"], "scenario bundle run");
        // The rejection has to say what was wrong with the invocation, not just that
        // something was: `actual` names the repeated bundle or the offending flag.
        assert!(
            body["failures"][0]["message"].is_string(),
            "{names:?} {extra_args:?}"
        );
        let actual = &body["failures"][0]["actual"];
        if extra_args.is_empty() {
            assert_eq!(
                body["failures"][0]["message"],
                "a bundle is named more than once"
            );
            assert_eq!(actual, &json!(["gate_a"]));
        } else {
            assert_eq!(actual["flag"], extra_args[0]);
            assert_eq!(actual["bundles"], json!(names));
        }
        assert!(body["data"].as_object().unwrap().is_empty());
        // Nothing ran: no artifacts root was created at all.
        assert!(
            !project_root.join(".qa/artifacts").exists(),
            "{names:?} {extra_args:?} wrote artifacts"
        );
    }

    fs::remove_dir_all(project_root).unwrap();
}

#[test]
fn scenario_bundle_run_multi_fails_fast_on_an_unknown_bundle_name() {
    let project_root = write_two_gate_project("scenario-bundle-cli-multi-missing", None);
    let godot = project_root.join(format!("fake-godot{}", env::consts::EXE_SUFFIX));

    let (status, body) = run_scenario_bundles(
        &project_root,
        &["gate_a", "typo_gate", "also_typo"],
        Some(&godot),
        &["--jobs", "1"],
    );

    assert_eq!(status.code(), Some(1), "body {body}");
    // Both typos in one result, and the resolvable gate did not run: fixing the invocation
    // costs one pass instead of one failed run per typo.
    let failures = body["failures"].as_array().unwrap();
    assert_eq!(failures.len(), 2);
    assert_eq!(failures[0]["type"], "missing_bundle");
    assert_eq!(failures[0]["actual"], "typo_gate");
    assert_eq!(failures[1]["actual"], "also_typo");
    assert!(body["data"].get("bundles").is_none());
    assert!(!project_root.join(".qa/artifacts").exists());

    fs::remove_dir_all(project_root).unwrap();
}

#[test]
fn scenario_bundle_run_single_name_keeps_the_solo_envelope() {
    let project_root = write_two_gate_project("scenario-bundle-cli-multi-solo", None);
    let godot = project_root.join(format!("fake-godot{}", env::consts::EXE_SUFFIX));

    let (status, body) =
        run_scenario_bundles(&project_root, &["gate_a"], Some(&godot), &["--jobs", "1"]);

    assert!(status.success(), "body {body}");
    // One name is not "a multi-bundle run of one": the envelope is the historical one.
    assert_eq!(body["message"], "scenario bundle run completed");
    assert!(body["data"].get("bundles").is_none());
    assert!(body["data"].get("summary").is_none());
    assert_eq!(body["data"]["bundle"], "gate_a");
    assert_eq!(body["data"]["runs"].as_array().unwrap().len(), 2);

    fs::remove_dir_all(project_root).unwrap();
}

#[test]
fn scenario_bundle_run_multi_warm_takes_one_skip_decision_for_the_invocation() {
    let project_root = write_two_gate_project("scenario-bundle-cli-multi-warm-skip", None);
    let godot = project_root.join(format!("fake-godot{}", env::consts::EXE_SUFFIX));

    let (status, body) = run_scenario_bundles(
        &project_root,
        &["gate_a", "gate_b"],
        Some(&godot),
        &["--warm", "--jobs", "8"],
    );

    assert!(status.success(), "body {body}");
    // Four scenarios over eight lanes: no session would ever be reused, so warm is skipped
    // — and the count that decided it is the SUM, not either gate's own two.
    assert_eq!(body["data"]["warm"]["reason"], "no_reuse_below_parallelism");
    assert_eq!(body["data"]["warm"]["runnable_count_total"], 4);
    assert_eq!(body["data"]["warm"]["ran_warm"], false);
    let bundles = body["data"]["bundles"].as_array().unwrap();
    assert_eq!(bundles.len(), 2);
    for bundle in bundles {
        assert!(bundle.get("warm_pool").is_none());
        for run in bundle["runs"].as_array().unwrap() {
            assert_eq!(run["warm"]["reason"], "warm_skipped_below_parallelism");
            assert_eq!(run["warm"]["ran_warm"], false);
        }
    }

    fs::remove_dir_all(project_root).unwrap();
}

#[test]
fn scenario_bundle_run_multi_warm_reports_one_seed_failure_and_runs_every_bundle_cold() {
    let project_root = write_two_gate_project("scenario-bundle-cli-multi-warm-seed", None);
    let godot = project_root.join(format!("fake-godot{}", env::consts::EXE_SUFFIX));

    // --jobs 1 with four scenarios keeps warm (one session reused serially), so the pool is
    // really seeded — and the fake Godot cannot host a session.
    let (status, body) = run_scenario_bundles(
        &project_root,
        &["gate_a", "gate_b"],
        Some(&godot),
        &["--warm", "--jobs", "1"],
    );

    assert!(status.success(), "body {body}");
    // ONE start failure for the invocation, not one per bundle, and every bundle still ran.
    assert_eq!(body["data"]["warm"]["mode"], "session_start_failed");
    assert_eq!(body["data"]["warm"]["session_started"], false);
    assert!(body["data"]["warm"]["start_failure"].is_object());
    assert_eq!(
        body["data"]["summary"],
        json!({"total": 2, "passed": 2, "failed": 0})
    );
    let bundles = body["data"]["bundles"].as_array().unwrap();
    for bundle in bundles {
        assert_eq!(bundle["runs"].as_array().unwrap().len(), 2);
        assert!(bundle.get("warm").is_none(), "{bundle}");
        assert!(bundle.get("warm_pool").is_none());
    }

    fs::remove_dir_all(project_root).unwrap();
}

#[test]
fn scenario_bundle_run_multi_prunes_pool_sessions_once_for_the_invocation() {
    let project_root = write_two_gate_project(
        "scenario-bundle-cli-multi-retention",
        Some("artifacts:\n  keep_runs: 1\n"),
    );
    let godot = project_root.join(format!("fake-godot{}", env::consts::EXE_SUFFIX));
    let live_dir = project_root.join(".qa/artifacts/live");
    fs::create_dir_all(&live_dir).unwrap();
    let now_millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis();
    let from_this_run = live_dir.join(format!("session-{}-aaaa", now_millis - 60_000));
    let abandoned = live_dir.join(format!("session-{}-bbbb", now_millis - 172_800_000));
    fs::create_dir_all(&from_this_run).unwrap();
    fs::create_dir_all(&abandoned).unwrap();

    let (status, body) = run_scenario_bundles(
        &project_root,
        &["gate_a", "gate_b"],
        Some(&godot),
        &["--warm", "--jobs", "1"],
    );

    assert!(status.success(), "body {body}");
    // The session prune belongs to the invocation, which is the only scope that knows the
    // oldest retained run across every named bundle.
    assert_eq!(body["data"]["retention"]["keep_runs"], 1);
    assert_eq!(body["data"]["retention"]["pruned"]["sessions"], 1);
    assert!(from_this_run.is_dir());
    assert!(!abandoned.exists());
    // No bundle claims a session prune it did not perform.
    for bundle in body["data"]["bundles"].as_array().unwrap() {
        assert_eq!(bundle["retention"]["pruned"]["sessions"], 0, "{bundle}");
    }

    fs::remove_dir_all(project_root).unwrap();
}
