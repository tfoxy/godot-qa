use crate::scenario_doc;
use godot_qa_core::config::ProjectConfig;
use godot_qa_core::results::{CommandResult, FailureDetail};
use serde::Serialize;
use serde_json::{Map, Value, json};
use std::fs;
use std::path::{Component, Path, PathBuf};

pub const DEFAULT_FRAMES: u32 = 60;
pub const DEFAULT_SCREENSHOT_EVERY: u32 = 30;
pub const DEFAULT_AUDIO_SECONDS: u32 = 10;
pub const MAX_FRAMES: u32 = 120_000;
pub const MAX_AUDIO_SECONDS: u32 = 3_600;

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ReviewTemplate {
    pub id: &'static str,
    pub description: &'static str,
    pub output_type: &'static str,
    pub generated_command_type: &'static str,
    pub required_parameters: &'static [&'static str],
    pub optional_parameters: &'static [&'static str],
    pub required_tools: &'static [&'static str],
}

#[derive(Debug, Clone)]
pub struct ReviewInitOptions {
    pub template_id: String,
    pub name: String,
    pub scene: String,
    pub targets: Vec<String>,
    pub project_args: Vec<String>,
    pub frames: u32,
    pub screenshot_every: u32,
    pub audio_seconds: u32,
    pub event_provider: Option<String>,
    pub out: PathBuf,
}

impl Default for ReviewInitOptions {
    fn default() -> Self {
        Self {
            template_id: "scenario-smoke".to_string(),
            name: String::new(),
            scene: String::new(),
            targets: Vec::new(),
            project_args: Vec::new(),
            frames: DEFAULT_FRAMES,
            screenshot_every: DEFAULT_SCREENSHOT_EVERY,
            audio_seconds: DEFAULT_AUDIO_SECONDS,
            event_provider: None,
            out: PathBuf::new(),
        }
    }
}

pub fn built_in_review_templates() -> &'static [ReviewTemplate] {
    &[
        ReviewTemplate {
            id: "scenario-smoke",
            description: "Create a saved scenario harness that opens a scene, waits for it to settle, captures runtime evidence, and asserts no runtime errors.",
            output_type: "scenario",
            generated_command_type: "scenario run --file",
            required_parameters: &["name", "scene", "out"],
            optional_parameters: &["project_args", "frames"],
            required_tools: &["scenario run --file"],
        },
        ReviewTemplate {
            id: "live-visual-review",
            description: "Create a live-session script for generic visual inspection with snapshots and optional target inspection.",
            output_type: "live_script",
            generated_command_type: "session run-script",
            required_parameters: &["name", "scene", "out"],
            optional_parameters: &["targets", "project_args", "frames", "screenshot_every"],
            required_tools: &[
                "session run-script",
                "screenshot sequence",
                "logs",
                "errors",
            ],
        },
        ReviewTemplate {
            id: "live-physics-review",
            description: "Create a live-session script for generic physics review evidence with snapshots, target inspection, logs, and errors.",
            output_type: "live_script",
            generated_command_type: "session run-script",
            required_parameters: &["name", "scene", "out"],
            optional_parameters: &["targets", "project_args", "frames", "screenshot_every"],
            required_tools: &[
                "session run-script",
                "trace record",
                "physics collision-report",
                "logs",
                "errors",
            ],
        },
        ReviewTemplate {
            id: "live-audio-review",
            description: "Create a live-session script that captures bounded runtime evidence for audio-sensitive review without interpreting project-specific audio state.",
            output_type: "live_script",
            generated_command_type: "session run-script",
            required_parameters: &["name", "scene", "out"],
            optional_parameters: &["project_args", "frames", "audio_seconds"],
            required_tools: &["session run-script", "audio capture", "logs", "errors"],
        },
        ReviewTemplate {
            id: "live-event-review",
            description: "Create a live-session script for generic event review evidence using an optional project-owned event provider plus logs and errors.",
            output_type: "live_script",
            generated_command_type: "session run-script",
            required_parameters: &["name", "scene", "out"],
            optional_parameters: &["targets", "project_args", "frames", "event_provider"],
            required_tools: &["session run-script", "event trace", "logs", "errors"],
        },
        ReviewTemplate {
            id: "visual-physics-audio",
            description: "Create a combined live-session review script for visual, physics, and audio evidence over the same bounded runtime window.",
            output_type: "live_script",
            generated_command_type: "session run-script",
            required_parameters: &["name", "scene", "out"],
            optional_parameters: &[
                "targets",
                "project_args",
                "frames",
                "screenshot_every",
                "audio_seconds",
            ],
            required_tools: &[
                "session run-script",
                "screenshot sequence",
                "physics collision-report",
                "audio capture",
            ],
        },
        ReviewTemplate {
            id: "deterministic-runtime-review",
            description: "Create a generic deterministic runtime evidence script that forwards project args exactly and captures snapshots, state, logs, and errors over time.",
            output_type: "live_script",
            generated_command_type: "session run-script",
            required_parameters: &["name", "scene", "out"],
            optional_parameters: &[
                "targets",
                "project_args",
                "frames",
                "screenshot_every",
                "event_provider",
                "audio_seconds",
            ],
            required_tools: &[
                "session run-script",
                "screenshot sequence",
                "trace record",
                "logs",
                "errors",
            ],
        },
    ]
}

