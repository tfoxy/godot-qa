//! Shared YAML parsing that produces the same `serde_json::Value` document shape
//! the rest of the workspace consumes.
//!
//! The workspace historically shipped a hand-rolled YAML subset parser that
//! silently mis-parsed inline arrays/maps and grouped mapping entries. This
//! module routes YAML through the real `yaml_serde` (libyaml) parser instead,
//! which resolves anchors/aliases and reports actionable line/column errors,
//! then converts the parsed value into a plain `serde_json::Value`.

use serde_json::{Map, Number as JsonNumber, Value as JsonValue};
use yaml_serde::Value as YamlValue;

/// A YAML parse or conversion error with an actionable message and, when the
/// underlying parser can locate it, the 1-based line and column of the problem.
#[derive(Debug, Clone)]
pub struct YamlParseError {
    /// The underlying parser/conversion message.
    pub message: String,
    /// 1-based line of the problem when the parser could locate it.
    pub line: Option<usize>,
    /// 1-based column of the problem when the parser could locate it.
    pub column: Option<usize>,
}

impl YamlParseError {
    /// A human-readable one-line description including the location when known.
    pub fn detail(&self) -> String {
        match (self.line, self.column) {
            (Some(line), Some(column)) => {
                format!("{} (line {line}, column {column})", self.message)
            }
            _ => self.message.clone(),
        }
    }

    /// A compact machine-friendly location summary, e.g. `line 4, column 3`.
    pub fn location_summary(&self) -> String {
        match (self.line, self.column) {
            (Some(line), Some(column)) => format!("line {line}, column {column}"),
            _ => "unknown location".to_string(),
        }
    }
}

impl std::fmt::Display for YamlParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.detail())
    }
}

impl std::error::Error for YamlParseError {}

/// Parse a YAML document into a `serde_json::Value`, resolving anchors and
/// aliases. Fails with location-aware errors on malformed YAML, non-scalar
/// mapping keys, or numbers that cannot be represented in JSON.
pub fn parse_yaml_document(raw: &str) -> Result<JsonValue, YamlParseError> {
    let mut value: YamlValue = yaml_serde::from_str(raw).map_err(map_yaml_error)?;
    // Resolve `<<` merge keys so anchor/alias field reuse (the classic YAML
    // boilerplate-killer) expands into the mapping before conversion.
    value.apply_merge().map_err(map_yaml_error)?;
    yaml_value_to_json(value).map_err(|message| YamlParseError {
        message,
        line: None,
        column: None,
    })
}

fn map_yaml_error(error: yaml_serde::Error) -> YamlParseError {
    let location = error.location();
    YamlParseError {
        message: error.to_string(),
        line: location.as_ref().map(yaml_serde::Location::line),
        column: location.as_ref().map(yaml_serde::Location::column),
    }
}

/// Convert an already-parsed `yaml_serde::Value` into a `serde_json::Value`.
pub fn yaml_value_to_json(value: YamlValue) -> Result<JsonValue, String> {
    match value {
        YamlValue::Null => Ok(JsonValue::Null),
        YamlValue::Bool(boolean) => Ok(JsonValue::Bool(boolean)),
        YamlValue::Number(number) => yaml_number_to_json(&number),
        YamlValue::String(text) => Ok(JsonValue::String(text)),
        YamlValue::Sequence(items) => {
            let mut converted = Vec::with_capacity(items.len());
            for item in items {
                converted.push(yaml_value_to_json(item)?);
            }
            Ok(JsonValue::Array(converted))
        }
        YamlValue::Mapping(mapping) => {
            let mut object = Map::new();
            for (key, value) in mapping {
                object.insert(yaml_key_to_string(key)?, yaml_value_to_json(value)?);
            }
            Ok(JsonValue::Object(object))
        }
        // `!Tag value` syntax: keep the tagged payload so documents that carry
        // an incidental tag still convert to plain JSON.
        YamlValue::Tagged(tagged) => yaml_value_to_json(tagged.value),
    }
}

