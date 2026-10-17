use serde_json::{Value, json};
use std::env;
use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus};
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
    fs::create_dir_all(root.join("qa/scenarios")).unwrap();
    fs::create_dir_all(root.join("addons/godot_qa")).unwrap();
    fs::write(root.join("project.godot"), "[application]\n").unwrap();
    // Install a matching-version addon so the version-drift check stays clean; tests that
    // exercise the mismatch warning write their own plugin.cfg version.
    fs::write(
        root.join("addons/godot_qa/plugin.cfg"),
        format!("[plugin]\nversion=\"{}\"\n", env!("CARGO_PKG_VERSION")),
    )
    .unwrap();
    root
}

fn write(path: &Path, contents: &str) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(path, contents).unwrap();
}

fn run_doctor(project_root: &Path, godot_bin: &Path) -> (std::process::ExitStatus, Value) {
    run_doctor_with_env(project_root, godot_bin, &[], None)
}

fn run_doctor_with_env(
    project_root: &Path,
    godot_bin: &Path,
    doctor_args: &[&str],
    path_override: Option<OsString>,
) -> (ExitStatus, Value) {
    let mut command = Command::new(env!("CARGO_BIN_EXE_godot-qa"));
    command
        .arg("--json")
        .arg("--project")
        .arg(project_root)
        .arg("--godot-bin")
        .arg(godot_bin)
        .arg("doctor");
    command.args(doctor_args);
    if let Some(path) = path_override {
        command.env("PATH", path);
    }
    let output = command.output().unwrap();
    let body = serde_json::from_slice(&output.stdout).unwrap_or_else(|error| {
        panic!(
            "invalid stdout JSON: {error}\nstdout: {}\nstderr: {}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        )
    });
    (output.status, body)
}

fn fake_godot(root: &Path) -> PathBuf {
    godot_qa_test_support::fixture(
        &root.join("tools/godot"),
        &[("stdout", "Godot_v4.3-stable\n")],
    )
}

fn current_binary_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_godot-qa"))
        .parent()
        .unwrap()
        .to_path_buf()
}

fn path_var(dirs: &[PathBuf]) -> OsString {
    env::join_paths(dirs).unwrap()
}

fn fake_godot_qa(dir: &Path, options: &[(&str, &str)]) -> PathBuf {
    godot_qa_test_support::fixture(&dir.join("godot-qa"), options)
}

fn warning_types(body: &Value) -> Vec<&str> {
    body["data"]["warnings"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|warning| warning["type"].as_str())
        .collect()
}

#[test]
fn doctor_json_succeeds_with_version_probe_direct_fallback() {
    let project_root = make_project_root("doctor-cli-fallback");
    let wrapper =
        godot_qa_test_support::fixture(&project_root.join("tools/wrapper"), &[("exit", "17")]);
    let godot = fake_godot(&project_root);
    write(
        &project_root.join("qa/config.yaml"),
        &format!(
            "godot:\n  launch_prefix:\n    - {}\n",
            wrapper.to_string_lossy()
        ),
    );

    let (status, body) = run_doctor(&project_root, &godot);

    fs::remove_dir_all(project_root).unwrap();

    assert!(status.success());
    assert_eq!(body["ok"], true);
    assert_eq!(body["data"]["godot"]["version"], "Godot_v4.3-stable");
    let probe = &body["data"]["godot"]["version_probe"];
    assert_eq!(probe["fallback_used"], true);
    assert_eq!(probe["fallback_reason"], "exited with status 17");
    assert_eq!(probe["attempts"].as_array().unwrap().len(), 2);
}

#[test]
fn doctor_json_fails_when_prefixed_and_direct_version_probes_fail() {
    let project_root = make_project_root("doctor-cli-both-fail");
    let wrapper =
        godot_qa_test_support::fixture(&project_root.join("tools/wrapper"), &[("exit", "17")]);
    let godot =
        godot_qa_test_support::fixture(&project_root.join("tools/godot"), &[("exit", "23")]);
    write(
        &project_root.join("qa/config.yaml"),
        &format!(
            "godot:\n  launch_prefix:\n    - {}\n",
            wrapper.to_string_lossy()
        ),
    );

    let (status, body) = run_doctor(&project_root, &godot);

    fs::remove_dir_all(project_root).unwrap();

    assert!(!status.success());
    assert_eq!(body["ok"], false);
    assert_eq!(body["failures"][0]["type"], "godot_version_probe_failed");
    assert_eq!(
        body["failures"][0]["version_probe"]["failure_type"],
        "godot_version_probe_failed"
    );
}

