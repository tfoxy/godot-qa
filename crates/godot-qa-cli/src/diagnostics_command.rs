use crate::args::*;
use crate::*;
use godot_qa_core::config::{ProjectConfig, load_project_config};
use godot_qa_core::results::{CommandResult, FailureDetail};
use serde_json::{Map, Value, json};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

pub(crate) fn editor_diagnostics_result(cli: &Cli, args: &EditorDiagnosticsArgs) -> CommandResult {
    let resolved_binary = godot_qa_process::resolve_godot_binary(cli.godot_bin.as_deref());
    let config = load_project_config(
        cli.project.as_deref().map(Path::new),
        cli.godot_bin.as_deref(),
        cli.artifact_dir.as_deref(),
        cli.verbose,
        resolved_binary,
    );
    let command = "diagnostics editor";
    let mut failures = Vec::new();
    if let Some(failure) = config.project_config_failure_for_command(command) {
        failures.push(failure);
    }
    if config.godot_binary.is_none() {
        failures.push(missing_godot_failure(command));
    }
    if !config.project_root.exists() {
        failures.push(missing_project_root_failure(command, &config.project_root));
    }
    if config.project_root.exists()
        && let Some(failure) = ensure_writable_artifact_dir(&config.artifacts_dir, command)
    {
        failures.push(failure);
    }
    if !failures.is_empty() {
        return CommandResult {
            ok: false,
            command: command.to_string(),
            message: "editor diagnostics failed".to_string(),
            data: Map::from_iter([("project_root".to_string(), json!(config.project_root))]),
            failures,
            exit_code: 1,
        };
    }

    let _project_lease = match godot_qa_process::project_access::LaunchLease::acquire(
        &config.project_root,
        godot_qa_process::project_access::LaunchClass::ImportOrEditor,
    ) {
        Ok(lease) => lease,
        Err(failure) => {
            return CommandResult {
                ok: false,
                command: command.into(),
                message: "project is reserved by an asset trial".into(),
                data: Map::new(),
                failures: vec![failure],
                exit_code: 1,
            };
        }
    };
    let artifacts = prepare_editor_diagnostics_artifacts(&config, command);
    let project_godot_snapshot = read_project_godot_snapshot(&config.project_root);
    let started = std::time::Instant::now();
    let completed = godot_qa_process::launch_godot_import_with_args(
        config.godot_binary.as_deref().unwrap_or_default(),
        &config.godot_launch_prefix,
        &config.godot_args,
        &config.project_root,
        &artifacts.logs_path,
    );
    let restore_failure =
        restore_project_godot_snapshot(command, &config.project_root, project_godot_snapshot);

    let bridge_artifact = read_editor_bridge_artifact(&artifacts.editor_bridge_path);
    let mut notes = Vec::new();
    if let Some(note) = bridge_artifact.note.clone() {
        notes.push(note);
    }
    let mut capture_methods = Vec::new();
    let mut seen = BTreeSet::new();
    let mut diagnostics = Vec::new();
    if bridge_artifact.available {
        if let Some(capture_method) = bridge_artifact.capture_method.clone() {
            capture_methods.push(capture_method);
        }
        push_unique_diagnostics(
            &mut diagnostics,
            &mut seen,
            bridge_artifact.diagnostics.clone(),
        );
    }
    capture_methods.push("godot --headless --import --log-file".to_string());
    push_unique_diagnostics(
        &mut diagnostics,
        &mut seen,
        collect_editor_diagnostics(&[
            DiagnosticLogSource {
                name: "logs",
                path: artifacts.logs_path.clone(),
            },
            DiagnosticLogSource {
                name: "raw_godot_log",
                path: artifacts.raw_godot_log_path.clone(),
            },
        ]),
    );
    diagnostics.sort_by(|left, right| {
        left.script
            .cmp(&right.script)
            .then(left.line.cmp(&right.line))
            .then(left.severity.cmp(&right.severity))
            .then(left.code.cmp(&right.code))
            .then(left.raw.cmp(&right.raw))
    });
    let warning_count = diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.severity == "warning")
        .count();
    let error_count = diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.severity == "error")
        .count();

    let mut result_failures = Vec::new();
    match completed {
        Ok(run) if run.returncode == 0 => {}
        Ok(run) => result_failures.push(FailureDetail {
            kind: "godot_editor_diagnostics_failed".to_string(),
            command: Some(godot_qa_process::format_command(&run.command)),
            path: Some(artifacts.logs_path.to_string_lossy().into_owned()),
            expected: Some(json!("Godot editor diagnostics import exits with status 0")),
            actual: Some(json!(run.returncode)),
            artifacts: value_as_object_map(artifacts.to_json()),
            ..FailureDetail::default()
        }),
        Err(failure) => result_failures.push(failure),
    }
    if let Some(failure) = restore_failure {
        result_failures.push(failure);
    }
    if error_count > 0 && result_failures.is_empty() {
        result_failures.push(FailureDetail {
            kind: "editor_diagnostics_errors_present".to_string(),
            command: Some(command.to_string()),
            path: Some(artifacts.logs_path.to_string_lossy().into_owned()),
            message: Some("Godot editor diagnostics reported errors".to_string()),
            expected: Some(json!("no editor diagnostics with severity=error")),
            actual: Some(json!(error_count)),
            artifacts: value_as_object_map(artifacts.to_json()),
            ..FailureDetail::default()
        });
    }
    if args.strict_warnings && warning_count > 0 {
        result_failures.push(FailureDetail {
            kind: "editor_diagnostics_warnings_present".to_string(),
            command: Some(command.to_string()),
            path: Some(artifacts.logs_path.to_string_lossy().into_owned()),
            message: Some("Godot editor diagnostics reported warnings in strict mode".to_string()),
            expected: Some(json!("no editor diagnostics with severity=warning")),
            actual: Some(json!(warning_count)),
            artifacts: value_as_object_map(artifacts.to_json()),
            ..FailureDetail::default()
        });
    }

    let diagnostics_json: Vec<Value> = diagnostics.iter().map(EditorDiagnostic::to_json).collect();
    let mut data = Map::from_iter([
        ("project_root".to_string(), json!(config.project_root)),
        ("artifacts".to_string(), artifacts.to_json()),
        (
            "duration_ms".to_string(),
            json!(started.elapsed().as_millis() as u64),
        ),
        (
            "diagnostic_count".to_string(),
            json!(diagnostics_json.len()),
        ),
        ("warning_count".to_string(), json!(warning_count)),
        ("error_count".to_string(), json!(error_count)),
        ("diagnostics".to_string(), json!(diagnostics_json)),
        (
            "capture_method".to_string(),
            json!(capture_methods.join(" + ")),
        ),
        ("capture_methods".to_string(), json!(capture_methods)),
        ("editor_bridge".to_string(), bridge_artifact.to_json()),
        ("notes".to_string(), json!(notes)),
    ]);
    data.insert(
        "note".to_string(),
        json!("captures the Godot QA editor bridge artifact when present, then merges diagnostics Godot emits through process output or log files"),
    );
    let exit_code = if result_failures.is_empty() { 0 } else { 1 };
    let result = CommandResult {
        ok: result_failures.is_empty(),
        command: command.to_string(),
        message: if result_failures.is_empty() {
            "editor diagnostics completed".to_string()
        } else {
            "editor diagnostics failed".to_string()
        },
        data,
        failures: result_failures,
        exit_code,
    };
    if let Err(error) = write_result_artifact(&artifacts.result_path, &result) {
        eprintln!(
            "godot-qa: warning: failed to write editor diagnostics result {}: {error}",
            artifacts.result_path.display()
        );
    }
    result
}

