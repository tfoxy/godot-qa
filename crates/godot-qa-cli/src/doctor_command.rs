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

pub(crate) fn doctor_result(cli: &Cli, args: &DoctorArgs) -> CommandResult {
    let resolved_binary = godot_qa_process::resolve_godot_binary(cli.godot_bin.as_deref());
    let config = load_project_config(
        cli.project.as_deref().map(Path::new),
        cli.godot_bin.as_deref(),
        cli.artifact_dir.as_deref(),
        cli.verbose,
        resolved_binary,
    );
    let command = "doctor";
    let mut failures = Vec::new();
    let project_exists = config.project_root.exists();
    if let Some(failure) = config.project_config_failure_for_command(command) {
        failures.push(failure);
    }
    let mut version_probe = None;
    let version = match &config.godot_binary {
        Some(binary) if config.project_config_failure.is_none() => {
            match godot_qa_process::probe_godot_version_with_metadata(
                binary,
                &config.godot_launch_prefix,
            ) {
                Ok(result) => {
                    version_probe = Some(result.metadata.to_json());
                    Some(result.version)
                }
                Err(failure) => {
                    version_probe = failure.details.get("version_probe").cloned();
                    failures.push(failure);
                    None
                }
            }
        }
        Some(_) => None,
        None => {
            failures.push(missing_godot_failure(command));
            None
        }
    };

    if !project_exists {
        failures.push(missing_project_root_failure(command, &config.project_root));
    }

    let addon_status = addon_status(&config);
    if addon_status == "missing" {
        failures.push(missing_addon_failure(command, &config.addon_dir));
    }
    let cli_version = env!("CARGO_PKG_VERSION");
    let installed_addon_version = read_plugin_cfg_version(&config.addon_dir.join("plugin.cfg"));
    let source_addon_version = read_plugin_cfg_version(&config.repo_addon_dir.join("plugin.cfg"));

    let artifact_failure = if project_exists {
        ensure_writable_artifact_dir(&config.artifacts_dir, command)
    } else {
        None
    };
    if let Some(failure) = artifact_failure.clone() {
        failures.push(failure);
    }

    let mut data = serde_json::Map::new();
    data.insert("project_root".to_string(), json!(config.project_root));
    data.insert("repo_root".to_string(), json!(config.repo_root));
    data.insert("qa_dir".to_string(), json!(config.qa_dir));
    data.insert("scenarios_dir".to_string(), json!(config.scenarios_dir));
    let mut godot_data = serde_json::Map::new();
    godot_data.insert("binary".to_string(), json!(config.godot_binary));
    godot_data.insert("version".to_string(), json!(version));
    if let Some(probe) = version_probe {
        godot_data.insert("version_probe".to_string(), probe);
    }
    data.insert("godot".to_string(), serde_json::Value::Object(godot_data));
    data.insert(
        "addon".to_string(),
        json!({
            "status": addon_status,
            "path": config.addon_dir,
            "source_path": config.repo_addon_dir,
            "version": installed_addon_version,
            "source_version": source_addon_version,
        }),
    );
    data.insert(
        "cli".to_string(),
        json!({
            "version": cli_version,
        }),
    );
    data.insert(
        "artifact_dir".to_string(),
        json!({
            "path": config.artifacts_dir,
            "writable": project_exists && artifact_failure.is_none(),
        }),
    );
    if config.verbose {
        data.insert("diagnostics".to_string(), verbose_diagnostics(&config, cli));
    }
    let binary_surface = doctor_binary_surface_report();
    let mut warnings = binary_surface_warnings(&binary_surface);
    if args.strict_binary_surface && binary_surface_has_incompatible_duplicates(&binary_surface) {
        failures.push(binary_surface_mismatch_failure(&binary_surface));
    }
    data.insert("binary_surface".to_string(), binary_surface);
    // Addon version drift: the installed addon plugin.cfg version must match the CLI version.
    // Symlinked addons point back at this repo, so they match by construction and stay clean.
    if let Some(installed_version) = installed_addon_version.as_deref()
        && installed_version != cli_version
    {
        warnings.push(addon_version_mismatch_warning(
            &config.addon_dir,
            installed_version,
            source_addon_version.as_deref(),
            cli_version,
        ));
        if args.strict_addon_version {
            failures.push(addon_version_mismatch_failure(
                &config.addon_dir,
                installed_version,
                cli_version,
            ));
        }
    }
    // Unsupported `qa/config.*` spellings are a warning, never a failure: they keep
    // doctor `ok: true`/exit 0, exactly like addon_version_mismatch above.
    warnings.extend(config.config_warnings.iter().cloned());
    if !warnings.is_empty() {
        data.insert("warnings".to_string(), json!(warnings));
    }

    let exit_code = if failures.is_empty() { 0 } else { 1 };
    CommandResult {
        ok: failures.is_empty(),
        command: command.to_string(),
        message: if failures.is_empty() {
            "environment ready".to_string()
        } else {
            "environment has issues".to_string()
        },
        data,
        failures,
        exit_code,
    }
}

