use crate::args::*;
use crate::*;
use godot_qa_core::config::{ProjectConfig, load_project_config};
use godot_qa_core::results::{CommandResult, FailureDetail};
use serde_json::{Map, Value, json};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

const COMMAND: &str = "profile editor export";
const SOURCE: &str = "editor_script_functions";
const CAPTURE_METHOD: &str = "editor_debugger_raw_servers_profile";
const MAX_COMMAND_FUNCTIONS: usize = 200;
const MAX_RETAINED_FRAMES: usize = 3000;

pub(crate) fn editor_profile_export_result(
    cli: &Cli,
    _args: &EditorProfileExportArgs,
) -> CommandResult {
    let resolved_binary = godot_qa_process::resolve_godot_binary(cli.godot_bin.as_deref());
    let config = load_project_config(
        cli.project.as_deref().map(Path::new),
        cli.godot_bin.as_deref(),
        cli.artifact_dir.as_deref(),
        cli.verbose,
        resolved_binary,
    );
    let mut failures = Vec::new();
    if let Some(failure) = config.project_config_failure_for_command(COMMAND) {
        failures.push(failure);
    }
    if !config.project_root.exists() {
        failures.push(missing_project_root_failure(COMMAND, &config.project_root));
    }
    if config.project_root.exists()
        && let Some(failure) = ensure_writable_artifact_dir(&config.artifacts_dir, COMMAND)
    {
        failures.push(failure);
    }
    if !failures.is_empty() {
        return CommandResult {
            ok: false,
            command: COMMAND.to_string(),
            message: "editor profile export failed".to_string(),
            data: Map::from_iter([("project_root".to_string(), json!(config.project_root))]),
            failures,
            exit_code: 1,
        };
    }

    let artifacts = prepare_editor_profile_artifacts(&config);
    let source_path = artifacts.editor_latest_path.clone();
    let raw = match std::fs::read_to_string(&source_path) {
        Ok(raw) => raw,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            let failure = editor_profile_missing_failure(&source_path, &artifacts);
            let result = failed_result(
                "editor profile export failed",
                failure,
                &config,
                &artifacts,
                None,
            );
            if let Err(error) = write_result_artifact(&artifacts.result_path, &result) {
                eprintln!(
                    "godot-qa: warning: failed to write editor profile result {}: {error}",
                    artifacts.result_path.display()
                );
            }
            return result;
        }
        Err(error) => {
            let failure = editor_profile_invalid_failure(
                &source_path,
                "readable JSON editor profile artifact",
                error.to_string(),
            );
            let result = failed_result(
                "editor profile export failed",
                failure,
                &config,
                &artifacts,
                None,
            );
            if let Err(error) = write_result_artifact(&artifacts.result_path, &result) {
                eprintln!(
                    "godot-qa: warning: failed to write editor profile result {}: {error}",
                    artifacts.result_path.display()
                );
            }
            return result;
        }
    };
    let parsed: Value = match serde_json::from_str(&raw) {
        Ok(value) => value,
        Err(error) => {
            let failure = editor_profile_invalid_failure(
                &source_path,
                "valid JSON object",
                format!("JSON parse error: {error}"),
            );
            let result = failed_result(
                "editor profile export failed",
                failure,
                &config,
                &artifacts,
                None,
            );
            if let Err(error) = write_result_artifact(&artifacts.result_path, &result) {
                eprintln!(
                    "godot-qa: warning: failed to write editor profile result {}: {error}",
                    artifacts.result_path.display()
                );
            }
            return result;
        }
    };
    let normalized = match normalize_editor_profile_artifact(&parsed, &source_path) {
        Ok(normalized) => normalized,
        Err(failure) => {
            let result = failed_result(
                "editor profile export failed",
                failure,
                &config,
                &artifacts,
                Some(parsed),
            );
            if let Err(error) = write_result_artifact(&artifacts.result_path, &result) {
                eprintln!(
                    "godot-qa: warning: failed to write editor profile result {}: {error}",
                    artifacts.result_path.display()
                );
            }
            return result;
        }
    };

    if let Err(error) = std::fs::write(
        &artifacts.profile_path,
        serde_json::to_string_pretty(&normalized).unwrap_or_else(|_| "{}".to_string()),
    ) {
        eprintln!(
            "godot-qa: warning: failed to write editor profile artifact {}: {error}",
            artifacts.profile_path.display()
        );
    }
    let script_functions = normalized
        .get("script_functions")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let warnings = normalized
        .get("warnings")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let frame_count = normalized
        .get("frame_count")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let summary_functions: Vec<Value> = script_functions
        .iter()
        .take(MAX_COMMAND_FUNCTIONS)
        .cloned()
        .collect();
    let data = Map::from_iter([
        ("project_root".to_string(), json!(config.project_root)),
        ("source".to_string(), json!(SOURCE)),
        ("capture_method".to_string(), json!(CAPTURE_METHOD)),
        ("experimental".to_string(), json!(true)),
        ("frame_count".to_string(), json!(frame_count)),
        (
            "script_function_count".to_string(),
            json!(script_functions.len()),
        ),
        ("warning_count".to_string(), json!(warnings.len())),
        ("script_functions".to_string(), json!(summary_functions)),
        ("warnings".to_string(), json!(warnings)),
        ("artifacts".to_string(), artifacts.to_json()),
        ("profile".to_string(), normalized.clone()),
        (
            "note".to_string(),
            json!(
                "reads the latest Godot QA editor-plugin profiler artifact; it does not launch or drive the editor"
            ),
        ),
    ]);
    let result = CommandResult {
        ok: true,
        command: COMMAND.to_string(),
        message: "editor profile exported".to_string(),
        data,
        failures: Vec::new(),
        exit_code: 0,
    };
    if let Err(error) = write_result_artifact(&artifacts.result_path, &result) {
        eprintln!(
            "godot-qa: warning: failed to write editor profile result {}: {error}",
            artifacts.result_path.display()
        );
    }
    result
}

