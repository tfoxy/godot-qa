use godot_qa_core::project_file::read_project_godot_cached;
use godot_qa_core::results::FailureDetail;
use serde_json::{Value, json};
use std::path::Path;

const DEFAULT_FIXED_FPS: i64 = 60;
const MAX_SAFE_FIXED_FPS: i64 = 100_000;
const PROJECT_MAX_FPS_KEY: &str = "application/run/max_fps";
const PROJECT_PHYSICS_TICKS_KEY: &str = "physics/common/physics_ticks_per_second";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FramePacing {
    pub fixed_fps: i64,
    pub source: String,
    pub project_setting: Option<String>,
}

impl FramePacing {
    pub fn generated_godot_args(&self) -> Vec<String> {
        vec![
            "--fixed-fps".to_string(),
            self.fixed_fps.to_string(),
            "--disable-vsync".to_string(),
        ]
    }

    pub fn to_json(&self) -> Value {
        json!({
            "mode": "fastest",
            "fixed_fps": self.fixed_fps,
            "source": self.source,
            "project_setting": self.project_setting,
            "generated_godot_args": self.generated_godot_args(),
        })
    }
}

pub fn scenario_frame_pacing(
    scenario: &Value,
    project_root: &Path,
) -> Result<Option<FramePacing>, FailureDetail> {
    if scenario.get("time_control").is_some() {
        return Ok(None);
    }
    if let Some(fixed_fps) = scenario
        .get("determinism")
        .and_then(Value::as_object)
        .and_then(|determinism| determinism.get("fixed_fps"))
    {
        return Ok(Some(FramePacing {
            fixed_fps: validate_fixed_fps(
                fixed_fps,
                "scenario_validation_failed",
                "determinism.fixed_fps",
                None,
            )?,
            source: "scenario".to_string(),
            project_setting: None,
        }));
    }
    Ok(Some(project_default_frame_pacing(project_root)))
}

pub fn manifest_frame_pacing(
    raw_fixed_fps: Option<&Value>,
    has_time_control: bool,
    project_root: &Path,
    manifest_path: &Path,
    target: &str,
) -> Result<Option<FramePacing>, FailureDetail> {
    if has_time_control {
        if raw_fixed_fps.is_some() {
            return Err(time_control_fixed_fps_conflict_failure(
                "test_manifest_invalid",
                Some(manifest_path),
                Some(target.to_string()),
            ));
        }
        return Ok(None);
    }
    if let Some(fixed_fps) = raw_fixed_fps {
        return Ok(Some(FramePacing {
            fixed_fps: validate_fixed_fps(
                fixed_fps,
                "test_manifest_invalid",
                "fixed_fps",
                Some((manifest_path, target)),
            )?,
            source: "manifest".to_string(),
            project_setting: None,
        }));
    }
    Ok(Some(project_default_frame_pacing(project_root)))
}

pub fn validate_fixed_fps_for_scenario(value: &Value, path: &Path) -> Result<(), FailureDetail> {
    validate_fixed_fps(
        value,
        "scenario_validation_failed",
        "determinism.fixed_fps",
        Some((path, "")),
    )
    .map(|_| ())
}

pub fn project_default_frame_pacing(project_root: &Path) -> FramePacing {
    let settings = parse_project_godot_settings(&project_root.join("project.godot"));
    if let Some(value) = settings
        .iter()
        .find(|setting| setting.key == PROJECT_MAX_FPS_KEY)
        .and_then(|setting| setting.positive_integer())
    {
        return FramePacing {
            fixed_fps: value,
            source: "project".to_string(),
            project_setting: Some(PROJECT_MAX_FPS_KEY.to_string()),
        };
    }
    if let Some(value) = settings
        .iter()
        .find(|setting| setting.key == PROJECT_PHYSICS_TICKS_KEY)
        .and_then(|setting| setting.positive_integer())
    {
        return FramePacing {
            fixed_fps: value,
            source: "project".to_string(),
            project_setting: Some(PROJECT_PHYSICS_TICKS_KEY.to_string()),
        };
    }
    FramePacing {
        fixed_fps: DEFAULT_FIXED_FPS,
        source: "godot_default".to_string(),
        project_setting: None,
    }
}

pub fn time_control_fixed_fps_conflict_failure(
    failure_kind: &str,
    path: Option<&Path>,
    target: Option<String>,
) -> FailureDetail {
    FailureDetail {
        kind: failure_kind.to_string(),
        path: path.map(|path| path.to_string_lossy().into_owned()),
        target,
        message: Some("fixed_fps cannot be combined with time_control".to_string()),
        expected: Some(json!("either fixed_fps or time_control, not both")),
        actual: Some(json!("fixed_fps and time_control")),
        ..FailureDetail::default()
    }
}

fn validate_fixed_fps(
    value: &Value,
    failure_kind: &str,
    field_path: &str,
    path_and_target: Option<(&Path, &str)>,
) -> Result<i64, FailureDetail> {
    let Some(fixed_fps) = value.as_i64() else {
        return Err(fixed_fps_failure(
            failure_kind,
            field_path,
            path_and_target,
            json!("positive integer"),
            json!(json_type_name(value)),
        ));
    };
    if !(1..=MAX_SAFE_FIXED_FPS).contains(&fixed_fps) {
        return Err(fixed_fps_failure(
            failure_kind,
            field_path,
            path_and_target,
            json!(format!("integer from 1 to {MAX_SAFE_FIXED_FPS}")),
            value.clone(),
        ));
    }
    Ok(fixed_fps)
}