pub(crate) fn doctor_binary_surface_report() -> Value {
    let candidates = godot_qa_path_candidates();
    doctor_binary_surface_report_for_candidates(candidates)
}

pub(crate) fn doctor_binary_surface_report_for_candidates(candidates: Vec<PathCandidate>) -> Value {
    let current_surface: BTreeSet<String> = public_command_surface()
        .into_iter()
        .map(str::to_string)
        .collect();
    let expected_commands: Vec<String> = current_surface.iter().cloned().collect();
    let current_exe = current_exe_canonical_string();
    let first_path_binary = candidates
        .first()
        .map(|candidate| candidate.canonical_path.to_string_lossy().into_owned());
    let mut candidate_payloads = Vec::new();
    let mut incompatible = 0usize;
    let mut probe_error_count = 0usize;
    for candidate in candidates {
        let mut probe_errors = Vec::new();
        let surface = match discover_binary_command_surface(&candidate.canonical_path) {
            Ok(surface) => surface,
            Err(error) => {
                probe_errors.push(error);
                BTreeSet::new()
            }
        };
        let version_probe = binary_version_probe(&candidate.canonical_path);
        let missing: Vec<String> = current_surface.difference(&surface).cloned().collect();
        let extra: Vec<String> = surface.difference(&current_surface).cloned().collect();
        let is_current_exe = current_exe
            .as_deref()
            .map(|current| current == candidate.canonical_path.to_string_lossy())
            .unwrap_or(false);
        let differs = !missing.is_empty() || !extra.is_empty();
        if differs {
            incompatible += 1;
        }
        if !probe_errors.is_empty() {
            probe_error_count += 1;
        }
        candidate_payloads.push(json!({
            "path_order": candidate.path_order,
            "raw_path": candidate.raw_path.to_string_lossy(),
            "canonical_path": candidate.canonical_path.to_string_lossy(),
            "is_current_exe": is_current_exe,
            "is_first_on_path": candidate.path_order == 0,
            "version": version_probe,
            "command_count": surface.len(),
            "missing_commands": missing,
            "extra_commands": extra,
            "probe_errors": probe_errors,
            "remediation": binary_surface_remediation(&candidate.canonical_path, candidate.path_order == 0, differs),
        }));
    }
    let candidate_count = candidate_payloads.len();
    json!({
        "current_exe": current_exe,
        "first_path_binary": first_path_binary,
        "expected_commands": expected_commands,
        "expected_source": "rust_public_command_surface",
        "candidates": candidate_payloads,
        "summary": {
            "candidate_count": candidate_count,
            "incompatible_candidate_count": incompatible,
            "probe_error_count": probe_error_count,
        }
    })
}

pub(crate) fn current_exe_canonical_string() -> Option<String> {
    std::env::current_exe()
        .ok()
        .and_then(|path| path.canonicalize().ok())
        .map(|path| path.to_string_lossy().into_owned())
}

