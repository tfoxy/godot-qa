use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct FailureDetail {
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expected: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub actual: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub severity: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub artifacts: Option<serde_json::Map<String, Value>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub viewport: Option<Vec<i64>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub a: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub b: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rect: Option<Vec<f64>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rect_a: Option<Vec<f64>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rect_b: Option<Vec<f64>>,
    #[serde(flatten, default, skip_serializing_if = "Map::is_empty")]
    pub details: Map<String, Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommandResult {
    pub ok: bool,
    pub command: String,
    pub message: String,
    pub data: serde_json::Map<String, Value>,
    pub failures: Vec<FailureDetail>,
    #[serde(skip_serializing, default)]
    pub exit_code: i32,
}

impl CommandResult {
    pub fn success(command: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            ok: true,
            command: command.into(),
            message: message.into(),
            data: serde_json::Map::new(),
            failures: Vec::new(),
            exit_code: 0,
        }
    }
}
