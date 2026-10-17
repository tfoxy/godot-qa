use crate::artifacts;
use godot_qa_core::config::ProjectConfig;
use godot_qa_core::results::{CommandResult, FailureDetail};
use godot_qa_process;
use godot_qa_runtime;
use serde_json::{Map, Value, json};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

pub fn run_edit_live_set(
    config: &ProjectConfig,
    target: &str,
    property: &str,
    value: &str,
) -> CommandResult {
    godot_qa_runtime::run_live_protocol_command(
        config,
        "edit-live set",
        godot_qa_runtime::EDIT_SET_PROPERTY,
        Map::from_iter([
            ("target".to_string(), json!(target)),
            ("property".to_string(), json!(property)),
            ("value_input".to_string(), json!(value)),
        ]),
    )
}

pub fn run_edit_live_call(config: &ProjectConfig, target: &str, method: &str) -> CommandResult {
    godot_qa_runtime::run_live_protocol_command(
        config,
        "edit-live call",
        godot_qa_runtime::EDIT_CALL_METHOD,
        Map::from_iter([
            ("target".to_string(), json!(target)),
            ("method".to_string(), json!(method)),
        ]),
    )
}

pub fn run_edit_live_diff(config: &ProjectConfig) -> CommandResult {
    godot_qa_runtime::run_live_protocol_command(
        config,
        "edit-live diff",
        godot_qa_runtime::EDIT_DIFF,
        Map::new(),
    )
}

pub fn run_edit_live_export_patch(
    config: &ProjectConfig,
    out: &str,
    command_line: &str,
) -> CommandResult {
    let command = "edit-live export-patch";
    let mut result = godot_qa_runtime::run_live_protocol_command(
        config,
        command,
        godot_qa_runtime::EDIT_EXPORT_PATCH,
        Map::new(),
    );
    if !result.ok {
        return result;
    }

    let session_record = match godot_qa_runtime::load_session_record(config, command) {
        Ok(record) => record,
        Err(failure) => {
            return CommandResult {
                ok: false,
                command: command.to_string(),
                message: "patch export failed".to_string(),
                data: Map::new(),
                failures: vec![failure],
                exit_code: 1,
            };
        }
    };
    result
        .data
        .insert("created_at".to_string(), json!(started_at_now()));
    result.data.insert(
        "created_from".to_string(),
        json!({
            "session_id": session_record.session_id,
            "session_meta": Path::new(&session_record.artifact_dir).join("meta.json"),
        }),
    );

    let patch_document = json!({
        "format": result.data.get("format").cloned().unwrap_or(Value::Null),
        "scene": result.data.get("scene").cloned().unwrap_or(Value::Null),
        "created_at": result.data.get("created_at").cloned().unwrap_or(Value::Null),
        "created_from": result.data.get("created_from").cloned().unwrap_or(Value::Null),
        "operations": result.data.get("operations").cloned().unwrap_or_else(|| json!([])),
    });
    let patch_path = resolve_output_path(&config.project_root, out);
    if let Some(parent) = patch_path.parent() {
        if let Err(error) = fs::create_dir_all(parent) {
            return artifact_write_failed(command, &patch_path, &error.to_string());
        }
    }
    if let Err(error) = artifacts::write_json(&patch_path, &patch_document) {
        return artifact_write_failed(command, &patch_path, &error.to_string());
    }

    result.data = Map::from_iter([
        ("path".to_string(), json!(patch_path)),
        ("patch".to_string(), patch_document),
        (
            "skipped_operations".to_string(),
            result
                .data
                .get("skipped_operations")
                .cloned()
                .unwrap_or_else(|| json!([])),
        ),
        ("command".to_string(), json!(command_line)),
    ]);
    result
}

