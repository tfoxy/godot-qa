use godot_qa_core::results::FailureDetail;
use serde_json::{Map, Value, json};

const PANEL_TYPES: &[&str] = &[
    "Panel",
    "PanelContainer",
    "ColorRect",
    "MarginContainer",
    "CenterContainer",
    "VBoxContainer",
    "HBoxContainer",
];

#[derive(Debug, Clone)]
pub struct LayoutAuditResult {
    pub ok: bool,
    pub mode: String,
    pub viewport: Vec<i64>,
    pub failures: Vec<FailureDetail>,
}

pub fn parse_viewports(raw: &str, command: &str) -> Result<Vec<Vec<i64>>, FailureDetail> {
    let mut viewports = Vec::new();
    for token in raw
        .split(',')
        .map(str::trim)
        .filter(|token| !token.is_empty())
    {
        let Some((width, height)) = token.split_once('x') else {
            return Err(invalid_viewports_failure(command, token, raw, false));
        };
        let (Ok(width), Ok(height)) = (width.parse::<i64>(), height.parse::<i64>()) else {
            return Err(invalid_viewports_failure(command, token, raw, false));
        };
        if width <= 0 || height <= 0 {
            return Err(invalid_viewports_failure(command, token, raw, false));
        }
        viewports.push(vec![width, height]);
    }
    if viewports.is_empty() {
        return Err(invalid_viewports_failure(command, raw, raw, true));
    }
    Ok(viewports)
}

pub fn audit_layout(
    snapshot_context: &Value,
    viewport_name: Option<Vec<i64>>,
    mode: &str,
    artifacts: Option<Map<String, Value>>,
) -> LayoutAuditResult {
    let (viewport, named_viewport) = viewport_size(snapshot_context, viewport_name);
    let mut failures = Vec::new();

    let interactive_controls = visible_interactive_controls(snapshot_context);
    for control in &interactive_controls {
        let Some(rect) = as_rect(control.get("global_rect")) else {
            continue;
        };
        if !rect_inside_viewport(&rect, &viewport) {
            failures.push(layout_failure(
                "outside_viewport",
                "visible interactive control extends outside the viewport",
                Some(json!([0.0, 0.0, viewport[0], viewport[1]])),
                Some(json!(rect.clone())),
                Some(target_name(control)),
                artifacts.clone(),
                Some(named_viewport.clone()),
                None,
                None,
                Some(rect),
                None,
                None,
            ));
        }
    }

    if let Some(controls) = snapshot_context
        .get("ui")
        .and_then(|ui| ui.get("controls"))
        .and_then(Value::as_array)
    {
        for control in controls.iter().filter_map(Value::as_object) {
            if !is_visible(control) {
                continue;
            }
            let Some(rect) = as_rect(control.get("global_rect")) else {
                continue;
            };
            let Some(control_type) = control.get("type").and_then(Value::as_str) else {
                continue;
            };
            if !PANEL_TYPES.contains(&control_type) {
                continue;
            }
            if rect[2] <= viewport[0] && rect[3] <= viewport[1] {
                continue;
            }
            failures.push(layout_failure(
                "panel_larger_than_viewport",
                "visible panel is larger than the viewport",
                Some(json!([viewport[0], viewport[1]])),
                Some(json!([rect[2], rect[3]])),
                Some(target_name(control)),
                artifacts.clone(),
                Some(named_viewport.clone()),
                None,
                None,
                Some(rect),
                None,
                None,
            ));
        }
    }

    for (index, left) in interactive_controls.iter().enumerate() {
        let Some(left_rect) = as_rect(left.get("global_rect")) else {
            continue;
        };
        let Some(left_path) = left.get("path").and_then(Value::as_str) else {
            continue;
        };
        for right in interactive_controls.iter().skip(index + 1) {
            let Some(right_rect) = as_rect(right.get("global_rect")) else {
                continue;
            };
            let Some(right_path) = right.get("path").and_then(Value::as_str) else {
                continue;
            };
            if is_ancestor_path(left_path, right_path) || is_ancestor_path(right_path, left_path) {
                continue;
            }
            if overlap_area(&left_rect, &right_rect) <= 0.0 {
                continue;
            }
            failures.push(layout_failure(
                "interactive_overlap",
                "visible interactive controls overlap with positive area",
                None,
                None,
                Some(target_name(left)),
                artifacts.clone(),
                Some(named_viewport.clone()),
                Some(target_name(left)),
                Some(target_name(right)),
                None,
                Some(left_rect.clone()),
                Some(right_rect.clone()),
            ));
        }
    }

    let focus_owner = snapshot_context
        .get("focus")
        .and_then(|focus| focus.get("focus_owner"))
        .cloned()
        .unwrap_or_else(|| json!(""));
    if !visible_focusable_controls(snapshot_context).is_empty()
        && !focus_owner
            .as_str()
            .is_some_and(|value| !value.trim().is_empty())
    {
        failures.push(layout_failure(
            "missing_initial_focus",
            "visible focusable controls exist but no initial focus owner was reported",
            Some(json!("one visible focusable control to own focus")),
            Some(if focus_owner.is_null() {
                json!("")
            } else {
                focus_owner
            }),
            Some("runtime".to_string()),
            artifacts,
            Some(named_viewport.clone()),
            None,
            None,
            None,
            None,
            None,
        ));
    }

    LayoutAuditResult {
        ok: failures.is_empty(),
        mode: mode.to_string(),
        viewport: named_viewport,
        failures,
    }
}

