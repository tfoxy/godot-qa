//! Final verdicts for a custom child launched by an external display wrapper.
//! This module reads receipts; it deliberately never launches the child.

use crate::{ArtifactBeginRunArgs, ArtifactValidateResultArgs};
use godot_qa_core::results::{CommandResult, FailureDetail};
use serde_json::{Map, Value, json};
use std::fs::{self, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

const SCHEMA_VERSION: u64 = 1;

pub(crate) fn artifact_begin_run_result(args: &ArtifactBeginRunArgs) -> CommandResult {
    let command = "artifact begin-run";
    let root = PathBuf::from(
        args.out
            .as_deref()
            .or(args.run_root.as_deref())
            .unwrap_or(""),
    );
    if root.file_name().and_then(|name| name.to_str()).is_none() {
        return failure_result(
            command,
            &root,
            "invalid_run_root",
            "run root must have a final directory name",
            Map::new(),
        );
    };
    let Some(parent) = root.parent() else {
        return failure_result(
            command,
            &root,
            "invalid_run_root",
            "run root needs a parent directory",
            Map::new(),
        );
    };
    let parent = if parent.as_os_str().is_empty() {
        Path::new(".")
    } else {
        parent
    };
    if let Err(error) = fs::create_dir_all(parent) {
        return failure_result(
            command,
            &root,
            "artifact_write_failed",
            &error.to_string(),
            Map::new(),
        );
    }
    let mut random = [0_u8; 32];
    if let Err(error) = getrandom::fill(&mut random) {
        return failure_result(
            command,
            &root,
            "run_nonce_failed",
            &error.to_string(),
            Map::new(),
        );
    }
    let run_id = format!("run-{}", hex_bytes(&random[..16]));
    let nonce = hex_bytes(&random[16..]);
    if let Err(error) = fs::create_dir(&root) {
        return failure_result(
            command,
            &root,
            "run_root_exists_or_unwritable",
            &error.to_string(),
            Map::new(),
        );
    }
    let root = match fs::canonicalize(&root) {
        Ok(root) => root,
        Err(error) => {
            return failure_result(
                command,
                &root,
                "artifact_write_failed",
                &error.to_string(),
                Map::new(),
            );
        }
    };
    let started_at_unix_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0);
    let identity = json!({
        "schema_version": SCHEMA_VERSION,
        "kind": "external_child",
        "run_id": run_id,
        "nonce": nonce,
        "started_at_unix_ms": started_at_unix_ms,
    });
    let path = root.join("run.json");
    if let Err(error) = write_new_json(&path, &identity) {
        return failure_result(
            command,
            &root,
            "artifact_write_failed",
            &error.to_string(),
            Map::new(),
        );
    }
    let mut result = CommandResult::success(command, "external run identity created");
    result.data = Map::from_iter([
        ("run_root".to_owned(), json!(root)),
        ("run_path".to_owned(), json!(path)),
        ("result_path".to_owned(), json!(root.join("result.json"))),
        (
            "child_status_path".to_owned(),
            json!(root.join("child-status.json")),
        ),
        ("run".to_owned(), identity),
    ]);
    result
}

pub(crate) fn artifact_validate_result_result(args: &ArtifactValidateResultArgs) -> CommandResult {
    validate_result(
        Path::new(&args.run_root),
        args.wrapper_exit,
        args.wrapper_timed_out,
    )
}

