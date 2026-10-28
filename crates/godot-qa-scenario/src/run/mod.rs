use crate::artifacts;
use crate::artifacts::write_json;
use crate::assertions;
use crate::audio;
use crate::frame_pacing::scenario_frame_pacing;
use crate::layout;
use crate::screenshots;
use crate::snapshot_context;
use crate::visual_risk;
use godot_qa_core::config::ProjectConfig;
use godot_qa_core::profile;
use godot_qa_core::project_file::read_project_godot_cached;
use godot_qa_core::results::{CommandResult, FailureDetail};
use godot_qa_process::{self, GodotRunResult};
use godot_qa_runtime;
use serde_json::{Map, Value, json};
use std::fs;
use std::io;
use std::path::{Component, Path, PathBuf};
use std::time::{Duration, Instant};

const PROFILE_MAX_FRAME_SAMPLES: usize = 600;
const PROFILE_TOP_SCRIPT_METHODS: usize = 10;
const DEFAULT_SLOW_SUMMARY_THRESHOLD_MS: u64 = 1_000;
const SLOW_SUMMARY_LIMIT: usize = 10;
// When the engine enforces the declared timeout as a frame budget, the process
// kill deadline is only a hang backstop: slack absorbs host load (frames are
// load-independent, wall clocks are not) and the cap bounds true hangs.
const SCENARIO_FRAME_BUDGET_WALL_SLACK: f64 = 3.0;
const SCENARIO_FRAME_BUDGET_WALL_CAP_SECONDS: f64 = 1800.0;

// The backstop never undercuts the declared wall timeout, so declarations
// above the cap keep their original deadline.
fn frame_budget_wall_backstop(
    timeout: Option<Duration>,
    frame_pacing_active: bool,
) -> Option<Duration> {
    let timeout = timeout?;
    if !frame_pacing_active {
        return Some(timeout);
    }
    let declared = timeout.as_secs_f64();
    let backstop = (declared * SCENARIO_FRAME_BUDGET_WALL_SLACK)
        .min(SCENARIO_FRAME_BUDGET_WALL_CAP_SECONDS)
        .max(declared);
    Some(Duration::from_secs_f64(backstop))
}