pub(crate) fn binary_surface_warnings(report: &Value) -> Vec<Value> {
    let Some(candidates) = report["candidates"].as_array() else {
        return Vec::new();
    };
    candidates
        .iter()
        .filter(|candidate| {
            candidate["missing_commands"]
                .as_array()
                .is_some_and(|values| !values.is_empty())
                || candidate["extra_commands"]
                    .as_array()
                    .is_some_and(|values| !values.is_empty())
                || candidate["probe_errors"]
                    .as_array()
                    .is_some_and(|values| !values.is_empty())
        })
        .map(|candidate| {
            json!({
                "type": "multiple_godot_qa_binaries",
                "severity": "warning",
                "message": "a godot-qa binary on PATH has an incompatible or unprobeable command surface",
                "current_exe": report["current_exe"].clone(),
                "first_path_binary": report["first_path_binary"].clone(),
                "path_order": candidate["path_order"].clone(),
                "raw_path": candidate["raw_path"].clone(),
                "canonical_path": candidate["canonical_path"].clone(),
                "is_current_exe": candidate["is_current_exe"].clone(),
                "is_first_on_path": candidate["is_first_on_path"].clone(),
                "version": candidate["version"].clone(),
                "command_count": candidate["command_count"].clone(),
                "missing_commands": candidate["missing_commands"].clone(),
                "extra_commands": candidate["extra_commands"].clone(),
                "probe_errors": candidate["probe_errors"].clone(),
                "remediation": candidate["remediation"].clone(),
            })
        })
        .collect()
}

pub(crate) fn binary_surface_has_incompatible_duplicates(report: &Value) -> bool {
    let candidate_count = report["summary"]["candidate_count"].as_u64().unwrap_or(0);
    if candidate_count < 2 {
        return false;
    }
    report["candidates"]
        .as_array()
        .into_iter()
        .flatten()
        .any(|candidate| {
            candidate["missing_commands"]
                .as_array()
                .is_some_and(|values| !values.is_empty())
                || candidate["extra_commands"]
                    .as_array()
                    .is_some_and(|values| !values.is_empty())
        })
}

/// Read the `version=` value from an addon `plugin.cfg`. Returns `None` when the file is
/// absent (no addon installed there), and `"0.0.0"` when the file exists but carries no
/// parsable version so drift is still surfaced with a clear, comparable value.
pub(crate) fn read_plugin_cfg_version(plugin_cfg_path: &Path) -> Option<String> {
    let contents = std::fs::read_to_string(plugin_cfg_path).ok()?;
    Some(parse_plugin_cfg_version(&contents))
}

pub(crate) fn parse_plugin_cfg_version(contents: &str) -> String {
    for line in contents.lines() {
        let trimmed = line.trim();
        let Some(rest) = trimmed.strip_prefix("version") else {
            continue;
        };
        let Some(value) = rest.trim_start().strip_prefix('=') else {
            continue;
        };
        let value = value.trim().trim_matches('"').trim();
        if !value.is_empty() {
            return value.to_string();
        }
    }
    "0.0.0".to_string()
}

pub(crate) fn addon_version_mismatch_warning(
    addon_dir: &Path,
    installed_version: &str,
    source_version: Option<&str>,
    cli_version: &str,
) -> Value {
    json!({
        "type": "addon_version_mismatch",
        "severity": "warning",
        "message": "installed godot_qa addon version does not match the godot-qa CLI version",
        "addon_path": addon_dir.to_string_lossy(),
        "addon_version": installed_version,
        "source_version": source_version,
        "cli_version": cli_version,
        "remediation": "reinstall or copy addons/godot_qa from this godot-qa release so the installed addon version matches the CLI version",
    })
}

pub(crate) fn addon_version_mismatch_failure(
    addon_dir: &Path,
    installed_version: &str,
    cli_version: &str,
) -> FailureDetail {
    FailureDetail {
        kind: "addon_version_mismatch".to_string(),
        command: Some("doctor".to_string()),
        path: Some(addon_dir.to_string_lossy().into_owned()),
        message: Some(
            "installed godot_qa addon version does not match the godot-qa CLI version".to_string(),
        ),
        expected: Some(json!(cli_version)),
        actual: Some(json!(installed_version)),
        ..FailureDetail::default()
    }
}

