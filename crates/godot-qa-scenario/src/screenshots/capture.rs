use super::output_paths::resolve_input_path;
use super::{analyze_screenshot_file, args_invalid, screenshot_args_invalid};
use godot_qa_core::config::ProjectConfig;
use godot_qa_core::results::{CommandResult, FailureDetail};
use godot_qa_runtime as runtime;
use serde_json::{Map, Value, json};
use std::fs;
use std::path::{Path, PathBuf};

pub fn capture_screenshot(
    config: &ProjectConfig,
    output: &str,
    state_guards: ScreenshotStateGuards,
) -> CommandResult {
    let command = "screenshot capture";
    let output_path = resolve_input_path(&config.project_root, Some(output));
    let Some(output_path) = output_path else {
        return args_invalid(
            command,
            "screenshot output path is required",
            json!("--out <path>"),
            json!(output),
        );
    };

    let guard_report = match evaluate_screenshot_state_guards(config, command, &state_guards) {
        Ok(report) => report,
        Err(result) => return result,
    };
    let capture =
        match capture_live_screenshot(config, command, capture_label_from_path(&output_path)) {
            Ok(capture) => capture,
            Err(result) => return result,
        };
    let _ = capture.has_frame_metadata();
    let quality = match validate_live_screenshot_quality(
        command,
        "screenshot capture failed",
        &capture.screenshot_path,
        Some(&output_path),
    ) {
        Ok(quality) => quality,
        Err(result) => return result,
    };

    if let Some(parent) = output_path.parent()
        && let Err(error) = fs::create_dir_all(parent)
    {
        return capture_artifact_write_failure(command, &output_path, error);
    }
    if let Err(error) = fs::copy(&capture.screenshot_path, &output_path) {
        return capture_artifact_write_failure(command, &output_path, error);
    }

    let mut data = Map::from_iter([
        (
            "output".to_string(),
            json!(output_path.to_string_lossy().into_owned()),
        ),
        (
            "artifacts".to_string(),
            json!({
                "screenshot": capture.screenshot_path,
            }),
        ),
        ("screenshot_quality".to_string(), quality),
    ]);
    if !guard_report.is_null() {
        data.insert("state_guards".to_string(), guard_report);
    }

    CommandResult {
        ok: true,
        command: command.to_string(),
        message: "screenshot captured".to_string(),
        data,
        failures: Vec::new(),
        exit_code: 0,
    }
}

#[derive(Debug, Clone, Default)]
pub struct ScreenshotStateGuards {
    pub require_visible: Vec<String>,
    pub require_hidden: Vec<String>,
    pub require_state: Option<Value>,
}

impl ScreenshotStateGuards {
    pub fn is_empty(&self) -> bool {
        self.require_visible.is_empty()
            && self.require_hidden.is_empty()
            && self.require_state.is_none()
    }

    pub(super) fn requirements_json(&self) -> Value {
        json!({
            "require_visible": self.require_visible,
            "require_hidden": self.require_hidden,
            "require_state": self.require_state,
        })
    }
}

#[derive(Debug, Clone)]
pub struct LiveScreenshotCapture {
    pub screenshot_path: PathBuf,
    pub engine_frame: Option<u64>,
    pub time_msec: Option<u64>,
    pub metadata: Map<String, Value>,
}

impl LiveScreenshotCapture {
    pub fn has_frame_metadata(&self) -> bool {
        self.engine_frame.is_some() || self.time_msec.is_some() || !self.metadata.is_empty()
    }
}

pub(super) fn collect_qa_visual_targets(value: &Value, targets: &mut Vec<Value>) {
    let Some(object) = value.as_object() else {
        return;
    };
    if let Some(qa_id) = object.get("qa_id").and_then(Value::as_str) {
        let qa_id = qa_id.trim();
        if !qa_id.is_empty() {
            let locator = if qa_id.starts_with("qa:") {
                qa_id.to_string()
            } else {
                format!("qa:{qa_id}")
            };
            let mut target = Map::from_iter([
                ("target".to_string(), json!(locator)),
                ("target_id".to_string(), json!(locator)),
                ("qa_id".to_string(), json!(qa_id)),
            ]);
            if let Some(path) = object.get("path").and_then(Value::as_str) {
                target.insert("path".to_string(), json!(path));
            }
            if let Some(visible) = object.get("visible").and_then(Value::as_bool) {
                target.insert("visible".to_string(), json!(visible));
            }
            if let Some(evidence) = object.get("visual_evidence") {
                target.insert("visual_evidence".to_string(), evidence.clone());
            }
            targets.push(Value::Object(target));
        }
    }
    if let Some(children) = object.get("children").and_then(Value::as_array) {
        for child in children {
            collect_qa_visual_targets(child, targets);
        }
    }
    if let Some(root) = object.get("root") {
        collect_qa_visual_targets(root, targets);
    }
}