fn invalid_viewports_failure(command: &str, actual: &str, raw: &str, empty: bool) -> FailureDetail {
    FailureDetail {
        kind: "invalid_viewports".to_string(),
        command: Some(command.to_string()),
        message: Some(if empty {
            "at least one viewport is required".to_string()
        } else {
            "viewport entries must use WIDTHxHEIGHT".to_string()
        }),
        expected: Some(json!("comma-separated WIDTHxHEIGHT entries")),
        actual: Some(json!(if empty { raw } else { actual })),
        ..FailureDetail::default()
    }
}

fn layout_failure(
    kind: &str,
    message: &str,
    expected: Option<Value>,
    actual: Option<Value>,
    target: Option<String>,
    artifacts: Option<Map<String, Value>>,
    viewport: Option<Vec<i64>>,
    a: Option<String>,
    b: Option<String>,
    rect: Option<Vec<f64>>,
    rect_a: Option<Vec<f64>>,
    rect_b: Option<Vec<f64>>,
) -> FailureDetail {
    FailureDetail {
        kind: kind.to_string(),
        message: Some(message.to_string()),
        expected,
        actual,
        target,
        severity: Some("error".to_string()),
        artifacts,
        viewport,
        a,
        b,
        rect,
        rect_a,
        rect_b,
        ..FailureDetail::default()
    }
}

fn viewport_size(
    snapshot_context: &Value,
    viewport_name: Option<Vec<i64>>,
) -> (Vec<f64>, Vec<i64>) {
    let numeric = snapshot_context
        .get("ui")
        .and_then(|ui| ui.get("viewport"))
        .and_then(Value::as_array)
        .filter(|viewport| viewport.len() == 2)
        .and_then(|viewport| {
            Some(vec![
                viewport.first()?.as_f64()?,
                viewport.get(1)?.as_f64()?,
            ])
        })
        .or_else(|| {
            viewport_name
                .as_ref()
                .map(|viewport| vec![viewport[0] as f64, viewport[1] as f64])
        })
        .unwrap_or_else(|| vec![0.0, 0.0]);
    let named = viewport_name.unwrap_or_else(|| vec![numeric[0] as i64, numeric[1] as i64]);
    (numeric, named)
}