pub(crate) fn binary_surface_mismatch_failure(report: &Value) -> FailureDetail {
    FailureDetail {
        kind: "binary_surface_mismatch".to_string(),
        command: Some("doctor".to_string()),
        message: Some(
            "multiple godot-qa binaries expose incompatible command surfaces".to_string(),
        ),
        expected: Some(json!({
            "source": report["expected_source"],
            "commands": report["expected_commands"],
        })),
        actual: Some(json!({
            "first_path_binary": report["first_path_binary"],
            "candidates": report["candidates"],
        })),
        details: Map::from_iter([(
            "remediation".to_string(),
            json!(
                "remove stale .cargo/bin, release archive, or repo-local godot-qa entries from PATH, or put the intended binary first"
            ),
        )]),
        ..FailureDetail::default()
    }
}

#[derive(Debug, Clone)]
pub(crate) struct PathCandidate {
    pub(crate) path_order: usize,
    pub(crate) raw_path: PathBuf,
    pub(crate) canonical_path: PathBuf,
}

pub(crate) fn godot_qa_path_candidates() -> Vec<PathCandidate> {
    let mut seen = BTreeSet::new();
    let mut candidates = Vec::new();
    let Some(path_var) = std::env::var_os("PATH") else {
        return candidates;
    };
    for candidate in std::env::split_paths(&path_var)
        .flat_map(|dir| godot_qa_process::executable_candidates(&dir, "godot-qa"))
    {
        if !is_executable_file(&candidate) {
            continue;
        }
        let canonical = candidate
            .canonicalize()
            .unwrap_or_else(|_| candidate.clone());
        let key = canonical.to_string_lossy().into_owned();
        if seen.insert(key) {
            candidates.push(PathCandidate {
                path_order: candidates.len(),
                raw_path: candidate,
                canonical_path: canonical,
            });
        }
    }
    candidates
}

pub(crate) fn binary_version_probe(binary: &Path) -> Value {
    let args = ["--version".to_string()];
    let output = godot_qa_process::run_safe_probe(binary, &args, Duration::from_secs(2));
    let stdout = output.stdout.trim().to_string();
    let stderr = output.stderr.trim().to_string();
    let error = if output.timed_out {
        Some(format!(
            "version command timed out after {:?}",
            output.duration
        ))
    } else if let Some(message) = output.error_message.clone() {
        Some(message)
    } else if output.status != Some(0) {
        Some(format!(
            "version command exited with status {:?}",
            output.status
        ))
    } else {
        None
    };
    json!({
        "command": output.command,
        "command_line": godot_qa_process::format_command(&output.command),
        "status": output.status,
        "stdout": stdout,
        "stderr": stderr,
        "version": stdout.lines().next().unwrap_or("").trim(),
        "build": null,
        "timed_out": output.timed_out,
        "error": error,
    })
}

pub(crate) fn binary_surface_remediation(
    path: &Path,
    first_on_path: bool,
    differs: bool,
) -> String {
    let text = path.to_string_lossy();
    let source = if text.contains(".cargo/bin") {
        "stale cargo install"
    } else if text.contains("target/debug") || text.contains("target/release") {
        "repo-local build"
    } else {
        "release archive or PATH entry"
    };
    if differs && first_on_path {
        format!(
            "{source} shadows the expected godot-qa surface; remove it or put the intended binary first on PATH"
        )
    } else if differs {
        format!("{source} has a stale godot-qa surface; remove or update that PATH entry")
    } else {
        format!("{source} matches the expected command surface")
    }
}

pub(crate) fn is_executable_file(path: &Path) -> bool {
    if !path.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        path.metadata()
            .map(|metadata| metadata.permissions().mode() & 0o111 != 0)
            .unwrap_or(false)
    }
    #[cfg(not(unix))]
    {
        true
    }
}

pub(crate) fn discover_binary_command_surface(binary: &Path) -> Result<BTreeSet<String>, Value> {
    let mut leaves = BTreeSet::new();
    let mut queue: Vec<Vec<String>> = vec![Vec::new()];
    let mut visited = BTreeSet::new();
    let known_leaves: BTreeSet<String> = public_command_surface()
        .into_iter()
        .map(str::to_string)
        .collect();
    while let Some(path) = queue.pop() {
        let key = path.join(" ");
        if !visited.insert(key.clone()) {
            continue;
        }
        let help = command_help(binary, &path)?;
        let subcommands = parse_help_subcommands(&help);
        if !path.is_empty() && known_leaves.contains(&key) {
            leaves.insert(key);
        }
        if subcommands.is_empty() {
            if !path.is_empty() {
                leaves.insert(path.join(" "));
            }
            continue;
        }
        for subcommand in subcommands {
            if subcommand == "help" {
                continue;
            }
            let mut child = path.clone();
            child.push(subcommand);
            queue.push(child);
        }
    }
    Ok(leaves)
}