fn read_project_godot_snapshot(project_root: &Path) -> Option<String> {
    std::fs::read_to_string(project_root.join("project.godot")).ok()
}

fn restore_project_godot_snapshot(
    command: &str,
    project_root: &Path,
    snapshot: Option<String>,
) -> Option<FailureDetail> {
    let snapshot = snapshot?;
    let project_path = project_root.join("project.godot");
    let current = std::fs::read_to_string(&project_path).ok();
    if current.as_deref() == Some(snapshot.as_str()) {
        return None;
    }
    std::fs::write(&project_path, snapshot)
        .err()
        .map(|error| FailureDetail {
            kind: "project_godot_restore_failed".to_string(),
            command: Some(command.to_string()),
            path: Some(project_path.to_string_lossy().into_owned()),
            message: Some(error.to_string()),
            expected: Some(json!(
                "project.godot restored after editor diagnostics import"
            )),
            actual: Some(json!("OSError")),
            ..FailureDetail::default()
        })
}

#[derive(Debug)]
pub(crate) struct EditorDiagnosticsArtifacts {
    root_dir: PathBuf,
    meta_path: PathBuf,
    result_path: PathBuf,
    logs_path: PathBuf,
    raw_godot_log_path: PathBuf,
    editor_bridge_path: PathBuf,
}