fn as_rect(value: Option<&Value>) -> Option<Vec<f64>> {
    let rect = value?.as_array()?;
    if rect.len() != 4 {
        return None;
    }
    rect.iter().map(Value::as_f64).collect()
}

fn target_name(control: &Map<String, Value>) -> String {
    if let Some(qa_id) = control.get("qa_id").and_then(Value::as_str) {
        if !qa_id.is_empty() {
            return format!("qa:{qa_id}");
        }
    }
    if let Some(path) = control.get("path").and_then(Value::as_str) {
        if !path.is_empty() {
            return format!("path:{path}");
        }
    }
    "runtime".to_string()
}

fn is_visible(control: &Map<String, Value>) -> bool {
    control.get("visible").and_then(Value::as_bool) == Some(true)
}

fn is_focusable(control: &Map<String, Value>) -> bool {
    matches!(
        control.get("focus_mode").and_then(Value::as_str),
        Some("all" | "click" | "accessibility")
    )
}

fn is_interactive(control: &Map<String, Value>) -> bool {
    if control.get("disabled").and_then(Value::as_bool) == Some(true) {
        return false;
    }
    if is_focusable(control) {
        return true;
    }
    matches!(
        control.get("type").and_then(Value::as_str),
        Some(
            "Button"
                | "BaseButton"
                | "CheckBox"
                | "CheckButton"
                | "LineEdit"
                | "TextEdit"
                | "OptionButton"
                | "MenuButton"
                | "Slider"
                | "HSlider"
                | "VSlider"
                | "SpinBox"
                | "Range"
        )
    )
}

fn visible_interactive_controls(snapshot_context: &Value) -> Vec<Map<String, Value>> {
    let controls = snapshot_context
        .get("ui")
        .and_then(|ui| ui.get("controls"))
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let interactive: Vec<Map<String, Value>> = controls
        .into_iter()
        .filter_map(|value| value.as_object().cloned())
        .filter(|control| is_visible(control))
        .filter(|control| as_rect(control.get("global_rect")).is_some())
        .filter(is_interactive)
        .collect();
    let interactive_paths: Vec<String> = interactive
        .iter()
        .filter_map(|control| {
            control
                .get("path")
                .and_then(Value::as_str)
                .map(str::to_owned)
        })
        .collect();
    interactive
        .into_iter()
        .filter(|control| {
            let Some(path) = control.get("path").and_then(Value::as_str) else {
                return true;
            };
            !interactive_paths
                .iter()
                .any(|other| other != path && other.starts_with(&format!("{path}/")))
        })
        .collect()
}

fn visible_focusable_controls(snapshot_context: &Value) -> Vec<Map<String, Value>> {
    snapshot_context
        .get("ui")
        .and_then(|ui| ui.get("controls"))
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .filter_map(|value| value.as_object().cloned())
        .filter(|control| is_visible(control))
        .filter(|control| as_rect(control.get("global_rect")).is_some())
        .filter(|control| is_focusable(control))
        .filter(|control| control.get("disabled").and_then(Value::as_bool) != Some(true))
        .collect()
}

fn rect_inside_viewport(rect: &[f64], viewport: &[f64]) -> bool {
    rect[0] >= 0.0
        && rect[1] >= 0.0
        && rect[0] + rect[2] <= viewport[0]
        && rect[1] + rect[3] <= viewport[1]
}

fn overlap_area(a: &[f64], b: &[f64]) -> f64 {
    let left = a[0].max(b[0]);
    let top = a[1].max(b[1]);
    let right = (a[0] + a[2]).min(b[0] + b[2]);
    let bottom = (a[1] + a[3]).min(b[1] + b[3]);
    let width = right - left;
    let height = bottom - top;
    if width <= 0.0 || height <= 0.0 {
        0.0
    } else {
        width * height
    }
}

fn is_ancestor_path(a: &str, b: &str) -> bool {
    b.starts_with(&format!("{a}/"))
}