#[derive(Debug)]
pub(crate) struct EditorProfileArtifacts {
    root_dir: PathBuf,
    meta_path: PathBuf,
    result_path: PathBuf,
    profile_path: PathBuf,
    editor_latest_path: PathBuf,
}

impl EditorProfileArtifacts {
    fn to_json(&self) -> Value {
        json!({
            "run_root": self.root_dir,
            "root": self.root_dir,
            "meta": self.meta_path,
            "result": self.result_path,
            "profile": self.profile_path,
            "editor_latest": self.editor_latest_path,
            "source": self.editor_latest_path,
        })
    }
}

pub(crate) fn prepare_editor_profile_artifacts(config: &ProjectConfig) -> EditorProfileArtifacts {
    let root_dir = config
        .artifacts_dir
        .join("profile-editor")
        .join(unique_run_id());
    let _ = std::fs::create_dir_all(&root_dir);
    let artifacts = EditorProfileArtifacts {
        root_dir: root_dir.clone(),
        meta_path: root_dir.join("meta.json"),
        result_path: root_dir.join("result.json"),
        profile_path: root_dir.join("profile.json"),
        editor_latest_path: config
            .artifacts_dir
            .join("editor-profiler")
            .join("latest.json"),
    };
    if let Err(error) = std::fs::write(
        &artifacts.meta_path,
        serde_json::to_string_pretty(&json!({
            "tool": "godot-qa",
            "project_root": config.project_root,
            "command": COMMAND,
            "source": SOURCE,
            "capture_method": CAPTURE_METHOD,
            "experimental": true,
        }))
        .unwrap_or_else(|_| "{}".to_string()),
    ) {
        eprintln!(
            "godot-qa: warning: failed to write editor profile meta {}: {error}",
            artifacts.meta_path.display()
        );
    }
    artifacts
}