fn validate_result(root: &Path, wrapper_exit: i32, wrapper_timed_out: bool) -> CommandResult {
    let command = "artifact validate-result";
    let run_path = root.join("run.json");
    let status_path = root.join("child-status.json");
    let result_path = root.join("result.json");
    let verdict_path = root.join("verdict.json");
    let mut failures = Vec::new();
    let run = read_object(&run_path, "run_identity", &mut failures);
    let valid_identity = run.as_ref().and_then(|value| {
        let run_id = value.get("run_id")?.as_str()?;
        let nonce = value.get("nonce")?.as_str()?;
        let started_at = value.get("started_at_unix_ms")?.as_u64()?;
        (value.get("schema_version")?.as_u64()? == SCHEMA_VERSION
            && value.get("kind")?.as_str()? == "external_child"
            && started_at > 0
            && run_id.len() == 36
            && run_id
                .strip_prefix("run-")?
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit())
            && nonce.len() == 32
            && nonce.bytes().all(|byte| byte.is_ascii_hexdigit()))
        .then(|| (run_id.to_owned(), nonce.to_owned(), started_at))
    });
    if run.is_some() && valid_identity.is_none() {
        failures.push(failure(
            "run_identity_invalid",
            &run_path,
            "run identity has the wrong schema, directory ID, or nonce",
        ));
    }
    let child = read_object(&status_path, "child_status", &mut failures);
    let result = read_object(&result_path, "child_result", &mut failures);
    if let Some((run_id, nonce, started_at)) = &valid_identity {
        for (name, value, path) in [
            ("child_status", child.as_ref(), &status_path),
            ("child_result", result.as_ref(), &result_path),
        ] {
            if let Some(value) = value {
                if value.get("run_id").and_then(Value::as_str) != Some(run_id)
                    || value.get("nonce").and_then(Value::as_str) != Some(nonce)
                    || value.get("schema_version").and_then(Value::as_u64) != Some(SCHEMA_VERSION)
                    || value.get("started_at_unix_ms").and_then(Value::as_u64) != Some(*started_at)
                    || value
                        .get("completed_at_unix_ms")
                        .and_then(Value::as_u64)
                        .is_none_or(|time| time < *started_at)
                {
                    failures.push(failure(
                        &format!("{name}_stale"),
                        path,
                        "receipt version, identity, or timestamps differ from this invocation",
                    ));
                }
            }
        }
    }
    if let Some(value) = &result {
        match (
            value.get("ok").and_then(Value::as_bool),
            value.get("failures").and_then(Value::as_array),
        ) {
            (Some(false), Some(items)) if !items.is_empty() => {
                for item in items {
                    let detail = serde_json::from_value::<FailureDetail>(item.clone())
                        .ok()
                        .filter(|detail| !detail.kind.is_empty());
                    if let Some(mut detail) = detail {
                        if detail.command.is_none() {
                            detail.command = Some(command.to_owned());
                        }
                        if detail.path.is_none() {
                            detail.path = Some(result_path.to_string_lossy().into_owned());
                        }
                        failures.push(detail);
                    } else {
                        failures.push(failure(
                            "child_result_invalid",
                            &result_path,
                            "child failure item must include a string type",
                        ));
                    }
                }
            }
            (Some(false), _) => failures.push(failure(
                "child_assertion_failed",
                &result_path,
                "child reported failure without a failure list",
            )),
            (Some(true), Some(items)) if !items.is_empty() => failures.push(failure(
                "child_result_invalid",
                &result_path,
                "successful result contains failures",
            )),
            (Some(true), Some(_)) => {}
            _ => failures.push(failure(
                "child_result_invalid",
                &result_path,
                "result needs boolean ok and failures array",
            )),
        }
    }
    if let Some(value) = &child {
        match value.get("termination").and_then(Value::as_str) {
            Some("exited") => match value.get("exit_code").and_then(Value::as_i64) {
                Some(0) => {}
                Some(_) => failures.push(failure(
                    "child_exit_nonzero",
                    &status_path,
                    "child exited nonzero",
                )),
                None => failures.push(failure(
                    "child_status_invalid",
                    &status_path,
                    "exited receipt needs integer exit_code",
                )),
            },
            Some("signaled") => {
                if value
                    .get("signal")
                    .and_then(Value::as_i64)
                    .is_some_and(|signal| signal > 0)
                {
                    failures.push(failure(
                        "child_crashed",
                        &status_path,
                        "child terminated from a recorded signal",
                    ));
                } else {
                    failures.push(failure(
                        "child_status_invalid",
                        &status_path,
                        "signaled receipt needs a positive integer signal",
                    ));
                }
            }
            Some("timed_out") => failures.push(failure(
                "child_timeout",
                &status_path,
                "child exceeded its orchestrator deadline",
            )),
            Some("launch_failed") => failures.push(failure(
                "child_launch_failed",
                &status_path,
                "child could not start",
            )),
            Some("unknown") => failures.push(failure(
                "child_completion_unknown",
                &status_path,
                "wrapper could not prove child completion",
            )),
            _ => failures.push(failure(
                "child_status_invalid",
                &status_path,
                "unknown child termination kind",
            )),
        }
    }
    let matching_child_nonzero = valid_identity
        .as_ref()
        .is_some_and(|(run_id, nonce, started)| {
            child.as_ref().is_some_and(|value| {
                value.get("run_id").and_then(Value::as_str) == Some(run_id)
                    && value.get("nonce").and_then(Value::as_str) == Some(nonce)
                    && value.get("schema_version").and_then(Value::as_u64) == Some(SCHEMA_VERSION)
                    && value.get("started_at_unix_ms").and_then(Value::as_u64) == Some(*started)
                    && value.get("termination").and_then(Value::as_str) == Some("exited")
                    && value
                        .get("exit_code")
                        .and_then(Value::as_i64)
                        .is_some_and(|code| code != 0)
            })
        });
    if result.is_none() && matching_child_nonzero {
        if godot_parser_diagnostic(&root.join("logs.txt")) {
            failures.push(failure(
                "child_parser_failed",
                &root.join("logs.txt"),
                "Godot emitted a parser diagnostic before writing a result",
            ));
        } else if child
            .as_ref()
            .and_then(|value| value.get("failure_phase"))
            .and_then(Value::as_str)
            == Some("startup")
        {
            failures.push(failure(
                "child_startup_failed",
                &status_path,
                "inner child receipt reports startup failure",
            ));
        }
    }
    if wrapper_timed_out {
        failures.push(failure(
            "wrapper_timeout",
            root,
            "external wrapper exceeded its orchestrator deadline",
        ));
    } else if wrapper_exit != 0 {
        failures.push(failure(
            "wrapper_failed",
            root,
            "external wrapper exited nonzero",
        ));
    }
    let current_result = result.as_ref().filter(|value| {
        valid_identity.as_ref().is_some_and(|(id, nonce, started)| {
            value.get("run_id").and_then(Value::as_str) == Some(id)
                && value.get("nonce").and_then(Value::as_str) == Some(nonce)
                && value.get("schema_version").and_then(Value::as_u64) == Some(SCHEMA_VERSION)
                && value.get("started_at_unix_ms").and_then(Value::as_u64) == Some(*started)
        })
    });
    let reported_first = current_result
        .and_then(|value| value.get("failures"))
        .and_then(Value::as_array)
        .and_then(|items| items.first())
        .and_then(|item| item.get("type"))
        .and_then(Value::as_str);
    let priority = reported_first
        .and_then(|kind| failures.iter().position(|failure| failure.kind == kind))
        .or_else(|| {
            [
                "child_parser_failed",
                "child_startup_failed",
                "child_timeout",
                "child_crashed",
                "child_exit_nonzero",
                "child_launch_failed",
                "wrapper_timeout",
            ]
            .iter()
            .find_map(|kind| failures.iter().position(|failure| failure.kind == *kind))
        });
    if let Some(index) = priority {
        failures.swap(0, index);
    }
    let ok = failures.is_empty();
    let first_failure = failures.first().map(|failure| failure.kind.clone());
    let data = Map::from_iter([
        ("run_root".to_owned(), json!(root)),
        (
            "run_id".to_owned(),
            json!(valid_identity.as_ref().map(|(id, _, _)| id)),
        ),
        (
            "wrapper".to_owned(),
            json!({"exit_code":wrapper_exit,"timed_out":wrapper_timed_out}),
        ),
        (
            "child_completion".to_owned(),
            json!(child.as_ref().and_then(|value| value.get("termination"))),
        ),
        (
            "assertions_completed".to_owned(),
            json!(
                current_result.is_some_and(|value| value
                    .get("ok")
                    .and_then(Value::as_bool)
                    .is_some()
                    && value.get("failures").and_then(Value::as_array).is_some())
            ),
        ),
        (
            "assertions_passed".to_owned(),
            json!(
                current_result
                    .and_then(|value| value.get("ok"))
                    .and_then(Value::as_bool)
            ),
        ),
        ("first_failure".to_owned(), json!(first_failure)),
        (
            "artifacts".to_owned(),
            json!({"run":run_path,"child_status":status_path,"result":result_path,"verdict":verdict_path}),
        ),
    ]);
    let mut verdict = CommandResult {
        ok,
        command: command.to_owned(),
        message: if ok {
            "external child passed"
        } else {
            "external child failed"
        }
        .to_owned(),
        data,
        failures,
        exit_code: i32::from(!ok),
    };
    if let Err(error) = write_new_json(
        &verdict_path,
        &serde_json::to_value(&verdict).unwrap_or_default(),
    ) {
        verdict.ok = false;
        verdict.exit_code = 1;
        verdict.message = "external child verdict could not be persisted".to_owned();
        verdict.failures.push(failure(
            "verdict_write_failed",
            &verdict_path,
            &error.to_string(),
        ));
    }
    verdict
}