#[derive(Debug, Clone, Copy, Default)]
struct RuntimeExecutionOptions {
    artifact_prepare_ms: Option<u64>,
    profile: bool,
    timeout: Option<Duration>,
    visual_risk: VisualRiskExecutionOptions,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct VisualRiskExecutionOptions {
    pub enabled: bool,
    pub strict: bool,
}

pub fn run_scenario_once(
    config: &ProjectConfig,
    scenario_path: &Path,
    scenario: &Value,
    command_line: &str,
    headless_override: Option<bool>,
    audio_override: Option<bool>,
    visual_risk: VisualRiskExecutionOptions,
) -> CommandResult {
    let command = "scenario run";
    if let Some(failure) = config.project_config_failure_for_command(command) {
        return CommandResult {
            ok: false,
            command: command.to_string(),
            message: "scenario run failed".to_string(),
            data: Map::new(),
            failures: vec![failure],
            exit_code: 1,
        };
    }
    let screenshot_assertions = screenshot_match_assertions(scenario);
    let layout_mode = layout_mode_from_scenario(scenario);
    let viewports = scenario_viewports(scenario);
    let scenario_timeout = match scenario_timeout_duration(command, scenario_path, scenario) {
        Ok(timeout) => timeout,
        Err(failure) => {
            return CommandResult {
                ok: false,
                command: command.to_string(),
                message: "scenario run failed".to_string(),
                data: Map::new(),
                failures: vec![failure],
                exit_code: 1,
            };
        }
    };
    if !screenshot_assertions.is_empty() && viewports.len() > 1 {
        return CommandResult {
            ok: false,
            command: command.to_string(),
            message: "scenario run failed".to_string(),
            data: Map::new(),
            failures: vec![FailureDetail {
                kind: "screenshot_match_viewports_unsupported".to_string(),
                command: Some(command.to_string()),
                path: Some(scenario_path.to_string_lossy().into_owned()),
                message: Some(
                    "screenshot_match does not support multi-viewport scenarios in S9".to_string(),
                ),
                expected: Some(json!("single viewport scenario without 'viewports'")),
                actual: Some(json!(viewports)),
                ..FailureDetail::default()
            }],
            exit_code: 1,
        };
    }
    for screenshot_assertion in &screenshot_assertions {
        if let Err(failure) = resolve_screenshot_baseline(
            config,
            scenario_path,
            screenshot_assertion
                .get("baseline")
                .and_then(Value::as_str)
                .unwrap_or_default(),
            command,
        ) {
            return CommandResult {
                ok: false,
                command: command.to_string(),
                message: "scenario run failed".to_string(),
                data: Map::new(),
                failures: vec![failure],
                exit_code: 1,
            };
        }
    }

    // A single-viewport (or viewport-less) layout audit now runs the NORMAL
    // single-run path: the engine only ever receives runtime-supported
    // assertions, so the runtime payload must strip the CLI-only `layout_audit`
    // (like `screenshot_match`/`saved_snapshot_pixels`) whenever the run does not
    // fan out into a viewport matrix. Only a real multi-viewport matrix
    // (`viewports.len() > 1`) keeps the full scenario for its aggregate metadata.
    let scenario_for_artifacts = if screenshot_assertions.is_empty() && viewports.len() <= 1 {
        runtime_scenario_payload(scenario)
    } else {
        scenario.clone()
    };
    let scenario_for_artifacts =
        scenario_with_effective_launch_args(config, &scenario_for_artifacts);
    let artifact_started = Instant::now();
    let artifacts = match artifacts::prepare_run_artifacts(
        &config.artifacts_dir,
        &scenario_for_artifacts,
        &config.project_root,
        scenario_path,
        command_line,
        config.godot_binary.as_deref(),
        &config.godot_launch_prefix,
        &config.godot_windowed_launch_prefix,
        None,
        None,
    ) {
        Ok(artifacts) => artifacts,
        Err(error) => {
            return CommandResult {
                ok: false,
                command: command.to_string(),
                message: "scenario run failed".to_string(),
                data: Map::new(),
                failures: vec![FailureDetail {
                    kind: "artifact_write_failed".to_string(),
                    command: Some(command.to_string()),
                    path: Some(config.artifacts_dir.to_string_lossy().into_owned()),
                    message: Some(error.to_string()),
                    expected: Some(json!(
                        "artifact metadata written under the artifact directory"
                    )),
                    actual: Some(json!("OSError")),
                    ..FailureDetail::default()
                }],
                exit_code: 1,
            };
        }
    };

    let artifact_prepare_ms = duration_millis_u64(artifact_started.elapsed());

    // The viewport matrix is reserved for genuine multi-viewport fan-out. A
    // scenario with zero or one `viewports` entry — including a single-viewport
    // `layout_audit` — runs the normal single-run path (the lone viewport still
    // sets the window resolution, applied in execute_runtime_run), and its layout
    // audit is folded in as a post-process by apply_layout_audit.
    if viewports.len() > 1 {
        return run_viewport_matrix(
            command,
            "scenario run failed",
            config,
            scenario,
            scenario_path,
            &artifacts,
            &viewports,
            layout_mode.as_deref(),
            command_line,
            headless_override,
            audio_override,
        );
    }

    let use_runtime_request =
        !screenshot_assertions.is_empty() || scenario.get("_asset_trial").is_some();
    let requires_windowed_media_evidence =
        scenario_requires_windowed_media_evidence(scenario, !screenshot_assertions.is_empty());
    let selected_headless = selected_headless_for_scenario(
        headless_override.or_else(|| config.launch_mode_headless_override()),
        requires_windowed_media_evidence,
    );
    let requires_audio = scenario_requires_audio_evidence(scenario);
    let selected_audio = selected_audio_for_scenario(
        audio_override.or_else(|| config.audio_mode_override()),
        requires_audio,
    );
    execute_runtime_run(
        command,
        "scenario run failed",
        config,
        scenario_path,
        scenario,
        &artifacts,
        command_line,
        use_runtime_request,
        selected_headless,
        selected_audio,
        requires_windowed_media_evidence,
        RuntimeExecutionOptions {
            timeout: scenario_timeout,
            artifact_prepare_ms: Some(artifact_prepare_ms),
            visual_risk,
            ..RuntimeExecutionOptions::default()
        },
    )
}

pub fn run_scenario_profile_once(
    config: &ProjectConfig,
    scenario_path: &Path,
    scenario: &Value,
    command_line: &str,
    headless_override: Option<bool>,
    audio_override: Option<bool>,
) -> CommandResult {
    let command = "scenario profile";
    if let Some(failure) = config.project_config_failure_for_command(command) {
        return CommandResult {
            ok: false,
            command: command.to_string(),
            message: "scenario profile failed".to_string(),
            data: Map::new(),
            failures: vec![failure],
            exit_code: 1,
        };
    }

    let screenshot_assertions = screenshot_match_assertions(scenario);
    let layout_mode = layout_mode_from_scenario(scenario);
    let viewports = scenario_viewports(scenario);
    if !screenshot_assertions.is_empty() && viewports.len() > 1 {
        return CommandResult {
            ok: false,
            command: command.to_string(),
            message: "scenario profile failed".to_string(),
            data: Map::new(),
            failures: vec![FailureDetail {
                kind: "screenshot_match_viewports_unsupported".to_string(),
                command: Some(command.to_string()),
                path: Some(scenario_path.to_string_lossy().into_owned()),
                message: Some(
                    "screenshot_match does not support multi-viewport scenarios in S9".to_string(),
                ),
                expected: Some(json!("single viewport scenario without 'viewports'")),
                actual: Some(json!(viewports)),
                ..FailureDetail::default()
            }],
            exit_code: 1,
        };
    }
    for screenshot_assertion in &screenshot_assertions {
        if let Err(failure) = resolve_screenshot_baseline(
            config,
            scenario_path,
            screenshot_assertion
                .get("baseline")
                .and_then(Value::as_str)
                .unwrap_or_default(),
            command,
        ) {
            return CommandResult {
                ok: false,
                command: command.to_string(),
                message: "scenario profile failed".to_string(),
                data: Map::new(),
                failures: vec![failure],
                exit_code: 1,
            };
        }
    }

    let scenario_for_artifacts = if screenshot_assertions.is_empty() && viewports.len() <= 1 {
        runtime_scenario_payload(scenario)
    } else {
        scenario.clone()
    };
    let scenario_for_artifacts =
        scenario_with_effective_launch_args(config, &scenario_for_artifacts);
    let artifact_started = Instant::now();
    let artifacts = match artifacts::prepare_run_artifacts(
        &config.artifacts_dir,
        &scenario_for_artifacts,
        &config.project_root,
        scenario_path,
        command_line,
        config.godot_binary.as_deref(),
        &config.godot_launch_prefix,
        &config.godot_windowed_launch_prefix,
        None,
        None,
    ) {
        Ok(artifacts) => artifacts,
        Err(error) => {
            return CommandResult {
                ok: false,
                command: command.to_string(),
                message: "scenario profile failed".to_string(),
                data: Map::new(),
                failures: vec![FailureDetail {
                    kind: "artifact_write_failed".to_string(),
                    command: Some(command.to_string()),
                    path: Some(config.artifacts_dir.to_string_lossy().into_owned()),
                    message: Some(error.to_string()),
                    expected: Some(json!(
                        "artifact metadata written under the artifact directory"
                    )),
                    actual: Some(json!("OSError")),
                    ..FailureDetail::default()
                }],
                exit_code: 1,
            };
        }
    };

    // As in `run_scenario_once`: the matrix is only for genuine multi-viewport
    // fan-out. A single-viewport `layout_audit` profile runs normally and its
    // layout audit is applied as a finalize post-process.
    let artifact_prepare_ms = duration_millis_u64(artifact_started.elapsed());

    if viewports.len() > 1 {
        return run_viewport_matrix(
            command,
            "scenario profile failed",
            config,
            scenario,
            scenario_path,
            &artifacts,
            &viewports,
            layout_mode.as_deref(),
            command_line,
            headless_override,
            audio_override,
        );
    }

    let use_runtime_request = true;
    let requires_windowed_media_evidence =
        scenario_requires_windowed_media_evidence(scenario, !screenshot_assertions.is_empty());
    let selected_headless = selected_headless_for_scenario(
        headless_override.or_else(|| config.launch_mode_headless_override()),
        requires_windowed_media_evidence,
    );
    let requires_audio = scenario_requires_audio_evidence(scenario);
    let selected_audio = selected_audio_for_scenario(
        audio_override.or_else(|| config.audio_mode_override()),
        requires_audio,
    );
    execute_runtime_run(
        command,
        "scenario profile failed",
        config,
        scenario_path,
        scenario,
        &artifacts,
        command_line,
        use_runtime_request,
        selected_headless,
        selected_audio,
        requires_windowed_media_evidence,
        RuntimeExecutionOptions {
            artifact_prepare_ms: Some(artifact_prepare_ms),
            profile: true,
            ..RuntimeExecutionOptions::default()
        },
    )
}

pub fn run_visual_preflight(
    config: &ProjectConfig,
    command_line: &str,
    headless: bool,
) -> CommandResult {
    let command = "visual preflight";
    if let Some(failure) = config.project_config_failure_for_command(command) {
        return CommandResult {
            ok: false,
            command: command.to_string(),
            message: "visual preflight failed".to_string(),
            data: Map::new(),
            failures: vec![failure],
            exit_code: 1,
        };
    }

    let launch_display =
        godot_qa_process::resolve_project_display_launch_metadata_with_windowed_prefix(
            &config.project_root,
            &config.godot_launch_prefix,
            &config.godot_windowed_launch_prefix,
            headless,
        );
    let mut data = Map::from_iter([
        (
            "project_root".to_string(),
            json!(config.project_root.to_string_lossy().into_owned()),
        ),
        ("headless".to_string(), json!(headless)),
        ("command_line".to_string(), json!(command_line)),
        (
            "launch_display".to_string(),
            display_launch_metadata_json(&launch_display),
        ),
    ]);
    data.insert(
        "visual_support".to_string(),
        visual_support_preflight_json(&launch_display),
    );

    CommandResult {
        ok: true,
        command: command.to_string(),
        message: "visual preflight completed".to_string(),
        data,
        failures: Vec::new(),
        exit_code: 0,
    }
}

pub fn run_generated_scenario_once(
    config: &ProjectConfig,
    assertion: &Map<String, Value>,
    command_line: &str,
) -> CommandResult {
    let command = "assert";
    let scene = match main_scene_from_project(config) {
        Ok(scene) => scene,
        Err(failure) => {
            return CommandResult {
                ok: false,
                command: command.to_string(),
                message: "assertion failed".to_string(),
                data: Map::new(),
                failures: vec![failure],
                exit_code: 1,
            };
        }
    };
    let scenario_assertion = generated_scenario_assertion(assertion);
    let scenario = json!({
        "name": "generated_assert_once",
        "scene": scene,
        "steps": [{"wait_frames": 1}],
        "assert": [scenario_assertion]
    });
    let scenario_path = config
        .project_root
        .join("qa")
        .join("scenarios")
        .join("generated")
        .join("assert_once.generated.json");
    run_scenario_once(
        config,
        &scenario_path,
        &scenario,
        command_line,
        Some(true),
        None,
        VisualRiskExecutionOptions::default(),
    )
}

pub fn run_generated_inspect_once(
    config: &ProjectConfig,
    target: &str,
    command_line: &str,
) -> CommandResult {
    let command = "node inspect";
    let target = target.trim();
    if target.is_empty() {
        return CommandResult {
            ok: false,
            command: command.to_string(),
            message: "node inspect failed".to_string(),
            data: Map::new(),
            failures: vec![FailureDetail {
                kind: "invalid_target".to_string(),
                command: Some(command.to_string()),
                target: Some(target.to_string()),
                message: Some("node inspect requires a non-empty target".to_string()),
                expected: Some(json!("non-empty target locator")),
                actual: Some(json!(target)),
                ..FailureDetail::default()
            }],
            exit_code: 1,
        };
    }
    let scene = match main_scene_from_project(config) {
        Ok(scene) => scene,
        Err(mut failure) => {
            failure.command = Some(command.to_string());
            return CommandResult {
                ok: false,
                command: command.to_string(),
                message: "node inspect failed".to_string(),
                data: Map::new(),
                failures: vec![failure],
                exit_code: 1,
            };
        }
    };
    let scenario = json!({
        "name": "generated_node_inspect_once",
        "scene": scene,
        "steps": [{"wait_frames": 1}],
        "assert": [{
            "state_matches": {
                "target": target,
                "source": "inspect",
                "path": "path",
                "operator": "present"
            }
        }]
    });
    let scenario_path = config
        .project_root
        .join("qa")
        .join("scenarios")
        .join("generated")
        .join("node_inspect_once.generated.json");
    let mut result = run_scenario_once(
        config,
        &scenario_path,
        &scenario,
        command_line,
        Some(true),
        None,
        VisualRiskExecutionOptions::default(),
    );
    result.command = command.to_string();
    result.message = if result.ok {
        "node inspect completed".to_string()
    } else {
        "node inspect failed".to_string()
    };
    result
}

pub fn run_generated_tree_once(
    config: &ProjectConfig,
    filters: &Map<String, Value>,
    command_line: &str,
) -> CommandResult {
    let command = "tree";
    let scene = match main_scene_from_project(config) {
        Ok(scene) => scene,
        Err(mut failure) => {
            failure.command = Some(command.to_string());
            return CommandResult {
                ok: false,
                command: command.to_string(),
                message: "tree failed".to_string(),
                data: Map::new(),
                failures: vec![failure],
                exit_code: 1,
            };
        }
    };
    let scenario = generated_tree_scenario(&scene, filters);
    let scenario_path = config
        .project_root
        .join("qa")
        .join("scenarios")
        .join("generated")
        .join("tree_once.generated.json");
    let result = run_scenario_once(
        config,
        &scenario_path,
        &scenario,
        command_line,
        Some(true),
        None,
        VisualRiskExecutionOptions::default(),
    );
    let data = result.data.clone();
    let artifacts = snapshot_artifacts_for(&data, "current");
    if result.ok {
        let artifacts = match artifacts {
            Ok(artifacts) => artifacts,
            Err(failure) => {
                return CommandResult {
                    ok: false,
                    command: command.to_string(),
                    message: "tree failed".to_string(),
                    data,
                    failures: vec![failure],
                    exit_code: 1,
                };
            }
        };
        let tree_path = match path_from_artifacts(&artifacts, "tree") {
            Ok(path) => path,
            Err(mut failure) => {
                failure.command = Some(command.to_string());
                return CommandResult {
                    ok: false,
                    command: command.to_string(),
                    message: "tree failed".to_string(),
                    data,
                    failures: vec![failure],
                    exit_code: 1,
                };
            }
        };
        match read_json_artifact(command, &tree_path, "tree.json", &artifacts) {
            Ok(tree) => {
                let mut tree_data = tree
                    .as_object()
                    .cloned()
                    .unwrap_or_else(|| Map::from_iter([("tree".to_string(), tree.clone())]));
                tree_data.insert("artifacts".to_string(), Value::Object(artifacts));
                return CommandResult {
                    ok: true,
                    command: command.to_string(),
                    message: "tree completed".to_string(),
                    data: tree_data,
                    failures: Vec::new(),
                    exit_code: 0,
                };
            }
            Err(failure) => {
                return CommandResult {
                    ok: false,
                    command: command.to_string(),
                    message: "tree failed".to_string(),
                    data,
                    failures: vec![failure],
                    exit_code: 1,
                };
            }
        }
    }

    CommandResult {
        ok: false,
        command: command.to_string(),
        message: "tree failed".to_string(),
        data,
        failures: result.failures,
        exit_code: result.exit_code,
    }
}

fn generated_tree_scenario(scene: &str, filters: &Map<String, Value>) -> Value {
    let mut snapshots = Map::from_iter([("save_screenshot".to_string(), json!(false))]);
    for (key, value) in filters {
        snapshots.insert(key.clone(), value.clone());
    }
    json!({
        "name": "generated_tree_once",
        "scene": scene,
        "snapshots": Value::Object(snapshots),
        "steps": [{"wait_frames": 1}, {"snapshot": "current"}]
    })
}

fn snapshot_artifacts_for(
    data: &Map<String, Value>,
    snapshot_name: &str,
) -> Result<Map<String, Value>, FailureDetail> {
    data.get("snapshot_artifacts")
        .and_then(Value::as_object)
        .and_then(|snapshots| snapshots.get(snapshot_name))
        .and_then(Value::as_object)
        .cloned()
        .ok_or_else(|| FailureDetail {
            kind: "artifact_read_failed".to_string(),
            command: Some("tree".to_string()),
            message: Some(format!(
                "snapshot.capture did not return artifact paths for snapshot '{snapshot_name}'"
            )),
            expected: Some(json!("snapshot_artifacts.current with tree artifact path")),
            actual: Some(
                data.get("snapshot_artifacts")
                    .cloned()
                    .unwrap_or(Value::Null),
            ),
            severity: Some("error".to_string()),
            ..FailureDetail::default()
        })
}

pub fn run_generated_visual_assertion_once(
    config: &ProjectConfig,
    assertion: &Map<String, Value>,
    snapshot_name: Option<&str>,
    command_line: &str,
) -> CommandResult {
    let command = "assert";
    let scene = match main_scene_from_project(config) {
        Ok(scene) => scene,
        Err(failure) => {
            return CommandResult {
                ok: false,
                command: command.to_string(),
                message: "assertion failed".to_string(),
                data: Map::new(),
                failures: vec![failure],
                exit_code: 1,
            };
        }
    };
    let snapshot = snapshot_name
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .unwrap_or("current");
    let mut assertion_with_snapshot = assertion.clone();
    assertion_with_snapshot.insert("snapshot".to_string(), json!(snapshot));
    let scenario_assertion = generated_scenario_assertion(&assertion_with_snapshot);
    let scenario = json!({
        "name": "generated_visual_assert_once",
        "scene": scene,
        "snapshots": {"save_screenshot": true},
        "steps": [{"wait_frames": 1}, {"snapshot": snapshot}],
        "assert": [scenario_assertion]
    });
    let scenario_path = config
        .project_root
        .join("qa")
        .join("scenarios")
        .join("generated")
        .join("visual_assert_once.generated.json");
    run_scenario_once(
        config,
        &scenario_path,
        &scenario,
        command_line,
        Some(false),
        None,
        VisualRiskExecutionOptions::default(),
    )
}

fn generated_scenario_assertion(assertion: &Map<String, Value>) -> Value {
    if assertion
        .get("temporal")
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        for key in [
            "max_delta_position",
            "follows",
            "never",
            "within_limits",
            "distance_changed",
        ] {
            if let Some(params) = assertion.get(key).and_then(Value::as_object) {
                return Value::Object(Map::from_iter([(
                    key.to_string(),
                    Value::Object(params.clone()),
                )]));
            }
        }
        return Value::Object(assertion.clone());
    }