impl EditorDiagnosticsArtifacts {
    fn to_json(&self) -> Value {
        json!({
            "run_root": self.root_dir,
            "root": self.root_dir,
            "meta": self.meta_path,
            "result": self.result_path,
            "logs": self.logs_path,
            "raw_godot_log": self.raw_godot_log_path,
            "editor_bridge": self.editor_bridge_path,
        })
    }
}

pub(crate) fn prepare_editor_diagnostics_artifacts(
    config: &ProjectConfig,
    command: &str,
) -> EditorDiagnosticsArtifacts {
    let root_dir = config
        .artifacts_dir
        .join("diagnostics")
        .join(unique_run_id());
    let _ = std::fs::create_dir_all(&root_dir);
    let artifacts = EditorDiagnosticsArtifacts {
        root_dir: root_dir.clone(),
        meta_path: root_dir.join("meta.json"),
        result_path: root_dir.join("result.json"),
        logs_path: root_dir.join("logs.txt"),
        raw_godot_log_path: root_dir.join("godot.log"),
        editor_bridge_path: config
            .artifacts_dir
            .join("editor-diagnostics")
            .join("latest.json"),
    };
    if let Err(error) = std::fs::write(
        &artifacts.meta_path,
        serde_json::to_string_pretty(&json!({
            "tool": "godot-qa",
            "project_root": config.project_root,
            "command": command,
            "capture_method": "godot --headless --import --log-file",
        }))
        .unwrap_or_else(|_| "{}".to_string()),
    ) {
        eprintln!(
            "godot-qa: warning: failed to write editor diagnostics meta {}: {error}",
            artifacts.meta_path.display()
        );
    }
    artifacts
}

fn unique_run_id() -> String {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis().to_string())
        .unwrap_or_else(|_| "0".to_string())
}

#[derive(Debug, Clone)]
struct DiagnosticLogSource {
    name: &'static str,
    path: PathBuf,
}

#[derive(Debug, Clone)]
struct EditorBridgeArtifact {
    path: PathBuf,
    available: bool,
    capture_method: Option<String>,
    diagnostic_count: Option<u64>,
    warning_count: Option<u64>,
    error_count: Option<u64>,
    run_id: Option<String>,
    captured_at: Option<String>,
    diagnostics: Vec<EditorDiagnostic>,
    note: Option<String>,
}