pub(crate) fn command_help(binary: &Path, path: &[String]) -> Result<String, Value> {
    let mut args = path.to_vec();
    args.push("--help".to_string());
    let output = godot_qa_process::run_safe_probe(binary, &args, Duration::from_secs(2));
    let stdout = output.stdout.trim().to_string();
    let stderr = output.stderr.trim().to_string();
    if output.timed_out {
        return Err(json!({
            "probe": "help",
            "command": output.command,
            "command_line": godot_qa_process::format_command(&output.command),
            "status": output.status,
            "stdout": stdout,
            "stderr": stderr,
            "timed_out": true,
            "error_kind": output.error_kind,
            "message": format!("help command timed out after {:?}", output.duration),
        }));
    }
    if let Some(message) = output.error_message {
        return Err(json!({
            "probe": "help",
            "command": output.command,
            "command_line": godot_qa_process::format_command(&output.command),
            "status": output.status,
            "stdout": stdout,
            "stderr": stderr,
            "timed_out": false,
            "error_kind": output.error_kind,
            "message": message,
        }));
    }
    if output.status != Some(0) {
        return Err(json!({
            "probe": "help",
            "command": output.command,
            "command_line": godot_qa_process::format_command(&output.command),
            "status": output.status,
            "stdout": stdout,
            "stderr": stderr,
            "timed_out": false,
            "error_kind": null,
            "message": format!("help command exited with status {:?}", output.status),
        }));
    }
    Ok(output.stdout)
}

pub(crate) fn parse_help_subcommands(help: &str) -> Vec<String> {
    let mut subcommands = Vec::new();
    let mut in_commands = false;
    for line in help.lines() {
        let trimmed = line.trim();
        if trimmed == "Commands:" || trimmed == "SUBCOMMANDS:" {
            in_commands = true;
            continue;
        }
        if in_commands && trimmed.ends_with(':') && !trimmed.starts_with('-') {
            break;
        }
        if !in_commands || trimmed.is_empty() {
            continue;
        }
        if trimmed.starts_with('-') {
            continue;
        }
        let Some(name) = trimmed.split_whitespace().next() else {
            continue;
        };
        if name
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_'))
        {
            subcommands.push(name.to_string());
        }
    }
    subcommands
}

pub(crate) fn public_command_surface() -> Vec<&'static str> {
    vec![
        "doctor",
        "install-skills",
        "import",
        "check",
        "diagnostics editor",
        "profile editor export",
        "scenario list",
        "scenario bundle list",
        "scenario bundle run",
        "scenario lint",
        "scenario run",
        "scenario sweep",
        "scenario profile",
        "scenario init",
        "scenario record",
        "review list-templates",
        "review init",
        "live-gate list",
        "live-gate run",
        "qa-state validate",
        "artifact query",
        "artifact summarize",
        "artifact prune",
        "artifact begin-run",
        "artifact validate-result",
        "artifact export",
        "session start",
        "session status",
        "session list",
        "session run-script",
        "session wait",
        "session time-scale",
        "session reload",
        "session watch",
        "session stop",
        "session pause",
        "session resume",
        "session step",
        "session profile",
        "input tap",
        "input hold",
        "input release",
        "input click",
        "input mouse click",
        "input mouse wheel",
        "input touch down",
        "input touch move",
        "input touch up",
        "input touch tap",
        "input drag",
        "input key",
        "input type",
        "input joy button",
        "input joy motion",
        "snapshot",
        "tree",
        "ui",
        "focus",
        "focus graph",
        "node inspect",
        "physics inspect",
        "physics overlay",
        "physics collision-report",
        "surface audit",
        "trace record",
        "event trace",
        "event summarize",
        "telemetry record",
        "audio capture",
        "audio export",
        "audio render",
        "audio analyze",
        "errors",
        "layout audit",
        "layout audit-current",
        "assert",
        "screenshot capture",
        "screenshot assert-startup-presented",
        "screenshot compare",
        "screenshot sequence",
        "screenshot contact-sheet",
        "visual preflight",
        "visual audit",
        "visual isolate",
        "edit-live set",
        "edit-live call",
        "edit-live diff",
        "edit-live export-patch",
        "apply-patch",
        "mcp",
        "logs",
    ]
}