    let Some(name) = assertion.get("name").and_then(Value::as_str) else {
        return Value::Object(assertion.clone());
    };
    if name == "surface_probe" {
        if let Some(surface_probe) = assertion.get("surface_probe").cloned() {
            return Value::Object(Map::from_iter([(
                "surface_probe".to_string(),
                surface_probe,
            )]));
        }
        return Value::Object(assertion.clone());
    }
    if matches!(
        name,
        "exists"
            | "not_exists"
            | "visible"
            | "hidden"
            | "enabled"
            | "disabled"
            | "focused"
            | "inside_viewport"
    ) {
        if let Some(target) = assertion.get("target").cloned() {
            return Value::Object(Map::from_iter([(name.to_string(), target)]));
        }
        return Value::Object(assertion.clone());
    }
    let mut params = Map::new();
    for (key, value) in assertion {
        if key != "name" {
            params.insert(key.clone(), value.clone());
        }
    }
    Value::Object(Map::from_iter([(name.to_string(), Value::Object(params))]))
}

fn main_scene_from_project(config: &ProjectConfig) -> Result<String, FailureDetail> {
    let project_file = config.project_root.join("project.godot");
    let content = match read_project_godot_cached(&project_file) {
        Some(content) => content,
        // Rare path: re-read only to name the underlying OS error, so the failure
        // stays as actionable as it was before the read went through the cache.
        None => {
            let err = fs::read_to_string(&project_file)
                .err()
                .map(|err| err.to_string())
                .unwrap_or_else(|| "unreadable".to_string());
            return Err(FailureDetail {
                kind: "main_scene_missing".to_string(),
                command: Some("assert".to_string()),
                path: Some(project_file.to_string_lossy().into_owned()),
                message: Some(format!("failed to read project.godot: {err}")),
                expected: Some(json!("project.godot with application/run/main_scene")),
                actual: Some(json!("missing_or_unreadable")),
                ..FailureDetail::default()
            });
        }
    };
    for line in content.lines() {
        let trimmed = line.trim();
        if let Some(raw) = trimmed.strip_prefix("run/main_scene=") {
            let value = raw.trim().trim_matches('"').trim().to_string();
            if !value.is_empty() {
                return Ok(value);
            }
        }
    }
    Err(FailureDetail {
        kind: "main_scene_missing".to_string(),
        command: Some("assert".to_string()),
        path: Some(project_file.to_string_lossy().into_owned()),
        message: Some("project main scene was not configured".to_string()),
        expected: Some(json!("application/run/main_scene in project.godot")),
        actual: Some(json!("missing")),
        ..FailureDetail::default()
    })
}