pub fn run_review_list_templates() -> CommandResult {
    let templates = built_in_review_templates()
        .iter()
        .map(template_inventory)
        .collect::<Vec<_>>();
    CommandResult {
        ok: true,
        command: "review list-templates".to_string(),
        message: "review templates listed".to_string(),
        data: Map::from_iter([("templates".to_string(), Value::Array(templates))]),
        failures: Vec::new(),
        exit_code: 0,
    }
}

pub fn run_review_init(config: &ProjectConfig, options: &ReviewInitOptions) -> CommandResult {
    let command = "review init";
    let Some(template) = review_template(&options.template_id) else {
        return review_failure(
            command,
            vec![validation_failure(
                command,
                "review_template_unknown",
                "review template id is not built in",
                json!(
                    built_in_review_templates()
                        .iter()
                        .map(|template| template.id)
                        .collect::<Vec<_>>()
                ),
                json!(options.template_id),
            )],
            Map::new(),
        );
    };

    if !scenario_doc::is_safe_scenario_name(options.name.trim()) {
        return review_failure(
            command,
            vec![validation_failure(
                command,
                "review_validation_failed",
                "review field 'name' must be a safe artifact path segment",
                json!("letters, numbers, dot, underscore, or hyphen"),
                json!(options.name),
            )],
            init_data(template, options, None, None),
        );
    }

    if !is_res_scene(&options.scene) {
        return review_failure(
            command,
            vec![validation_failure(
                command,
                "review_validation_failed",
                "review field 'scene' must be a res:// path",
                json!("res:// scene path"),
                json!(options.scene),
            )],
            init_data(template, options, None, None),
        );
    }

    if let Err(failure) = validate_options(command, options) {
        return review_failure(
            command,
            vec![failure],
            init_data(template, options, None, None),
        );
    }

    let output_path = resolve_output_path(config, &options.out);
    if output_path.exists() && output_path.is_dir() {
        return review_failure(
            command,
            vec![FailureDetail {
                kind: "review_output_invalid".to_string(),
                command: Some(command.to_string()),
                path: Some(output_path.to_string_lossy().into_owned()),
                message: Some("review init output path is a directory".to_string()),
                expected: Some(json!("writable file path")),
                actual: Some(json!("directory")),
                ..FailureDetail::default()
            }],
            init_data(template, options, Some(&output_path), None),
        );
    }

    let document = if template.output_type == "scenario" {
        render_scenario_smoke_template(options, &output_path)
    } else {
        render_live_script_template(template, options, &output_path)
    };

    if let Err(error) = write_json_document(&output_path, &document) {
        return review_failure(
            command,
            vec![FailureDetail {
                kind: "artifact_write_failed".to_string(),
                command: Some(command.to_string()),
                path: Some(output_path.to_string_lossy().into_owned()),
                message: Some(error.to_string()),
                expected: Some(json!("review template written to disk")),
                actual: Some(json!("OSError")),
                ..FailureDetail::default()
            }],
            init_data(template, options, Some(&output_path), None),
        );
    }

    let run_command = run_command_for(template, &output_path);
    CommandResult {
        ok: true,
        command: command.to_string(),
        message: "review template created".to_string(),
        data: init_data(template, options, Some(&output_path), Some(&run_command)),
        failures: Vec::new(),
        exit_code: 0,
    }
}