pub fn run_apply_patch(config: &ProjectConfig, patch: &str, command_line: &str) -> CommandResult {
    let command = "apply-patch";
    let patch_path = resolve_output_path(&config.project_root, patch);
    let payload = match read_patch_payload(&patch_path, command) {
        Ok(payload) => payload,
        Err(result) => return result,
    };

    if let Some(failure) = config.project_config_failure_for_command(command) {
        return CommandResult {
            ok: false,
            command: command.to_string(),
            message: "patch apply failed".to_string(),
            data: Map::new(),
            failures: vec![failure],
            exit_code: 1,
        };
    }

    let Some(binary) = config.godot_binary.as_deref() else {
        return CommandResult {
            ok: false,
            command: command.to_string(),
            message: "patch apply failed".to_string(),
            data: Map::new(),
            failures: vec![FailureDetail {
                kind: "missing_godot".to_string(),
                command: Some(command.to_string()),
                message: Some("Godot binary not found in PATH or GODOT_BIN".to_string()),
                expected: Some(json!(
                    "discoverable Godot binary via --godot-bin, GODOT_BIN, or PATH"
                )),
                actual: Some(json!("missing")),
                ..FailureDetail::default()
            }],
            exit_code: 1,
        };
    };

    let artifacts =
        prepare_patch_artifacts(&config.project_root, &config.artifacts_dir, &patch_path);
    if let Err(error) = artifacts::write_json(
        &artifacts.request_path,
        &json!({
            "patch": payload,
            "patch_path": patch_path,
            "artifacts": artifacts.to_json(),
            "run": {
                "id": artifacts.run_id,
                "started_at": started_at_now(),
                "command": command_line,
            }
        }),
    ) {
        return artifact_write_failed(command, &artifacts.request_path, &error.to_string());
    }

    let run = match godot_qa_process::launch_godot_patch_runner(
        binary,
        &config.godot_launch_prefix,
        &config.godot_args,
        &config.project_root,
        &artifacts.request_path,
        &artifacts.logs_path,
    ) {
        Ok(run) => run,
        Err(mut failure) => {
            failure.artifacts = artifacts.to_json().as_object().cloned();
            return CommandResult {
                ok: false,
                command: command.to_string(),
                message: "patch apply failed".to_string(),
                data: Map::from_iter([("artifacts".to_string(), artifacts.to_json())]),
                failures: vec![failure],
                exit_code: 1,
            };
        }
    };

    let runtime_result = match read_json_file(&artifacts.result_path) {
        Ok(result) => result,
        Err(failure) => {
            return CommandResult {
                ok: false,
                command: command.to_string(),
                message: "patch apply failed".to_string(),
                data: Map::from_iter([("artifacts".to_string(), artifacts.to_json())]),
                failures: vec![failure_with_artifacts(failure, &artifacts.to_json())],
                exit_code: 1,
            };
        }
    };
    let Some(object) = runtime_result.as_object() else {
        return patch_invalid_result(
            command,
            &artifacts,
            "patch runner result must decode to a JSON object",
            json!("JSON object result document"),
            json!(type_name(&runtime_result)),
        );
    };
    if !matches!(object.get("ok"), Some(Value::Bool(_))) {
        return patch_invalid_result(
            command,
            &artifacts,
            "result.json field 'ok' must be a boolean",
            json!("JSON boolean"),
            json!(type_name(object.get("ok").unwrap_or(&Value::Null))),
        );
    }
    let Some(raw_failures) = object.get("failures").and_then(Value::as_array) else {
        return patch_invalid_result(
            command,
            &artifacts,
            "result.json field 'failures' must be an array",
            json!("JSON array"),
            json!(type_name(object.get("failures").unwrap_or(&Value::Null))),
        );
    };

    let mut failures = Vec::new();
    for raw_failure in raw_failures {
        let Some(item) = raw_failure.as_object() else {
            return patch_invalid_result(
                command,
                &artifacts,
                "result.json failures entries must be objects",
                json!("JSON object entries"),
                json!(raw_failures.iter().map(type_name).collect::<Vec<_>>()),
            );
        };
        failures.push(FailureDetail {
            kind: item
                .get("type")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            message: item
                .get("message")
                .and_then(Value::as_str)
                .map(str::to_string),
            command: item
                .get("command")
                .and_then(Value::as_str)
                .map(str::to_string),
            path: item.get("path").and_then(Value::as_str).map(str::to_string),
            expected: item.get("expected").cloned(),
            actual: item.get("actual").cloned(),
            target: item
                .get("target")
                .and_then(Value::as_str)
                .map(str::to_string),
            severity: item
                .get("severity")
                .and_then(Value::as_str)
                .map(str::to_string),
            artifacts: item.get("artifacts").and_then(Value::as_object).cloned(),
            ..FailureDetail::default()
        });
    }

    if run.returncode != 0 && failures.is_empty() {
        failures.push(FailureDetail {
            kind: "patch_apply_failed".to_string(),
            command: Some(command.to_string()),
            path: Some(artifacts.result_path.to_string_lossy().into_owned()),
            message: Some("Godot exited with a non-zero status".to_string()),
            expected: Some(json!(0)),
            actual: Some(json!(run.returncode)),
            artifacts: artifacts.to_json().as_object().cloned(),
            ..FailureDetail::default()
        });
    }

    let ok = object.get("ok").and_then(Value::as_bool).unwrap_or(false)
        && run.returncode == 0
        && failures.is_empty();
    let data = Map::from_iter([
        (
            "scene".to_string(),
            object.get("scene").cloned().unwrap_or(Value::Null),
        ),
        (
            "applied_operations".to_string(),
            object
                .get("applied_operations")
                .cloned()
                .unwrap_or(Value::Null),
        ),
        (
            "saved_scene_path".to_string(),
            object
                .get("saved_scene_path")
                .cloned()
                .unwrap_or(Value::Null),
        ),
        ("artifacts".to_string(), artifacts.to_json()),
    ]);
    CommandResult {
        ok,
        command: command.to_string(),
        message: if ok {
            "patch applied".to_string()
        } else {
            "patch apply failed".to_string()
        },
        data,
        failures,
        exit_code: if ok { 0 } else { 1 },
    }
}