pub fn run_layout_audit(
    config: &ProjectConfig,
    scene: &str,
    viewports_raw: &str,
    strict: bool,
    command_line: &str,
) -> CommandResult {
    let command = "layout audit";
    let mut failures = Vec::new();
    if let Some(failure) = config.project_config_failure_for_command(command) {
        failures.push(failure);
    }
    if !config.project_root.exists() {
        failures.push(FailureDetail {
            kind: "missing_project_root".to_string(),
            command: Some(command.to_string()),
            path: Some(config.project_root.to_string_lossy().into_owned()),
            message: Some("project root does not exist".to_string()),
            expected: Some(json!("existing Godot project root")),
            actual: Some(json!("missing")),
            ..FailureDetail::default()
        });
    }
    if config.godot_binary.is_none() {
        failures.push(FailureDetail {
            kind: "missing_godot".to_string(),
            command: Some(command.to_string()),
            message: Some("Godot binary was not configured".to_string()),
            expected: Some(json!("--godot-bin, GODOT_BIN, or godot on PATH")),
            actual: Some(json!("missing")),
            ..FailureDetail::default()
        });
    }
    if !(config.addon_dir.join("plugin.cfg")).exists() {
        failures.push(FailureDetail {
            kind: "runtime_addon_missing".to_string(),
            command: Some(command.to_string()),
            path: Some(config.addon_dir.to_string_lossy().into_owned()),
            message: Some(
                "godot_qa addon must be installed in the target project for layout audit"
                    .to_string(),
            ),
            expected: Some(json!("addons/godot_qa/plugin.cfg")),
            actual: Some(json!("missing")),
            ..FailureDetail::default()
        });
    }
    let viewports = match layout::parse_viewports(viewports_raw, command) {
        Ok(viewports) => viewports,
        Err(failure) => {
            failures.push(failure);
            Vec::new()
        }
    };
    if !failures.is_empty() {
        return CommandResult {
            ok: false,
            command: command.to_string(),
            message: "layout audit failed".to_string(),
            data: Map::new(),
            failures,
            exit_code: 1,
        };
    }

    let scene_name = Path::new(scene)
        .file_stem()
        .and_then(|value| value.to_str())
        .filter(|value| !value.is_empty())
        .unwrap_or("layout_audit");
    let scenario_name = format!("{scene_name}_layout_audit");
    let scenario = json!({
        "name": scenario_name,
        "scene": scene,
        "viewport": {},
        "determinism": {},
        "snapshots": {"save_screenshot": true},
        "steps": [{"wait_frames": 5}, {"snapshot": "audit"}],
        "assert": [{"no_errors": true}],
        "viewports": viewports,
    });
    let scenario_path = config
        .project_root
        .join("qa")
        .join("scenarios")
        .join("generated")
        .join(format!("{scene_name}_layout_audit.json"));
    let artifacts = match artifacts::prepare_run_artifacts(
        &config.artifacts_dir,
        &scenario,
        &config.project_root,
        &scenario_path,
        command_line,
        config.godot_binary.as_deref(),
        &config.godot_launch_prefix,
        &config.godot_windowed_launch_prefix,
        None,
        None,
    ) {
        Ok(artifacts) => artifacts,
        Err(error) => {
            return CommandResult {
                ok: false,
                command: command.to_string(),
                message: "layout audit failed".to_string(),
                data: Map::new(),
                failures: vec![FailureDetail {
                    kind: "artifact_write_failed".to_string(),
                    command: Some(command.to_string()),
                    path: Some(config.artifacts_dir.to_string_lossy().into_owned()),
                    message: Some(error.to_string()),
                    expected: Some(json!(
                        "artifact metadata written under the artifact directory"
                    )),
                    actual: Some(json!("OSError")),
                    ..FailureDetail::default()
                }],
                exit_code: 1,
            };
        }
    };

    run_viewport_matrix(
        command,
        "layout audit failed",
        config,
        &scenario,
        &scenario_path,
        &artifacts,
        &viewports,
        Some(if strict { "strict" } else { "default" }),
        command_line,
        None,
        None,
    )
}