fn hex_bytes(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn godot_parser_diagnostic(path: &Path) -> bool {
    let Ok(file) = fs::File::open(path) else {
        return false;
    };
    let mut bounded = Vec::new();
    if file
        .take(4 * 1024 * 1024)
        .read_to_end(&mut bounded)
        .is_err()
    {
        return false;
    }
    String::from_utf8_lossy(&bounded).lines().any(|line| {
        let line = line.trim_start();
        line.starts_with("SCRIPT ERROR: Parse Error") || line.starts_with("Parse Error:")
    })
}

fn read_object(path: &Path, kind: &str, failures: &mut Vec<FailureDetail>) -> Option<Value> {
    let bytes = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) => {
            failures.push(failure(
                &format!("{kind}_missing"),
                path,
                &error.to_string(),
            ));
            return None;
        }
    };
    match serde_json::from_slice::<Value>(&bytes) {
        Ok(value) if value.is_object() => Some(value),
        Ok(_) => {
            failures.push(failure(
                &format!("{kind}_malformed"),
                path,
                "receipt root must be a JSON object",
            ));
            None
        }
        Err(error) => {
            failures.push(failure(
                &format!("{kind}_malformed"),
                path,
                &error.to_string(),
            ));
            None
        }
    }
}

fn write_new_json(path: &Path, value: &Value) -> std::io::Result<()> {
    let mut file = OpenOptions::new().write(true).create_new(true).open(path)?;
    serde_json::to_writer_pretty(&mut file, value)?;
    file.write_all(b"\n")
}

fn failure(kind: &str, path: &Path, message: &str) -> FailureDetail {
    FailureDetail {
        kind: kind.to_owned(),
        command: Some("artifact validate-result".to_owned()),
        path: Some(path.to_string_lossy().into_owned()),
        message: Some(message.to_owned()),
        expected: Some(json!("valid receipt for the current child run")),
        actual: Some(json!(message)),
        ..FailureDetail::default()
    }
}

fn failure_result(
    command: &str,
    root: &Path,
    kind: &str,
    message: &str,
    data: Map<String, Value>,
) -> CommandResult {
    CommandResult {
        ok: false,
        command: command.to_owned(),
        message: format!("{command} failed"),
        data,
        failures: vec![FailureDetail {
            kind: kind.to_owned(),
            command: Some(command.to_owned()),
            path: Some(root.to_string_lossy().into_owned()),
            message: Some(message.to_owned()),
            ..FailureDetail::default()
        }],
        exit_code: 1,
    }
}
