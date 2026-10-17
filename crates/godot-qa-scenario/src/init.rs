use crate::scenario_doc;
use godot_qa_core::config::ProjectConfig;
use godot_qa_core::results::{CommandResult, FailureDetail};
use serde_json::{Map, Value, json};
use std::fs;
use std::path::{Path, PathBuf};

pub fn run_scenario_init(
    config: &ProjectConfig,
    name: &str,
    scene: &str,
    viewport_raw: &str,
    out: Option<&str>,
) -> CommandResult {
    let command = "scenario init";
    let name = name.trim();
    let scene = scene.trim();

    if !scenario_doc::is_safe_scenario_name(name) {
        return failure_result(
            command,
            vec![FailureDetail {
                kind: "scenario_validation_failed".to_string(),
                command: Some(command.to_string()),
                message: Some(
                    "scenario field 'name' must be a safe artifact path segment".to_string(),
                ),
                expected: Some(json!("letters, numbers, dot, underscore, or hyphen")),
                actual: Some(json!(name)),
                ..FailureDetail::default()
            }],
            Map::new(),
        );
    }

    if !scene.starts_with("res://") || scene == "res://" {
        return failure_result(
            command,
            vec![FailureDetail {
                kind: "scenario_validation_failed".to_string(),
                command: Some(command.to_string()),
                message: Some("scenario field 'scene' must be a res:// path".to_string()),
                expected: Some(json!("res:// scene path")),
                actual: Some(json!(scene)),
                ..FailureDetail::default()
            }],
            Map::new(),
        );
    }

    let Some(viewport) = parse_viewport(viewport_raw) else {
        return failure_result(
            command,
            vec![FailureDetail {
                kind: "invalid_viewport".to_string(),
                command: Some(command.to_string()),
                message: Some("viewport must use WIDTHxHEIGHT".to_string()),
                expected: Some(json!("positive WIDTHxHEIGHT viewport string")),
                actual: Some(json!(viewport_raw)),
                ..FailureDetail::default()
            }],
            Map::new(),
        );
    };

    let template = scenario_template(name, scene, &viewport);
    let mut data = Map::from_iter([
        ("scenario".to_string(), json!(name)),
        ("scene".to_string(), json!(scene)),
        ("viewport".to_string(), json!(viewport)),
        ("template".to_string(), template.clone()),
    ]);

    if let Some(out) = out {
        let output_path = resolve_output_path(config, out);
        if output_path.exists() {
            return failure_result(
                command,
                vec![FailureDetail {
                    kind: "scenario_output_exists".to_string(),
                    command: Some(command.to_string()),
                    path: Some(output_path.to_string_lossy().into_owned()),
                    message: Some("scenario init output already exists".to_string()),
                    expected: Some(json!("missing output path")),
                    actual: Some(json!("existing_file")),
                    ..FailureDetail::default()
                }],
                data,
            );
        }
        if let Err(error) = write_template(&output_path, &template) {
            return failure_result(
                command,
                vec![FailureDetail {
                    kind: "artifact_write_failed".to_string(),
                    command: Some(command.to_string()),
                    path: Some(output_path.to_string_lossy().into_owned()),
                    message: Some(error.to_string()),
                    expected: Some(json!("scenario template written to disk")),
                    actual: Some(json!("OSError")),
                    ..FailureDetail::default()
                }],
                data,
            );
        }
        data.insert("scenario_path".to_string(), json!(output_path));
    }

    CommandResult {
        ok: true,
        command: command.to_string(),
        message: "scenario template created".to_string(),
        data,
        failures: Vec::new(),
        exit_code: 0,
    }
}

fn scenario_template(name: &str, scene: &str, viewport: &[i64]) -> Value {
    json!({
        "name": name,
        "scene": scene,
        "viewport": {
            "width": viewport[0],
            "height": viewport[1],
        },
        "steps": [
            {"wait_frames": 5},
            {"snapshot": "initial"}
        ],
        "assert": [
            {"no_errors": true}
        ]
    })
}

fn parse_viewport(value: &str) -> Option<Vec<i64>> {
    let normalized = value.to_ascii_lowercase();
    let (width, height) = normalized.split_once('x')?;
    let width = width.parse::<i64>().ok()?;
    let height = height.parse::<i64>().ok()?;
    if width <= 0 || height <= 0 {
        return None;
    }
    Some(vec![width, height])
}

fn resolve_output_path(config: &ProjectConfig, out: &str) -> PathBuf {
    let path = PathBuf::from(out);
    if path.is_absolute() {
        path
    } else {
        config.project_root.join(path)
    }
}

fn write_template(path: &Path, template: &Value) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let content = serde_json::to_string_pretty(template).map_err(std::io::Error::other)?;
    fs::write(path, content)
}

fn failure_result(
    command: &str,
    failures: Vec<FailureDetail>,
    data: Map<String, Value>,
) -> CommandResult {
    CommandResult {
        ok: false,
        command: command.to_string(),
        message: "scenario template failed".to_string(),
        data,
        failures,
        exit_code: 1,
    }
}
