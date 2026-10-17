use serde_json::Value;
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

fn run_scenario_list(
    project_root: &Path,
    extra_args: &[&str],
) -> (std::process::ExitStatus, Value) {
    let output = Command::new(env!("CARGO_BIN_EXE_godot-qa"))
        .arg("--json")
        .arg("--project")
        .arg(project_root)
        .args(extra_args)
        .arg("scenario")
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

#[test]
fn scenario_list_succeeds_with_missing_scenarios_dir() {
    let project_root = make_project_root("scenario-list-missing-dir");

    let (status, body) = run_scenario_list(&project_root, &[]);

    fs::remove_dir_all(project_root).unwrap();

    assert!(status.success());
    assert_eq!(body["ok"], true);
    assert_eq!(body["command"], "scenario list");
    assert_eq!(body["message"], "found 0 scenario(s)");
    assert!(body["data"]["scenarios"].as_array().unwrap().is_empty());
    assert!(body["failures"].as_array().unwrap().is_empty());
}

#[test]
fn scenario_list_verbose_reports_missing_scenarios_dir_as_diagnostic() {
    let project_root = make_project_root("scenario-list-missing-dir-verbose");

    let (status, body) = run_scenario_list(&project_root, &["--verbose"]);

    fs::remove_dir_all(project_root).unwrap();

    assert!(status.success());
    assert_eq!(body["ok"], true);
    assert_eq!(
        body["data"]["diagnostics"]["paths"]["project_root_exists"],
        true
    );
    assert_eq!(
        body["data"]["diagnostics"]["paths"]["scenarios_dir_exists"],
        false
    );
    assert_eq!(body["data"]["diagnostics"]["scenario_files_scanned"], 0);
    assert!(body["failures"].as_array().unwrap().is_empty());
}

#[test]
fn scenario_list_succeeds_with_empty_scenarios_dir() {
    let project_root = make_project_root("scenario-list-empty-dir");
    fs::create_dir_all(project_root.join("qa/scenarios")).unwrap();

    let (status, body) = run_scenario_list(&project_root, &[]);

    fs::remove_dir_all(project_root).unwrap();

    assert!(status.success());
    assert_eq!(body["ok"], true);
    assert!(body["data"]["scenarios"].as_array().unwrap().is_empty());
    assert!(body["failures"].as_array().unwrap().is_empty());
}

fn sample_project_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples/sample_project")
        .canonicalize()
        .expect("sample project must exist")
}

#[test]
fn scenario_list_excludes_abstract_templates_and_lists_children() {
    let project_root = sample_project_root();

    let (status, body) = run_scenario_list(&project_root, &[]);

    assert!(status.success());
    let names: Vec<String> = body["data"]["scenarios"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|scenario| scenario["name"].as_str().map(str::to_string))
        .collect();
    assert!(names.iter().any(|name| name == "menu_smoke_default"));
    assert!(names.iter().any(|name| name == "menu_smoke_visual"));
    // The abstract base template (menu_smoke.base.yaml) is excluded from listing.
    assert!(!names.iter().any(|name| name == "menu_smoke.base"));
}