#[derive(Debug, Clone)]
struct PatchArtifacts {
    run_id: String,
    root_dir: PathBuf,
    request_path: PathBuf,
    result_path: PathBuf,
    logs_path: PathBuf,
    patch_path: PathBuf,
    project_root: PathBuf,
}

impl PatchArtifacts {
    fn to_json(&self) -> Value {
        json!({
            "root": self.root_dir,
            "request": self.request_path,
            "result": self.result_path,
            "logs": self.logs_path,
            "patch": self.patch_path,
            "project_root": self.project_root,
        })
    }
}

fn prepare_patch_artifacts(
    project_root: &Path,
    artifacts_root: &Path,
    patch_path: &Path,
) -> PatchArtifacts {
    let run_id = unique_run_id();
    let root_dir = artifacts_root
        .join("patch-apply")
        .join(
            patch_path
                .file_stem()
                .and_then(|value| value.to_str())
                .unwrap_or("patch"),
        )
        .join(&run_id);
    let _ = fs::create_dir_all(&root_dir);
    PatchArtifacts {
        run_id,
        request_path: root_dir.join("request.json"),
        result_path: root_dir.join("result.json"),
        logs_path: root_dir.join("logs.txt"),
        patch_path: patch_path.to_path_buf(),
        project_root: project_root.to_path_buf(),
        root_dir,
    }
}

fn read_patch_payload(path: &Path, command: &str) -> Result<Value, CommandResult> {
    let contents = match fs::read_to_string(path) {
        Ok(contents) => contents,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Err(patch_invalid(
                command,
                path,
                "patch file was not found",
                json!("existing patch file"),
                json!("missing"),
            ));
        }
        Err(error) => {
            return Err(patch_invalid(
                command,
                path,
                &error.to_string(),
                json!("readable patch file"),
                json!("OSError"),
            ));
        }
    };
    let payload = match serde_json::from_str::<Value>(&contents) {
        Ok(payload) => payload,
        Err(_) => {
            return Err(patch_invalid(
                command,
                path,
                "patch file must be valid JSON",
                json!("JSON object patch document"),
                json!("invalid_json"),
            ));
        }
    };
    if !payload.is_object() {
        return Err(patch_invalid(
            command,
            path,
            "patch file must decode to a JSON object",
            json!("JSON object patch document"),
            json!(type_name(&payload)),
        ));
    }
    Ok(payload)
}