pub fn render_scenario_smoke_template(options: &ReviewInitOptions, out: &Path) -> Value {
    let mut object = Map::from_iter([
        ("name".to_string(), json!(options.name.trim())),
        ("scene".to_string(), json!(options.scene.trim())),
        (
            "snapshots".to_string(),
            json!({
                "save_tree": true,
                "save_ui": true,
                "save_screenshot": true,
                "save_logs": true
            }),
        ),
        (
            "steps".to_string(),
            json!([
                {"wait_frames": options.frames},
                {"snapshot": "after_wait"}
            ]),
        ),
        ("assert".to_string(), json!([{"no_errors": true}])),
        (
            "review".to_string(),
            json!({
                "template": "scenario-smoke",
                "generated_command_type": "scenario run --file",
                "run_command": run_command_for(review_template("scenario-smoke").unwrap(), out),
                "notes": "Run this generated scenario with scenario run --file. Review templates accelerate exploratory QA and do not approve baselines."
            }),
        ),
    ]);
    if !options.project_args.is_empty() {
        object.insert("project_args".to_string(), json!(options.project_args));
    }
    Value::Object(object)
}

fn render_live_script_template(
    template: &ReviewTemplate,
    options: &ReviewInitOptions,
    out: &Path,
) -> Value {
    match template.id {
        "live-visual-review" => render_live_visual_review_template(template, options, out),
        "live-physics-review" => render_live_physics_review_template(template, options, out),
        "live-audio-review" => render_live_audio_review_template(template, options, out),
        "live-event-review" => render_live_event_review_template(template, options, out),
        "visual-physics-audio" => render_visual_physics_audio_template(template, options, out),
        "deterministic-runtime-review" => {
            render_deterministic_runtime_review_template(template, options, out)
        }
        _ => render_live_visual_review_template(template, options, out),
    }
}

pub fn render_live_visual_review_template(
    template: &ReviewTemplate,
    options: &ReviewInitOptions,
    out: &Path,
) -> Value {
    let mut steps = live_start_steps(options);
    push_optional_wait(&mut steps, options.screenshot_every);
    push_screenshot_sequence(&mut steps, options, "visual");
    push_logs_errors_and_stop(&mut steps);
    live_script_document(
        template,
        options,
        out,
        steps,
        template_warnings(template, options),
    )
}

pub fn render_live_physics_review_template(
    template: &ReviewTemplate,
    options: &ReviewInitOptions,
    out: &Path,
) -> Value {
    let mut steps = live_start_steps(options);
    push_optional_wait(&mut steps, options.screenshot_every);
    if !options.targets.is_empty() {
        push_trace_record(&mut steps, options, true);
        for target in &options.targets {
            push_physics_collision_report(&mut steps, options, target);
        }
    }
    push_logs_errors_and_stop(&mut steps);
    live_script_document(
        template,
        options,
        out,
        steps,
        template_warnings(template, options),
    )
}

pub fn render_live_audio_review_template(
    template: &ReviewTemplate,
    options: &ReviewInitOptions,
    out: &Path,
) -> Value {
    let mut steps = live_start_steps(options);
    push_optional_wait(&mut steps, options.screenshot_every);
    push_audio_capture(&mut steps, options, "review_audio");
    push_logs_errors_and_stop(&mut steps);
    live_script_document(
        template,
        options,
        out,
        steps,
        template_warnings(template, options),
    )
}

