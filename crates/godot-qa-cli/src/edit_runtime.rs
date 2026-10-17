#![allow(unused_imports)]

use crate::args::*;
use crate::*;
use godot_qa_core::config::{ProjectConfig, load_project_config};
use godot_qa_core::results::{CommandResult, FailureDetail};
use godot_qa_runtime as runtime;
use godot_qa_scenario::{
    artifact_query, artifact_summary,
    audio::{AudioAnalysisOptions, analyze_wav_file_with_options},
    bundle as scenario_bundle, check as scenario_check, contact_sheet as scenario_contact_sheet,
    edit as scenario_edit, init as scenario_init, lint as scenario_lint, live_gate,
    record as scenario_record, review as scenario_review, run as scenario_run, scenario_doc,
    screenshots as scenario_screenshots, time_control, visual_audit,
};
use notify::{Config as NotifyConfig, PollWatcher, RecursiveMode, Watcher};
use serde_json::{Map, Value, json};
use std::collections::BTreeSet;
use std::fmt::Write as _;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
    mpsc,
};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

pub(crate) fn edit_live_result(cli: &Cli, args: &EditArgs) -> CommandResult {
    let config = runtime_config(cli);
    match &args.command {
        EditCommands::Set(args) => {
            scenario_edit::run_edit_live_set(&config, &args.target, &args.property, &args.value)
        }
        EditCommands::Call(args) => {
            scenario_edit::run_edit_live_call(&config, &args.target, &args.method)
        }
        EditCommands::Diff => scenario_edit::run_edit_live_diff(&config),
        EditCommands::ExportPatch(args) => scenario_edit::run_edit_live_export_patch(
            &config,
            &args.out,
            &edit_live_export_patch_command_string(cli, args),
        ),
    }
}

pub(crate) fn edit_live_export_patch_command_string(
    cli: &Cli,
    args: &EditExportPatchArgs,
) -> String {
    let mut parts = vec![
        "godot-qa".to_string(),
        "edit-live".to_string(),
        "export-patch".to_string(),
        "--out".to_string(),
        shell_escape(&args.out),
    ];
    if let Some(project) = &cli.project {
        parts.push("--project".to_string());
        parts.push(shell_escape(project));
    }
    if let Some(godot_bin) = &cli.godot_bin {
        parts.push("--godot-bin".to_string());
        parts.push(shell_escape(godot_bin));
    }
    if let Some(artifact_dir) = &cli.artifact_dir {
        parts.push("--artifact-dir".to_string());
        parts.push(shell_escape(artifact_dir));
    }
    if cli.json {
        parts.push("--json".to_string());
    }
    if cli.verbose {
        parts.push("--verbose".to_string());
    }
    parts.join(" ")
}

pub(crate) fn apply_patch_result(cli: &Cli, args: &ApplyPatchArgs) -> CommandResult {
    let resolved_binary = godot_qa_process::resolve_godot_binary(cli.godot_bin.as_deref());
    let config = load_project_config(
        cli.project.as_deref().map(Path::new),
        cli.godot_bin.as_deref(),
        cli.artifact_dir.as_deref(),
        cli.verbose,
        resolved_binary,
    );
    scenario_edit::run_apply_patch(&config, &args.patch, &apply_patch_command_string(cli, args))
}

pub(crate) fn apply_patch_command_string(cli: &Cli, args: &ApplyPatchArgs) -> String {
    let mut parts = vec![
        "godot-qa".to_string(),
        "apply-patch".to_string(),
        shell_escape(&args.patch),
    ];
    if let Some(project) = &cli.project {
        parts.push("--project".to_string());
        parts.push(shell_escape(project));
    }
    if let Some(godot_bin) = &cli.godot_bin {
        parts.push("--godot-bin".to_string());
        parts.push(shell_escape(godot_bin));
    }
    if let Some(artifact_dir) = &cli.artifact_dir {
        parts.push("--artifact-dir".to_string());
        parts.push(shell_escape(artifact_dir));
    }
    if cli.json {
        parts.push("--json".to_string());
    }
    if cli.verbose {
        parts.push("--verbose".to_string());
    }
    parts.join(" ")
}