pub fn run_layout_audit_current(
    config: &ProjectConfig,
    scenario: Option<&str>,
    run_id: Option<&str>,
    snapshot: Option<&str>,
    live: bool,
    strict: bool,
) -> CommandResult {
    let command = "layout audit-current";
    if live {
        if run_id.is_some() || snapshot.is_some() {
            return CommandResult {
                ok: false,
                command: command.to_string(),
                message: "layout audit failed".to_string(),
                data: Map::new(),
                failures: vec![FailureDetail {
                    kind: "invalid_layout_args".to_string(),
                    command: Some(command.to_string()),
                    message: Some(
                        "--live cannot be combined with snapshot artifact selection flags"
                            .to_string(),
                    ),
                    expected: Some(json!("use --live without --run-id or --snapshot")),
                    actual: Some(json!({
                        "run_id": run_id,
                        "snapshot": snapshot,
                    })),
                    ..FailureDetail::default()
                }],
                exit_code: 1,
            };
        }
        return run_layout_audit_current_live(config, scenario, strict);
    }
    // Without a live session, the scenario name is how the saved snapshot is
    // located, so it is required.
    let Some(scenario) = scenario else {
        return CommandResult {
            ok: false,
            command: command.to_string(),
            message: "layout audit failed".to_string(),
            data: Map::new(),
            failures: vec![FailureDetail {
                kind: "invalid_layout_args".to_string(),
                command: Some(command.to_string()),
                message: Some(
                    "layout audit-current requires --scenario unless --live is set".to_string(),
                ),
                expected: Some(json!("--scenario <name>, or --live for the active session")),
                actual: Some(json!({ "scenario": Value::Null, "live": false })),
                ..FailureDetail::default()
            }],
            exit_code: 1,
        };
    };
    if !config.project_root.exists() {
        return CommandResult {
            ok: false,
            command: command.to_string(),
            message: "layout audit failed".to_string(),
            data: Map::new(),
            failures: vec![FailureDetail {
                kind: "missing_project_root".to_string(),
                command: Some(command.to_string()),
                path: Some(config.project_root.to_string_lossy().into_owned()),
                message: Some("project root does not exist".to_string()),
                expected: Some(json!("existing Godot project root")),
                actual: Some(json!("missing")),
                ..FailureDetail::default()
            }],
            exit_code: 1,
        };
    }
    let loaded = match snapshot_context::load_snapshot_context(
        &config.artifacts_dir,
        scenario,
        run_id,
        snapshot,
        command,
    ) {
        Ok(loaded) => loaded,
        Err(failure) => {
            return CommandResult {
                ok: false,
                command: command.to_string(),
                message: "layout audit failed".to_string(),
                data: Map::new(),
                failures: vec![failure],
                exit_code: 1,
            };
        }
    };
    let audit_result = layout::audit_layout(
        &loaded.snapshot_context,
        loaded
            .snapshot_context
            .get("ui")
            .and_then(|ui| ui.get("viewport"))
            .and_then(Value::as_array)
            .map(|viewport| {
                viewport
                    .iter()
                    .filter_map(Value::as_i64)
                    .collect::<Vec<_>>()
            }),
        if strict { "strict" } else { "default" },
        Some(loaded.artifacts.clone()),
    );
    CommandResult {
        ok: audit_result.ok,
        command: command.to_string(),
        message: if audit_result.ok {
            "layout audit passed".to_string()
        } else {
            "layout audit failed".to_string()
        },
        data: Map::from_iter([
            ("scenario".to_string(), json!(scenario)),
            (
                "run_id".to_string(),
                json!(
                    loaded
                        .run_root
                        .file_name()
                        .and_then(|value| value.to_str())
                        .unwrap_or_default()
                ),
            ),
            ("snapshot".to_string(), json!(loaded.snapshot_name)),
            ("viewport".to_string(), json!(audit_result.viewport)),
            ("artifacts".to_string(), Value::Object(loaded.artifacts)),
        ]),
        failures: audit_result.failures,
        exit_code: if audit_result.ok { 0 } else { 1 },
    }
}