pub fn render_live_event_review_template(
    template: &ReviewTemplate,
    options: &ReviewInitOptions,
    out: &Path,
) -> Value {
    let mut steps = live_start_steps(options);
    push_optional_wait(&mut steps, options.screenshot_every);
    if let (Some(target), Some(provider)) =
        (options.targets.first(), options.event_provider.as_ref())
    {
        steps.push(json!({
            "event_trace": {
                "target": target,
                "provider": provider,
                "frames": options.frames,
                "sample_every_frames": sample_every_frames(options)
            }
        }));
    }
    push_logs_errors_and_stop(&mut steps);
    live_script_document(
        template,
        options,
        out,
        steps,
        template_warnings(template, options),
    )
}

pub fn render_visual_physics_audio_template(
    template: &ReviewTemplate,
    options: &ReviewInitOptions,
    out: &Path,
) -> Value {
    let mut steps = live_start_steps(options);
    push_optional_wait(&mut steps, options.screenshot_every);
    push_screenshot_sequence(&mut steps, options, "visual");
    if !options.targets.is_empty() {
        push_trace_record(&mut steps, options, true);
        for target in &options.targets {
            push_physics_collision_report(&mut steps, options, target);
        }
    }
    push_audio_capture(&mut steps, options, "review_audio");
    push_logs_errors_and_stop(&mut steps);
    live_script_document(
        template,
        options,
        out,
        steps,
        template_warnings(template, options),
    )
}

pub fn render_deterministic_runtime_review_template(
    template: &ReviewTemplate,
    options: &ReviewInitOptions,
    out: &Path,
) -> Value {
    let mut steps = live_start_steps(options);
    push_optional_wait(&mut steps, options.screenshot_every);
    push_screenshot_sequence(&mut steps, options, "deterministic");
    if !options.targets.is_empty() {
        push_trace_record(&mut steps, options, false);
    } else {
        steps.push(json!({"tree": {"qa_only": true}}));
        steps.push(json!({"ui": {}}));
    }
    push_logs_errors_and_stop(&mut steps);
    live_script_document(
        template,
        options,
        out,
        steps,
        template_warnings(template, options),
    )
}

fn live_start_steps(options: &ReviewInitOptions) -> Vec<Value> {
    let mut start = Map::from_iter([("scene".to_string(), json!(options.scene.trim()))]);
    if !options.project_args.is_empty() {
        start.insert(
            "project_args".to_string(),
            json!(options.project_args.clone()),
        );
    }

    vec![json!({"session_start": Value::Object(start)})]
}

fn push_optional_wait(steps: &mut Vec<Value>, frames: u32) {
    if frames > 0 {
        steps.push(json!({"wait": {"frames": frames}}));
    }
}

fn push_screenshot_sequence(steps: &mut Vec<Value>, options: &ReviewInitOptions, label: &str) {
    steps.push(json!({
        "screenshot_sequence": {
            "frames": options.frames,
            "every": options.screenshot_every,
            "out": format!(".qa/review-artifacts/{}/{}-frames", options.name.trim(), label),
            "contact_sheet": true,
            "contact_sheet_out": format!(
                ".qa/review-artifacts/{}/{}-contact-sheet.png",
                options.name.trim(),
                label
            )
        }
    }));
}

fn push_trace_record(steps: &mut Vec<Value>, options: &ReviewInitOptions, physics: bool) {
    steps.push(json!({
        "trace_record": {
            "targets": options.targets,
            "frames": options.frames,
            "sample_every_frames": sample_every_frames(options),
            "physics": physics,
            "snapshot_start": true,
            "snapshot_end": true
        }
    }));
}

fn push_physics_collision_report(
    steps: &mut Vec<Value>,
    options: &ReviewInitOptions,
    target: &str,
) {
    steps.push(json!({
        "physics_collision_report": {
            "target": target,
            "frames": options.frames,
            "sample_every_frames": sample_every_frames(options),
            "physics": true
        }
    }));
}

fn push_audio_capture(steps: &mut Vec<Value>, options: &ReviewInitOptions, name: &str) {
    steps.push(json!({
        "audio_capture": {
            "seconds": options.audio_seconds,
            "name": name,
            "analyze": true,
            "strict_warnings": false
        }
    }));
}