pub(super) fn evaluate_screenshot_state_guards(
    config: &ProjectConfig,
    command: &str,
    guards: &ScreenshotStateGuards,
) -> Result<Value, CommandResult> {
    if guards.is_empty() {
        return Ok(Value::Null);
    }

    let mut checks = Vec::new();
    for target in guards.require_visible.iter().map(String::as_str) {
        let target = target.trim();
        if target.is_empty() {
            continue;
        }
        let result = runtime::run_live_protocol_command(
            config,
            command,
            runtime::ASSERT_VISIBLE,
            Map::from_iter([("target".to_string(), json!(target))]),
        );
        let check = json!({
            "type": "require_visible",
            "target": target,
            "ok": result.ok,
            "data": result.data,
            "failures": result.failures,
        });
        if !result.ok {
            checks.push(check);
            return Err(screenshot_state_guard_failure(
                config, command, guards, checks,
            ));
        }
        checks.push(check);
    }

    for target in guards.require_hidden.iter().map(String::as_str) {
        let target = target.trim();
        if target.is_empty() {
            continue;
        }
        let result = runtime::run_live_protocol_command(
            config,
            command,
            runtime::ASSERT_HIDDEN,
            Map::from_iter([("target".to_string(), json!(target))]),
        );
        let check = json!({
            "type": "require_hidden",
            "target": target,
            "ok": result.ok,
            "data": result.data,
            "failures": result.failures,
        });
        if !result.ok {
            checks.push(check);
            return Err(screenshot_state_guard_failure(
                config, command, guards, checks,
            ));
        }
        checks.push(check);
    }

    if let Some(require_state) = &guards.require_state {
        let Some(params) = require_state.as_object() else {
            return Err(screenshot_args_invalid(
                command,
                &format!("{command} failed"),
                "screenshot_state_guard_args_invalid",
                "screenshot state guard requires --require-state to be a JSON object",
                require_state.clone(),
                json!("state_matches JSON object"),
            ));
        };
        let result = runtime::run_live_protocol_command(
            config,
            command,
            runtime::ASSERT_STATE_MATCHES,
            params.clone(),
        );
        let check = json!({
            "type": "require_state",
            "ok": result.ok,
            "data": result.data,
            "failures": result.failures,
        });
        if !result.ok {
            checks.push(check);
            return Err(screenshot_state_guard_failure(
                config, command, guards, checks,
            ));
        }
        checks.push(check);
    }

    Ok(json!({
        "ok": true,
        "requirements": guards.requirements_json(),
        "checks": checks,
    }))
}

fn screenshot_state_guard_failure(
    config: &ProjectConfig,
    command: &str,
    guards: &ScreenshotStateGuards,
    checks: Vec<Value>,
) -> CommandResult {
    let context = screenshot_guard_context(config, command);
    let actual = json!({
        "requirements": guards.requirements_json(),
        "checks": checks.clone(),
        "context": context,
    });
    CommandResult {
        ok: false,
        command: command.to_string(),
        message: format!("{command} state guard failed"),
        data: Map::from_iter([
            ("state_guards".to_string(), guards.requirements_json()),
            ("guard_checks".to_string(), json!(checks)),
            ("context".to_string(), context),
        ]),
        failures: vec![FailureDetail {
            kind: "screenshot_state_guard_failed".to_string(),
            command: Some(command.to_string()),
            message: Some(
                "screenshot state guard failed before capture; no new screenshot was accepted"
                    .to_string(),
            ),
            expected: Some(json!("all screenshot state guards pass before capture")),
            actual: Some(actual),
            severity: Some("error".to_string()),
            ..FailureDetail::default()
        }],
        exit_code: 1,
    }
}