fn run_layout_audit_current_live(
    config: &ProjectConfig,
    scenario: Option<&str>,
    strict: bool,
) -> CommandResult {
    let command = "layout audit-current";
    // A live session already knows where its artifacts live, so `--scenario`
    // only labels the captured snapshot. Default the label when it is absent.
    let snapshot_label = scenario.unwrap_or("current");
    let snapshot_name = format!("layout-audit-current-{}", slug_value(snapshot_label));
    let live_result = godot_qa_runtime::run_live_protocol_command(
        config,
        command,
        godot_qa_runtime::SNAPSHOT_CAPTURE,
        Map::from_iter([("name".to_string(), json!(snapshot_name))]),
    );
    if !live_result.ok {
        return live_result;
    }
    let Some(artifacts) = live_result
        .data
        .get("artifacts")
        .and_then(Value::as_object)
        .cloned()
    else {
        return CommandResult {
            ok: false,
            command: command.to_string(),
            message: "layout audit failed".to_string(),
            data: Map::new(),
            failures: vec![FailureDetail {
                kind: "artifact_read_failed".to_string(),
                command: Some(command.to_string()),
                message: Some("snapshot.capture did not return artifact paths".to_string()),
                expected: Some(json!("snapshot.capture response with artifacts map")),
                actual: Some(json!("missing")),
                ..FailureDetail::default()
            }],
            exit_code: 1,
        };
    };
    let loaded = match snapshot_context::load_snapshot_context_from_artifacts(&artifacts, command) {
        Ok(loaded) => loaded,
        Err(failure) => {
            return CommandResult {
                ok: false,
                command: command.to_string(),
                message: "layout audit failed".to_string(),
                data: Map::new(),
                failures: vec![failure],
                exit_code: 1,
            };
        }
    };
    let audit_result = layout::audit_layout(
        &loaded,
        loaded
            .get("ui")
            .and_then(|ui| ui.get("viewport"))
            .and_then(Value::as_array)
            .map(|viewport| {
                viewport
                    .iter()
                    .filter_map(Value::as_i64)
                    .collect::<Vec<_>>()
            }),
        if strict { "strict" } else { "default" },
        Some(artifacts.clone()),
    );
    CommandResult {
        ok: audit_result.ok,
        command: command.to_string(),
        message: if audit_result.ok {
            "layout audit passed".to_string()
        } else {
            "layout audit failed".to_string()
        },
        data: Map::from_iter([
            ("scenario".to_string(), json!(scenario)),
            ("live".to_string(), json!(true)),
            ("snapshot".to_string(), json!(snapshot_name)),
            ("viewport".to_string(), json!(audit_result.viewport)),
            ("artifacts".to_string(), Value::Object(artifacts)),
        ]),
        failures: audit_result.failures,
        exit_code: if audit_result.ok { 0 } else { 1 },
    }
}