pub(crate) fn layout_audit_result(cli: &Cli, args: &LayoutAuditArgs) -> CommandResult {
    let config = load_project_config(
        cli.project.as_deref().map(Path::new),
        cli.godot_bin.as_deref(),
        cli.artifact_dir.as_deref(),
        cli.verbose,
        godot_qa_process::resolve_godot_binary(cli.godot_bin.as_deref()),
    );
    scenario_run::run_layout_audit(
        &config,
        &args.scene,
        &args.viewports,
        args.strict,
        &layout_audit_command_string(cli, args),
    )
}

pub(crate) fn layout_audit_command_string(cli: &Cli, args: &LayoutAuditArgs) -> String {
    let mut parts = vec![
        "godot-qa".to_string(),
        "layout".to_string(),
        "audit".to_string(),
    ];
    if let Some(project) = &cli.project {
        parts.push("--project".to_string());
        parts.push(shell_escape(project));
    }
    if let Some(godot_bin) = &cli.godot_bin {
        parts.push("--godot-bin".to_string());
        parts.push(shell_escape(godot_bin));
    }
    if let Some(artifact_dir) = &cli.artifact_dir {
        parts.push("--artifact-dir".to_string());
        parts.push(shell_escape(artifact_dir));
    }
    parts.push("--scene".to_string());
    parts.push(shell_escape(&args.scene));
    parts.push("--viewports".to_string());
    parts.push(shell_escape(&args.viewports));
    if args.strict {
        parts.push("--strict".to_string());
    }
    if cli.json {
        parts.push("--json".to_string());
    }
    parts.join(" ")
}

pub(crate) fn layout_audit_current_result(
    cli: &Cli,
    args: &LayoutAuditCurrentArgs,
) -> CommandResult {
    let config = load_project_config(
        cli.project.as_deref().map(Path::new),
        cli.godot_bin.as_deref(),
        cli.artifact_dir.as_deref(),
        cli.verbose,
        godot_qa_process::resolve_godot_binary(cli.godot_bin.as_deref()),
    );
    scenario_run::run_layout_audit_current(
        &config,
        args.scenario.as_deref(),
        args.run_id.as_deref(),
        args.snapshot.as_deref(),
        args.live,
        args.strict,
    )
}

pub(crate) fn assert_result(cli: &Cli, args: &AssertArgs) -> CommandResult {
    let config = load_project_config(
        cli.project.as_deref().map(Path::new),
        cli.godot_bin.as_deref(),
        cli.artifact_dir.as_deref(),
        cli.verbose,
        godot_qa_process::resolve_godot_binary(cli.godot_bin.as_deref()),
    );
    godot_qa_scenario::assertions::run_assert(
        &config,
        &args.assertion,
        args.target.as_deref(),
        args.expected.as_deref(),
        godot_qa_scenario::assertions::AssertOptions {
            window_frames: args.window_frames,
            window_seconds: args.window_seconds,
            sample_every_frames: args.sample_every_frames,
            dwell_frames: args.dwell_frames,
            dwell_seconds: args.dwell_seconds,
            max: args.max,
            min: args.min,
            min_ratio: args.min_ratio,
            min_count: args.min_count,
            tolerance: args.tolerance,
            leader: args.leader.clone(),
            property: args.property.clone(),
            forbidden: args.forbidden.clone(),
            occluder: args.occluder.clone(),
            evidence_samples: args.evidence_samples,
            point: args.point.clone(),
            surface: args.surface.clone(),
            provider: args.provider.clone(),
            provider_id: args.provider_id.clone(),
            provider_method: args.provider_method.clone(),
            coordinate_space: args.coordinate_space.clone(),
            radius: args.radius,
            max_points: args.max_points,
        },
        args.live,
        args.scenario.as_deref(),
        args.run_id.as_deref(),
        args.snapshot.as_deref(),
    )
}