fn push_logs_errors_and_stop(steps: &mut Vec<Value>) {
    steps.push(json!({"errors": {}}));
    steps.push(json!({"logs": {}}));
    steps.push(json!({"no_errors": true}));
    steps.push(json!({"session_stop": {}}));
}

fn sample_every_frames(options: &ReviewInitOptions) -> u32 {
    options.screenshot_every.clamp(1, 240)
}

fn live_script_document(
    template: &ReviewTemplate,
    options: &ReviewInitOptions,
    out: &Path,
    steps: Vec<Value>,
    warnings: Vec<String>,
) -> Value {
    json!({
        "name": options.name.trim(),
        "steps": steps,
        "review": {
            "template": template.id,
            "generated_command_type": template.generated_command_type,
            "run_command": run_command_for(template, out),
            "targets": options.targets,
            "frames": options.frames,
            "screenshot_every": options.screenshot_every,
            "audio_seconds": options.audio_seconds,
            "event_provider": options.event_provider,
            "warnings": warnings,
            "notes": "Run this generated live script with session run-script. Review templates accelerate exploratory QA and do not approve baselines."
        }
    })
}

fn template_warnings(template: &ReviewTemplate, options: &ReviewInitOptions) -> Vec<String> {
    let mut warnings = Vec::new();
    match template.id {
        "live-physics-review" | "visual-physics-audio" if options.targets.is_empty() => {
            warnings.push(
                "No target was supplied; target-scoped trace and physics collision-report evidence was not generated. Re-run review init with --target or --trace-target to include it."
                    .to_string(),
            );
        }
        "live-event-review" if options.targets.is_empty() || options.event_provider.is_none() => {
            warnings.push(
                "Event trace requires both --target or --trace-target and --event-provider; no event_trace step was generated without caller-supplied values."
                    .to_string(),
            );
        }
        "deterministic-runtime-review" if options.targets.is_empty() => {
            warnings.push(
                "No target was supplied; deterministic runtime review generated tree/ui evidence instead of target-scoped trace evidence."
                    .to_string(),
            );
        }
        _ => {}
    }
    warnings
}

fn review_template(id: &str) -> Option<&'static ReviewTemplate> {
    built_in_review_templates()
        .iter()
        .find(|template| template.id == id)
}

fn template_inventory(template: &ReviewTemplate) -> Value {
    json!({
        "id": template.id,
        "description": template.description,
        "output_type": template.output_type,
        "generated_command_type": template.generated_command_type,
        "required_parameters": template.required_parameters,
        "optional_parameters": template.optional_parameters,
        "required_tools": template.required_tools
    })
}

fn validate_options(command: &str, options: &ReviewInitOptions) -> Result<(), FailureDetail> {
    validate_output_path(command, &options.out)?;
    if options.frames == 0 || options.frames > MAX_FRAMES {
        return Err(validation_failure(
            command,
            "review_validation_failed",
            "review field 'frames' must be positive and bounded",
            json!({"min": 1, "max": MAX_FRAMES}),
            json!(options.frames),
        ));
    }
    if options.screenshot_every == 0 || options.screenshot_every > options.frames {
        return Err(validation_failure(
            command,
            "review_validation_failed",
            "review field 'screenshot_every' must be positive and no larger than frames",
            json!({"min": 1, "max": options.frames}),
            json!(options.screenshot_every),
        ));
    }
    if options.audio_seconds == 0 || options.audio_seconds > MAX_AUDIO_SECONDS {
        return Err(validation_failure(
            command,
            "review_validation_failed",
            "review field 'audio_seconds' must be positive and bounded",
            json!({"min": 1, "max": MAX_AUDIO_SECONDS}),
            json!(options.audio_seconds),
        ));
    }
    for target in &options.targets {
        if target.trim().is_empty() {
            return Err(validation_failure(
                command,
                "review_validation_failed",
                "review targets must be non-empty strings",
                json!("non-empty target locator"),
                json!(target),
            ));
        }
    }
    if let Some(provider) = options.event_provider.as_ref()
        && provider.trim().is_empty()
    {
        return Err(validation_failure(
            command,
            "review_validation_failed",
            "review field 'event_provider' must be non-empty when supplied",
            json!("non-empty target locator"),
            json!(provider),
        ));
    }
    Ok(())
}