fn screenshot_guard_context(config: &ProjectConfig, command: &str) -> Value {
    let status =
        runtime::run_live_protocol_command(config, command, runtime::SESSION_STATUS, Map::new());
    let tree = runtime::run_live_protocol_command(
        config,
        command,
        runtime::TREE_DUMP,
        Map::from_iter([
            ("qa_only".to_string(), json!(true)),
            ("include_metadata".to_string(), json!(true)),
        ]),
    );
    let mut targets = Vec::new();
    if tree.ok {
        collect_qa_visual_targets(&Value::Object(tree.data.clone()), &mut targets);
    }
    json!({
        "status_ok": status.ok,
        "scene": status.data.get("runtime").and_then(|runtime| runtime.get("scene")).cloned(),
        "visible_qa_targets": targets
            .into_iter()
            .filter(|target| target.get("visible").and_then(Value::as_bool).unwrap_or(false))
            .collect::<Vec<_>>(),
        "status_failures": status.failures,
        "tree_ok": tree.ok,
        "tree_failures": tree.failures,
    })
}

pub(super) fn capture_label_from_path(path: &Path) -> String {
    let stem = path
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or("current")
        .trim()
        .to_lowercase();
    if stem.is_empty() {
        return "current".to_string();
    }
    let normalized: String = stem
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || character == '_' {
                character
            } else {
                '-'
            }
        })
        .collect();
    let label = normalized
        .split('-')
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("-");
    if label.is_empty() {
        "current".to_string()
    } else {
        label
    }
}

pub fn capture_live_screenshot(
    config: &ProjectConfig,
    command: &str,
    label: String,
) -> Result<LiveScreenshotCapture, CommandResult> {
    let result = runtime::run_live_screenshot_protocol_command(
        config,
        command,
        runtime::SCREENSHOT_CAPTURE,
        Map::from_iter([("name".to_string(), json!(label))]),
    );
    if !result.ok {
        return Err(result);
    }
    live_screenshot_capture_from_response(command, result.data)
}

pub(super) fn live_screenshot_capture_from_response(
    command: &str,
    mut data: Map<String, Value>,
) -> Result<LiveScreenshotCapture, CommandResult> {
    let Some(artifacts) = data.get("artifacts").and_then(Value::as_object) else {
        return Err(protocol_error(
            command,
            "screenshot capture failed",
            "live screenshot response must include an artifacts object",
            json!("artifacts object with screenshot path"),
            json!("missing"),
        ));
    };
    let Some(screenshot_raw) = artifacts.get("screenshot").and_then(Value::as_str) else {
        return Err(protocol_error(
            command,
            "screenshot capture failed",
            "live screenshot response must include a screenshot artifact path",
            json!("string artifacts.screenshot path"),
            json!("missing"),
        ));
    };
    let screenshot_path = PathBuf::from(screenshot_raw);
    if !screenshot_path.is_file() {
        return Err(CommandResult {
            ok: false,
            command: command.to_string(),
            message: "screenshot capture failed".to_string(),
            data: Map::new(),
            failures: vec![FailureDetail {
                kind: "screenshot_file_missing".to_string(),
                command: Some(command.to_string()),
                path: Some(screenshot_path.to_string_lossy().into_owned()),
                message: Some("live screenshot artifact file was not found".to_string()),
                expected: Some(json!("existing staged screenshot artifact file")),
                actual: Some(json!("missing")),
                ..FailureDetail::default()
            }],
            exit_code: 1,
        });
    }
    let engine_frame = data.get("engine_frame").and_then(json_u64);
    let time_msec = data.get("time_msec").and_then(json_u64);
    data.remove("artifacts");
    Ok(LiveScreenshotCapture {
        screenshot_path,
        engine_frame,
        time_msec,
        metadata: data,
    })
}

pub(super) fn json_u64(value: &Value) -> Option<u64> {
    value
        .as_u64()
        .or_else(|| value.as_i64().and_then(|value| u64::try_from(value).ok()))
}

