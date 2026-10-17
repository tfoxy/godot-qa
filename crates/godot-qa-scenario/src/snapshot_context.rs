use godot_qa_core::results::FailureDetail;
use serde_json::{Map, Value, json};
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub struct SnapshotLoadResult {
    pub run_root: PathBuf,
    pub snapshot_name: String,
    pub snapshot_context: Value,
    pub artifacts: Map<String, Value>,
}

pub fn load_snapshot_context_from_artifacts(
    artifacts: &Map<String, Value>,
    command: &str,
) -> Result<Value, FailureDetail> {
    Ok(json!({
        "tree": read_json(&path_from_artifacts(artifacts, "tree"), "tree.json", artifacts, command)?,
        "ui": read_json(&path_from_artifacts(artifacts, "ui"), "ui.json", artifacts, command)?,
        "focus": read_json(&path_from_artifacts(artifacts, "focus"), "focus.json", artifacts, command)?,
        "errors": read_json(&path_from_artifacts(artifacts, "errors"), "errors.json", artifacts, command)?,
    }))
}

pub fn load_snapshot_context(
    artifacts_dir: &Path,
    scenario: &str,
    run_id: Option<&str>,
    snapshot: Option<&str>,
    command: &str,
) -> Result<SnapshotLoadResult, FailureDetail> {
    let run_root = resolve_run_root(artifacts_dir, scenario, run_id, command)?;
    let result_path = run_root.join("result.json");
    let result_data = read_json(
        &result_path,
        "result.json",
        &Map::from_iter([(
            "run_root".to_string(),
            json!(run_root.to_string_lossy().into_owned()),
        )]),
        command,
    )?;
    let Some(result_object) = result_data.as_object() else {
        return Err(failure(
            "artifact_read_failed",
            "result.json root must be an object",
            command,
            Some(result_path.to_string_lossy().into_owned()),
            Some(json!("JSON object")),
            Some(json!(python_type_name(&result_data))),
            Some("error".to_string()),
            Some(Map::from_iter([(
                "run_root".to_string(),
                json!(run_root.to_string_lossy().into_owned()),
            )])),
        ));
    };

    let snapshot_names = result_object
        .get("snapshots")
        .and_then(Value::as_array)
        .ok_or_else(|| {
            failure(
                "artifact_read_failed",
                "result.json snapshots must be an array of strings",
                command,
                Some(result_path.to_string_lossy().into_owned()),
                Some(json!("JSON array of strings")),
                Some(json!(python_type_name(
                    result_object.get("snapshots").unwrap_or(&Value::Null)
                ))),
                Some("error".to_string()),
                Some(Map::from_iter([(
                    "run_root".to_string(),
                    json!(run_root.to_string_lossy().into_owned()),
                )])),
            )
        })?;
    if snapshot_names.iter().any(|item| !item.is_string()) {
        return Err(failure(
            "artifact_read_failed",
            "result.json snapshots must be an array of strings",
            command,
            Some(result_path.to_string_lossy().into_owned()),
            Some(json!("JSON array of strings")),
            Some(json!(python_type_name(
                result_object.get("snapshots").unwrap_or(&Value::Null)
            ))),
            Some("error".to_string()),
            Some(Map::from_iter([(
                "run_root".to_string(),
                json!(run_root.to_string_lossy().into_owned()),
            )])),
        ));
    }

    let snapshot_name = if let Some(snapshot) = snapshot {
        snapshot.to_string()
    } else if let Some(last) = snapshot_names.last().and_then(Value::as_str) {
        last.to_string()
    } else {
        return Err(failure(
            "snapshot_not_found",
            "result.json did not record any snapshots; pass --snapshot explicitly after creating one",
            command,
            Some(result_path.to_string_lossy().into_owned()),
            Some(json!("non-empty snapshots array or explicit --snapshot")),
            Some(json!([])),
            Some("error".to_string()),
            Some(Map::from_iter([(
                "run_root".to_string(),
                json!(run_root.to_string_lossy().into_owned()),
            )])),
        ));
    };

    let snapshot_entry = result_object
        .get("snapshot_artifacts")
        .and_then(Value::as_object)
        .and_then(|entries| entries.get(&snapshot_name))
        .cloned()
        .unwrap_or_else(|| json!({}));
    let artifacts = snapshot_paths(&run_root, &snapshot_name, &snapshot_entry);

    Ok(SnapshotLoadResult {
        run_root,
        snapshot_name,
        snapshot_context: load_snapshot_context_from_artifacts(&artifacts, command)?,
        artifacts,
    })
}

