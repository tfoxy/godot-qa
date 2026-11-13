mod discovery;
mod normalize;
mod validation;

#[allow(unused_imports)]
pub(crate) use discovery::is_abstract_scenario_file;
pub use discovery::{
    ScenarioIndex, find_scenario_by_name, find_scenarios_by_name, list_abstract_scenario_files,
    list_scenarios, load_scenario_document,
};
pub(crate) use discovery::{parse_scenario_file, parse_scenario_value};

pub use normalize::{load_and_normalize_scenario_document, normalize_scenario_document};
pub use validation::validate_scenario_document;

use godot_qa_core::results::FailureDetail;
use serde_json::Value;
use std::error::Error;
use std::fmt;
use std::path::Path;

#[derive(Debug, Clone)]
pub struct ScenarioError {
    pub failure: FailureDetail,
}

impl fmt::Display for ScenarioError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}",
            self.failure
                .message
                .as_deref()
                .unwrap_or(self.failure.kind.as_str())
        )
    }
}

impl Error for ScenarioError {}

pub fn is_safe_scenario_name(name: &str) -> bool {
    is_safe_artifact_segment(name.trim())
}

fn validation_failure(path: &Path, message: &str, expected: Value, actual: Value) -> ScenarioError {
    ScenarioError {
        failure: FailureDetail {
            kind: "scenario_validation_failed".to_string(),
            path: Some(path.to_string_lossy().into_owned()),
            message: Some(message.to_string()),
            expected: Some(expected),
            actual: Some(actual),
            ..FailureDetail::default()
        },
    }
}

fn is_non_empty_string(value: &Value) -> bool {
    value
        .as_str()
        .map(|value| !value.trim().is_empty())
        .unwrap_or(false)
}

pub(crate) fn is_safe_artifact_segment(value: &str) -> bool {
    if value == "." || value == ".." || value.is_empty() {
        return false;
    }
    let mut chars = value.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    if !first.is_alphanumeric() {
        return false;
    }
    chars.all(|ch| ch.is_alphanumeric() || matches!(ch, '.' | '_' | '-'))
}

fn python_type_name(value: &Value) -> &'static str {
    match value {
        Value::Null => "NoneType",
        Value::Bool(_) => "bool",
        Value::Number(number) if number.is_i64() || number.is_u64() => "int",
        Value::Number(_) => "float",
        Value::String(_) => "str",
        Value::Array(_) => "list",
        Value::Object(_) => "dict",
    }
}

#[cfg(test)]
mod tests;