pub(crate) fn normalize_editor_profile_artifact(
    value: &Value,
    path: &Path,
) -> Result<Value, FailureDetail> {
    let object = value.as_object().ok_or_else(|| {
        editor_profile_invalid_failure(path, "top-level object", type_name(value).to_string())
    })?;
    let source = object.get("source").and_then(Value::as_str).unwrap_or("");
    if source != SOURCE {
        return Err(editor_profile_invalid_failure(
            path,
            format!("source={SOURCE}"),
            format!("source={source}"),
        ));
    }
    let script_functions = object
        .get("script_functions")
        .and_then(Value::as_array)
        .ok_or_else(|| {
            editor_profile_invalid_failure(
                path,
                "script_functions array",
                object
                    .get("script_functions")
                    .map(type_name)
                    .unwrap_or("missing")
                    .to_string(),
            )
        })?;
    let mut frames = object
        .get("frames")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    if frames.len() > MAX_RETAINED_FRAMES {
        frames = frames.split_off(frames.len() - MAX_RETAINED_FRAMES);
    }
    let warnings = object
        .get("warnings")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();

    let mut normalized_functions = Vec::new();
    for (index, function) in script_functions.iter().enumerate() {
        normalized_functions.push(normalize_editor_script_function(function, path, index)?);
    }
    normalized_functions.sort_by(|left, right| {
        let left_total = left.get("total_ms").and_then(Value::as_f64).unwrap_or(0.0);
        let right_total = right.get("total_ms").and_then(Value::as_f64).unwrap_or(0.0);
        right_total
            .partial_cmp(&left_total)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    Ok(json!({
        "source": SOURCE,
        "capture_method": object.get("capture_method").and_then(Value::as_str).unwrap_or(CAPTURE_METHOD),
        "experimental": object.get("experimental").and_then(Value::as_bool).unwrap_or(true),
        "schema_version": object.get("schema_version").and_then(Value::as_u64).unwrap_or(1),
        "run_id": object.get("run_id").and_then(Value::as_str).unwrap_or("unknown"),
        "captured_at": object.get("captured_at").and_then(Value::as_str).unwrap_or(""),
        "godot_version": object.get("godot_version").and_then(Value::as_str).unwrap_or("unknown"),
        "project_root": object.get("project_root").cloned().unwrap_or(Value::Null),
        "session_id": object.get("session_id").and_then(Value::as_str).unwrap_or("unknown"),
        "frame_count": object.get("frame_count").and_then(Value::as_u64).unwrap_or(frames.len() as u64),
        "script_functions": normalized_functions,
        "frames": frames,
        "warnings": warnings,
    }))
}

fn normalize_editor_script_function(
    value: &Value,
    path: &Path,
    index: usize,
) -> Result<Value, FailureDetail> {
    let object = value.as_object().ok_or_else(|| {
        editor_profile_invalid_failure(
            path,
            format!("script_functions[{index}] object"),
            type_name(value).to_string(),
        )
    })?;
    let signature = object
        .get("signature")
        .and_then(Value::as_str)
        .ok_or_else(|| {
            editor_profile_invalid_failure(
                path,
                format!("script_functions[{index}].signature string"),
                object
                    .get("signature")
                    .map(type_name)
                    .unwrap_or("missing")
                    .to_string(),
            )
        })?;
    let parsed = parse_signature(signature);
    Ok(json!({
        "signature": signature,
        "script": object.get("script").and_then(Value::as_str).or(parsed.script.as_deref()),
        "function": object.get("function").and_then(Value::as_str).or(parsed.function.as_deref()).unwrap_or(signature),
        "call_count": object.get("call_count").and_then(Value::as_u64).unwrap_or(0),
        "total_ms": number_field(object, "total_ms"),
        "self_ms": number_field(object, "self_ms"),
        "internal_ms": number_field(object, "internal_ms"),
        "max_total_ms": number_field(object, "max_total_ms"),
    }))
}

fn number_field(object: &Map<String, Value>, key: &str) -> f64 {
    object.get(key).and_then(Value::as_f64).unwrap_or(0.0)
}

fn failed_result(
    message: &str,
    failure: FailureDetail,
    config: &ProjectConfig,
    artifacts: &EditorProfileArtifacts,
    raw_profile: Option<Value>,
) -> CommandResult {
    let mut data = Map::from_iter([
        ("project_root".to_string(), json!(config.project_root)),
        ("source".to_string(), json!(SOURCE)),
        ("experimental".to_string(), json!(true)),
        ("artifacts".to_string(), artifacts.to_json()),
        (
            "guidance".to_string(),
            json!(
                "Enable the Godot QA plugin, run the project from the Godot Editor, use the Godot QA Profiler tab, then rerun godot-qa profile editor export --json."
            ),
        ),
    ]);
    if let Some(raw_profile) = raw_profile {
        data.insert("raw_profile".to_string(), raw_profile);
    }
    CommandResult {
        ok: false,
        command: COMMAND.to_string(),
        message: message.to_string(),
        data,
        failures: vec![failure],
        exit_code: 1,
    }
}

fn editor_profile_missing_failure(
    path: &Path,
    artifacts: &EditorProfileArtifacts,
) -> FailureDetail {
    FailureDetail {
        kind: "editor_profile_missing".to_string(),
        command: Some(COMMAND.to_string()),
        path: Some(path.to_string_lossy().into_owned()),
        message: Some("No Godot QA editor profiler artifact was found".to_string()),
        expected: Some(json!("existing .qa/artifacts/editor-profiler/latest.json")),
        actual: Some(json!("missing")),
        artifacts: value_as_object_map(artifacts.to_json()),
        details: Map::from_iter([(
            "guidance".to_string(),
            json!(
                "Enable the Godot QA plugin, run the project from the Godot Editor, use the Godot QA Profiler tab, then rerun this command."
            ),
        )]),
        ..FailureDetail::default()
    }
}

fn editor_profile_invalid_failure(
    path: &Path,
    expected: impl Into<String>,
    actual: impl Into<String>,
) -> FailureDetail {
    FailureDetail {
        kind: "editor_profile_invalid".to_string(),
        command: Some(COMMAND.to_string()),
        path: Some(path.to_string_lossy().into_owned()),
        message: Some("The Godot QA editor profiler artifact is malformed".to_string()),
        expected: Some(json!(expected.into())),
        actual: Some(json!(actual.into())),
        ..FailureDetail::default()
    }
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ParsedSignature {
    pub(crate) script: Option<String>,
    pub(crate) function: Option<String>,
}

pub(crate) fn parse_signature(signature: &str) -> ParsedSignature {
    let parts: Vec<&str> = signature.split("::").collect();
    if parts.len() >= 3 {
        return ParsedSignature {
            script: Some(parts[0].to_string()),
            function: parts.last().map(|value| (*value).to_string()),
        };
    }
    ParsedSignature {
        script: None,
        function: parts.last().map(|value| (*value).to_string()),
    }
}

#[derive(Debug, Clone, PartialEq)]
#[allow(dead_code)]
pub(crate) struct ParsedEditorScriptFunction {
    pub(crate) signature: String,
    pub(crate) script: Option<String>,
    pub(crate) function: Option<String>,
    pub(crate) call_count: u64,
    pub(crate) total_ms: f64,
    pub(crate) self_ms: f64,
    pub(crate) internal_ms: f64,
    pub(crate) max_total_ms: f64,
}

#[allow(dead_code)]
pub(crate) fn parse_function_signature_message(value: &Value) -> Result<(i64, String), String> {
    let array = value.as_array().ok_or_else(|| {
        format!(
            "expected function signature array, got {}",
            type_name(value)
        )
    })?;
    if array.len() < 2 {
        return Err(format!(
            "expected function signature array with 2 values, got {}",
            array.len()
        ));
    }
    let signature = array[0]
        .as_str()
        .ok_or_else(|| "expected function signature name string".to_string())?
        .to_string();
    let signature_id = array[1]
        .as_i64()
        .ok_or_else(|| "expected function signature id integer".to_string())?;
    Ok((signature_id, signature))
}

#[allow(dead_code)]
pub(crate) fn parse_servers_profile_frame_message(
    value: &Value,
    signatures: &BTreeMap<i64, String>,
) -> Result<Vec<ParsedEditorScriptFunction>, String> {
    let array = value
        .as_array()
        .ok_or_else(|| format!("expected profile frame array, got {}", type_name(value)))?;
    if array.len() < 7 {
        return Err(format!(
            "expected profile frame array with at least 7 values, got {}",
            array.len()
        ));
    }
    let mut index = 6;
    let server_count = as_usize(array, index, "server_count")?;
    index += 1;
    for _ in 0..server_count {
        if index + 1 >= array.len() {
            return Err("profile frame server block is truncated".to_string());
        }
        index += 1;
        let server_value_count = as_usize(array, index, "server_value_count")?;
        index += 1 + server_value_count;
        if index > array.len() {
            return Err("profile frame server values exceed array length".to_string());
        }
    }
    if index >= array.len() {
        return Err("profile frame is missing script function block".to_string());
    }
    let script_value_count = as_usize(array, index, "script_value_count")?;
    index += 1;
    if script_value_count % 5 != 0 {
        return Err("script function value count must be divisible by 5".to_string());
    }
    if index + script_value_count > array.len() {
        return Err("script function values exceed array length".to_string());
    }
    let mut functions = Vec::new();
    for _ in 0..(script_value_count / 5) {
        let signature_id = as_i64(array, index, "signature_id")?;
        index += 1;
        let call_count = as_u64(array, index, "call_count")?;
        index += 1;
        let self_ms = as_f64(array, index, "self_time")? * 1000.0;
        index += 1;
        let total_ms = as_f64(array, index, "total_time")? * 1000.0;
        index += 1;
        let internal_ms = as_f64(array, index, "internal_time")? * 1000.0;
        index += 1;
        let signature = signatures
            .get(&signature_id)
            .cloned()
            .unwrap_or_else(|| format!("unknown:{signature_id}"));
        let parsed = parse_signature(&signature);
        functions.push(ParsedEditorScriptFunction {
            signature,
            script: parsed.script,
            function: parsed.function,
            call_count,
            total_ms,
            self_ms,
            internal_ms,
            max_total_ms: total_ms,
        });
    }
    Ok(functions)
}

#[allow(dead_code)]
fn as_i64(array: &[Value], index: usize, name: &str) -> Result<i64, String> {
    array
        .get(index)
        .and_then(Value::as_i64)
        .ok_or_else(|| format!("expected {name} integer at index {index}"))
}

#[allow(dead_code)]
fn as_u64(array: &[Value], index: usize, name: &str) -> Result<u64, String> {
    array
        .get(index)
        .and_then(Value::as_u64)
        .ok_or_else(|| format!("expected {name} unsigned integer at index {index}"))
}

#[allow(dead_code)]
fn as_usize(array: &[Value], index: usize, name: &str) -> Result<usize, String> {
    as_u64(array, index, name).and_then(|value| {
        usize::try_from(value).map_err(|_| format!("{name} at index {index} is too large"))
    })
}

#[allow(dead_code)]
fn as_f64(array: &[Value], index: usize, name: &str) -> Result<f64, String> {
    array
        .get(index)
        .and_then(Value::as_f64)
        .ok_or_else(|| format!("expected {name} number at index {index}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_function_signature_message() {
        let value = json!(["res://player.gd::Player::_process", 42]);

        let (id, signature) = parse_function_signature_message(&value).unwrap();

        assert_eq!(id, 42);
        assert_eq!(signature, "res://player.gd::Player::_process");
    }

    #[test]
    fn parses_profile_frame_script_functions() {
        let mut signatures = BTreeMap::new();
        signatures.insert(42, "res://player.gd::Player::_process".to_string());
        let value = json!([
            7, 0.016, 0.004, 0.003, 0.016, 0.002, 0, 5, 42, 12, 0.0081, 0.0142, 0.0
        ]);

        let rows = parse_servers_profile_frame_message(&value, &signatures).unwrap();

        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].signature, "res://player.gd::Player::_process");
        assert_eq!(rows[0].script.as_deref(), Some("res://player.gd"));
        assert_eq!(rows[0].function.as_deref(), Some("_process"));
        assert_eq!(rows[0].call_count, 12);
        assert!((rows[0].total_ms - 14.2).abs() < 0.000_001);
        assert!((rows[0].self_ms - 8.1).abs() < 0.000_001);
    }

    #[test]
    fn parses_profile_total_with_server_block() {
        let mut signatures = BTreeMap::new();
        signatures.insert(9, "res://enemy.gd::Enemy::_physics_process".to_string());
        let value = json!([
            8,
            0.017,
            0.005,
            0.006,
            0.016,
            0.003,
            1,
            "PhysicsServer2D",
            2,
            "step",
            0.001,
            5,
            9,
            3,
            0.001,
            0.002,
            0.0005
        ]);

        let rows = parse_servers_profile_frame_message(&value, &signatures).unwrap();

        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].function.as_deref(), Some("_physics_process"));
        assert!((rows[0].total_ms - 2.0).abs() < 0.000_001);
        assert!((rows[0].internal_ms - 0.5).abs() < 0.000_001);
    }

    #[test]
    fn rejects_malformed_profile_frame_script_count() {
        let value = json!([1, 0.016, 0.0, 0.0, 0.0, 0.0, 0, 4, 42, 1, 0.1, 0.2]);

        let error = parse_servers_profile_frame_message(&value, &BTreeMap::new()).unwrap_err();

        assert!(error.contains("divisible by 5"));
    }

    #[test]
    fn normalizes_valid_editor_profile_artifact() {
        let value = json!({
            "source": SOURCE,
            "capture_method": CAPTURE_METHOD,
            "experimental": true,
            "godot_version": "4.6.test",
            "frame_count": 12,
            "script_functions": [
                {
                    "signature": "res://player.gd::Player::_process",
                    "call_count": 120,
                    "total_ms": 14.2,
                    "self_ms": 8.1,
                    "internal_ms": 0.0,
                    "max_total_ms": 3.4
                }
            ],
            "frames": [],
            "warnings": []
        });

        let normalized =
            normalize_editor_profile_artifact(&value, Path::new("latest.json")).unwrap();

        assert_eq!(normalized["source"], SOURCE);
        assert_eq!(normalized["frame_count"], 12);
        assert_eq!(
            normalized["script_functions"][0]["script"],
            "res://player.gd"
        );
        assert_eq!(normalized["script_functions"][0]["function"], "_process");
    }
}