fn validate_output_path(command: &str, out: &Path) -> Result<(), FailureDetail> {
    if out.as_os_str().is_empty() {
        return Err(validation_failure(
            command,
            "review_output_invalid",
            "review init requires an output path",
            json!("non-empty file path"),
            json!(""),
        ));
    }
    if out
        .components()
        .any(|component| component == Component::ParentDir)
    {
        return Err(validation_failure(
            command,
            "review_output_invalid",
            "review init output path must not contain parent-directory traversal",
            json!("path without '..' components"),
            json!(out),
        ));
    }
    Ok(())
}

fn is_res_scene(scene: &str) -> bool {
    let scene = scene.trim();
    scene.starts_with("res://") && scene != "res://"
}

fn resolve_output_path(config: &ProjectConfig, out: &Path) -> PathBuf {
    if out.is_absolute() {
        out.components().collect()
    } else {
        config.project_root.join(out).components().collect()
    }
}

fn write_json_document(path: &Path, document: &Value) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let content = serde_json::to_string_pretty(document).map_err(std::io::Error::other)?;
    fs::write(path, content)
}

fn run_command_for(template: &ReviewTemplate, output_path: &Path) -> String {
    let path = output_path.to_string_lossy();
    if template.output_type == "scenario" {
        format!("godot-qa scenario run --file {path} --project <project> --json")
    } else {
        format!("godot-qa session run-script {path} --project <project> --json")
    }
}

fn init_data(
    template: &ReviewTemplate,
    options: &ReviewInitOptions,
    output_path: Option<&Path>,
    run_command: Option<&str>,
) -> Map<String, Value> {
    let mut data = Map::from_iter([
        ("template".to_string(), json!(template.id)),
        ("output_type".to_string(), json!(template.output_type)),
        (
            "generated_command_type".to_string(),
            json!(template.generated_command_type),
        ),
        ("name".to_string(), json!(options.name.trim())),
        ("scene".to_string(), json!(options.scene.trim())),
        ("project_args".to_string(), json!(options.project_args)),
        ("required_tools".to_string(), json!(template.required_tools)),
        (
            "warnings".to_string(),
            json!(template_warnings(template, options)),
        ),
    ]);
    if let Some(output_path) = output_path {
        data.insert("output_file".to_string(), json!(output_path));
    }
    if let Some(run_command) = run_command {
        data.insert("run_command".to_string(), json!(run_command));
    }
    data
}

fn validation_failure(
    command: &str,
    kind: &str,
    message: &str,
    expected: Value,
    actual: Value,
) -> FailureDetail {
    FailureDetail {
        kind: kind.to_string(),
        command: Some(command.to_string()),
        message: Some(message.to_string()),
        expected: Some(expected),
        actual: Some(actual),
        ..FailureDetail::default()
    }
}

