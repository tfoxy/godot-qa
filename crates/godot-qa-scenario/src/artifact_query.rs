use godot_qa_core::results::FailureDetail;
use serde::Serialize;
use serde_json::{Map, Value, json};
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Default)]
pub struct QueryRequest {
    pub target: Option<String>,
    pub node_type: Option<String>,
    pub visible: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct NodeMatch {
    pub snapshot: String,
    pub path: String,
    pub name: String,
    #[serde(rename = "type")]
    pub node_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub qa_id: Option<String>,
    pub visible: bool,
    pub inside_tree: bool,
    pub groups: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct QueryResult {
    pub snapshot: String,
    pub tree_path: Option<PathBuf>,
    pub matches: Vec<NodeMatch>,
}

impl QueryResult {
    pub fn data(&self, run_root: Option<&Path>) -> Map<String, Value> {
        let mut data = Map::from_iter([
            ("snapshot".to_string(), json!(self.snapshot)),
            ("count".to_string(), json!(self.matches.len())),
            ("matches".to_string(), json!(self.matches)),
        ]);
        if let Some(run_root) = run_root {
            data.insert(
                "run_root".to_string(),
                json!(run_root.to_string_lossy().into_owned()),
            );
        }
        if let Some(tree_path) = &self.tree_path {
            data.insert(
                "artifacts".to_string(),
                json!({"tree": tree_path.to_string_lossy().into_owned()}),
            );
        }
        data
    }
}

pub fn query_run_snapshot(
    run_root: &Path,
    snapshot: &str,
    request: QueryRequest,
    command: &str,
) -> Result<QueryResult, FailureDetail> {
    let tree_path = run_root.join("snapshots").join(snapshot).join("tree.json");
    let tree = read_json(&tree_path, "tree.json", command)?;
    let mut result = query_tree_value(snapshot, &tree, request, command, Some(&tree_path))?;
    result.tree_path = Some(tree_path);
    Ok(result)
}

pub fn query_snapshot_context(
    snapshot: &str,
    snapshot_context: &Value,
    request: QueryRequest,
    command: &str,
) -> Result<QueryResult, FailureDetail> {
    let tree = snapshot_context.get("tree").ok_or_else(|| {
        artifact_failure(
            command,
            None,
            "snapshot context must contain a tree object",
            json!("tree object"),
            json!("missing"),
        )
    })?;
    query_tree_value(snapshot, tree, request, command, None)
}

fn query_tree_value(
    snapshot: &str,
    tree: &Value,
    request: QueryRequest,
    command: &str,
    path: Option<&Path>,
) -> Result<QueryResult, FailureDetail> {
    let root = tree.get("root").and_then(Value::as_object).ok_or_else(|| {
        artifact_failure(
            command,
            None,
            "tree.json must contain a root object",
            json!("root object"),
            json!("missing"),
        )
    })?;
    let mut matches = Vec::new();
    for node in flatten_tree(root) {
        if !matches_target(&node, request.target.as_deref()) {
            continue;
        }
        if !matches_type(&node, request.node_type.as_deref()) {
            continue;
        }
        let visible = node
            .get("visible")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        if request.visible && !visible {
            continue;
        }
        matches.push(node_match(snapshot, &node, visible));
    }
    if matches.is_empty()
        && filtered_tree(tree)
        && (request
            .target
            .as_deref()
            .is_some_and(|target| !target.trim().is_empty())
            || request
                .node_type
                .as_deref()
                .is_some_and(|node_type| !node_type.trim().is_empty()))
    {
        return Err(filtered_evidence_failure(command, path, tree, &request));
    }
    Ok(QueryResult {
        snapshot: snapshot.to_string(),
        tree_path: None,
        matches,
    })
}

fn read_json(path: &Path, artifact_name: &str, command: &str) -> Result<Value, FailureDetail> {
    let raw = fs::read_to_string(path).map_err(|_| {
        artifact_failure(
            command,
            Some(path),
            &format!("could not load {artifact_name}"),
            json!("valid JSON artifact file"),
            json!("OSError"),
        )
    })?;
    serde_json::from_str(&raw).map_err(|_| {
        artifact_failure(
            command,
            Some(path),
            &format!("could not load {artifact_name}"),
            json!("valid JSON artifact file"),
            json!("JSONDecodeError"),
        )
    })
}

fn flatten_tree(root: &Map<String, Value>) -> Vec<Map<String, Value>> {
    let mut entries = vec![root.clone()];
    if let Some(children) = root.get("children").and_then(Value::as_array) {
        for child in children.iter().filter_map(Value::as_object) {
            entries.extend(flatten_tree(child));
        }
    }
    entries
}

fn matches_target(node: &Map<String, Value>, target: Option<&str>) -> bool {
    let Some(target) = target.filter(|value| !value.trim().is_empty()) else {
        return true;
    };
    if let Some(qa_id) = target.strip_prefix("qa:") {
        return node.get("qa_id").and_then(Value::as_str) == Some(qa_id);
    }
    if let Some(path) = target.strip_prefix("path:") {
        return node.get("path").and_then(Value::as_str) == Some(path);
    }
    if let Some(name) = target.strip_prefix("name:") {
        return node.get("name").and_then(Value::as_str) == Some(name);
    }
    false
}

fn matches_type(node: &Map<String, Value>, node_type: Option<&str>) -> bool {
    let Some(node_type) = node_type.filter(|value| !value.trim().is_empty()) else {
        return true;
    };
    node.get("type").and_then(Value::as_str) == Some(node_type)
}

fn node_match(snapshot: &str, node: &Map<String, Value>, visible: bool) -> NodeMatch {
    NodeMatch {
        snapshot: snapshot.to_string(),
        path: node
            .get("path")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        name: node
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        node_type: node
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        qa_id: node
            .get("qa_id")
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())
            .map(str::to_string),
        visible,
        inside_tree: node
            .get("inside_tree")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        groups: node
            .get("groups")
            .and_then(Value::as_array)
            .map(|groups| {
                groups
                    .iter()
                    .filter_map(Value::as_str)
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default(),
    }
}

fn filtered_tree(tree: &Value) -> bool {
    tree.get("filter").is_some()
}

fn filtered_evidence_failure(
    command: &str,
    path: Option<&Path>,
    tree: &Value,
    request: &QueryRequest,
) -> FailureDetail {
    let target = request.target.clone().or_else(|| {
        request
            .node_type
            .as_ref()
            .map(|node_type| format!("type:{node_type}"))
    });
    FailureDetail {
        kind: "snapshot_evidence_omitted".to_string(),
        command: Some(command.to_string()),
        path: path.map(|path| path.to_string_lossy().into_owned()),
        message: Some(
            "filtered snapshot evidence did not contain the requested node; rerun with a broader snapshots filter or a full snapshot"
                .to_string(),
        ),
        expected: Some(json!("matching node evidence in tree.json")),
        actual: Some(json!({
            "matches": 0,
            "filter": tree.get("filter").cloned().unwrap_or(Value::Null),
        })),
        target,
        severity: Some("error".to_string()),
        ..FailureDetail::default()
    }
}

fn artifact_failure(
    command: &str,
    path: Option<&Path>,
    message: &str,
    expected: Value,
    actual: Value,
) -> FailureDetail {
    FailureDetail {
        kind: "artifact_read_failed".to_string(),
        command: Some(command.to_string()),
        path: path.map(|path| path.to_string_lossy().into_owned()),
        message: Some(message.to_string()),
        expected: Some(expected),
        actual: Some(actual),
        severity: Some("error".to_string()),
        ..FailureDetail::default()
    }
}
