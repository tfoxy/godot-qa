#![allow(dead_code, unused_imports)]

use super::ReloadClass;
use super::runtime;
use super::{
    AudioCommands, Cli, Commands, InputCommands, InputJoyCommands, PathCandidate,
    PhysicsCollisionReportArgs, PhysicsCommands, PhysicsInspectArgs, PhysicsOverlayArgs,
    SESSION_SCRIPT_WAIT_BUSY_UNKNOWN_BACKOFF_MS, ScreenshotCommands, SessionCommands,
    SessionTimeScaleArgs, SurfaceAuditArgs, SurfaceCommands, TraceCommands, TraceRecordArgs,
    audio_analyze_result, binary_surface_has_incompatible_duplicates,
    binary_surface_mismatch_failure, binary_surface_warnings, coalesced_reload_count, command_path,
    discover_binary_command_surface, doctor_binary_surface_report_for_candidates,
    event_summarize_params, event_trace_params, external_harness_path, is_relevant_watch_path,
    physics_collision_report_params, physics_inspect_params, physics_overlay_params,
    public_command_surface, reload_class_for_path, screenshot_result, script_diagnostic_excerpt,
    session_frame_pacing_matches, session_frame_pacing_request,
    session_script_failure_is_retryable, session_script_frame_pacing_request,
    session_script_project_args, session_script_step_op, session_script_wait_busy_backoff_ms,
    session_time_scale_params, snapshot_filter_params, surface_audit_params,
    surface_audit_provider_methods, to_res_path, trace_record_params,
};
use clap::Parser;
use godot_qa_core::config::ProjectConfig;
use godot_qa_core::results::{CommandResult, FailureDetail};
use image::{ImageBuffer, Rgba};
use serde_json::{Map, json};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

fn temp_dir(name: &str) -> PathBuf {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let path = std::env::temp_dir().join(format!("godot-qa-cli-{name}-{unique}"));
    fs::create_dir_all(&path).unwrap();
    path
}

fn test_project_config(root: PathBuf) -> ProjectConfig {
    ProjectConfig {
        project_root: root.clone(),
        repo_root: root.clone(),
        verbose: false,
        godot_binary: None,
        qa_dir: root.join("qa"),
        artifacts_dir: root.join(".qa/artifacts"),
        scenarios_dir: root.join("qa/scenarios"),
        addon_dir: root.join("addons/godot_qa"),
        repo_addon_dir: root.join("addons/godot_qa"),
        godot_launch_prefix: Vec::new(),
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

#[test]
fn watch_filter_accepts_supported_extensions_and_ignores_generated_paths() {
    let root = Path::new("/project");
    assert!(is_relevant_watch_path(
        root,
        Path::new("/project/ui/menu.tscn")
    ));
    assert!(is_relevant_watch_path(
        root,
        Path::new("/project/scripts/player.gd")
    ));
    assert!(is_relevant_watch_path(
        root,
        Path::new("/project/materials/water.gdshader")
    ));
    assert!(!is_relevant_watch_path(
        root,
        Path::new("/project/README.md")
    ));
    assert!(!is_relevant_watch_path(
        root,
        Path::new("/project/.godot/imported/player.gd")
    ));
    assert!(!is_relevant_watch_path(
        root,
        Path::new("/project/.qa/artifacts/live/result.tscn")
    ));
    assert!(!is_relevant_watch_path(
        root,
        Path::new("/project/.qa/session.tres")
    ));
    assert!(!is_relevant_watch_path(
        root,
        Path::new("/project/target/debug/generated.gd")
    ));
}

#[test]
fn watch_debounce_coalesces_event_bursts() {
    assert_eq!(coalesced_reload_count(&[0, 50, 100], 300), 1);
    assert_eq!(coalesced_reload_count(&[0, 50, 400], 300), 2);
    assert_eq!(coalesced_reload_count(&[], 300), 0);
}

#[test]
fn watch_classifies_changes_by_extension() {
    assert_eq!(reload_class_for_path("ui/main.gd"), ReloadClass::Script);
    assert_eq!(
        reload_class_for_path("ui/theme.tres"),
        ReloadClass::Resource
    );
    assert_eq!(
        reload_class_for_path("shaders/glow.gdshader"),
        ReloadClass::Resource
    );
    // `.tscn` changes are faithfully structurally live-edited (with full-scene fallback).
    assert_eq!(
        reload_class_for_path("ui/main.tscn"),
        ReloadClass::Structure
    );
    assert_eq!(reload_class_for_path("data.json"), ReloadClass::FullScene);
}

#[test]
fn to_res_path_normalizes_inputs() {
    let root = Path::new("/project");
    assert_eq!(to_res_path(root, "res://player.gd"), "res://player.gd");
    assert_eq!(to_res_path(root, "/project/ui/main.gd"), "res://ui/main.gd");
    assert_eq!(to_res_path(root, "ui/main.gd"), "res://ui/main.gd");
}