fn yaml_number_to_json(number: &yaml_serde::Number) -> Result<JsonValue, String> {
    if let Some(value) = number.as_i64() {
        Ok(JsonValue::Number(value.into()))
    } else if let Some(value) = number.as_u64() {
        Ok(JsonValue::Number(value.into()))
    } else if let Some(value) = number.as_f64() {
        JsonNumber::from_f64(value)
            .map(JsonValue::Number)
            .ok_or_else(|| format!("YAML number {value} cannot be represented in JSON"))
    } else {
        Err("unsupported YAML number".to_string())
    }
}

fn yaml_key_to_string(key: YamlValue) -> Result<String, String> {
    match key {
        YamlValue::String(text) => Ok(text),
        YamlValue::Bool(boolean) => Ok(boolean.to_string()),
        YamlValue::Number(number) => Ok(number.to_string()),
        YamlValue::Null => Err("YAML mapping keys must not be null".to_string()),
        YamlValue::Sequence(_) | YamlValue::Mapping(_) | YamlValue::Tagged(_) => {
            Err("YAML mapping keys must be scalars".to_string())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::parse_yaml_document;
    use serde_json::json;

    #[test]
    fn parses_grouped_mapping_with_nested_list_of_maps() {
        let document = parse_yaml_document(
            &[
                "assert:",
                "  - state_matches:",
                "      target: qa:main_menu",
                "      source: method",
                "      method: get_raft_motion_qa_state",
                "      matches:",
                "        - {path: mode, operator: equals, expected: steady}",
                "        - {path: speed, operator: approx_equals, expected: 4.2, epsilon: 0.001}",
                "        - path: distance",
                "          operator: greater_than",
                "          expected: 12.0",
            ]
            .join("\n"),
        )
        .expect("grouped state_matches should parse");

        assert_eq!(
            document,
            json!({
                "assert": [{
                    "state_matches": {
                        "target": "qa:main_menu",
                        "source": "method",
                        "method": "get_raft_motion_qa_state",
                        "matches": [
                            {"path": "mode", "operator": "equals", "expected": "steady"},
                            {"path": "speed", "operator": "approx_equals", "expected": 4.2, "epsilon": 0.001},
                            {"path": "distance", "operator": "greater_than", "expected": 12.0},
                        ]
                    }
                }]
            })
        );
    }

    #[test]
    fn parses_inline_arrays_and_maps() {
        let document = parse_yaml_document(
            &[
                "metadata_keys: [qa_id, role]",
                "text_contains: {target: qa:status, expected: ready}",
                "offset: {x: 3.0, y: 0.0}",
            ]
            .join("\n"),
        )
        .expect("inline arrays/maps should parse");

        assert_eq!(document["metadata_keys"], json!(["qa_id", "role"]));
        assert_eq!(
            document["text_contains"],
            json!({"target": "qa:status", "expected": "ready"})
        );
        assert_eq!(document["offset"], json!({"x": 3.0, "y": 0.0}));
    }

    #[test]
    fn resolves_anchors_and_aliases() {
        let document = parse_yaml_document(
            &[
                "defaults: &defaults",
                "  window_frames: 12",
                "  sample_every_frames: 1",
                "assert:",
                "  - never:",
                "      target: qa:warning",
                "      forbidden: DESYNC",
                "      <<: *defaults",
            ]
            .join("\n"),
        )
        .expect("anchors/aliases should resolve");

        assert_eq!(
            document["assert"][0]["never"],
            json!({
                "target": "qa:warning",
                "forbidden": "DESYNC",
                "window_frames": 12,
                "sample_every_frames": 1,
            })
        );
    }

    #[test]
    fn reports_line_and_column_on_malformed_yaml() {
        let error = parse_yaml_document("name: ok\nsteps:\n  - foo: [1, 2\n").unwrap_err();
        assert!(error.line.is_some(), "expected a line in {error:?}");
        assert!(!error.detail().is_empty());
    }
}