fn review_failure(
    command: &str,
    failures: Vec<FailureDetail>,
    data: Map<String, Value>,
) -> CommandResult {
    CommandResult {
        ok: false,
        command: command.to_string(),
        message: "review template failed".to_string(),
        data,
        failures,
        exit_code: 1,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use godot_qa_core::config::{GodotAudioMode, GodotLaunchMode};
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_dir(name: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!("godot-qa-{name}-{unique}"));
        fs::create_dir_all(&path).unwrap();
        path
    }

    fn config(root: &Path) -> ProjectConfig {
        ProjectConfig {
            project_root: root.to_path_buf(),
            repo_root: root.to_path_buf(),
            verbose: false,
            godot_binary: None,
            qa_dir: root.join("qa"),
            artifacts_dir: root.join(".qa/artifacts"),
            scenarios_dir: root.join("qa/scenarios"),
            addon_dir: root.join("addons/godot_qa"),
            repo_addon_dir: root.join("addons/godot_qa"),
            addon_identity_cache: Default::default(),
            godot_launch_prefix: Vec::new(),
            godot_windowed_launch_prefix: Vec::new(),
            godot_args: Vec::new(),
            godot_project_args: Vec::new(),
            godot_launch_mode: GodotLaunchMode::Auto,
            godot_audio_mode: GodotAudioMode::Auto,
            scenario_warm_default: false,
            scenario_warm_strict_default: false,
            scenario_jobs_default: None,
            project_config_failure: None,
            artifacts_keep_runs: None,
            scenario_schedule_default: None,
            log_ignore_patterns: Vec::new(),
            config_warnings: Vec::new(),
        }
    }

    #[test]
    fn review_template_catalog_contract() {
        let result = run_review_list_templates();
        assert!(result.ok);
        let templates = result.data["templates"].as_array().unwrap();
        assert_eq!(templates.len(), 7);

        let ids = templates
            .iter()
            .map(|template| template["id"].as_str().unwrap())
            .collect::<Vec<_>>();
        assert_eq!(
            ids,
            vec![
                "scenario-smoke",
                "live-visual-review",
                "live-physics-review",
                "live-audio-review",
                "live-event-review",
                "visual-physics-audio",
                "deterministic-runtime-review"
            ]
        );

        for template in templates {
            let id = template["id"].as_str().unwrap();
            assert!(scenario_doc::is_safe_scenario_name(id), "{id}");
            assert!(
                id.chars()
                    .all(|ch| { ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '-' })
            );
            assert!(template["description"].as_str().unwrap().len() > 20);
            assert!(
                ["scenario", "live_script"].contains(&template["output_type"].as_str().unwrap())
            );
            assert!(
                !template["generated_command_type"]
                    .as_str()
                    .unwrap()
                    .is_empty()
            );
            assert!(
                template["required_parameters"]
                    .as_array()
                    .unwrap()
                    .contains(&json!("name"))
            );
            assert!(
                template["required_parameters"]
                    .as_array()
                    .unwrap()
                    .contains(&json!("scene"))
            );
            assert!(
                template["required_parameters"]
                    .as_array()
                    .unwrap()
                    .contains(&json!("out"))
            );
            assert!(!template["required_tools"].as_array().unwrap().is_empty());
        }
    }

    #[test]
    fn review_init_scenario_smoke_document_contract() {
        let root = temp_dir("review-init-scenario-smoke");
        let cfg = config(&root);
        let options = ReviewInitOptions {
            template_id: "scenario-smoke".to_string(),
            name: "smoke_review".to_string(),
            scene: "res://ui/main_menu.tscn".to_string(),
            project_args: vec![
                "--seed".to_string(),
                "123456".to_string(),
                "--save-slot=qa profile".to_string(),
            ],
            frames: 5,
            screenshot_every: 5,
            out: PathBuf::from(".qa/review-scenarios/smoke_review.yaml"),
            ..ReviewInitOptions::default()
        };

        let result = run_review_init(&cfg, &options);
        assert!(result.ok, "{:?}", result.failures);
        assert_eq!(result.data["template"], "scenario-smoke");
        assert_eq!(
            result.data["project_args"],
            json!(["--seed", "123456", "--save-slot=qa profile"])
        );

        let output_path = root.join(".qa/review-scenarios/smoke_review.yaml");
        let document: Value =
            serde_json::from_str(&fs::read_to_string(&output_path).unwrap()).unwrap();
        assert_eq!(document["name"], "smoke_review");
        assert_eq!(document["scene"], "res://ui/main_menu.tscn");
        assert_eq!(
            document["project_args"],
            json!(["--seed", "123456", "--save-slot=qa profile"])
        );
        assert_eq!(
            document["steps"],
            json!([{"wait_frames": 5}, {"snapshot": "after_wait"}])
        );
        assert_eq!(document["assert"], json!([{"no_errors": true}]));
        assert_eq!(document["review"]["template"], "scenario-smoke");
        assert_eq!(
            document["review"]["generated_command_type"],
            "scenario run --file"
        );
        assert!(
            document["review"]["run_command"]
                .as_str()
                .unwrap()
                .contains("godot-qa scenario run --file")
        );
    }
}