fn resolve_run_root(
    artifacts_dir: &Path,
    scenario: &str,
    run_id: Option<&str>,
    command: &str,
) -> Result<PathBuf, FailureDetail> {
    let scenario_root = artifacts_dir.join(scenario);
    if !scenario_root.is_dir() {
        return Err(failure(
            "run_not_found",
            "no artifact runs exist for the requested scenario",
            command,
            Some(scenario_root.to_string_lossy().into_owned()),
            Some(json!("existing scenario run directory")),
            Some(json!("missing")),
            Some("error".to_string()),
            None,
        ));
    }
    if let Some(run_id) = run_id {
        let run_root = scenario_root.join(run_id);
        if run_root.is_dir() {
            return Ok(run_root);
        }
        return Err(failure(
            "run_not_found",
            "requested run id was not found",
            command,
            Some(run_root.to_string_lossy().into_owned()),
            Some(json!("existing run directory")),
            Some(json!(run_id)),
            Some("error".to_string()),
            None,
        ));
    }
    let mut candidates: Vec<PathBuf> = fs::read_dir(&scenario_root)
        .map_err(|_| {
            failure(
                "run_not_found",
                "no artifact runs exist for the requested scenario",
                command,
                Some(scenario_root.to_string_lossy().into_owned()),
                Some(json!("existing run directory")),
                Some(json!("missing")),
                Some("error".to_string()),
                None,
            )
        })?
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|path| path.is_dir())
        .collect();
    candidates.sort();
    candidates.pop().ok_or_else(|| {
        failure(
            "run_not_found",
            "no artifact runs exist for the requested scenario",
            command,
            Some(scenario_root.to_string_lossy().into_owned()),
            Some(json!("existing run directory")),
            Some(json!("missing")),
            Some("error".to_string()),
            None,
        )
    })
}

fn read_json(
    path: &Path,
    artifact_name: &str,
    artifacts: &Map<String, Value>,
    command: &str,
) -> Result<Value, FailureDetail> {
    let raw = fs::read_to_string(path).map_err(|_| {
        failure(
            "artifact_read_failed",
            &format!("could not load {artifact_name}"),
            command,
            Some(path.to_string_lossy().into_owned()),
            Some(json!("valid JSON artifact file")),
            Some(json!("OSError")),
            Some("error".to_string()),
            Some(artifacts.clone()),
        )
    })?;
    serde_json::from_str(&raw).map_err(|_| {
        failure(
            "artifact_read_failed",
            &format!("could not load {artifact_name}"),
            command,
            Some(path.to_string_lossy().into_owned()),
            Some(json!("valid JSON artifact file")),
            Some(json!("JSONDecodeError")),
            Some("error".to_string()),
            Some(artifacts.clone()),
        )
    })
}

fn snapshot_paths(
    run_root: &Path,
    snapshot_name: &str,
    snapshot_entry: &Value,
) -> Map<String, Value> {
    let mut paths = Map::from_iter([
        (
            "run_root".to_string(),
            json!(run_root.to_string_lossy().into_owned()),
        ),
        (
            "root".to_string(),
            json!(
                run_root
                    .join("snapshots")
                    .join(snapshot_name)
                    .to_string_lossy()
                    .into_owned()
            ),
        ),
        (
            "tree".to_string(),
            json!(
                run_root
                    .join("snapshots")
                    .join(snapshot_name)
                    .join("tree.json")
                    .to_string_lossy()
                    .into_owned()
            ),
        ),
        (
            "ui".to_string(),
            json!(
                run_root
                    .join("snapshots")
                    .join(snapshot_name)
                    .join("ui.json")
                    .to_string_lossy()
                    .into_owned()
            ),
        ),
        (
            "focus".to_string(),
            json!(
                run_root
                    .join("snapshots")
                    .join(snapshot_name)
                    .join("focus.json")
                    .to_string_lossy()
                    .into_owned()
            ),
        ),
        (
            "errors".to_string(),
            json!(
                run_root
                    .join("snapshots")
                    .join(snapshot_name)
                    .join("errors.json")
                    .to_string_lossy()
                    .into_owned()
            ),
        ),
        (
            "screenshot".to_string(),
            json!(
                run_root
                    .join("snapshots")
                    .join(snapshot_name)
                    .join("screenshot.png")
                    .to_string_lossy()
                    .into_owned()
            ),
        ),
    ]);
    if let Some(entry) = snapshot_entry.as_object() {
        for (key, value) in entry {
            if value.is_string() {
                paths.insert(key.clone(), value.clone());
            }
        }
    }
    paths
}

fn path_from_artifacts(artifacts: &Map<String, Value>, key: &str) -> PathBuf {
    artifacts
        .get(key)
        .and_then(Value::as_str)
        .map(PathBuf::from)
        .unwrap_or_default()
}

fn failure(
    kind: &str,
    message: &str,
    command: &str,
    path: Option<String>,
    expected: Option<Value>,
    actual: Option<Value>,
    severity: Option<String>,
    artifacts: Option<Map<String, Value>>,
) -> FailureDetail {
    FailureDetail {
        kind: kind.to_string(),
        command: Some(command.to_string()),
        message: Some(message.to_string()),
        path,
        expected,
        actual,
        severity,
        artifacts,
        ..FailureDetail::default()
    }
}

fn python_type_name(value: &Value) -> String {
    match value {
        Value::Null => "NoneType".to_string(),
        Value::Bool(_) => "bool".to_string(),
        Value::Number(number) => {
            if number.is_i64() || number.is_u64() {
                "int".to_string()
            } else {
                "float".to_string()
            }
        }
        Value::String(_) => "str".to_string(),
        Value::Array(_) => "list".to_string(),
        Value::Object(_) => "dict".to_string(),
    }
}