fn patch_invalid(
    command: &str,
    path: &Path,
    message: &str,
    expected: Value,
    actual: Value,
) -> CommandResult {
    CommandResult {
        ok: false,
        command: command.to_string(),
        message: "patch apply failed".to_string(),
        data: Map::new(),
        failures: vec![FailureDetail {
            kind: "patch_invalid".to_string(),
            command: Some(command.to_string()),
            path: Some(path.to_string_lossy().into_owned()),
            message: Some(message.to_string()),
            expected: Some(expected),
            actual: Some(actual),
            ..FailureDetail::default()
        }],
        exit_code: 1,
    }
}

fn patch_invalid_result(
    command: &str,
    artifacts: &PatchArtifacts,
    message: &str,
    expected: Value,
    actual: Value,
) -> CommandResult {
    CommandResult {
        ok: false,
        command: command.to_string(),
        message: "patch apply failed".to_string(),
        data: Map::from_iter([("artifacts".to_string(), artifacts.to_json())]),
        failures: vec![FailureDetail {
            kind: "patch_invalid".to_string(),
            command: Some(command.to_string()),
            path: Some(artifacts.result_path.to_string_lossy().into_owned()),
            message: Some(message.to_string()),
            expected: Some(expected),
            actual: Some(actual),
            artifacts: artifacts.to_json().as_object().cloned(),
            ..FailureDetail::default()
        }],
        exit_code: 1,
    }
}

fn read_json_file(path: &Path) -> Result<Value, FailureDetail> {
    let contents = fs::read_to_string(path).map_err(|error| FailureDetail {
        kind: "patch_invalid".to_string(),
        path: Some(path.to_string_lossy().into_owned()),
        message: Some(if error.kind() == std::io::ErrorKind::NotFound {
            "patch runner did not produce result.json".to_string()
        } else {
            error.to_string()
        }),
        expected: Some(if error.kind() == std::io::ErrorKind::NotFound {
            json!("result.json written by qa_patch_runner.gd")
        } else {
            json!("readable result.json")
        }),
        actual: Some(if error.kind() == std::io::ErrorKind::NotFound {
            json!("missing")
        } else {
            json!("OSError")
        }),
        ..FailureDetail::default()
    })?;
    serde_json::from_str::<Value>(&contents).map_err(|_| FailureDetail {
        kind: "patch_invalid".to_string(),
        path: Some(path.to_string_lossy().into_owned()),
        message: Some("patch runner result must be valid JSON".to_string()),
        expected: Some(json!("JSON object result document")),
        actual: Some(json!("invalid_json")),
        ..FailureDetail::default()
    })
}

fn resolve_output_path(project_root: &Path, output: &str) -> PathBuf {
    let path = PathBuf::from(output);
    if path.is_absolute() {
        path
    } else {
        project_root.join(path)
    }
}

fn started_at_now() -> String {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| format!("{}Z", duration.as_secs()))
        .unwrap_or_else(|_| "0Z".to_string())
}

fn unique_run_id() -> String {
    started_at_now()
}

fn artifact_write_failed(command: &str, path: &Path, message: &str) -> CommandResult {
    CommandResult {
        ok: false,
        command: command.to_string(),
        message: if command == "edit-live export-patch" {
            "patch export failed".to_string()
        } else {
            "patch apply failed".to_string()
        },
        data: Map::new(),
        failures: vec![FailureDetail {
            kind: "artifact_write_failed".to_string(),
            command: Some(command.to_string()),
            path: Some(path.to_string_lossy().into_owned()),
            message: Some(message.to_string()),
            expected: Some(json!(if command == "edit-live export-patch" {
                "patch file written to the requested output path"
            } else {
                "artifact metadata written under the artifact directory"
            })),
            actual: Some(json!("OSError")),
            ..FailureDetail::default()
        }],
        exit_code: 1,
    }
}

fn failure_with_artifacts(mut failure: FailureDetail, artifacts: &Value) -> FailureDetail {
    failure.artifacts = artifacts.as_object().cloned();
    failure
}

fn type_name(value: &Value) -> &'static str {
    match value {
        Value::Null => "NoneType",
        Value::Bool(_) => "bool",
        Value::Number(_) => "number",
        Value::String(_) => "str",
        Value::Array(_) => "list",
        Value::Object(_) => "dict",
    }
}