fn fixed_fps_failure(
    failure_kind: &str,
    field_path: &str,
    path_and_target: Option<(&Path, &str)>,
    expected: Value,
    actual: Value,
) -> FailureDetail {
    let (path, target) = path_and_target
        .map(|(path, target)| {
            (
                Some(path.to_string_lossy().into_owned()),
                if target.is_empty() {
                    None
                } else {
                    Some(target.to_string())
                },
            )
        })
        .unwrap_or((None, None));
    FailureDetail {
        kind: failure_kind.to_string(),
        path,
        target,
        message: Some(format!("{field_path} must be a positive integer")),
        expected: Some(expected),
        actual: Some(actual),
        ..FailureDetail::default()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ProjectSetting {
    key: String,
    value: String,
}

impl ProjectSetting {
    fn positive_integer(&self) -> Option<i64> {
        let value = self.value.trim().trim_matches('"').parse::<i64>().ok()?;
        (value > 0).then_some(value)
    }
}

fn parse_project_godot_settings(path: &Path) -> Vec<ProjectSetting> {
    let Some(raw) = read_project_godot_cached(path) else {
        return Vec::new();
    };
    let mut section = String::new();
    let mut settings = Vec::new();
    for line in raw.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with(';') || trimmed.starts_with('#') {
            continue;
        }
        if trimmed.starts_with('[') && trimmed.ends_with(']') {
            section = trimmed[1..trimmed.len() - 1].trim().to_string();
            continue;
        }
        let Some((key, value)) = trimmed.split_once('=') else {
            continue;
        };
        let key = key.trim();
        if key.is_empty() {
            continue;
        }
        let full_key = if section.is_empty() {
            key.to_string()
        } else {
            format!("{section}/{key}")
        };
        settings.push(ProjectSetting {
            key: full_key,
            value: value.trim().to_string(),
        });
    }
    settings
}

fn json_type_name(value: &Value) -> &'static str {
    match value {
        Value::Null => "NoneType",
        Value::Bool(_) => "bool",
        Value::Number(_) => "number",
        Value::String(_) => "str",
        Value::Array(_) => "list",
        Value::Object(_) => "dict",
    }
}

#[cfg(test)]
mod tests {
    use super::{project_default_frame_pacing, scenario_frame_pacing};
    use serde_json::json;
    use std::fs;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_project(name: &str, project_godot: &str) -> PathBuf {
        let mut path = std::env::temp_dir();
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        path.push(format!("godot-qa-{name}-{unique}"));
        fs::create_dir_all(&path).unwrap();
        fs::write(path.join("project.godot"), project_godot).unwrap();
        path
    }

    #[test]
    fn derives_from_project_max_fps() {
        let project = temp_project(
            "max-fps",
            "[application]\nrun/max_fps=144\n[physics]\ncommon/physics_ticks_per_second=30\n",
        );
        let pacing = project_default_frame_pacing(&project);
        assert_eq!(pacing.fixed_fps, 144);
        assert_eq!(pacing.source, "project");
        assert_eq!(
            pacing.project_setting.as_deref(),
            Some("application/run/max_fps")
        );
    }

    #[test]
    fn derives_from_project_physics_ticks_when_max_fps_is_zero() {
        let project = temp_project(
            "physics-fps",
            "[application]\nrun/max_fps=0\n[physics]\ncommon/physics_ticks_per_second=120\n",
        );
        let pacing = project_default_frame_pacing(&project);
        assert_eq!(pacing.fixed_fps, 120);
        assert_eq!(
            pacing.project_setting.as_deref(),
            Some("physics/common/physics_ticks_per_second")
        );
    }

    #[test]
    fn falls_back_to_godot_default() {
        let project = temp_project("default-fps", "[application]\nconfig/name=\"sample\"\n");
        let pacing = project_default_frame_pacing(&project);
        assert_eq!(pacing.fixed_fps, 60);
        assert_eq!(pacing.source, "godot_default");
        assert_eq!(pacing.project_setting, None);
    }

    #[test]
    fn scenario_declared_fixed_fps_wins() {
        let project = temp_project("scenario-fps", "[application]\nrun/max_fps=144\n");
        let scenario = json!({
            "determinism": {"fixed_fps": 30},
        });
        let pacing = scenario_frame_pacing(&scenario, &project).unwrap().unwrap();
        assert_eq!(pacing.fixed_fps, 30);
        assert_eq!(pacing.source, "scenario");
        assert_eq!(pacing.project_setting, None);
    }

    #[test]
    fn time_control_disables_frame_pacing() {
        let project = temp_project("time-control", "[application]\nrun/max_fps=144\n");
        let scenario = json!({
            "time_control": {"time_scale": 1.0},
        });
        assert_eq!(scenario_frame_pacing(&scenario, &project).unwrap(), None);
    }
}