fn slug_value(value: &str) -> String {
    let cleaned = value
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() {
                character.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect::<String>();
    cleaned.trim_matches('-').to_string()
}

fn execute_runtime_run(
    command: &str,
    result_message: &str,
    config: &ProjectConfig,
    scenario_path: &Path,
    scenario: &Value,
    artifacts: &artifacts::RunArtifacts,
    command_line: &str,
    use_runtime_request: bool,
    headless: bool,
    audio: bool,
    requires_windowed_media_evidence: bool,
    options: RuntimeExecutionOptions,
) -> CommandResult {
    let preparation_started = Instant::now();
    let scenario_godot_args = scenario_godot_args(scenario);
    let scenario_project_args = scenario_project_args(scenario);
    let installed_addon_identity = addon_provenance(config, artifacts);
    let godot_args = merged_launch_args(&config.godot_args, &scenario_godot_args);
    let project_args = merged_launch_args(&config.godot_project_args, &scenario_project_args);
    // Frame pacing (the scenario's `determinism.fixed_fps` / project timestep) is applied
    // for both `run` and `profile`. Profiling measures real wall-clock frame cost, so a
    // fixed simulation timestep keeps time-integrated state deterministic without distorting
    // the perf numbers.
    let frame_pacing = match scenario_frame_pacing(scenario, &config.project_root) {
        Ok(frame_pacing) => frame_pacing,
        Err(failure) => {
            return persist_result(
                &artifacts.result_path,
                CommandResult {
                    ok: false,
                    command: command.to_string(),
                    message: result_message.to_string(),
                    data: Map::from_iter([
                        (
                            "scenario".to_string(),
                            json!(
                                scenario
                                    .get("name")
                                    .and_then(Value::as_str)
                                    .unwrap_or_default()
                            ),
                        ),
                        (
                            "scene".to_string(),
                            json!(
                                scenario
                                    .get("scene")
                                    .and_then(Value::as_str)
                                    .unwrap_or_default()
                            ),
                        ),
                        (
                            "scenario_path".to_string(),
                            json!(scenario_path.to_string_lossy().into_owned()),
                        ),
                        ("artifacts".to_string(), artifacts.to_json()),
                    ]),
                    failures: vec![failure],
                    exit_code: 1,
                },
            );
        }
    };
    let mut base_data = Map::from_iter([
        (
            "scenario".to_string(),
            json!(
                scenario
                    .get("name")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
            ),
        ),
        (
            "scene".to_string(),
            json!(
                scenario
                    .get("scene")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
            ),
        ),
        (
            "scenario_path".to_string(),
            json!(scenario_path.to_string_lossy().into_owned()),
        ),
        ("artifacts".to_string(), artifacts.to_json()),
    ]);
    base_data.insert("addon_identity".to_string(), installed_addon_identity);
    base_data.insert("godot_args".to_string(), json!(godot_args));
    if let Some(frame_pacing) = &frame_pacing {
        base_data.insert("frame_pacing".to_string(), frame_pacing.to_json());
    }
    base_data.insert(
        "project_args".to_string(),
        redact_project_args_for_metadata(&json!(project_args)),
    );
    base_data.insert("headless".to_string(), json!(headless));
    base_data.insert("audio".to_string(), json!(audio));
    let launch_display =
        godot_qa_process::resolve_project_display_launch_metadata_with_windowed_prefix(
            &config.project_root,
            &config.godot_launch_prefix,
            &config.godot_windowed_launch_prefix,
            headless,
        );
    base_data.insert(
        "launch_display".to_string(),
        display_launch_metadata_json(&launch_display),
    );
    if requires_windowed_media_evidence && !launch_display.display_backend_available {
        let artifacts_json = artifacts.to_json();
        let artifacts_map = artifacts_json.as_object().cloned().unwrap_or_default();
        return persist_result(
            &artifacts.result_path,
            CommandResult {
                ok: false,
                command: command.to_string(),
                message: result_message.to_string(),
                data: base_data,
                failures: vec![failure_with_artifacts(
                    FailureDetail {
                        kind: "display_backend_unavailable".to_string(),
                        command: Some(command.to_string()),
                        path: Some(scenario_path.to_string_lossy().into_owned()),
                        message: Some(
                            "scenario requires windowed media evidence but no display backend is available"
                                .to_string(),
                        ),
                        expected: Some(json!(
                            "real display backend or detected virtual display wrapper"
                        )),
                        actual: Some(display_launch_metadata_json(&launch_display)),
                        ..FailureDetail::default()
                    },
                    &artifacts_map,
                )],
                exit_code: 1,
            },
        );
    }

    let runtime_request_path = if !use_runtime_request {
        None
    } else {
        let request_path = artifacts.root_dir.join("runtime-request.json");
        let runtime_scenario = runtime_scenario_payload(&scenario_with_launch_args(
            scenario,
            &godot_args,
            &project_args,
        ));
        if let Err(error) = write_runtime_request(
            &request_path,
            &runtime_scenario,
            &config.project_root,
            artifacts,
            command_line,
            options,
        ) {
            return persist_result(
                &artifacts.result_path,
                CommandResult {
                    ok: false,
                    command: command.to_string(),
                    message: result_message.to_string(),
                    data: base_data,
                    failures: vec![failure_with_artifacts(
                        FailureDetail {
                            kind: "artifact_write_failed".to_string(),
                            command: Some(command.to_string()),
                            path: Some(request_path.to_string_lossy().into_owned()),
                            message: Some(error.to_string()),
                            expected: Some(json!("runtime request artifact written to disk")),
                            actual: Some(json!("OSError")),
                            ..FailureDetail::default()
                        },
                        &artifacts.to_json().as_object().cloned().unwrap_or_default(),
                    )],
                    exit_code: 1,
                },
            );
        }
        Some(request_path)
    };

    base_data.insert(
        "cold_timing".to_string(),
        json!({
            "artifact_prepare_ms": options.artifact_prepare_ms,
            "request_prepare_ms": duration_millis_u64(preparation_started.elapsed()),
        }),
    );
    let process_result =
        match godot_qa_process::launch_godot_one_shot(godot_qa_process::GodotOneShotLaunch {
            binary: config.godot_binary.as_deref().unwrap_or_default(),
            launch_prefix: &config.godot_launch_prefix,
            windowed_launch_prefix: &config.godot_windowed_launch_prefix,
            default_godot_args: &config.godot_args,
            extra_godot_args: &scenario_godot_args,
            default_project_args: &config.godot_project_args,
            extra_project_args: &scenario_project_args,
            project_root: &config.project_root,
            request_path: runtime_request_path
                .as_deref()
                .unwrap_or(&artifacts.request_path),
            logs_path: &artifacts.logs_path,
            headless,
            audio,
            viewport: scenario_viewports(scenario).first().map(Vec::as_slice),
            timeout: frame_budget_wall_backstop(options.timeout, frame_pacing.is_some()),
            fixed_fps: frame_pacing.as_ref().map(|pacing| pacing.fixed_fps),
        }) {
            Ok(result) => result,
            Err(failure) => {
                let result = CommandResult {
                    ok: false,
                    command: command.to_string(),
                    message: result_message.to_string(),
                    data: base_data,
                    failures: vec![failure_with_artifacts(
                        failure,
                        &artifacts.to_json().as_object().cloned().unwrap_or_default(),
                    )],
                    exit_code: 1,
                };
                return persist_result(&artifacts.result_path, result);
            }
        };
    finalize_runtime_run(
        command,
        result_message,
        config,
        scenario_path,
        scenario,
        artifacts,
        use_runtime_request,
        headless,
        options,
        base_data,
        &process_result,
    )
}

// Post-launch finalize shared by the cold one-shot path (execute_runtime_run)
// and the warm in-session path (run/warm.rs). Everything from reading the
// engine's result.json onward — validation, profiling, screenshot/visual
// passes, screenshot-match — runs through this single code path so cold and
// warm produce identical CommandResult/result.json. The warm path synthesizes a
// GodotRunResult (returncode 0, not timed out, session display) since there is
// no per-scenario process.
//
// This is also where the project's `logs.ignore` patterns enter the engine-log
// scan: every caller already carries the `ProjectConfig`, so cold, warm and
// Android all pass the identical list down to the two ignore predicates and
// nobody can drift from it.
fn finalize_runtime_run(
    command: &str,
    result_message: &str,
    config: &ProjectConfig,
    scenario_path: &Path,
    scenario: &Value,
    artifacts: &artifacts::RunArtifacts,
    use_runtime_request: bool,
    headless: bool,
    options: RuntimeExecutionOptions,
    mut base_data: Map<String, Value>,
    process_result: &godot_qa_process::GodotRunResult,
) -> CommandResult {
    let finalization_started = Instant::now();
    let persist_finalized = |mut result: CommandResult| {
        if let Some(timing) = result
            .data
            .get_mut("cold_timing")
            .and_then(Value::as_object_mut)
        {
            timing.insert(
                "cli_finalize_ms".to_string(),
                json!(duration_millis_u64(finalization_started.elapsed())),
            );
        }
        persist_result(&artifacts.result_path, result)
    };
    if let Ok(request_bytes) = fs::read(&artifacts.request_path)
        && let Ok(request) = serde_json::from_slice::<Value>(&request_bytes)
        && let Some(run) = request.get("run")
    {
        base_data.insert(
            "run_identity".to_string(),
            json!({
                "run_id": run.get("id"),
                "nonce": run.get("nonce"),
                "started_at": run.get("started_at"),
            }),
        );
    }
    base_data.insert("result_identity_verified".to_string(), json!(false));
    base_data.insert("headless".to_string(), json!(headless));
    base_data.insert(
        "launch_display".to_string(),
        display_launch_metadata_json(&process_result.display),
    );
    if process_result.timed_out {
        let artifacts_map = artifacts.to_json().as_object().cloned().unwrap_or_default();
        let raw_result = fs::read(&artifacts.result_path)
            .ok()
            .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
            .filter(Value::is_object);
        let completed_result = raw_result.as_ref().filter(|value| {
            value.get("ok").and_then(Value::as_bool).is_some()
                && value.get("failures").and_then(Value::as_array).is_some()
                && base_data.get("run_identity").is_some_and(|expected| {
                    value.get("run_identity").is_some_and(|actual| {
                        expected.get("run_id") == actual.get("run_id")
                            && expected.get("nonce") == actual.get("nonce")
                    })
                })
        });
        base_data.insert(
            "result_identity_verified".to_string(),
            json!(completed_result.is_some()),
        );
        let assertions_passed = completed_result
            .as_ref()
            .and_then(|value| value.get("ok"))
            .and_then(Value::as_bool);
        let assertions_completed = assertions_passed.is_some();
        base_data.insert(
            "completion".to_string(),
            json!({
                "result_written": artifacts.result_path.is_file(),
                "result_identity_verified": completed_result.is_some(),
                "assertions_completed": assertions_completed,
                "assertions_passed": assertions_passed,
                "process_exited_cleanly": false,
                "post_result_grace_ms": 3000,
            }),
        );
        base_data.insert("process".to_string(), process_summary_json(process_result));
        if let Some(progress_path) = artifacts_map
            .get("progress")
            .or_else(|| artifacts_map.get("progress_path"))
            .and_then(Value::as_str)
        {
            base_data.insert(
                "progress".to_string(),
                progress_evidence_summary_json(progress_path),
            );
            let slow_summary_path = artifacts_map
                .get("slow_summary")
                .and_then(Value::as_str)
                .map(PathBuf::from)
                .unwrap_or_else(|| artifacts.root_dir.join("slow-summary.json"));
            let slow_summary =
                slow_scenario_summary_json(scenario, progress_path, &slow_summary_path);
            let _ = write_json(&slow_summary_path, &slow_summary);
            base_data.insert("slow_summary".to_string(), slow_summary);
        }
        base_data.insert(
            "runtime_result".to_string(),
            runtime_result_summary_json(
                &artifacts.result_path,
                if completed_result.is_some() {
                    "completed_before_timeout"
                } else {
                    "timeout"
                },
                assertions_passed,
                completed_result
                    .as_ref()
                    .and_then(|value| value.get("failures"))
                    .and_then(Value::as_array)
                    .map_or(0, Vec::len),
            ),
        );
        base_data.insert(
            "logs".to_string(),
            log_status_summary_json(&artifacts.logs_path, &[], 0, 0),
        );
        let mut failure = scenario_run_timeout_failure(
            command,
            &artifacts.result_path,
            &artifacts_map,
            process_result,
            options.timeout,
        );
        if assertions_completed {
            failure.kind = "scenario_completion_timeout".to_string();
            failure.message = Some(
                "Godot wrote a scenario result but did not exit cleanly within the deadline"
                    .to_string(),
            );
        }
        let mut failures = completed_result
            .as_ref()
            .and_then(|value| value.get("failures"))
            .and_then(|value| serde_json::from_value::<Vec<FailureDetail>>(value.clone()).ok())
            .unwrap_or_default();
        failures.push(failure.clone());
        base_data.insert(
            "outcome".to_string(),
            outcome_summary_json(
                "fail",
                failures.first().map(|failure| failure.kind.as_str()),
                Some(&failures),
            ),
        );
        let result = CommandResult {
            ok: false,
            command: command.to_string(),
            message: result_message.to_string(),
            data: base_data,
            failures,
            exit_code: 1,
        };
        return persist_finalized(result);
    }

    base_data.insert(
        "completion".to_string(),
        json!({
        "result_written": artifacts.result_path.is_file(),
        "assertions_completed": false,
        "assertions_passed": Value::Null,
        "result_identity_verified": false,
            "process_exited_cleanly": process_result.returncode == 0,
            "post_result_grace_ms": 3000,
        }),
    );

    let raw_runtime_result = match load_runtime_result(
        command,
        &artifacts.result_path,
        &artifacts.to_json(),
        Some(&process_result.display),
    ) {
        Ok(result) => result,
        Err(failure) => {
            let reason = failure.kind.clone();
            base_data.insert("process".to_string(), process_summary_json(process_result));
            base_data.insert(
                "runtime_result".to_string(),
                runtime_result_summary_json(
                    &artifacts.result_path,
                    reason
                        .strip_prefix("scenario_result_")
                        .unwrap_or(reason.as_str()),
                    None,
                    0,
                ),
            );
            base_data.insert(
                "logs".to_string(),
                log_status_summary_json(&artifacts.logs_path, &[], 0, 0),
            );
            base_data.insert(
                "outcome".to_string(),
                outcome_summary_json("fail", Some(&reason), Some(std::slice::from_ref(&failure))),
            );
            let result = CommandResult {
                ok: false,
                command: command.to_string(),
                message: result_message.to_string(),
                data: base_data,
                failures: vec![failure],
                exit_code: 1,
            };
            return persist_finalized(result);
        }
    };

    let result_identity_verified = match (
        base_data.get("run_identity"),
        raw_runtime_result.get("run_identity"),
    ) {
        (Some(expected), Some(actual)) => {
            expected.get("run_id") == actual.get("run_id")
                && expected.get("nonce") == actual.get("nonce")
        }
        _ => false,
    };
    base_data.insert(
        "result_identity_verified".to_string(),
        json!(result_identity_verified),
    );

    if let (Some(expected), Some(actual)) = (
        base_data.get("run_identity"),
        raw_runtime_result.get("run_identity"),
    ) && (expected.get("run_id") != actual.get("run_id")
        || expected.get("nonce") != actual.get("nonce"))
    {
        let failure = failure_with_artifacts(
            FailureDetail {
                kind: "scenario_result_stale".to_string(),
                command: Some(command.to_string()),
                path: Some(artifacts.result_path.to_string_lossy().into_owned()),
                message: Some("runtime result identity differs from this run request".to_string()),
                expected: Some(expected.clone()),
                actual: Some(actual.clone()),
                ..FailureDetail::default()
            },
            &artifacts.to_json().as_object().cloned().unwrap_or_default(),
        );
        base_data.insert(
            "outcome".to_string(),
            outcome_summary_json(
                "fail",
                Some(&failure.kind),
                Some(std::slice::from_ref(&failure)),
            ),
        );
        return persist_finalized(CommandResult {
            ok: false,
            command: command.to_string(),
            message: result_message.to_string(),
            data: base_data,
            failures: vec![failure],
            exit_code: 1,
        });
    }
    if let Some(completion) = base_data.get_mut("completion") {
        let passed = raw_runtime_result.get("ok").and_then(Value::as_bool);
        completion["assertions_completed"] = json!(passed.is_some());
        completion["assertions_passed"] = json!(passed);
        completion["result_identity_verified"] = json!(result_identity_verified);
    }

    let runtime_result = validate_runtime_command_result(
        command,
        scenario,
        &mut base_data,
        &artifacts.result_path,
        &artifacts.to_json(),
        process_result,
        &raw_runtime_result,
        &artifacts.logs_path,
        &config.log_ignore_patterns,
    );
    let runtime_result = if options.profile && runtime_result.ok {
        match profile_result_from_runtime(
            command,
            scenario,
            &mut base_data,
            artifacts,
            &raw_runtime_result,
            &runtime_result,
        ) {
            Ok(result) => result,
            Err(result) => return result,
        }
    } else {
        runtime_result
    };
    let mut screenshot_analysis = screenshots::ScreenshotAnalysisCache::default();
    let runtime_result = apply_screenshot_quality_cached(
        command,
        scenario,
        runtime_result,
        &mut screenshot_analysis,
    );
    let runtime_result = apply_saved_snapshot_pixels(
        command,
        config,
        scenario,
        artifacts,
        runtime_result,
        &mut screenshot_analysis,
    );
    let runtime_result = apply_visual_risk_analysis(command, scenario, runtime_result, options);
    // Fold a single-run `layout_audit` (zero/one viewport) over the run's final
    // snapshot BEFORE screenshot-match compares, so layout + screenshot_match can
    // coexist on one run. Matrix variant scenarios are stripped of `layout_audit`
    // upstream (runtime_scenario_payload), so this is a no-op there — the matrix
    // path applies layout auditing itself, with no double-application.
    let runtime_result = apply_layout_audit(command, scenario, artifacts, runtime_result);
    let result = if use_runtime_request {
        apply_screenshot_matches(
            command,
            scenario,
            scenario_path,
            config,
            artifacts,
            runtime_result,
            &mut screenshot_analysis,
        )
    } else {
        runtime_result
    };
    persist_finalized(result)
}

include!("visual_preflight.rs");
include!("visual_screenshots.rs");
include!("process.rs");
include!("runtime_result.rs");
include!("audio.rs");
include!("warm.rs");
include!("android.rs");
include!("tests.rs");
include!("addon_provenance.rs");