#[cfg(test)]
mod public_command_surface_docs_tests {
    use super::{parse_plugin_cfg_version, public_command_surface};
    use std::collections::BTreeSet;
    use std::fs;
    use std::path::{Path, PathBuf};

    fn repo_root() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .components()
            .collect()
    }

    /// Extracts the fenced ```text inventory block that immediately follows the
    /// "## Current Public Command Surface" heading in `docs/cli-spec.md`.
    fn documented_command_surface() -> BTreeSet<String> {
        let cli_spec = fs::read_to_string(repo_root().join("docs/cli-spec.md"))
            .expect("docs/cli-spec.md must be readable");
        let heading = "## Current Public Command Surface";
        let after_heading = cli_spec
            .split_once(heading)
            .unwrap_or_else(|| panic!("docs/cli-spec.md must contain the `{heading}` heading"))
            .1;
        let fence_marker = "```text";
        let fence_start = after_heading.find(fence_marker).unwrap_or_else(|| {
            panic!("`{heading}` section must open a ```text fenced inventory block")
        });
        let after_fence_open = &after_heading[fence_start + fence_marker.len()..];
        let fence_end = after_fence_open.find("```").unwrap_or_else(|| {
            panic!("`{heading}` section's fenced inventory block must be closed")
        });
        after_fence_open[..fence_end]
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty())
            .map(str::to_string)
            .collect()
    }

    #[test]
    fn parse_plugin_cfg_version_reads_value_and_falls_back() {
        assert_eq!(
            parse_plugin_cfg_version("[plugin]\nname=\"Godot QA\"\nversion=\"0.1.0\"\n"),
            "0.1.0"
        );
        assert_eq!(
            parse_plugin_cfg_version("[plugin]\nversion = 2.3.4\n"),
            "2.3.4"
        );
        // A plugin.cfg that carries no parsable version drifts to a clear, comparable "0.0.0".
        assert_eq!(
            parse_plugin_cfg_version("[plugin]\nname=\"Godot QA\"\n"),
            "0.0.0"
        );
        // "versioning" must not be mistaken for the version key.
        assert_eq!(
            parse_plugin_cfg_version("[plugin]\nversioning=\"9.9.9\"\n"),
            "0.0.0"
        );
    }

    #[test]
    fn addon_plugin_cfg_version_matches_cli_version() {
        let plugin_cfg = fs::read_to_string(repo_root().join("addons/godot_qa/plugin.cfg"))
            .expect("addons/godot_qa/plugin.cfg must be readable");
        let addon_version = parse_plugin_cfg_version(&plugin_cfg);
        assert_eq!(
            addon_version,
            env!("CARGO_PKG_VERSION"),
            "addons/godot_qa/plugin.cfg version must stay in sync with the workspace \
             CARGO_PKG_VERSION. Bump [workspace.package] version and \
             addons/godot_qa/plugin.cfg version together."
        );
    }

    #[test]
    fn cli_spec_inventory_matches_public_command_surface() {
        let documented = documented_command_surface();
        let implemented: BTreeSet<String> = public_command_surface()
            .into_iter()
            .map(str::to_string)
            .collect();

        let missing_from_docs: Vec<&String> = implemented.difference(&documented).collect();
        let stale_in_docs: Vec<&String> = documented.difference(&implemented).collect();

        assert!(
            missing_from_docs.is_empty() && stale_in_docs.is_empty(),
            "docs/cli-spec.md's \"Current Public Command Surface\" inventory has drifted from \
             doctor_command::public_command_surface().\n\
             Shipped in public_command_surface() but missing from docs/cli-spec.md: {missing_from_docs:?}\n\
             Listed in docs/cli-spec.md but not returned by public_command_surface(): {stale_in_docs:?}\n\
             Update docs/cli-spec.md's inventory (or public_command_surface()) so both stay exhaustive and in sync."
        );
    }
}