#[test]
fn doctor_json_fails_when_prefixed_and_direct_version_probes_are_empty() {
    let project_root = make_project_root("doctor-cli-both-empty");
    let wrapper =
        godot_qa_test_support::fixture(&project_root.join("tools/wrapper"), &[("exit", "0")]);
    let godot = godot_qa_test_support::fixture(&project_root.join("tools/godot"), &[("exit", "0")]);
    write(
        &project_root.join("qa/config.yaml"),
        &format!(
            "godot:\n  launch_prefix:\n    - {}\n",
            wrapper.to_string_lossy()
        ),
    );

    let (status, body) = run_doctor(&project_root, &godot);

    fs::remove_dir_all(project_root).unwrap();

    assert!(!status.success());
    assert_eq!(body["ok"], false);
    assert_eq!(body["failures"][0]["type"], "godot_version_probe_failed");
    assert_eq!(body["failures"][0]["actual"], "empty_output");
}

#[test]
fn doctor_binary_surface_reports_current_and_path_binaries() {
    let project_root = make_project_root("doctor-binary-surface-current");
    let godot = fake_godot(&project_root);
    let path = path_var(&[current_binary_dir()]);

    let (status, body) = run_doctor_with_env(&project_root, &godot, &[], Some(path));

    fs::remove_dir_all(project_root).unwrap();

    assert!(status.success());
    assert_eq!(body["ok"], true);
    assert!(body["failures"].as_array().unwrap().is_empty());

    let surface = &body["data"]["binary_surface"];
    let current_exe = surface["current_exe"].as_str().unwrap();
    let first_path_binary = surface["first_path_binary"].as_str().unwrap();
    assert_eq!(first_path_binary, current_exe);
    assert_eq!(surface["summary"]["candidate_count"], 1);
    let candidates = surface["candidates"].as_array().unwrap();
    assert_eq!(candidates.len(), 1);
    assert_eq!(candidates[0]["canonical_path"], current_exe);
    assert_eq!(candidates[0]["is_current_exe"], true);
    assert_eq!(candidates[0]["is_first_on_path"], true);
    assert!(
        candidates[0]["missing_commands"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    assert!(candidates[0]["probe_errors"].as_array().unwrap().is_empty());
    assert!(warning_types(&body).is_empty());
}

#[test]
fn doctor_reports_addon_version_fields_and_drift() {
    let project_root = make_project_root("doctor-addon-version-match");
    let godot = fake_godot(&project_root);

    let (status, body) = run_doctor(&project_root, &godot);
    assert!(status.success());
    assert_eq!(body["ok"], true);
    assert_eq!(body["data"]["cli"]["version"], env!("CARGO_PKG_VERSION"));
    assert_eq!(body["data"]["addon"]["version"], env!("CARGO_PKG_VERSION"));
    assert!(body["data"]["addon"]["source_version"].is_string());
    assert!(!warning_types(&body).contains(&"addon_version_mismatch"));
    fs::remove_dir_all(project_root).unwrap();

    let drift_root = make_project_root("doctor-addon-version-drift");
    let drift_godot = fake_godot(&drift_root);
    fs::write(
        drift_root.join("addons/godot_qa/plugin.cfg"),
        "[plugin]\nversion=\"0.0.1\"\n",
    )
    .unwrap();

    let (status, body) = run_doctor(&drift_root, &drift_godot);
    // Drift is a default warning, not a failure.
    assert!(status.success());
    assert_eq!(body["ok"], true);
    assert!(body["failures"].as_array().unwrap().is_empty());
    assert_eq!(body["data"]["addon"]["version"], "0.0.1");
    assert!(warning_types(&body).contains(&"addon_version_mismatch"));

    // --strict-addon-version promotes the drift to a structured failure.
    let (status, body) =
        run_doctor_with_env(&drift_root, &drift_godot, &["--strict-addon-version"], None);
    assert!(!status.success());
    assert_eq!(body["ok"], false);
    let failure_types: Vec<&str> = body["failures"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|failure| failure["type"].as_str())
        .collect();
    assert!(failure_types.contains(&"addon_version_mismatch"));
    fs::remove_dir_all(drift_root).unwrap();
}

#[test]
fn doctor_binary_surface_warns_for_stale_binary_before_current_on_path() {
    let project_root = make_project_root("doctor-binary-surface-stale");
    let godot = fake_godot(&project_root);
    let stale_dir = project_root.join("stale-bin");
    fake_godot_qa(
        &stale_dir,
        &[
            ("version", "godot-qa 0.0.1-stale"),
            ("response:--help", "Commands:\n  doctor\n  scenario\n"),
            ("response:doctor --help", "Usage: godot-qa doctor\n"),
            ("response:scenario --help", "Commands:\n  run\n"),
            (
                "response:scenario run --help",
                "Usage: godot-qa scenario run\n",
            ),
            ("exit", "2"),
        ],
    );
    let path = path_var(&[stale_dir, current_binary_dir()]);

    let (status, body) = run_doctor_with_env(&project_root, &godot, &[], Some(path));

    fs::remove_dir_all(project_root).unwrap();

    assert!(status.success());
    assert_eq!(body["ok"], true);
    assert!(body["failures"].as_array().unwrap().is_empty());
    assert!(warning_types(&body).contains(&"multiple_godot_qa_binaries"));

    let surface = &body["data"]["binary_surface"];
    assert_eq!(surface["summary"]["candidate_count"], 2);
    assert_eq!(surface["summary"]["incompatible_candidate_count"], 1);
    let stale = &surface["candidates"].as_array().unwrap()[0];
    assert_eq!(stale["path_order"], 0);
    assert_eq!(stale["is_first_on_path"], true);
    assert_eq!(stale["version"]["version"], "godot-qa 0.0.1-stale");
    assert!(
        stale["missing_commands"]
            .as_array()
            .unwrap()
            .iter()
            .any(|command| command == "scenario profile")
    );
    assert!(
        stale["remediation"]
            .as_str()
            .unwrap()
            .contains("shadows the expected godot-qa surface")
    );
    let warning = &body["data"]["warnings"].as_array().unwrap()[0];
    assert_eq!(warning["path_order"], 0);
    assert_eq!(warning["missing_commands"], stale["missing_commands"]);
    assert_eq!(warning["remediation"], stale["remediation"]);
}

#[test]
fn doctor_binary_surface_strict_fails_for_incompatible_duplicate() {
    let project_root = make_project_root("doctor-binary-surface-strict");
    let godot = fake_godot(&project_root);
    let stale_dir = project_root.join("stale-bin");
    fake_godot_qa(
        &stale_dir,
        &[
            ("version", "godot-qa 0.0.1-stale"),
            ("response:--help", "Commands:\n  doctor\n"),
            ("response:doctor --help", "Usage: godot-qa doctor\n"),
            ("exit", "2"),
        ],
    );
    let path = path_var(&[stale_dir, current_binary_dir()]);

    let (status, body) = run_doctor_with_env(
        &project_root,
        &godot,
        &["--strict-binary-surface"],
        Some(path),
    );

    fs::remove_dir_all(project_root).unwrap();

    assert!(!status.success());
    assert_eq!(body["ok"], false);
    assert!(warning_types(&body).contains(&"multiple_godot_qa_binaries"));
    let failures = body["failures"].as_array().unwrap();
    let failure = failures
        .iter()
        .find(|failure| failure["type"] == "binary_surface_mismatch")
        .unwrap();
    assert_eq!(failure["actual"]["candidates"].as_array().unwrap().len(), 2);
    assert_eq!(
        failure["actual"]["candidates"][0]["missing_commands"],
        body["data"]["binary_surface"]["candidates"][0]["missing_commands"]
    );
}

#[test]
fn doctor_binary_surface_reports_malformed_and_hanging_duplicate_probes() {
    let project_root = make_project_root("doctor-binary-surface-probes");
    let godot = fake_godot(&project_root);
    let nonzero_dir = project_root.join("nonzero-bin");
    let malformed_dir = project_root.join("malformed-bin");
    let hanging_dir = project_root.join("hanging-bin");
    let inaccessible_dir = project_root.join("inaccessible-bin");
    let inaccessible = fake_godot_qa(&inaccessible_dir, &[]);
    let inaccessible_path = inaccessible.to_string_lossy();
    fake_godot_qa(
        &nonzero_dir,
        &[
            ("version", "nonzero-version"),
            ("remove_file", &inaccessible_path),
            ("stderr", "bad-help\n"),
            ("exit", "12"),
        ],
    );
    fake_godot_qa(
        &malformed_dir,
        &[
            ("version", "malformed-version"),
            ("stdout", "not clap help\n"),
        ],
    );
    fake_godot_qa(
        &hanging_dir,
        &[("version", "hanging-version"), ("sleep_ms", "5000")],
    );
    // All PATH candidates are discovered before probing. The first executable
    // removes this later candidate to reproduce a stale-discovery spawn failure.
    let path = path_var(&[
        nonzero_dir,
        malformed_dir,
        hanging_dir,
        inaccessible_dir,
        current_binary_dir(),
    ]);

    let (status, body) = run_doctor_with_env(&project_root, &godot, &[], Some(path));

    fs::remove_dir_all(project_root).unwrap();

    assert!(status.success());
    assert_eq!(body["ok"], true);
    let candidates = body["data"]["binary_surface"]["candidates"]
        .as_array()
        .unwrap();
    assert_eq!(candidates.len(), 5);
    assert_eq!(
        body["data"]["binary_surface"]["summary"]["probe_error_count"],
        3
    );

    let nonzero_error = &candidates[0]["probe_errors"].as_array().unwrap()[0];
    assert_eq!(nonzero_error["probe"], "help");
    assert_eq!(nonzero_error["status"], 12);
    assert_eq!(nonzero_error["timed_out"], false);
    assert!(
        nonzero_error["message"]
            .as_str()
            .unwrap()
            .contains("exited with status")
    );

    assert!(candidates[1]["probe_errors"].as_array().unwrap().is_empty());
    assert!(
        candidates[1]["missing_commands"]
            .as_array()
            .unwrap()
            .iter()
            .any(|command| command == "doctor")
    );

    let timeout_error = &candidates[2]["probe_errors"].as_array().unwrap()[0];
    assert_eq!(timeout_error["probe"], "help");
    assert_eq!(timeout_error["timed_out"], true);
    assert_eq!(timeout_error["error_kind"], "timeout");

    let spawn_error = &candidates[3]["probe_errors"].as_array().unwrap()[0];
    assert_eq!(spawn_error["probe"], "help");
    assert_eq!(spawn_error["status"], Value::Null);
    assert_eq!(spawn_error["timed_out"], false);
    assert_eq!(spawn_error["error_kind"], "NotFound");
    assert!(!spawn_error["message"].as_str().unwrap().is_empty());
    assert!(warning_types(&body).contains(&"multiple_godot_qa_binaries"));
}

#[test]
fn doctor_binary_surface_does_not_mask_project_failures() {
    let project_root = env::temp_dir().join(format!(
        "godot-qa-doctor-binary-surface-missing-{}",
        unique_suffix()
    ));
    fs::create_dir_all(&project_root).unwrap();
    fs::write(project_root.join("project.godot"), "[application]\n").unwrap();
    write(&project_root.join("qa/config.yaml"), "godot: true\n");
    let fixture_root = make_project_root("doctor-binary-surface-missing-fixtures");
    let godot = fake_godot(&fixture_root);
    let stale_dir = fixture_root.join("stale-bin");
    fake_godot_qa(
        &stale_dir,
        &[
            ("version", "stale"),
            ("response:--help", "Commands:\n  doctor\n"),
            ("response:doctor --help", "Usage: godot-qa doctor\n"),
            ("exit", "2"),
        ],
    );
    let path = path_var(&[stale_dir, current_binary_dir()]);

    let (status, body) = run_doctor_with_env(
        &project_root,
        &godot,
        &["--strict-binary-surface"],
        Some(path),
    );

    fs::remove_dir_all(fixture_root).unwrap();
    fs::remove_dir_all(project_root).unwrap();

    assert!(!status.success());
    assert_eq!(body["ok"], false);
    let failure_types: Vec<&str> = body["failures"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|failure| failure["type"].as_str())
        .collect();
    assert!(failure_types.contains(&"binary_surface_mismatch"));
    assert!(failure_types.contains(&"invalid_project_config"));
    assert!(!failure_types.contains(&"missing_scenarios_dir"));
    assert!(warning_types(&body).contains(&"multiple_godot_qa_binaries"));
}

#[test]
fn doctor_warns_for_unsupported_config_file() {
    let project_root = make_project_root("doctor-cli-unsupported-config");
    let godot = fake_godot(&project_root);
    // A real consumer shipped qa/config.json expecting it to be honored; it is ignored.
    write(
        &project_root.join("qa/config.json"),
        r#"{"godot":{"windowed_launch_prefix":["xvfb-run","-a"]}}"#,
    );

    let (status, body) = run_doctor(&project_root, &godot);

    fs::remove_dir_all(project_root).unwrap();

    // Unsupported spellings are a warning, not a failure: exit 0, ok true.
    assert!(status.success());
    assert_eq!(body["ok"], true);
    assert!(body["failures"].as_array().unwrap().is_empty());
    assert!(warning_types(&body).contains(&"unsupported_config_file"));
    let warning = body["data"]["warnings"]
        .as_array()
        .unwrap()
        .iter()
        .find(|warning| warning["type"] == "unsupported_config_file")
        .unwrap();
    assert_eq!(warning["severity"], "warning");
    assert_eq!(warning["path"], "qa/config.json");
    assert_eq!(
        warning["expected"],
        "qa/config.yaml (or qa/config.local.yaml)"
    );
    assert!(warning["remediation"].is_string());
}

#[test]
fn doctor_warns_for_unknown_config_keys() {
    let project_root = make_project_root("doctor-cli-unknown-config-key");
    let godot = fake_godot(&project_root);
    // A guessed key silently disables the feature the author thought they were enabling.
    write(
        &project_root.join("qa/config.yaml"),
        "warmStrict: true\nscenario:\n  warm: true\n  parallelism: 8\n",
    );

    let (status, body) = run_doctor(&project_root, &godot);

    fs::remove_dir_all(project_root).unwrap();

    // Unknown keys are a warning, not a failure: exit 0, ok true.
    assert!(status.success());
    assert_eq!(body["ok"], true);
    assert!(body["failures"].as_array().unwrap().is_empty());
    assert!(warning_types(&body).contains(&"unknown_config_key"));
    let keys: Vec<&str> = body["data"]["warnings"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|warning| warning["type"] == "unknown_config_key")
        .filter_map(|warning| warning["key"].as_str())
        .collect();
    assert_eq!(keys, vec!["warmStrict", "scenario.parallelism"]);
    let warning = body["data"]["warnings"]
        .as_array()
        .unwrap()
        .iter()
        .find(|warning| warning["key"] == "scenario.parallelism")
        .unwrap();
    assert_eq!(warning["severity"], "warning");
    assert_eq!(warning["path"], "qa/config.yaml");
    assert_eq!(
        warning["expected"],
        json!(["warm", "warm_strict", "jobs", "schedule"])
    );
    assert!(warning["remediation"].is_string());
}

#[test]
fn doctor_known_config_keys_have_no_unknown_key_warning() {
    let project_root = make_project_root("doctor-cli-known-config-keys");
    let godot = fake_godot(&project_root);
    write(
        &project_root.join("qa/config.yaml"),
        concat!(
            "godot:\n  launch_mode: headless\n",
            "scenario:\n  warm: true\n  schedule: longest-first\n",
            "artifacts:\n  keep_runs: 5\n",
            "logs:\n  ignore:\n    - ObjectDB instances leaked\n",
        ),
    );

    let (status, body) = run_doctor(&project_root, &godot);

    fs::remove_dir_all(project_root).unwrap();

    assert!(status.success());
    assert_eq!(body["ok"], true);
    assert!(!warning_types(&body).contains(&"unknown_config_key"));
}

#[test]
fn doctor_supported_config_has_no_unsupported_config_warning() {
    let project_root = make_project_root("doctor-cli-supported-config");
    let godot = fake_godot(&project_root);
    write(
        &project_root.join("qa/config.yaml"),
        "godot:\n  launch_mode: headless\n",
    );

    let (status, body) = run_doctor(&project_root, &godot);

    fs::remove_dir_all(project_root).unwrap();

    assert!(status.success());
    assert_eq!(body["ok"], true);
    assert!(!warning_types(&body).contains(&"unsupported_config_file"));
}

#[test]
fn doctor_json_succeeds_without_scenarios_dir() {
    let project_root = make_project_root("doctor-cli-no-scenarios-dir");
    fs::remove_dir_all(project_root.join("qa/scenarios")).unwrap();
    let godot = fake_godot(&project_root);

    let (status, body) = run_doctor(&project_root, &godot);

    fs::remove_dir_all(project_root).unwrap();

    assert!(status.success());
    assert_eq!(body["ok"], true);
    assert!(body["data"]["scenarios_dir"].is_string());
    let failure_types: Vec<&str> = body["failures"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|failure| failure["type"].as_str())
        .collect();
    assert!(!failure_types.contains(&"missing_scenarios_dir"));
}