impl EditorBridgeArtifact {
    fn to_json(&self) -> Value {
        json!({
            "path": self.path,
            "available": self.available,
            "capture_method": self.capture_method,
            "run_id": self.run_id,
            "captured_at": self.captured_at,
            "diagnostic_count": self.diagnostic_count,
            "warning_count": self.warning_count,
            "error_count": self.error_count,
            "note": self.note,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct EditorDiagnostic {
    pub(crate) severity: String,
    pub(crate) code: Option<String>,
    pub(crate) message: String,
    pub(crate) script: Option<String>,
    pub(crate) line: Option<u64>,
    pub(crate) column: Option<u64>,
    pub(crate) source: String,
    pub(crate) raw: String,
}

impl EditorDiagnostic {
    fn to_json(&self) -> Value {
        json!({
            "severity": self.severity,
            "code": self.code,
            "message": self.message,
            "script": self.script,
            "line": self.line,
            "column": self.column,
            "source": self.source,
            "raw": self.raw,
        })
    }

    fn dedupe_key(&self) -> String {
        format!(
            "{}\u{1f}{}\u{1f}{}\u{1f}{}\u{1f}{:?}\u{1f}{:?}",
            self.severity,
            self.code.as_deref().unwrap_or_default(),
            self.message,
            self.script.as_deref().unwrap_or_default(),
            self.line,
            self.column,
        )
    }
}

fn push_unique_diagnostics(
    diagnostics: &mut Vec<EditorDiagnostic>,
    seen: &mut BTreeSet<String>,
    incoming: Vec<EditorDiagnostic>,
) {
    for diagnostic in incoming {
        if seen.insert(diagnostic.dedupe_key()) {
            diagnostics.push(diagnostic);
        }
    }
}

fn read_editor_bridge_artifact(path: &Path) -> EditorBridgeArtifact {
    let mut artifact = EditorBridgeArtifact {
        path: path.to_path_buf(),
        available: path.exists(),
        capture_method: None,
        diagnostic_count: None,
        warning_count: None,
        error_count: None,
        run_id: None,
        captured_at: None,
        diagnostics: Vec::new(),
        note: None,
    };
    if !artifact.available {
        artifact.note = Some(format!(
            "No editor-run diagnostics bridge artifact found at {}; enable the Godot QA plugin, run the project from the Godot Editor, then rerun this command",
            path.to_string_lossy()
        ));
        return artifact;
    }
    let raw = match std::fs::read_to_string(path) {
        Ok(raw) => raw,
        Err(error) => {
            artifact.note = Some(format!(
                "Editor diagnostics bridge artifact could not be read: {error}"
            ));
            return artifact;
        }
    };
    let value: Value = match serde_json::from_str(&raw) {
        Ok(value) => value,
        Err(error) => {
            artifact.note = Some(format!(
                "Editor diagnostics bridge artifact is not valid JSON: {error}"
            ));
            return artifact;
        }
    };
    let Some(object) = value.as_object() else {
        artifact.note =
            Some("Editor diagnostics bridge artifact root must be a JSON object".to_string());
        return artifact;
    };
    artifact.capture_method = object
        .get("capture_method")
        .and_then(Value::as_str)
        .map(str::to_string)
        .or_else(|| Some("editor_plugin_debugger_errors_panel".to_string()));
    artifact.diagnostic_count = object.get("diagnostic_count").and_then(Value::as_u64);
    artifact.warning_count = object.get("warning_count").and_then(Value::as_u64);
    artifact.error_count = object.get("error_count").and_then(Value::as_u64);
    artifact.run_id = object
        .get("run_id")
        .and_then(Value::as_str)
        .map(str::to_string);
    artifact.captured_at = object
        .get("captured_at")
        .and_then(Value::as_str)
        .map(str::to_string);
    let Some(entries) = object.get("diagnostics").and_then(Value::as_array) else {
        artifact.note = Some(
            "Editor diagnostics bridge artifact does not contain a diagnostics array".to_string(),
        );
        return artifact;
    };
    artifact.diagnostics = entries
        .iter()
        .filter_map(|entry| editor_diagnostic_from_json(entry, "editor_bridge"))
        .collect();
    artifact
}

fn editor_diagnostic_from_json(value: &Value, default_source: &str) -> Option<EditorDiagnostic> {
    let object = value.as_object()?;
    let severity = object
        .get("severity")
        .and_then(Value::as_str)
        .filter(|value| *value == "warning" || *value == "error")
        .unwrap_or("warning")
        .to_string();
    let message = object
        .get("message")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    if message.is_empty() && object.get("code").and_then(Value::as_str).is_none() {
        return None;
    }
    Some(EditorDiagnostic {
        severity,
        code: optional_string(object.get("code")),
        message,
        script: optional_string(object.get("script")),
        line: object.get("line").and_then(Value::as_u64),
        column: object.get("column").and_then(Value::as_u64),
        source: object
            .get("source")
            .and_then(Value::as_str)
            .unwrap_or(default_source)
            .to_string(),
        raw: object
            .get("raw")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
    })
}

fn optional_string(value: Option<&Value>) -> Option<String> {
    value
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn collect_editor_diagnostics(sources: &[DiagnosticLogSource]) -> Vec<EditorDiagnostic> {
    let mut seen = BTreeSet::new();
    let mut diagnostics = Vec::new();
    for source in sources {
        let Ok(text) = std::fs::read_to_string(&source.path) else {
            continue;
        };
        for diagnostic in parse_editor_diagnostics(&text, source.name) {
            if seen.insert(diagnostic.dedupe_key()) {
                diagnostics.push(diagnostic);
            }
        }
    }
    diagnostics
}

pub(crate) fn parse_editor_diagnostics(text: &str, source: &str) -> Vec<EditorDiagnostic> {
    let lines: Vec<&str> = text.lines().collect();
    let mut diagnostics = Vec::new();
    let mut consumed = BTreeSet::new();
    for (index, raw_line) in lines.iter().enumerate() {
        let line = strip_ansi(raw_line).trim().to_string();
        if line.is_empty() {
            continue;
        }
        if line.starts_with("<GDScript Error>") {
            let code = value_after_prefix(&line, "<GDScript Error>").map(str::to_string);
            let message = previous_gdscript_reload_message(&lines, index)
                .or_else(|| next_message_line(&lines, index))
                .unwrap_or_else(|| line.clone());
            let (script, line_no, column) = adjacent_source_location(&lines, index);
            diagnostics.push(EditorDiagnostic {
                severity: severity_for_gdscript_code(code.as_deref(), &message).to_string(),
                code,
                message,
                script,
                line: line_no,
                column,
                source: source.to_string(),
                raw: nearby_raw(&lines, index),
            });
            consumed.insert(index);
            continue;
        }
        if line.starts_with("SCRIPT ERROR:")
            || line.starts_with("ERROR:")
            || line.starts_with("SCRIPT WARNING:")
            || line.starts_with("WARNING:")
        {
            let severity = if line.contains("WARNING:") {
                "warning"
            } else {
                "error"
            };
            let code = code_for_log_line(&line);
            let message = message_for_log_line(&line);
            let (script, line_no, column) = adjacent_source_location(&lines, index);
            diagnostics.push(EditorDiagnostic {
                severity: severity.to_string(),
                code,
                message,
                script,
                line: line_no,
                column,
                source: source.to_string(),
                raw: nearby_raw(&lines, index),
            });
            consumed.insert(index);
            continue;
        }
        if !consumed.contains(&index)
            && line.contains("GDScript::reload:")
            && looks_like_gdscript_warning(&line)
            && !next_has_gdscript_error(&lines, index)
        {
            let message = message_after_gdscript_reload(&line).unwrap_or(line.clone());
            let code = code_for_warning_message(&message);
            let (script, line_no, column) = adjacent_source_location(&lines, index);
            diagnostics.push(EditorDiagnostic {
                severity: "warning".to_string(),
                code,
                message,
                script,
                line: line_no,
                column,
                source: source.to_string(),
                raw: nearby_raw(&lines, index),
            });
        }
    }
    diagnostics
}

fn previous_gdscript_reload_message(lines: &[&str], index: usize) -> Option<String> {
    let start = index.saturating_sub(3);
    for raw in lines[start..index].iter().rev() {
        let line = strip_ansi(raw).trim().to_string();
        if let Some(message) = message_after_gdscript_reload(&line) {
            return Some(message);
        }
    }
    None
}

fn next_message_line(lines: &[&str], index: usize) -> Option<String> {
    for raw in lines.iter().skip(index + 1).take(3) {
        let line = strip_ansi(raw).trim().to_string();
        if line.starts_with('<') || line.starts_with("at:") {
            continue;
        }
        if !line.is_empty() {
            return Some(line);
        }
    }
    None
}

fn next_has_gdscript_error(lines: &[&str], index: usize) -> bool {
    lines
        .iter()
        .skip(index + 1)
        .take(3)
        .map(|line| strip_ansi(line).trim().to_string())
        .any(|line| line.starts_with("<GDScript Error>"))
}

fn adjacent_source_location(
    lines: &[&str],
    index: usize,
) -> (Option<String>, Option<u64>, Option<u64>) {
    let start = index.saturating_sub(3);
    let end = (index + 4).min(lines.len());
    for raw in &lines[start..end] {
        let line = strip_ansi(raw).trim().to_string();
        if line.starts_with("<GDScript Source>") {
            if let Some(value) = value_after_prefix(&line, "<GDScript Source>") {
                return parse_source_location(value.split('@').next().unwrap_or(value).trim());
            }
        }
        if let Some(location) = line.strip_prefix("at:").and_then(location_in_parentheses) {
            return parse_source_location(location);
        }
    }
    (None, None, None)
}

fn parse_source_location(raw: &str) -> (Option<String>, Option<u64>, Option<u64>) {
    let trimmed = raw.trim().trim_end_matches(')');
    let Some(last_colon) = trimmed.rfind(':') else {
        return (Some(trimmed.to_string()), None, None);
    };
    let head = &trimmed[..last_colon];
    let tail = &trimmed[last_colon + 1..];
    let Some(last_number) = tail.parse::<u64>().ok() else {
        return (Some(trimmed.to_string()), None, None);
    };
    if let Some(previous_colon) = head.rfind(':') {
        let possible_line = &head[previous_colon + 1..];
        if let Ok(line) = possible_line.parse::<u64>() {
            return (
                Some(head[..previous_colon].to_string()),
                Some(line),
                Some(last_number),
            );
        }
    }
    (Some(head.to_string()), Some(last_number), None)
}

fn value_after_prefix<'a>(line: &'a str, prefix: &str) -> Option<&'a str> {
    line.strip_prefix(prefix)
        .map(str::trim)
        .filter(|value| !value.is_empty())
}

fn message_after_gdscript_reload(line: &str) -> Option<String> {
    line.split_once("GDScript::reload:")
        .map(|(_, message)| message.trim().to_string())
        .filter(|message| !message.is_empty())
}

fn location_in_parentheses(line: &str) -> Option<&str> {
    let start = line.rfind('(')?;
    let end = line[start + 1..]
        .find(')')
        .map(|offset| start + 1 + offset)
        .unwrap_or(line.len());
    Some(&line[start + 1..end])
}

fn severity_for_gdscript_code(code: Option<&str>, message: &str) -> &'static str {
    if code
        .map(|code| {
            matches!(
                code,
                "SHADOWED_VARIABLE"
                    | "SHADOWED_GLOBAL_IDENTIFIER"
                    | "UNUSED_PARAMETER"
                    | "INTEGER_DIVISION"
                    | "INT_AS_ENUM_WITHOUT_CAST"
                    | "NARROWING_CONVERSION"
            )
        })
        .unwrap_or(false)
        || looks_like_gdscript_warning(message)
    {
        "warning"
    } else {
        "error"
    }
}

fn looks_like_gdscript_warning(line: &str) -> bool {
    let lower = line.to_ascii_lowercase();
    lower.contains("shadowing")
        || lower.contains("same name as a built-in function")
        || lower.contains("same name as a global class")
        || lower.contains("is never used")
        || lower.contains("integer division")
        || lower.contains("integer used when an enum value is expected")
}

fn code_for_warning_message(message: &str) -> Option<String> {
    let lower = message.to_ascii_lowercase();
    let code = if lower.contains("integer division") {
        "INTEGER_DIVISION"
    } else if lower.contains("integer used when an enum value is expected") {
        "INT_AS_ENUM_WITHOUT_CAST"
    } else if lower.contains("same name as a built-in function")
        || lower.contains("same name as a global class")
    {
        "SHADOWED_GLOBAL_IDENTIFIER"
    } else if lower.contains("is never used") {
        "UNUSED_PARAMETER"
    } else if lower.contains("shadowing") {
        "SHADOWED_VARIABLE"
    } else {
        return None;
    };
    Some(code.to_string())
}

fn code_for_log_line(line: &str) -> Option<String> {
    let lower = line.to_ascii_lowercase();
    if lower.contains("failed to load script") {
        Some("SCRIPT_LOAD_FAILED".to_string())
    } else if lower.contains("parse error") {
        Some("PARSE_ERROR".to_string())
    } else if line.starts_with("SCRIPT ERROR:") {
        Some("SCRIPT_ERROR".to_string())
    } else if line.starts_with("SCRIPT WARNING:") {
        Some("SCRIPT_WARNING".to_string())
    } else {
        None
    }
}

fn message_for_log_line(line: &str) -> String {
    line.split_once(':')
        .map(|(_, message)| message.trim().to_string())
        .filter(|message| !message.is_empty())
        .unwrap_or_else(|| line.to_string())
}

fn nearby_raw(lines: &[&str], index: usize) -> String {
    let start = index.saturating_sub(1);
    let end = (index + 2).min(lines.len());
    lines[start..end]
        .iter()
        .map(|line| strip_ansi(line).trim().to_string())
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

fn strip_ansi(value: &str) -> String {
    let mut result = String::with_capacity(value.len());
    let mut chars = value.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '\u{1b}' && chars.peek() == Some(&'[') {
            let _ = chars.next();
            for code_ch in chars.by_ref() {
                if code_ch.is_ascii_alphabetic() {
                    break;
                }
            }
            continue;
        }
        result.push(ch);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_gdscript_editor_warning_triads() {
        let text = r#"
GDScript::reload: The local function parameter "device_index" is shadowing an already-declared variable at line 448 in the current class.
<GDScript Error> SHADOWED_VARIABLE
<GDScript Source> match_config.gd:464 @ GDScript::reload()
GDScript::reload: The parameter "character" is never used in the function "_s6_1_bot_intent_qa_state()". If this is intended, prefix it with an underscore: "_character".
<GDScript Error> UNUSED_PARAMETER
<GDScript Source> bot_navigation.gd:1625 @ GDScript::reload()
"#;
        let diagnostics = parse_editor_diagnostics(text, "logs");

        assert_eq!(diagnostics.len(), 2);
        assert_eq!(diagnostics[0].severity, "warning");
        assert_eq!(diagnostics[0].code.as_deref(), Some("SHADOWED_VARIABLE"));
        assert_eq!(diagnostics[0].script.as_deref(), Some("match_config.gd"));
        assert_eq!(diagnostics[0].line, Some(464));
        assert_eq!(diagnostics[1].code.as_deref(), Some("UNUSED_PARAMETER"));
    }

    #[test]
    fn parses_standalone_warning_messages() {
        let text = r#"
GDScript::reload: Integer division. Decimal part will be discarded.
GDScript::reload: Integer used when an enum value is expected. If this is intended, cast the integer to the enum type using the "as" keyword.
"#;
        let diagnostics = parse_editor_diagnostics(text, "logs");

        assert_eq!(diagnostics.len(), 2);
        assert_eq!(diagnostics[0].code.as_deref(), Some("INTEGER_DIVISION"));
        assert_eq!(
            diagnostics[1].code.as_deref(),
            Some("INT_AS_ENUM_WITHOUT_CAST")
        );
    }

    #[test]
    fn parses_script_errors_with_at_locations() {
        let text = r#"
SCRIPT ERROR: Parse Error: Expected statement, found "Indent" instead.
          at: GDScript::reload (res://src/river_generation.gd:417)
ERROR: Failed to load script "res://src/main.gd" with error "Parse error".
"#;
        let diagnostics = parse_editor_diagnostics(text, "logs");

        assert_eq!(diagnostics.len(), 2);
        assert_eq!(diagnostics[0].severity, "error");
        assert_eq!(diagnostics[0].code.as_deref(), Some("PARSE_ERROR"));
        assert_eq!(
            diagnostics[0].script.as_deref(),
            Some("res://src/river_generation.gd")
        );
        assert_eq!(diagnostics[0].line, Some(417));
        assert_eq!(diagnostics[1].code.as_deref(), Some("SCRIPT_LOAD_FAILED"));
    }
}