pub(super) fn validate_live_screenshot_quality(
    command: &str,
    message: &str,
    screenshot_path: &Path,
    output_path: Option<&Path>,
) -> Result<Value, CommandResult> {
    match analyze_screenshot_file(screenshot_path) {
        Ok(quality) => {
            if quality
                .get("low_information")
                .and_then(Value::as_bool)
                .unwrap_or(false)
            {
                Err(screenshot_low_information_result(
                    command,
                    message,
                    screenshot_path,
                    output_path,
                    quality,
                ))
            } else {
                Ok(quality)
            }
        }
        Err(error) => Err(CommandResult {
            ok: false,
            command: command.to_string(),
            message: message.to_string(),
            data: live_screenshot_quality_data(screenshot_path, output_path, None),
            failures: vec![FailureDetail {
                kind: "image_decode_failed".to_string(),
                command: Some(command.to_string()),
                path: Some(screenshot_path.to_string_lossy().into_owned()),
                message: Some(error),
                expected: Some(json!("decodable PNG screenshot artifact")),
                actual: Some(json!("decode_failed")),
                artifacts: Some(Map::from_iter([(
                    "screenshot".to_string(),
                    json!(screenshot_path.to_string_lossy().into_owned()),
                )])),
                ..FailureDetail::default()
            }],
            exit_code: 1,
        }),
    }
}

pub(super) fn screenshot_low_information_result(
    command: &str,
    message: &str,
    screenshot_path: &Path,
    output_path: Option<&Path>,
    quality: Value,
) -> CommandResult {
    CommandResult {
        ok: false,
        command: command.to_string(),
        message: message.to_string(),
        data: live_screenshot_quality_data(screenshot_path, output_path, Some(quality.clone())),
        failures: vec![FailureDetail {
            kind: "screenshot_low_information".to_string(),
            command: Some(command.to_string()),
            path: Some(screenshot_path.to_string_lossy().into_owned()),
            message: Some(
                "screenshot appears blank, too small, or visually low-information".to_string(),
            ),
            expected: Some(json!("screenshot with useful visual evidence")),
            actual: Some(quality),
            severity: Some("error".to_string()),
            artifacts: Some(Map::from_iter([(
                "screenshot".to_string(),
                json!(screenshot_path.to_string_lossy().into_owned()),
            )])),
            ..FailureDetail::default()
        }],
        exit_code: 1,
    }
}

fn live_screenshot_quality_data(
    screenshot_path: &Path,
    output_path: Option<&Path>,
    quality: Option<Value>,
) -> Map<String, Value> {
    let mut data = Map::from_iter([(
        "artifacts".to_string(),
        json!({
            "screenshot": screenshot_path.to_string_lossy().into_owned(),
        }),
    )]);
    if let Some(output_path) = output_path {
        data.insert(
            "output".to_string(),
            json!(output_path.to_string_lossy().into_owned()),
        );
    }
    if let Some(quality) = quality {
        data.insert("screenshot_quality".to_string(), quality);
    }
    data
}

pub(super) fn protocol_error(
    command: &str,
    message: &str,
    failure_message: &str,
    expected: Value,
    actual: Value,
) -> CommandResult {
    CommandResult {
        ok: false,
        command: command.to_string(),
        message: message.to_string(),
        data: Map::new(),
        failures: vec![FailureDetail {
            kind: "protocol_error".to_string(),
            command: Some(command.to_string()),
            message: Some(failure_message.to_string()),
            expected: Some(expected),
            actual: Some(actual),
            ..FailureDetail::default()
        }],
        exit_code: 1,
    }
}

fn capture_artifact_write_failure(
    command: &str,
    path: &Path,
    error: std::io::Error,
) -> CommandResult {
    CommandResult {
        ok: false,
        command: command.to_string(),
        message: "screenshot capture failed".to_string(),
        data: Map::new(),
        failures: vec![FailureDetail {
            kind: "artifact_write_failed".to_string(),
            command: Some(command.to_string()),
            path: Some(path.to_string_lossy().into_owned()),
            message: Some(error.to_string()),
            expected: Some(json!("writable output path for copied screenshot artifact")),
            actual: Some(json!("OSError")),
            ..FailureDetail::default()
        }],
        exit_code: 1,
    }
}
