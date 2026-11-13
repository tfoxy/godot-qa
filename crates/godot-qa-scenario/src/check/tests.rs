use super::artifacts::{
    append_check_progress_event, check_profile_artifact_json, check_profile_summary_json,
    prepare_check_artifacts, prepare_test_artifacts, slow_check_summary_json, write_json,
};
use super::execution::test_check_request;
use super::manifest::{load_test_manifest, resolve_check_test_selection};
use super::result::{validate_runtime_test_result, validate_runtime_test_result_with_evidence};
use super::{
    CheckTestArtifacts, CheckTestExecutionMode, CheckTestRunOptions, CheckTestSelectionRequest,
    run_check_tests,
};
use godot_qa_core::config::ProjectConfig;
use serde_json::{Value, json};
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

mod artifacts;
mod execution;
mod manifest;
mod result;

fn project_with_manifest(name: &str, manifest: &str) -> PathBuf {
    let project_root = unique_temp_dir(name);
    fs::create_dir_all(project_root.join("addons/godot_qa")).unwrap();
    fs::create_dir_all(project_root.join("qa/scenarios")).unwrap();
    fs::create_dir_all(project_root.join("qa/tests")).unwrap();
    fs::create_dir_all(project_root.join("scenes")).unwrap();
    fs::create_dir_all(project_root.join("tests")).unwrap();
    fs::write(project_root.join("project.godot"), "[application]\n").unwrap();
    fs::write(
        project_root.join("addons/godot_qa/plugin.cfg"),
        "[plugin]\n",
    )
    .unwrap();
    fs::write(project_root.join("scenes/main.tscn"), "[gd_scene]\n").unwrap();
    fs::write(project_root.join("tests/test_main.gd"), "extends Node\n").unwrap();
    fs::write(project_root.join("qa/tests/manifest.json"), manifest).unwrap();
    project_root
}

fn test_config(project_root: &Path) -> ProjectConfig {
    ProjectConfig {
        project_root: project_root.to_path_buf(),
        repo_root: project_root.to_path_buf(),
        verbose: false,
        godot_binary: Some(
            godot_qa_test_support::fixture(
                &project_root.join("fake-godot"),
                &[("version", "4.5.2.test")],
            )
            .to_string_lossy()
            .into_owned(),
        ),
        qa_dir: project_root.join("qa"),
        artifacts_dir: project_root.join(".qa/artifacts"),
        scenarios_dir: project_root.join("qa/scenarios"),
        addon_dir: project_root.join("addons/godot_qa"),
        repo_addon_dir: project_root.join("addons/godot_qa"),
        addon_identity_cache: Default::default(),
        godot_launch_prefix: Vec::new(),
        godot_joypad_isolation: false,
        godot_windowed_launch_prefix: Vec::new(),
        godot_args: Vec::new(),
        godot_project_args: Vec::new(),
        godot_launch_mode: godot_qa_core::config::GodotLaunchMode::Auto,
        godot_audio_mode: godot_qa_core::config::GodotAudioMode::Auto,
        scenario_warm_default: false,
        scenario_warm_strict_default: false,
        scenario_jobs_default: None,
        project_config_failure: None,
        artifacts_keep_runs: None,
        scenario_schedule_default: None,
        log_ignore_patterns: Vec::new(),
        config_warnings: Vec::new(),
    }
}

fn test_artifacts(project_root: &std::path::Path) -> CheckTestArtifacts {
    let root_dir = project_root.join(".qa/artifacts/check/run-1/tests/accelerated-scene");
    fs::create_dir_all(&root_dir).unwrap();
    CheckTestArtifacts {
        root_dir: root_dir.clone(),
        meta_path: root_dir.join("meta.json"),
        request_path: root_dir.join("request.json"),
        result_path: root_dir.join("result.json"),
        logs_path: root_dir.join("logs.txt"),
        progress_path: root_dir.join("progress.jsonl"),
    }
}

fn successful_process_result(project_root: &Path) -> godot_qa_process::GodotRunResult {
    godot_qa_process::GodotRunResult {
        command: vec!["godot".to_string()],
        returncode: 0,
        termination: None,
        process_timing: None,
        stdout: String::new(),
        stderr: String::new(),
        display: godot_qa_process::resolve_project_display_launch_metadata(project_root, &[], true),
        pid: None,
        process_group: None,
        cleanup: None,
        infrastructure_failure_kind: None,
        timed_out: false,
        timeout_ms: None,
        output_truncation: None,
    }
}

fn selected_ids(
    manifest: &super::TestManifest,
    selection: &super::ResolvedCheckTestSelection,
) -> Vec<String> {
    selection
        .selected_indices
        .iter()
        .map(|index| manifest.entries[*index].id.clone())
        .collect()
}

fn unique_temp_dir(name: &str) -> PathBuf {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    env::temp_dir().join(format!("godot-qa-check-{name}-{unique}"))
}
