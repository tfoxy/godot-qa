use crate::frame_pacing::{
    time_control_fixed_fps_conflict_failure, validate_fixed_fps_for_scenario,
};
use crate::time_control::validate_time_control_for_scenario;
use godot_qa_core::results::FailureDetail;
use serde_json::{Map, Value, json};
use std::collections::HashMap;
use std::error::Error;
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};

const REQUIRED_FIELDS: [&str; 3] = ["name", "scene", "steps"];
const LAYOUT_AUDIT_MODES: [&str; 2] = ["default", "strict"];

/// Every step operation the scenario runner understands. A step map may only
/// carry one of these keys (plus the shared timeout siblings). Unknown keys are
/// rejected during validation instead of being silently ignored/normalized.
const KNOWN_STEP_KEYS: &[&str] = &[
    "wait_frames",
    "wait_seconds",
    "wait_until",
    "snapshot",
    "tap_action",
    "hold_action",
    "release_action",
    "input_joy_button",
    "input_joy_motion",
    "input_joy_state",
    "drive_policy",
    "click",
    "node_inspect",
    "record_state",
    "teleport_node",
    "fixture_teleport",
    "spawn_scene",
    "set_property",
    "set_state",
    "joy_connect",
    "joy_disconnect",
    "call_method",
    "fixture_call",
    "audio_capture",
    "audio_export",
    "physics_overlay",
    "assert",
    "no_errors",
];

/// Every assertion the scenario runner understands. An assertion map may only
/// carry one of these keys (plus the shared timeout siblings).
const KNOWN_ASSERTION_KEYS: &[&str] = &[
    "exists",
    "not_exists",
    "visible",
    "hidden",
    "enabled",
    "disabled",
    "focused",
    "inside_viewport",
    "text_equals",
    "text_contains",
    "no_overlap",
    "no_errors",
    "layout_audit",
    "screenshot_match",
    "focus_path",
    "max_delta_position",
    "follows",
    "within_limits",
    "never",
    "observe_until",
    "distance",
    "distance_between",
    "distance_changed",
    "visible_pixels",
    "saved_snapshot_pixels",
    "not_occluded",
    "renders_above",
    "renders_below",
    "surface_probe",
    "surface_matches",
    "state_matches",
    "trend",
    "frame_time",
];

/// Operation-local metadata that may accompany the single step/assertion key.
const STEP_ASSERTION_SIBLING_KEYS: &[&str] = &["timeout", "timeout_seconds"];

/// State-source vocabulary shared by `state_matches` and `wait_until`.
const STATE_MATCH_SOURCES: &[&str] = &[
    "provider",
    "method",
    "qa_method",
    "metadata",
    "meta",
    "inspect",
    "auto",
];

/// Comparison operators understood by the runtime `state_matches` evaluator
/// (kept in sync with `_state_match_operators()` in qa_assertions_impl.gd).
const STATE_MATCH_OPERATORS: &[&str] = &[
    "equals",
    "contains",
    "present",
    "absent",
    "approx_equals",
    "approximately_equals",
    "greater_than",
    "less_than",
    "within",
];

/// Operators that require an `expected` value.
const OPERATORS_REQUIRING_EXPECTED: &[&str] = &[
    "equals",
    "contains",
    "approx_equals",
    "approximately_equals",
    "greater_than",
    "less_than",
    "within",
];

/// Trend directions understood by `assert.temporal.trend` (kept in sync with
/// `TREND_DIRECTIONS` in qa_assertions_impl.gd).
const TREND_DIRECTIONS: &[&str] = &[
    "increasing",
    "decreasing",
    "non_increasing",
    "non_decreasing",
];

/// Frame-timing bounds accepted by the `frame_time` assertion; at least one is
/// required.
const FRAME_TIME_BOUND_FIELDS: &[&str] = &["max_avg_ms", "max_p95_ms", "min_fps"];
const SURFACE_COORDINATE_SPACES: [&str; 3] = ["world_2d", "viewport_2d", "world_3d"];
const SURFACE_MAX_POINTS: usize = 32;
const SURFACE_MAX_RADIUS: f64 = 1024.0;
const AUDIO_CAPTURE_MAX_DURATION_SECONDS: f64 = 60.0;

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

/// One runnable scenario file as discovery sees it: the name it is addressed by
/// (declared `name`, else the file stem) and the file it came from.
struct ScenarioEntry {
    name: String,
    path: PathBuf,
}

/// Walk `scenarios_dir` once and return every runnable scenario, sorted by
/// (name, path). This is the single discovery pass behind `list_scenarios`,
/// `find_scenarios_by_name`, and `ScenarioIndex`; each file is read exactly once.
fn list_scenario_entries(scenarios_dir: &Path) -> Vec<ScenarioEntry> {
    if !scenarios_dir.exists() {
        return Vec::new();
    }

    let mut scenario_paths = Vec::new();
    collect_scenario_paths(scenarios_dir, &mut scenario_paths);

    let mut entries: Vec<ScenarioEntry> = scenario_paths
        .into_iter()
        .filter_map(|path| {
            let header = read_scenario_header(&path);
            // Abstract templates (`abstract: true`) are not runnable scenarios; they
            // are excluded from listing, and therefore from run-by-name, bundle
            // resolution, and lint's duplicate-scenario-name index.
            if header.is_abstract {
                return None;
            }
            let name = header.name.unwrap_or_else(|| path.file_stem_string());
            Some(ScenarioEntry { name, path })
        })
        .collect();
    entries.sort_by(|a, b| {
        a.name
            .cmp(&b.name)
            .then_with(|| a.path.to_string_lossy().cmp(&b.path.to_string_lossy()))
    });
    entries
}

pub fn list_scenarios(scenarios_dir: &Path) -> Vec<Value> {
    list_scenario_entries(scenarios_dir)
        .into_iter()
        .map(|entry| {
            json!({
                "name": entry.name,
                "path": entry.path,
            })
        })
        .collect()
}

/// Name-to-paths index over one `list_scenario_entries` pass.
///
/// Callers that resolve many scenario names against the same `scenarios_dir`
/// (bundle children, most of all) should build this once instead of calling
/// `find_scenarios_by_name` per name, which re-walks and re-reads the whole
/// scenarios tree every time. Lookups return the same paths, in the same order,
/// that `find_scenarios_by_name` would return for that name.
///
/// The index is a snapshot: it is not invalidated by scenario files changing on
/// disk after `load`.
pub struct ScenarioIndex {
    by_name: HashMap<String, Vec<PathBuf>>,
}

impl ScenarioIndex {
    /// Build the index with a single discovery pass over `scenarios_dir`.
    pub fn load(scenarios_dir: &Path) -> Self {
        let mut by_name: HashMap<String, Vec<PathBuf>> = HashMap::new();
        // `list_scenario_entries` is sorted by (name, path), so pushing in
        // iteration order preserves `find_scenarios_by_name` ordering per name.
        for entry in list_scenario_entries(scenarios_dir) {
            by_name.entry(entry.name).or_default().push(entry.path);
        }
        Self { by_name }
    }

    /// Every scenario file declaring `name` (exact match), in listing order.
    /// Empty when nothing matches.
    pub fn paths_for(&self, name: &str) -> &[PathBuf] {
        self.by_name
            .get(name)
            .map(Vec::as_slice)
            .unwrap_or_default()
    }
}

pub fn find_scenario_by_name(scenarios_dir: &Path, name: &str) -> Option<PathBuf> {
    find_scenarios_by_name(scenarios_dir, name)
        .into_iter()
        .next()
}

pub fn find_scenarios_by_name(scenarios_dir: &Path, name: &str) -> Vec<PathBuf> {
    list_scenario_entries(scenarios_dir)
        .into_iter()
        .filter(|entry| entry.name == name)
        .map(|entry| entry.path)
        .collect()
}

pub fn is_safe_scenario_name(name: &str) -> bool {
    is_safe_artifact_segment(name.trim())
}

pub fn load_scenario_document(path: &Path) -> Result<Value, ScenarioError> {
    let data = parse_scenario_file(path)?;
    // Resolve scenario templating (`extends` + `vars` + `abstract`). Files with
    // no template keys take a fast path and are returned untouched. Every
    // consumer funnels through here, so downstream sees plain resolved docs.
    crate::template::resolve_scenario_document(path, data)
}

/// Read and parse a scenario file into its raw root-object document, without
/// resolving templating. This is the primitive that both `load_scenario_document`
/// and the template resolver's base loading share.
pub(crate) fn parse_scenario_file(path: &Path) -> Result<Value, ScenarioError> {
    let raw_text = fs::read_to_string(path).map_err(|error| {
        scenario_parse_failure(
            path,
            &error.to_string(),
            json!(python_type_name_from_error(&error)),
        )
    })?;
    parse_scenario_value(path, &raw_text)
}

/// Parse already-read scenario text into a root-object document. The extension
/// selects JSON vs YAML parsing (YAML is a strict superset of JSON, so
/// flow/JSON-style `.yaml` scenarios still parse through here).
pub(crate) fn parse_scenario_value(path: &Path, raw_text: &str) -> Result<Value, ScenarioError> {
    let data = match path.extension().and_then(|ext| ext.to_str()) {
        Some("json") => serde_json::from_str::<Value>(raw_text).map_err(|error| {
            scenario_parse_failure(path, &error.to_string(), json!("JSONDecodeError"))
        })?,
        _ => godot_qa_core::yaml::parse_yaml_document(raw_text)
            .map_err(|error| yaml_parse_failure(path, &error))?,
    };

    if !data.is_object() {
        return Err(scenario_parse_failure(
            path,
            "scenario document root must be an object",
            json!(python_type_name(&data)),
        ));
    }
    Ok(data)
}

pub fn validate_scenario_document(data: &Value, path: &Path) -> Result<(), ScenarioError> {
    let Some(document) = data.as_object() else {
        return Err(validation_failure(
            path,
            "scenario document root must be an object",
            json!("object"),
            json!(python_type_name(data)),
        ));
    };

    let missing_fields: Vec<Value> = REQUIRED_FIELDS
        .iter()
        .filter(|field| !document.contains_key(**field))
        .map(|field| json!(field))
        .collect();
    if !missing_fields.is_empty() {
        return Err(validation_failure(
            path,
            "scenario document is missing required fields",
            json!(REQUIRED_FIELDS),
            json!({ "missing_fields": missing_fields }),
        ));
    }

    let name = require_non_empty_string(
        document,
        "name",
        "scenario field 'name' must be a non-empty string",
        path,
    )?;
    let _scene = require_non_empty_string(
        document,
        "scene",
        "scenario field 'scene' must be a non-empty string",
        path,
    )?;
    let steps = document
        .get("steps")
        .and_then(Value::as_array)
        .ok_or_else(|| {
            validation_failure(
                path,
                "scenario field 'steps' must be a list",
                json!("list"),
                json!(python_type_name(
                    document.get("steps").unwrap_or(&Value::Null)
                )),
            )
        })?;

    if !is_safe_artifact_segment(name.trim()) {
        return Err(validation_failure(
            path,
            "scenario field 'name' must be a safe artifact path segment",
            json!("letters, numbers, dot, underscore, or hyphen"),
            document.get("name").cloned().unwrap_or(Value::Null),
        ));
    }

    for step in steps {
        let Some(step_object) = step.as_object() else {
            continue;
        };
        if let Some(snapshot_name) = step_object.get("snapshot") {
            let Some(snapshot_name) = snapshot_name.as_str() else {
                return Err(validation_failure(
                    path,
                    "snapshot step name must be a safe artifact path segment",
                    json!("letters, numbers, dot, underscore, or hyphen"),
                    snapshot_name.clone(),
                ));
            };
            if !is_safe_artifact_segment(snapshot_name.trim()) {
                return Err(validation_failure(
                    path,
                    "snapshot step name must be a safe artifact path segment",
                    json!("letters, numbers, dot, underscore, or hyphen"),
                    json!(snapshot_name),
                ));
            }
        }
    }

    for field_name in ["viewport", "determinism", "snapshots"] {
        if let Some(value) = document.get(field_name) {
            if !value.is_object() {
                return Err(validation_failure(
                    path,
                    &format!("scenario field '{field_name}' must be an object"),
                    json!("object"),
                    json!(python_type_name(value)),
                ));
            }
        }
    }
    if let Some(time_control) = document.get("time_control") {
        validate_time_control_for_scenario(time_control, path)
            .map_err(|failure| ScenarioError { failure })?;
    }
    validate_scenario_timeout(document, path)?;
    if let Some(determinism) = document.get("determinism").and_then(Value::as_object) {
        if document.get("time_control").is_some() && determinism.get("fixed_fps").is_some() {
            return Err(ScenarioError {
                failure: time_control_fixed_fps_conflict_failure(
                    "scenario_validation_failed",
                    Some(path),
                    None,
                ),
            });
        }
        if let Some(fixed_fps) = determinism.get("fixed_fps") {
            validate_fixed_fps_for_scenario(fixed_fps, path)
                .map_err(|failure| ScenarioError { failure })?;
        }
        validate_temporal_sampling(determinism, path)?;
    }
    if let Some(snapshots) = document.get("snapshots").and_then(Value::as_object) {
        if let Some(value) = snapshots.get("fail_on_low_information") {
            if !value.is_boolean() {
                return Err(validation_failure(
                    path,
                    "snapshots.fail_on_low_information must be a boolean",
                    json!("boolean"),
                    json!(python_type_name(value)),
                ));
            }
        }
        validate_snapshot_filters(snapshots, path)?;
    }

    if let Some(value) = document.get("assert") {
        if !value.is_array() {
            return Err(validation_failure(
                path,
                "scenario field 'assert' must be a list",
                json!("list"),
                json!(python_type_name(value)),
            ));
        }
    }

    if let Some(viewports) = document.get("viewports") {
        validate_viewports(viewports, path)?;
    }
    if let Some(policy) = document.get("process_outcome_policy") {
        validate_process_outcome_policy(policy, path)?;
    }
    if let Some(execution) = document.get("execution") {
        validate_execution(execution, path)?;
    }
    validate_argument_contract(document, path)?;

    validate_supported_step_payloads(steps, path)?;
    if let Some(assertions) = document.get("assert").and_then(Value::as_array) {
        validate_supported_assertion_payloads(assertions, path)?;
    }
    validate_frame_time_frame_pacing_conflict(document, steps, path)?;
    validate_screenshot_match_viewport_matrix_conflict(document, path)?;

    Ok(())
}

/// `screenshot_match` compares a rendered snapshot against a single host-viewport
/// baseline, so it has no meaning across a multi-entry `viewports` matrix (each
/// cell renders at a different resolution with no per-cell baseline contract).
/// Reject the combination statically — the compile-time mirror of the runtime
/// `screenshot_match_viewports_unsupported` guard — so authors learn at lint time
/// instead of at run time. A single `viewports` entry (or none) is fine: it just
/// sets the window resolution and `screenshot_match` runs the normal single path.
fn validate_screenshot_match_viewport_matrix_conflict(
    document: &Map<String, Value>,
    path: &Path,
) -> Result<(), ScenarioError> {
    let viewports = document
        .get("viewports")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or(&[]);
    if viewports.len() <= 1 {
        return Ok(());
    }
    let has_screenshot_match = document
        .get("assert")
        .and_then(Value::as_array)
        .is_some_and(|assertions| {
            assertions
                .iter()
                .any(|assertion| assertion.get("screenshot_match").is_some())
        });
    if !has_screenshot_match {
        return Ok(());
    }
    Err(validation_failure(
        path,
        "screenshot_match does not support multi-viewport scenarios; use a single viewport or drop the 'viewports' matrix",
        json!("single viewport scenario without a multi-entry 'viewports' matrix"),
        json!(viewports),
    ))
}

/// `frame_time` asserts wall-clock frame durations, which are meaningless when
/// `determinism.fixed_fps` forces frame pacing: the run simulates a fixed frame
/// delta while processing frames as fast as it can. Reject the combination so
/// authors either drop `fixed_fps` or keep the perf gate in a separate
/// realtime-pacing perf scenario.
fn validate_frame_time_frame_pacing_conflict(
    document: &Map<String, Value>,
    steps: &[Value],
    path: &Path,
) -> Result<(), ScenarioError> {
    let Some(fixed_fps) = document
        .get("determinism")
        .and_then(Value::as_object)
        .and_then(|determinism| determinism.get("fixed_fps"))
    else {
        return Ok(());
    };
    let top_level = document
        .get("assert")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default();
    let step_local = steps
        .iter()
        .filter_map(|step| step.get("assert").and_then(Value::as_array))
        .flatten();
    let declares_frame_time = top_level
        .iter()
        .chain(step_local)
        .any(|assertion| assertion.get("frame_time").is_some());
    if declares_frame_time {
        return Err(validation_failure(
            path,
            "frame_time assertions cannot be combined with determinism.fixed_fps: forced frame pacing makes wall-clock frame times meaningless; drop fixed_fps or move the frame_time gate into a separate perf scenario",
            json!("no determinism.fixed_fps alongside frame_time assertions"),
            json!({ "determinism.fixed_fps": fixed_fps }),
        ));
    }
    Ok(())
}

fn validate_scenario_timeout(
    document: &Map<String, Value>,
    path: &Path,
) -> Result<(), ScenarioError> {
    let scenario = Value::Object(document.clone());
    let declared = crate::run::declared_scenario_timeout_fields(&scenario);
    if declared.len() > 1 {
        return Err(validation_failure(
            path,
            "scenario timeout must use exactly one of timeout_seconds, timeout, run_timeout_seconds, or run_timeout",
            json!("one timeout field"),
            json!(declared),
        ));
    }
    for field in ["timeout_seconds", "run_timeout_seconds"] {
        if let Some(value) = document.get(field) {
            if crate::run::parse_timeout_seconds_value(value).is_none() {
                return Err(invalid_scenario_timeout(path, value.clone()));
            }
        }
    }
    for field in ["timeout", "run_timeout"] {
        if let Some(value) = document.get(field) {
            if crate::run::parse_timeout_duration_string(value).is_none() {
                return Err(invalid_scenario_timeout(path, value.clone()));
            }
        }
    }
    Ok(())
}

fn invalid_scenario_timeout(path: &Path, actual: Value) -> ScenarioError {
    validation_failure(
        path,
        "scenario timeout must be a positive finite duration",
        json!("positive number of seconds or duration string with ms, s, or m suffix"),
        actual,
    )
}

pub fn normalize_scenario_document(data: &Value) -> Value {
    let Some(document) = data.as_object() else {
        return json!({});
    };

    let steps = document
        .get("steps")
        .and_then(Value::as_array)
        .map(|steps| steps.iter().map(normalize_step).collect::<Vec<_>>())
        .unwrap_or_default();
    let assertions = document
        .get("assert")
        .and_then(Value::as_array)
        .map(|assertions| {
            assertions
                .iter()
                .map(normalize_assertion)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    let mut normalized = json!({
        "name": document.get("name").and_then(Value::as_str).unwrap_or_default().trim(),
        "scene": document.get("scene").and_then(Value::as_str).unwrap_or_default().trim(),
        "viewport": clone_object_or_default(document.get("viewport")),
        "determinism": clone_object_or_default(document.get("determinism")),
        "snapshots": normalize_snapshots(document.get("snapshots")),
        "steps": steps,
        "assert": assertions,
        "viewports": clone_array_or_default(document.get("viewports")),
    });
    if let Some(godot_args) = document.get("godot_args").and_then(Value::as_array) {
        normalized["godot_args"] = Value::Array(godot_args.clone());
    }
    if let Some(project_args) = document.get("project_args").and_then(Value::as_array) {
        normalized["project_args"] = Value::Array(project_args.clone());
    }
    for field in crate::run::SCENARIO_TIMEOUT_FIELDS {
        if let Some(timeout) = document.get(field) {
            normalized[field] = timeout.clone();
        }
    }
    if let Some(threshold) = document.get("slow_summary_threshold_ms") {
        normalized["slow_summary_threshold_ms"] = threshold.clone();
    }
    if let Some(slow_summary) = document.get("slow_summary").and_then(Value::as_object) {
        normalized["slow_summary"] = Value::Object(slow_summary.clone());
    }
    if let Some(policy) = document
        .get("process_outcome_policy")
        .and_then(Value::as_object)
    {
        normalized["process_outcome_policy"] = Value::Object(policy.clone());
    }
    if let Some(execution) = document.get("execution").and_then(Value::as_object) {
        normalized["execution"] = Value::Object(execution.clone());
    }
    if let Some(time_control) = document.get("time_control") {
        if let Ok(settings) = validate_time_control_for_scenario(time_control, Path::new("")) {
            normalized["time_control"] = settings.to_normalized_value();
        }
    }
    normalized
}

// The optional `execution` block controls per-scenario process isolation.
// `execution.isolation` is "warm" (default, warm-eligible) or "cold" (always run
// in a fresh isolated process, opting out of the warm in-process runner).
fn validate_execution(value: &Value, path: &Path) -> Result<(), ScenarioError> {
    let Some(execution) = value.as_object() else {
        return Err(validation_failure(
            path,
            "scenario field 'execution' must be an object",
            json!("object"),
            json!(python_type_name(value)),
        ));
    };
    if let Some(isolation) = execution.get("isolation") {
        let valid = isolation
            .as_str()
            .map(|isolation| matches!(isolation, "warm" | "cold"))
            .unwrap_or(false);
        if !valid {
            return Err(validation_failure(
                path,
                "scenario field 'execution.isolation' must be \"warm\" or \"cold\"",
                json!("\"warm\" or \"cold\""),
                isolation.clone(),
            ));
        }
    }
    Ok(())
}

fn validate_process_outcome_policy(value: &Value, path: &Path) -> Result<(), ScenarioError> {
    let Some(policy) = value.as_object() else {
        return Err(validation_failure(
            path,
            "scenario field 'process_outcome_policy' must be an object",
            json!("object"),
            json!(python_type_name(value)),
        ));
    };
    let Some(exit_codes) = policy.get("known_wrapper_exit_codes") else {
        return Err(validation_failure(
            path,
            "process_outcome_policy.known_wrapper_exit_codes must be a non-empty list of positive integers",
            json!("non-empty list of positive integers"),
            Value::Null,
        ));
    };
    let Some(exit_codes) = exit_codes.as_array() else {
        return Err(validation_failure(
            path,
            "process_outcome_policy.known_wrapper_exit_codes must be a non-empty list of positive integers",
            json!("non-empty list of positive integers"),
            json!(python_type_name(exit_codes)),
        ));
    };
    if exit_codes.is_empty()
        || exit_codes
            .iter()
            .any(|value| !value.as_i64().is_some_and(|code| code > 0))
    {
        return Err(validation_failure(
            path,
            "process_outcome_policy.known_wrapper_exit_codes must be a non-empty list of positive integers",
            json!("non-empty list of positive integers"),
            Value::Array(exit_codes.clone()),
        ));
    }
    Ok(())
}

fn validate_argument_contract(
    document: &Map<String, Value>,
    path: &Path,
) -> Result<(), ScenarioError> {
    if let Some(value) = document.get("launch_args") {
        return Err(validation_failure(
            path,
            "scenario field 'launch_args' is not supported; use 'godot_args' for Godot engine flags before '--' or 'project_args' for reserved project/game flags after '--'",
            json!("godot_args or project_args"),
            value.clone(),
        ));
    }
    for field_name in ["godot_args", "project_args"] {
        let Some(value) = document.get(field_name) else {
            continue;
        };
        let Some(args) = value.as_array() else {
            return Err(validation_failure(
                path,
                &format!("scenario field '{field_name}' must be a list of non-empty strings"),
                json!("list of non-empty strings"),
                json!(python_type_name(value)),
            ));
        };
        for (index, arg) in args.iter().enumerate() {
            let Some(arg_value) = arg.as_str() else {
                return Err(validation_failure(
                    path,
                    &format!("scenario field '{field_name}' must contain only strings"),
                    json!("string"),
                    json!({"index": index, "actual": python_type_name(arg)}),
                ));
            };
            if arg_value.is_empty() {
                return Err(validation_failure(
                    path,
                    &format!("scenario field '{field_name}' must contain only non-empty strings"),
                    json!("non-empty string"),
                    json!({"index": index, "actual": arg_value}),
                ));
            }
        }
    }
    Ok(())
}

pub fn load_and_normalize_scenario_document(path: &Path) -> Result<Value, ScenarioError> {
    let document = load_scenario_document(path)?;
    validate_scenario_document(&document, path)?;
    Ok(normalize_scenario_document(&document))
}

fn collect_scenario_paths(dir: &Path, paths: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_scenario_paths(&path, paths);
            continue;
        }
        if matches!(
            path.extension().and_then(|ext| ext.to_str()),
            Some("yaml" | "yml" | "json")
        ) {
            paths.push(path);
        }
    }
}

/// The two facts discovery needs from a scenario file: the declared top-level
/// `name` (`None` when absent, empty, or unreadable/unparsable, so the caller
/// falls back to the file stem) and whether it is an abstract template.
struct ScenarioHeader {
    name: Option<String>,
    is_abstract: bool,
}

/// Read a scenario file once and derive both header facts from that read.
///
/// An unreadable file, or a `.json` file that does not parse, yields no declared
/// name (stem fallback) and is not abstract.
fn read_scenario_header(path: &Path) -> ScenarioHeader {
    let Ok(raw_text) = fs::read_to_string(path) else {
        return ScenarioHeader {
            name: None,
            is_abstract: false,
        };
    };
    match path.extension().and_then(|ext| ext.to_str()) {
        Some("json") => json_scenario_header(&raw_text),
        _ => yaml_scenario_header(&raw_text),
    }
}

fn json_scenario_header(raw_text: &str) -> ScenarioHeader {
    let Ok(data) = serde_json::from_str::<Value>(raw_text) else {
        return ScenarioHeader {
            name: None,
            is_abstract: false,
        };
    };
    ScenarioHeader {
        name: data
            .get("name")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_owned),
        is_abstract: data.get("abstract") == Some(&Value::Bool(true)),
    }
}

/// Scan top-level `name:` / `abstract:` lines without a full YAML parse. Only
/// the first top-level occurrence of each key counts, mirroring how a YAML
/// document's first mapping entry wins.
fn yaml_scenario_header(raw_text: &str) -> ScenarioHeader {
    let mut header = ScenarioHeader {
        name: None,
        is_abstract: false,
    };
    let mut name_seen = false;
    let mut abstract_seen = false;
    for line in raw_text.lines() {
        if name_seen && abstract_seen {
            break;
        }
        let stripped = line.trim();
        if stripped.is_empty()
            || stripped.starts_with('#')
            || line.starts_with(' ')
            || line.starts_with('\t')
        {
            continue;
        }
        if !name_seen {
            if let Some(value) = stripped.strip_prefix("name:") {
                name_seen = true;
                let value = value.trim();
                if !value.is_empty() {
                    header.name = Some(strip_wrapping_quotes(value));
                }
                continue;
            }
        }
        if !abstract_seen {
            if let Some(value) = stripped.strip_prefix("abstract:") {
                abstract_seen = true;
                // Accept `abstract: true` with an optional trailing comment.
                header.is_abstract = value.split_whitespace().next() == Some("true");
            }
        }
    }
    header
}

/// Cheaply detect whether a scenario file declares `abstract: true` at the top
/// level, without a full parse. Abstract templates are excluded from listing.
pub(crate) fn is_abstract_scenario_file(path: &Path) -> bool {
    read_scenario_header(path).is_abstract
}

/// Enumerate abstract template files under `scenarios_dir` (sorted), the mirror
/// of `list_scenarios` for templates. Used by lint's template-validation pass.
pub fn list_abstract_scenario_files(scenarios_dir: &Path) -> Vec<PathBuf> {
    if !scenarios_dir.exists() {
        return Vec::new();
    }
    let mut paths = Vec::new();
    collect_scenario_paths(scenarios_dir, &mut paths);
    let mut abstract_paths: Vec<PathBuf> = paths
        .into_iter()
        .filter(|path| is_abstract_scenario_file(path))
        .collect();
    abstract_paths.sort();
    abstract_paths
}

fn require_non_empty_string<'a>(
    document: &'a Map<String, Value>,
    field: &str,
    message: &str,
    path: &Path,
) -> Result<&'a str, ScenarioError> {
    document
        .get(field)
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| {
            validation_failure(
                path,
                message,
                json!("non-empty string"),
                document.get(field).cloned().unwrap_or(Value::Null),
            )
        })
}

fn validate_viewports(viewports: &Value, path: &Path) -> Result<(), ScenarioError> {
    let Some(viewports) = viewports.as_array() else {
        return Err(validation_failure(
            path,
            "scenario field 'viewports' must be a list",
            json!("list"),
            json!(python_type_name(viewports)),
        ));
    };

    for viewport in viewports {
        let Some(parts) = viewport.as_array() else {
            return Err(validation_failure(
                path,
                "viewports entries must be [width, height] arrays",
                json!("two-integer array"),
                viewport.clone(),
            ));
        };
        if parts.len() != 2 {
            return Err(validation_failure(
                path,
                "viewports entries must be [width, height] arrays",
                json!("two-integer array"),
                viewport.clone(),
            ));
        }
        let (Some(width), Some(height)) = (parts[0].as_i64(), parts[1].as_i64()) else {
            return Err(validation_failure(
                path,
                "viewports width and height must be positive integers",
                json!("positive integers"),
                viewport.clone(),
            ));
        };
        if width <= 0 || height <= 0 {
            return Err(validation_failure(
                path,
                "viewports width and height must be positive integers",
                json!("positive integers"),
                viewport.clone(),
            ));
        }
    }
    Ok(())
}

fn reject_unknown_keys(
    object: &Map<String, Value>,
    known: &[&str],
    kind_label: &str,
    index: usize,
    path: &Path,
) -> Result<(), ScenarioError> {
    for key in object.keys() {
        if known.contains(&key.as_str()) || STEP_ASSERTION_SIBLING_KEYS.contains(&key.as_str()) {
            continue;
        }
        return Err(validation_failure(
            path,
            &format!("{kind_label} {index} uses unsupported key '{key}'"),
            json!(known),
            json!({ "unknown_key": key, "index": index }),
        ));
    }
    Ok(())
}

fn validate_supported_step_payloads(steps: &[Value], path: &Path) -> Result<(), ScenarioError> {
    // `record_state` names key the top-level `state_records` evidence map, so a
    // duplicate name across the scenario's record_state steps would silently
    // overwrite earlier evidence. Reject collisions during validation.
    let mut record_state_names: Vec<String> = Vec::new();
    for (index, step) in steps.iter().enumerate() {
        let Some(step) = step.as_object() else {
            continue;
        };

        reject_unknown_keys(step, KNOWN_STEP_KEYS, "step", index, path)?;

        if let Some(wait_until) = step.get("wait_until") {
            validate_wait_until_step(path, wait_until)?;
        }

        if let Some(wait_frames) = step.get("wait_frames") {
            if wait_frames.as_i64().is_none()
                || !wait_frames.is_i64()
                || wait_frames.as_i64().unwrap() < 0
            {
                return Err(validation_failure(
                    path,
                    "wait_frames step value must be a non-negative integer",
                    json!("non-negative integer"),
                    wait_frames.clone(),
                ));
            }
        }

        if let Some(tap_action) = step.get("tap_action") {
            if !is_non_empty_string(tap_action) {
                return Err(validation_failure(
                    path,
                    "tap_action step value must be a non-empty string",
                    json!("non-empty string"),
                    tap_action.clone(),
                ));
            }
        }

        if let Some(hold_action) = step.get("hold_action") {
            let Some(hold_action) = hold_action.as_object() else {
                return Err(validation_failure(
                    path,
                    "hold_action step value must be an object",
                    json!("object"),
                    json!(python_type_name(hold_action)),
                ));
            };
            if !hold_action
                .get("action")
                .map(is_non_empty_string)
                .unwrap_or(false)
            {
                return Err(validation_failure(
                    path,
                    "hold_action step field 'action' must be a non-empty string",
                    json!("non-empty string"),
                    hold_action.get("action").cloned().unwrap_or(Value::Null),
                ));
            }
            let frames = hold_action.get("frames").and_then(Value::as_i64);
            if frames.is_none()
                || !hold_action.get("frames").unwrap_or(&Value::Null).is_i64()
                || frames.unwrap() < 0
            {
                return Err(validation_failure(
                    path,
                    "hold_action step field 'frames' must be a non-negative integer",
                    json!("non-negative integer"),
                    hold_action.get("frames").cloned().unwrap_or(Value::Null),
                ));
            }
        }

        if let Some(release_action) = step.get("release_action") {
            if !is_non_empty_string(release_action) {
                return Err(validation_failure(
                    path,
                    "release_action step value must be a non-empty string",
                    json!("non-empty string"),
                    release_action.clone(),
                ));
            }
        }

        if let Some(input_joy_button) = step.get("input_joy_button") {
            validate_input_joy_button_step(input_joy_button, path)?;
        }

        if let Some(input_joy_motion) = step.get("input_joy_motion") {
            validate_input_joy_motion_step(input_joy_motion, path)?;
        }

        if let Some(input_joy_state) = step.get("input_joy_state") {
            validate_input_joy_state_step(input_joy_state, path)?;
        }

        if let Some(joy_connect) = step.get("joy_connect") {
            validate_joy_connection_step(joy_connect, path, "joy_connect", true)?;
        }

        if let Some(joy_disconnect) = step.get("joy_disconnect") {
            validate_joy_connection_step(joy_disconnect, path, "joy_disconnect", false)?;
        }

        if let Some(drive_policy) = step.get("drive_policy") {
            validate_drive_policy_step(path, drive_policy)?;
        }

        if let Some(click) = step.get("click") {
            if !is_non_empty_string(click) {
                return Err(validation_failure(
                    path,
                    "click step value must be a non-empty string",
                    json!("non-empty string"),
                    click.clone(),
                ));
            }
        }

        if let Some(node_inspect) = step.get("node_inspect") {
            if !is_non_empty_string(node_inspect) {
                return Err(validation_failure(
                    path,
                    "node_inspect step value must be a non-empty string",
                    json!("non-empty target locator"),
                    node_inspect.clone(),
                ));
            }
        }

        if let Some(record_state) = step.get("record_state") {
            validate_record_state_step(path, record_state, &mut record_state_names)?;
        }

        if let Some(teleport_node) = step.get("teleport_node") {
            let Some(teleport_node) = teleport_node.as_object() else {
                return Err(validation_failure(
                    path,
                    "teleport_node step value must be an object",
                    json!("object"),
                    json!(python_type_name(teleport_node)),
                ));
            };
            if !teleport_node
                .get("target")
                .map(is_non_empty_string)
                .unwrap_or(false)
            {
                return Err(validation_failure(
                    path,
                    "teleport_node step field 'target' must be a non-empty string",
                    json!("non-empty string"),
                    teleport_node.get("target").cloned().unwrap_or(Value::Null),
                ));
            }
            validate_optional_position_or_locator(path, "teleport_node", teleport_node, true)?;
            validate_optional_bool_field(
                path,
                "teleport_node",
                teleport_node,
                "preserve_velocity",
            )?;
        }

        if let Some(fixture_teleport) = step.get("fixture_teleport") {
            let Some(fixture_teleport) = fixture_teleport.as_object() else {
                return Err(validation_failure(
                    path,
                    "fixture_teleport step value must be an object",
                    json!("object"),
                    json!(python_type_name(fixture_teleport)),
                ));
            };
            if !fixture_teleport
                .get("target")
                .map(is_non_empty_string)
                .unwrap_or(false)
            {
                return Err(validation_failure(
                    path,
                    "fixture_teleport step field 'target' must be a non-empty string",
                    json!("non-empty string"),
                    fixture_teleport
                        .get("target")
                        .cloned()
                        .unwrap_or(Value::Null),
                ));
            }
            validate_optional_position_or_locator(
                path,
                "fixture_teleport",
                fixture_teleport,
                true,
            )?;
            validate_optional_bool_field(
                path,
                "fixture_teleport",
                fixture_teleport,
                "preserve_velocity",
            )?;
        }

        if let Some(spawn_scene) = step.get("spawn_scene") {
            let Some(spawn_scene) = spawn_scene.as_object() else {
                return Err(validation_failure(
                    path,
                    "spawn_scene step value must be an object",
                    json!("object"),
                    json!(python_type_name(spawn_scene)),
                ));
            };
            if !spawn_scene
                .get("scene")
                .map(is_non_empty_string)
                .unwrap_or(false)
            {
                return Err(validation_failure(
                    path,
                    "spawn_scene step field 'scene' must be a non-empty string",
                    json!("non-empty string"),
                    spawn_scene.get("scene").cloned().unwrap_or(Value::Null),
                ));
            }
            for field_name in ["parent", "name", "qa_id", "at"] {
                if let Some(value) = spawn_scene.get(field_name) {
                    if !is_non_empty_string(value) {
                        return Err(validation_failure(
                            path,
                            &format!(
                                "spawn_scene step field '{field_name}' must be a non-empty string"
                            ),
                            json!("non-empty string"),
                            value.clone(),
                        ));
                    }
                }
            }
            validate_optional_position_or_locator(path, "spawn_scene", spawn_scene, false)?;
        }

        if let Some(set_property) = step.get("set_property") {
            let Some(set_property) = set_property.as_object() else {
                return Err(validation_failure(
                    path,
                    "set_property step value must be an object",
                    json!("object"),
                    json!(python_type_name(set_property)),
                ));
            };
            for field_name in ["target", "property"] {
                if !set_property
                    .get(field_name)
                    .map(is_non_empty_string)
                    .unwrap_or(false)
                {
                    return Err(validation_failure(
                        path,
                        &format!(
                            "set_property step field '{field_name}' must be a non-empty string"
                        ),
                        json!("non-empty string"),
                        set_property.get(field_name).cloned().unwrap_or(Value::Null),
                    ));
                }
            }
            if !set_property.contains_key("value") {
                return Err(validation_failure(
                    path,
                    "set_property step field 'value' is required",
                    json!("any JSON value"),
                    Value::Null,
                ));
            }
        }

        if let Some(call_method) = step.get("call_method") {
            let Some(call_method) = call_method.as_object() else {
                return Err(validation_failure(
                    path,
                    "call_method step value must be an object",
                    json!("object"),
                    json!(python_type_name(call_method)),
                ));
            };
            for field_name in ["target", "method"] {
                if !call_method
                    .get(field_name)
                    .map(is_non_empty_string)
                    .unwrap_or(false)
                {
                    return Err(validation_failure(
                        path,
                        &format!(
                            "call_method step field '{field_name}' must be a non-empty string"
                        ),
                        json!("non-empty string"),
                        call_method.get(field_name).cloned().unwrap_or(Value::Null),
                    ));
                }
            }
            if let Some(method) = call_method.get("method").and_then(Value::as_str) {
                if !method.trim().starts_with("qa_") {
                    return Err(validation_failure(
                        path,
                        "call_method step field 'method' must name a qa_ fixture method",
                        json!("method name beginning with qa_"),
                        json!(method),
                    ));
                }
            }
            if let Some(args) = call_method.get("args") {
                if !args.is_array() {
                    return Err(validation_failure(
                        path,
                        "call_method step field 'args' must be a list when present",
                        json!("list"),
                        json!(python_type_name(args)),
                    ));
                }
            }
        }

        if let Some(fixture_call) = step.get("fixture_call") {
            let Some(fixture_call) = fixture_call.as_object() else {
                return Err(validation_failure(
                    path,
                    "fixture_call step value must be an object",
                    json!("object"),
                    json!(python_type_name(fixture_call)),
                ));
            };
            for field_name in ["driver", "action"] {
                if !fixture_call
                    .get(field_name)
                    .map(is_non_empty_string)
                    .unwrap_or(false)
                {
                    return Err(validation_failure(
                        path,
                        &format!(
                            "fixture_call step field '{field_name}' must be a non-empty string"
                        ),
                        json!("non-empty string"),
                        fixture_call.get(field_name).cloned().unwrap_or(Value::Null),
                    ));
                }
            }
            if let Some(params) = fixture_call.get("params")
                && !params.is_object()
            {
                return Err(validation_failure(
                    path,
                    "fixture_call step field 'params' must be an object when present",
                    json!("object"),
                    json!(python_type_name(params)),
                ));
            }
        }

        if let Some(audio_capture) = step.get("audio_capture") {
            validate_audio_capture_step(path, audio_capture)?;
        }

        if let Some(audio_export) = step.get("audio_export") {
            validate_audio_export_step(path, audio_export)?;
        }

        if let Some(assertions) = step.get("assert") {
            let Some(assertions) = assertions.as_array() else {
                return Err(validation_failure(
                    path,
                    "assert step value must be a list",
                    json!("list"),
                    json!(python_type_name(assertions)),
                ));
            };
            validate_supported_assertion_payloads(assertions, path)?;
        }
    }
    Ok(())
}

fn validate_optional_position_or_locator(
    path: &Path,
    step_name: &str,
    step: &Map<String, Value>,
    require_one: bool,
) -> Result<(), ScenarioError> {
    let locator_field = if step_name == "spawn_scene" {
        "at"
    } else {
        "to"
    };
    let has_locator = step.contains_key(locator_field);
    let has_position = step.contains_key("position");
    let has_to_point = step.contains_key("to_point");
    if require_one && !has_locator && !has_position && !has_to_point {
        return Err(validation_failure(
            path,
            &format!("{step_name} step requires '{locator_field}', 'position', or 'to_point'"),
            json!(format!(
                "non-empty '{locator_field}' locator, position object, or to_point provider"
            )),
            Value::Object(step.clone()),
        ));
    }
    let destination_count = [has_locator, has_position, has_to_point]
        .into_iter()
        .filter(|present| *present)
        .count();
    if destination_count > 1 {
        return Err(validation_failure(
            path,
            &format!("{step_name} step cannot combine destination fields"),
            json!(format!(
                "only one of '{locator_field}', 'position', or 'to_point'"
            )),
            Value::Object(step.clone()),
        ));
    }
    if let Some(locator) = step.get(locator_field) {
        if !is_non_empty_string(locator) {
            return Err(validation_failure(
                path,
                &format!("{step_name} step field '{locator_field}' must be a non-empty string"),
                json!("non-empty string"),
                locator.clone(),
            ));
        }
    }
    if let Some(position) = step.get("position") {
        validate_position_object(path, step_name, position)?;
    }
    if let Some(to_point) = step.get("to_point") {
        validate_to_point_object(path, step_name, to_point)?;
    }
    if let Some(offset) = step.get("offset") {
        validate_position_object(path, step_name, offset)?;
    }
    Ok(())
}

fn validate_audio_capture_step(path: &Path, audio_capture: &Value) -> Result<(), ScenarioError> {
    let Some(audio_capture) = audio_capture.as_object() else {
        return Err(validation_failure(
            path,
            "audio_capture step value must be an object",
            json!("object"),
            json!(python_type_name(audio_capture)),
        ));
    };
    validate_audio_artifact_name(path, audio_capture, "audio_capture")?;
    let duration = audio_capture
        .get("duration_seconds")
        .and_then(Value::as_f64)
        .unwrap_or(f64::NAN);
    if !duration.is_finite() || duration <= 0.0 || duration > AUDIO_CAPTURE_MAX_DURATION_SECONDS {
        return Err(validation_failure(
            path,
            "audio_capture step field 'duration_seconds' must be a bounded positive number",
            json!(format!(
                "number in range (0.0, {AUDIO_CAPTURE_MAX_DURATION_SECONDS}]"
            )),
            audio_capture
                .get("duration_seconds")
                .cloned()
                .unwrap_or(Value::Null),
        ));
    }
    if let Some(bus) = audio_capture.get("bus") {
        if !is_non_empty_string(bus) {
            return Err(validation_failure(
                path,
                "audio_capture step field 'bus' must be a non-empty string when present",
                json!("non-empty string"),
                bus.clone(),
            ));
        }
    }
    validate_optional_bool_field(path, "audio_capture", audio_capture, "analyze")?;
    Ok(())
}

fn validate_audio_export_step(path: &Path, audio_export: &Value) -> Result<(), ScenarioError> {
    let Some(audio_export) = audio_export.as_object() else {
        return Err(validation_failure(
            path,
            "audio_export step value must be an object",
            json!("object"),
            json!(python_type_name(audio_export)),
        ));
    };
    validate_audio_artifact_name(path, audio_export, "audio_export")?;
    for field_name in ["target", "method"] {
        if !audio_export
            .get(field_name)
            .map(is_non_empty_string)
            .unwrap_or(false)
        {
            return Err(validation_failure(
                path,
                &format!("audio_export step field '{field_name}' must be a non-empty string"),
                json!("non-empty string"),
                audio_export.get(field_name).cloned().unwrap_or(Value::Null),
            ));
        }
    }
    if let Some(method) = audio_export.get("method").and_then(Value::as_str) {
        if !method.trim().starts_with("qa_") {
            return Err(validation_failure(
                path,
                "audio_export step field 'method' must name a qa_ fixture method",
                json!("method name beginning with qa_"),
                json!(method),
            ));
        }
    }
    if let Some(args) = audio_export.get("args") {
        if !args.is_array() {
            return Err(validation_failure(
                path,
                "audio_export step field 'args' must be a list when present",
                json!("list"),
                json!(python_type_name(args)),
            ));
        }
    }
    validate_optional_bool_field(path, "audio_export", audio_export, "analyze")?;
    Ok(())
}

fn validate_audio_artifact_name(
    path: &Path,
    step: &Map<String, Value>,
    step_name: &str,
) -> Result<(), ScenarioError> {
    let Some(name) = step.get("name").and_then(Value::as_str) else {
        return Err(validation_failure(
            path,
            &format!("{step_name} step field 'name' must be a safe artifact name"),
            json!("safe artifact name"),
            step.get("name").cloned().unwrap_or(Value::Null),
        ));
    };
    if !is_safe_artifact_segment(name.trim()) {
        return Err(validation_failure(
            path,
            &format!("{step_name} step field 'name' must be a safe artifact name"),
            json!("safe artifact name"),
            json!(name),
        ));
    }
    Ok(())
}

fn validate_to_point_object(
    path: &Path,
    step_name: &str,
    to_point: &Value,
) -> Result<(), ScenarioError> {
    let Some(to_point) = to_point.as_object() else {
        return Err(validation_failure(
            path,
            &format!("{step_name} step field 'to_point' must be an object"),
            json!("object with provider, method, and id"),
            json!(python_type_name(to_point)),
        ));
    };
    for field_name in ["provider", "method", "id"] {
        if !to_point
            .get(field_name)
            .map(is_non_empty_string)
            .unwrap_or(false)
        {
            return Err(validation_failure(
                path,
                &format!(
                    "{step_name} step to_point field '{field_name}' must be a non-empty string"
                ),
                json!("non-empty string"),
                to_point.get(field_name).cloned().unwrap_or(Value::Null),
            ));
        }
    }
    if let Some(method) = to_point.get("method").and_then(Value::as_str) {
        if !method.trim().starts_with("qa_") {
            return Err(validation_failure(
                path,
                &format!("{step_name} step to_point field 'method' must name a qa_ probe method"),
                json!("method name beginning with qa_"),
                json!(method),
            ));
        }
    }
    Ok(())
}

fn validate_optional_bool_field(
    path: &Path,
    step_name: &str,
    step: &Map<String, Value>,
    field_name: &str,
) -> Result<(), ScenarioError> {
    if let Some(value) = step.get(field_name) {
        if !value.is_boolean() {
            return Err(validation_failure(
                path,
                &format!("{step_name} step field '{field_name}' must be a boolean when present"),
                json!("boolean"),
                value.clone(),
            ));
        }
    }
    Ok(())
}

fn validate_position_object(
    path: &Path,
    step_name: &str,
    position: &Value,
) -> Result<(), ScenarioError> {
    let Some(position) = position.as_object() else {
        return Err(validation_failure(
            path,
            &format!("{step_name} step field 'position' must be an object"),
            json!("object with numeric x and y"),
            json!(python_type_name(position)),
        ));
    };
    for field_name in ["x", "y"] {
        if !position
            .get(field_name)
            .map(Value::is_number)
            .unwrap_or(false)
        {
            return Err(validation_failure(
                path,
                &format!("{step_name} step position field '{field_name}' must be a number"),
                json!("number"),
                position.get(field_name).cloned().unwrap_or(Value::Null),
            ));
        }
    }
    if let Some(z) = position.get("z") {
        if !z.is_number() {
            return Err(validation_failure(
                path,
                &format!("{step_name} step position field 'z' must be a number when present"),
                json!("number"),
                z.clone(),
            ));
        }
    }
    Ok(())
}

fn validate_supported_assertion_payloads(
    assertions: &[Value],
    path: &Path,
) -> Result<(), ScenarioError> {
    for (index, assertion) in assertions.iter().enumerate() {
        let Some(assertion) = assertion.as_object() else {
            continue;
        };

        reject_unknown_keys(assertion, KNOWN_ASSERTION_KEYS, "assertion", index, path)?;

        if let Some(state_matches) = assertion.get("state_matches") {
            validate_state_matches_assertion(path, state_matches)?;
        }

        for name in [
            "exists",
            "not_exists",
            "visible",
            "hidden",
            "enabled",
            "disabled",
            "focused",
            "inside_viewport",
        ] {
            if let Some(value) = assertion.get(name) {
                if !is_non_empty_string(value) {
                    return Err(validation_failure(
                        path,
                        &format!("{name} assertion value must be a non-empty string"),
                        json!("non-empty string"),
                        value.clone(),
                    ));
                }
            }
        }

        for name in ["text_equals", "text_contains"] {
            if let Some(value) = assertion.get(name) {
                let Some(value) = value.as_object() else {
                    return Err(validation_failure(
                        path,
                        &format!("{name} assertion value must be an object"),
                        json!("object"),
                        json!(python_type_name(value)),
                    ));
                };
                if !value
                    .get("target")
                    .map(is_non_empty_string)
                    .unwrap_or(false)
                {
                    return Err(validation_failure(
                        path,
                        &format!("{name} assertion field 'target' must be a non-empty string"),
                        json!("non-empty string"),
                        value.get("target").cloned().unwrap_or(Value::Null),
                    ));
                }
                if !matches!(value.get("expected"), Some(Value::String(_))) {
                    return Err(validation_failure(
                        path,
                        &format!("{name} assertion field 'expected' must be a string"),
                        json!("string"),
                        value.get("expected").cloned().unwrap_or(Value::Null),
                    ));
                }
            }
        }

        if let Some(value) = assertion.get("no_overlap") {
            let Some(value) = value.as_object() else {
                return Err(validation_failure(
                    path,
                    "no_overlap assertion value must be an object",
                    json!("object"),
                    json!(python_type_name(value)),
                ));
            };
            for field_name in ["a", "b"] {
                if !value
                    .get(field_name)
                    .map(is_non_empty_string)
                    .unwrap_or(false)
                {
                    return Err(validation_failure(
                        path,
                        &format!(
                            "no_overlap assertion field '{field_name}' must be a non-empty string"
                        ),
                        json!("non-empty string"),
                        value.get(field_name).cloned().unwrap_or(Value::Null),
                    ));
                }
            }
        }

        if let Some(value) = assertion.get("no_errors") {
            if !value.is_boolean() {
                return Err(validation_failure(
                    path,
                    "no_errors assertion value must be a boolean",
                    json!("boolean"),
                    value.clone(),
                ));
            }
        }

        if let Some(value) = assertion.get("layout_audit") {
            let Some(mode) = value.as_str() else {
                return Err(validation_failure(
                    path,
                    "layout_audit assertion value must be a non-empty string",
                    json!("non-empty string"),
                    value.clone(),
                ));
            };
            let trimmed = mode.trim();
            if trimmed.is_empty() {
                return Err(validation_failure(
                    path,
                    "layout_audit assertion value must be a non-empty string",
                    json!("non-empty string"),
                    value.clone(),
                ));
            }
            if !LAYOUT_AUDIT_MODES.contains(&trimmed) {
                return Err(validation_failure(
                    path,
                    "layout_audit assertion value must be a supported mode",
                    json!(LAYOUT_AUDIT_MODES),
                    value.clone(),
                ));
            }
        }

        if let Some(value) = assertion.get("screenshot_match") {
            let Some(value) = value.as_object() else {
                return Err(validation_failure(
                    path,
                    "screenshot_match assertion value must be an object",
                    json!("object"),
                    json!(python_type_name(value)),
                ));
            };
            if !value
                .get("snapshot")
                .map(is_non_empty_string)
                .unwrap_or(false)
            {
                return Err(validation_failure(
                    path,
                    "screenshot_match assertion field 'snapshot' must be a non-empty string",
                    json!("non-empty string"),
                    value.get("snapshot").cloned().unwrap_or(Value::Null),
                ));
            }
            if !value
                .get("baseline")
                .map(is_non_empty_string)
                .unwrap_or(false)
            {
                return Err(validation_failure(
                    path,
                    "screenshot_match assertion field 'baseline' must be a non-empty string",
                    json!("non-empty string"),
                    value.get("baseline").cloned().unwrap_or(Value::Null),
                ));
            }
            if let Some(threshold) = value.get("threshold") {
                let Some(number) = threshold.as_f64() else {
                    return Err(validation_failure(
                        path,
                        "screenshot_match assertion field 'threshold' must be between 0.0 and 1.0",
                        json!("number between 0.0 and 1.0"),
                        threshold.clone(),
                    ));
                };
                if !(0.0..=1.0).contains(&number) {
                    return Err(validation_failure(
                        path,
                        "screenshot_match assertion field 'threshold' must be between 0.0 and 1.0",
                        json!("number between 0.0 and 1.0"),
                        threshold.clone(),
                    ));
                }
            }
        }

        if let Some(value) = assertion.get("max_delta_position") {
            let Some(value) = value.as_object() else {
                return Err(validation_failure(
                    path,
                    "max_delta_position assertion value must be an object",
                    json!("object"),
                    json!(python_type_name(value)),
                ));
            };
            require_non_empty_string_field(path, value, "max_delta_position", "target")?;
            validate_assertion_temporal_sampling(path, value, "max_delta_position")?;
            if value.contains_key("window_frames") || value.contains_key("window_seconds") {
                validate_temporal_window(path, value, "max_delta_position")?;
            }
            validate_optional_positive_integer(
                path,
                value,
                "max_delta_position",
                "sample_every_frames",
            )?;
            if !value.contains_key("max") && !value.contains_key("max_delta") {
                return Err(validation_failure(
                    path,
                    "max_delta_position assertion requires 'max' or compatibility field 'max_delta'",
                    json!("numeric max or max_delta"),
                    Value::Null,
                ));
            }
            if value.contains_key("max") {
                require_numeric_field(path, value, "max_delta_position", "max")?;
            }
            if value.contains_key("max_delta") {
                require_numeric_field(path, value, "max_delta_position", "max_delta")?;
            }
        }

        if let Some(value) = assertion.get("follows") {
            let Some(value) = value.as_object() else {
                return Err(validation_failure(
                    path,
                    "follows assertion value must be an object",
                    json!("object"),
                    json!(python_type_name(value)),
                ));
            };
            require_non_empty_string_field(path, value, "follows", "leader")?;
            require_non_empty_string_field(path, value, "follows", "follower")?;
            validate_assertion_temporal_sampling(path, value, "follows")?;
            validate_temporal_window(path, value, "follows")?;
            validate_optional_positive_integer(path, value, "follows", "sample_every_frames")?;
            require_numeric_field(path, value, "follows", "tolerance")?;
        }

        if let Some(value) = assertion.get("within_limits") {
            let Some(value) = value.as_object() else {
                return Err(validation_failure(
                    path,
                    "within_limits assertion value must be an object",
                    json!("object"),
                    json!(python_type_name(value)),
                ));
            };
            let observes_state = validate_temporal_state_observation(path, value, "within_limits")?;
            if !observes_state {
                require_non_empty_string_field(path, value, "within_limits", "target")?;
            }
            validate_assertion_temporal_sampling(path, value, "within_limits")?;
            validate_temporal_window(path, value, "within_limits")?;
            validate_optional_positive_integer(
                path,
                value,
                "within_limits",
                "sample_every_frames",
            )?;
            require_numeric_field(path, value, "within_limits", "min")?;
            require_numeric_field(path, value, "within_limits", "max")?;
        }

        if let Some(value) = assertion.get("trend") {
            let Some(value) = value.as_object() else {
                return Err(validation_failure(
                    path,
                    "trend assertion value must be an object",
                    json!("object"),
                    json!(python_type_name(value)),
                ));
            };
            let direction = value
                .get("trend")
                .and_then(Value::as_str)
                .map(str::trim)
                .unwrap_or_default();
            if !TREND_DIRECTIONS.contains(&direction) {
                return Err(validation_failure(
                    path,
                    "trend assertion field 'trend' must be a supported direction",
                    json!(TREND_DIRECTIONS),
                    value.get("trend").cloned().unwrap_or(Value::Null),
                ));
            }
            let observes_state = validate_temporal_state_observation(path, value, "trend")?;
            if !observes_state {
                require_non_empty_string_field(path, value, "trend", "target")?;
            }
            validate_assertion_temporal_sampling(path, value, "trend")?;
            validate_temporal_window(path, value, "trend")?;
            validate_optional_positive_integer(path, value, "trend", "sample_every_frames")?;
            if value.contains_key("min_delta") {
                require_finite_non_negative_number(path, value, "trend", "min_delta")?;
            }
            if value.contains_key("tolerance") {
                require_finite_non_negative_number(path, value, "trend", "tolerance")?;
            }
        }

        if let Some(value) = assertion.get("frame_time") {
            let Some(value) = value.as_object() else {
                return Err(validation_failure(
                    path,
                    "frame_time assertion value must be an object",
                    json!("object"),
                    json!(python_type_name(value)),
                ));
            };
            if value.contains_key("temporal_sampling") {
                return Err(validation_failure(
                    path,
                    "frame_time assertion does not support 'temporal_sampling' because it measures wall-clock frame times",
                    json!("no temporal_sampling field"),
                    value
                        .get("temporal_sampling")
                        .cloned()
                        .unwrap_or(Value::Null),
                ));
            }
            validate_temporal_window(path, value, "frame_time")?;
            validate_optional_positive_integer(path, value, "frame_time", "sample_every_frames")?;
            let mut bound_count = 0;
            for field_name in FRAME_TIME_BOUND_FIELDS {
                if value.contains_key(*field_name) {
                    require_positive_number_field(path, value, "frame_time", field_name)?;
                    bound_count += 1;
                }
            }
            if bound_count == 0 {
                return Err(validation_failure(
                    path,
                    "frame_time assertion requires at least one bound: 'max_avg_ms', 'max_p95_ms', or 'min_fps'",
                    json!(FRAME_TIME_BOUND_FIELDS),
                    Value::Object(value.clone()),
                ));
            }
        }

        if let Some(value) = assertion.get("distance") {
            validate_distance_assertion_payload(path, value, "distance", "min", "max")?;
        }

        if let Some(value) = assertion.get("distance_between") {
            validate_distance_assertion_payload(
                path,
                value,
                "distance_between",
                "min_distance",
                "max_distance",
            )?;
        }

        if let Some(value) = assertion.get("distance_changed") {
            let Some(value) = value.as_object() else {
                return Err(validation_failure(
                    path,
                    "distance_changed assertion value must be an object",
                    json!("object"),
                    json!(python_type_name(value)),
                ));
            };
            require_non_empty_string_field(path, value, "distance_changed", "a")?;
            require_non_empty_string_field(path, value, "distance_changed", "b")?;
            validate_assertion_temporal_sampling(path, value, "distance_changed")?;
            validate_temporal_window(path, value, "distance_changed")?;
            validate_optional_positive_integer(
                path,
                value,
                "distance_changed",
                "sample_every_frames",
            )?;
            require_finite_non_negative_number(path, value, "distance_changed", "min")?;
        }

        if let Some(value) = assertion.get("visible_pixels") {
            let Some(value) = value.as_object() else {
                return Err(validation_failure(
                    path,
                    "visible_pixels assertion value must be an object",
                    json!("object"),
                    json!(python_type_name(value)),
                ));
            };
            require_non_empty_string_field(path, value, "visible_pixels", "target")?;
            require_non_empty_string_field(path, value, "visible_pixels", "snapshot")?;
            require_visual_threshold(path, value, "visible_pixels")?;
            validate_visual_threshold_fields(path, value, "visible_pixels")?;
        }

        if let Some(value) = assertion.get("saved_snapshot_pixels") {
            let Some(value) = value.as_object() else {
                return Err(validation_failure(
                    path,
                    "saved_snapshot_pixels assertion value must be an object",
                    json!("object"),
                    json!(python_type_name(value)),
                ));
            };
            require_non_empty_string_field(path, value, "saved_snapshot_pixels", "target")?;
            require_non_empty_string_field(path, value, "saved_snapshot_pixels", "snapshot")?;
            require_visual_threshold(path, value, "saved_snapshot_pixels")?;
            validate_visual_threshold_fields(path, value, "saved_snapshot_pixels")?;
        }

        if let Some(value) = assertion.get("not_occluded") {
            let Some(value) = value.as_object() else {
                return Err(validation_failure(
                    path,
                    "not_occluded assertion value must be an object",
                    json!("object"),
                    json!(python_type_name(value)),
                ));
            };
            require_non_empty_string_field(path, value, "not_occluded", "target")?;
            require_non_empty_string_field(path, value, "not_occluded", "snapshot")?;
            if value.contains_key("occluder") {
                require_non_empty_string_field(path, value, "not_occluded", "occluder")?;
            }
            validate_visual_threshold_fields(path, value, "not_occluded")?;
        }

        for assertion_name in ["renders_above", "renders_below"] {
            if let Some(value) = assertion.get(assertion_name) {
                let Some(value) = value.as_object() else {
                    return Err(validation_failure(
                        path,
                        &format!("{assertion_name} assertion value must be an object"),
                        json!("object"),
                        json!(python_type_name(value)),
                    ));
                };
                if !value.contains_key("target") && !value.contains_key("a") {
                    require_non_empty_string_field(path, value, assertion_name, "target")?;
                }
                if !value.contains_key("other") && !value.contains_key("b") {
                    require_non_empty_string_field(path, value, assertion_name, "other")?;
                }
                if value.contains_key("target") {
                    require_non_empty_string_field(path, value, assertion_name, "target")?;
                }
                if value.contains_key("other") {
                    require_non_empty_string_field(path, value, assertion_name, "other")?;
                }
                if value.contains_key("a") {
                    require_non_empty_string_field(path, value, assertion_name, "a")?;
                }
                if value.contains_key("b") {
                    require_non_empty_string_field(path, value, assertion_name, "b")?;
                }
                if value.contains_key("snapshot") {
                    require_non_empty_string_field(path, value, assertion_name, "snapshot")?;
                }
            }
        }

        if let Some(value) = assertion.get("never") {
            let Some(value) = value.as_object() else {
                return Err(validation_failure(
                    path,
                    "never assertion value must be an object",
                    json!("object"),
                    json!(python_type_name(value)),
                ));
            };
            let observes_state = validate_temporal_state_observation(path, value, "never")?;
            if observes_state {
                for field_name in ["property", "text", "token"] {
                    if value.contains_key(field_name) {
                        return Err(validation_failure(
                            path,
                            &format!(
                                "never assertion cannot combine 'source' with observation field '{field_name}'"
                            ),
                            json!("either a state 'source' or one of 'property'/'text'/'token'"),
                            Value::Object(value.clone()),
                        ));
                    }
                }
            } else {
                require_non_empty_string_field(path, value, "never", "target")?;
                validate_never_observation(path, value)?;
            }
            validate_assertion_temporal_sampling(path, value, "never")?;
            validate_temporal_window(path, value, "never")?;
            validate_optional_positive_integer(path, value, "never", "sample_every_frames")?;
            validate_never_forbidden(path, value)?;
        }
        if let Some(value) = assertion.get("observe_until") {
            validate_observe_until_assertion(path, value)?;
        }
        if let Some(value) = assertion.get("surface_probe") {
            let Some(value) = value.as_object() else {
                return Err(validation_failure(
                    path,
                    "surface_probe assertion value must be an object",
                    json!("object"),
                    json!(python_type_name(value)),
                ));
            };
            if value.contains_key("probe_id") {
                if value.contains_key("provider") {
                    require_non_empty_string_field(path, value, "surface_probe", "provider")?;
                } else {
                    require_non_empty_string_field(path, value, "surface_probe", "provider_id")?;
                }
                require_non_empty_string_field(path, value, "surface_probe", "probe_id")?;
                if let Some(expected_surface) = value.get("expected_surface")
                    && !matches!(expected_surface, Value::String(_))
                {
                    return Err(validation_failure(
                        path,
                        "surface_probe assertion field 'expected_surface' must be a string",
                        json!("string"),
                        expected_surface.clone(),
                    ));
                }
                if let Some(expected_walkable) = value.get("expected_walkable")
                    && !expected_walkable.is_boolean()
                {
                    return Err(validation_failure(
                        path,
                        "surface_probe assertion field 'expected_walkable' must be a boolean",
                        json!("boolean"),
                        expected_walkable.clone(),
                    ));
                }
                continue;
            }
            require_non_empty_string_field(path, value, "surface_probe", "target")?;
            if value.contains_key("provider") {
                require_non_empty_string_field(path, value, "surface_probe", "provider")?;
            } else {
                require_non_empty_string_field(path, value, "surface_probe", "provider_id")?;
            }
            if let Some(space) = value.get("coordinate_space").and_then(Value::as_str) {
                if !SURFACE_COORDINATE_SPACES.contains(&space.trim()) {
                    return Err(validation_failure(
                        path,
                        "surface_probe assertion field 'coordinate_space' must be supported",
                        json!(SURFACE_COORDINATE_SPACES),
                        value
                            .get("coordinate_space")
                            .cloned()
                            .unwrap_or(Value::Null),
                    ));
                }
            }
            if let Some(radius) = value.get("radius") {
                let Some(radius) = radius.as_f64() else {
                    return Err(validation_failure(
                        path,
                        "surface_probe assertion field 'radius' must be bounded",
                        json!(format!("number in range [0.0, {SURFACE_MAX_RADIUS}]")),
                        radius.clone(),
                    ));
                };
                if !radius.is_finite() || !(0.0..=SURFACE_MAX_RADIUS).contains(&radius) {
                    return Err(validation_failure(
                        path,
                        "surface_probe assertion field 'radius' must be bounded",
                        json!(format!("number in range [0.0, {SURFACE_MAX_RADIUS}]")),
                        json!(radius),
                    ));
                }
            }
            let Some(points) = value.get("points").and_then(Value::as_array) else {
                return Err(validation_failure(
                    path,
                    "surface_probe assertion field 'points' must be an array",
                    json!("array"),
                    value.get("points").cloned().unwrap_or(Value::Null),
                ));
            };
            if points.is_empty() || points.len() > SURFACE_MAX_POINTS {
                return Err(validation_failure(
                    path,
                    "surface_probe assertion field 'points' must be bounded",
                    json!(format!("1..={SURFACE_MAX_POINTS} points")),
                    json!(points.len()),
                ));
            }
            let surfaces = value.get("surfaces").and_then(Value::as_array);
            for point in points {
                let Some(point) = point.as_object() else {
                    return Err(validation_failure(
                        path,
                        "surface_probe point must be an object",
                        json!("object"),
                        point.clone(),
                    ));
                };
                for field_name in ["x", "y"] {
                    if !point.get(field_name).map(Value::is_number).unwrap_or(false) {
                        return Err(validation_failure(
                            path,
                            &format!("surface_probe point field '{field_name}' must be numeric"),
                            json!("number"),
                            point.get(field_name).cloned().unwrap_or(Value::Null),
                        ));
                    }
                }
                if let Some(z) = point.get("z")
                    && !z.is_number()
                {
                    return Err(validation_failure(
                        path,
                        "surface_probe point field 'z' must be numeric",
                        json!("number"),
                        z.clone(),
                    ));
                }
                if let Some(expected) = point.get("expected").and_then(Value::as_str)
                    && let Some(surfaces) = surfaces
                {
                    let labels: Vec<&str> = surfaces
                        .iter()
                        .filter_map(Value::as_str)
                        .map(str::trim)
                        .collect();
                    if !labels.contains(&expected.trim()) {
                        return Err(validation_failure(
                            path,
                            "surface_probe point expected label must exist in surfaces",
                            json!(labels),
                            json!(expected),
                        ));
                    }
                }
            }
        }
        if let Some(value) = assertion.get("surface_matches") {
            let Some(value) = value.as_object() else {
                return Err(validation_failure(
                    path,
                    "surface_matches assertion value must be an object",
                    json!("object"),
                    json!(python_type_name(value)),
                ));
            };
            require_non_empty_string_field(path, value, "surface_matches", "target")?;
            if !value.contains_key("surface") && !value.contains_key("expected") {
                return Err(validation_failure(
                    path,
                    "surface_matches assertion requires 'surface' or compatibility field 'expected'",
                    json!("surface or expected"),
                    Value::Null,
                ));
            }
            if value.contains_key("surface") {
                require_non_empty_string_field(path, value, "surface_matches", "surface")?;
            }
            if value.contains_key("expected") {
                require_non_empty_string_field(path, value, "surface_matches", "expected")?;
            }
        }
    }

    Ok(())
}

fn normalize_step(step: &Value) -> Value {
    let Some(step) = step.as_object() else {
        return step.clone();
    };
    let mut normalized = step.clone();
    if let Some(snapshot) = normalized.get_mut("snapshot") {
        if let Some(snapshot_name) = snapshot.as_str() {
            *snapshot = json!(snapshot_name.trim());
        }
    }
    if let Some(node_inspect) = normalized.get_mut("node_inspect") {
        if let Some(target) = node_inspect.as_str() {
            *node_inspect = json!(target.trim());
        }
    }
    if let Some(record_state) = normalized.get_mut("record_state") {
        trim_step_fields(
            record_state,
            &[
                "name",
                "source",
                "provider",
                "provider_id",
                "method",
                "target",
                "path",
            ],
        );
    }
    if let Some(teleport_node) = normalized.get_mut("teleport_node") {
        trim_step_fields(teleport_node, &["target", "to"]);
        trim_to_point_fields(teleport_node);
    }
    if let Some(fixture_teleport) = normalized.get_mut("fixture_teleport") {
        trim_step_fields(fixture_teleport, &["target", "to"]);
        trim_to_point_fields(fixture_teleport);
    }
    if let Some(spawn_scene) = normalized.get_mut("spawn_scene") {
        trim_step_fields(spawn_scene, &["scene", "parent", "name", "qa_id", "at"]);
    }
    if let Some(set_property) = normalized.get_mut("set_property") {
        trim_step_fields(set_property, &["target", "property"]);
    }
    if let Some(call_method) = normalized.get_mut("call_method") {
        trim_step_fields(call_method, &["target", "method"]);
    }
    if let Some(fixture_call) = normalized.get_mut("fixture_call") {
        trim_step_fields(fixture_call, &["driver", "action"]);
    }
    if let Some(input_joy_button) = normalized.get_mut("input_joy_button") {
        if let Some(button) = input_joy_button.as_str() {
            *input_joy_button = json!(button.trim());
        } else {
            trim_step_fields(input_joy_button, &["button"]);
        }
    }
    if let Some(input_joy_motion) = normalized.get_mut("input_joy_motion") {
        trim_step_fields(input_joy_motion, &["axis"]);
    }
    if let Some(drive_policy) = normalized.get_mut("drive_policy") {
        trim_step_fields(drive_policy, &["policy"]);
    }
    if let Some(audio_capture) = normalized.get_mut("audio_capture") {
        trim_step_fields(audio_capture, &["name", "bus"]);
    }
    if let Some(audio_export) = normalized.get_mut("audio_export") {
        trim_step_fields(audio_export, &["name", "target", "method"]);
    }
    if let Some(assertions) = normalized.get_mut("assert").and_then(Value::as_array_mut) {
        for assertion in assertions {
            *assertion = normalize_assertion(assertion);
        }
    }
    Value::Object(normalized)
}

fn validate_input_joy_button_step(value: &Value, path: &Path) -> Result<(), ScenarioError> {
    if is_non_empty_string(value) {
        return Ok(());
    }
    let Some(object) = value.as_object() else {
        return Err(validation_failure(
            path,
            "input_joy_button step value must be a non-empty string or object",
            json!("non-empty string or object"),
            json!(python_type_name(value)),
        ));
    };
    if !object
        .get("button")
        .map(is_non_empty_string)
        .unwrap_or(false)
    {
        return Err(validation_failure(
            path,
            "input_joy_button step field 'button' must be a non-empty string",
            json!("non-empty string"),
            object.get("button").cloned().unwrap_or(Value::Null),
        ));
    }
    validate_optional_non_negative_int_field(path, "input_joy_button", object, "device")?;
    validate_optional_non_negative_int_field(path, "input_joy_button", object, "frames")?;
    Ok(())
}

fn validate_input_joy_motion_step(value: &Value, path: &Path) -> Result<(), ScenarioError> {
    let Some(object) = value.as_object() else {
        return Err(validation_failure(
            path,
            "input_joy_motion step value must be an object",
            json!("object"),
            json!(python_type_name(value)),
        ));
    };
    if !object.get("axis").map(is_non_empty_string).unwrap_or(false) {
        return Err(validation_failure(
            path,
            "input_joy_motion step field 'axis' must be a non-empty string",
            json!("non-empty string"),
            object.get("axis").cloned().unwrap_or(Value::Null),
        ));
    }
    let raw_value = object.get("value").unwrap_or(&Value::Null);
    let Some(axis_value) = raw_value.as_f64() else {
        return Err(validation_failure(
            path,
            "input_joy_motion step field 'value' must be a number between -1.0 and 1.0",
            json!("number between -1.0 and 1.0"),
            raw_value.clone(),
        ));
    };
    if !(-1.0..=1.0).contains(&axis_value) {
        return Err(validation_failure(
            path,
            "input_joy_motion step field 'value' must be between -1.0 and 1.0",
            json!("number between -1.0 and 1.0"),
            json!(axis_value),
        ));
    }
    validate_optional_non_negative_int_field(path, "input_joy_motion", object, "device")?;
    validate_optional_non_negative_int_field(path, "input_joy_motion", object, "frames")?;
    Ok(())
}

/// Validate an `input_joy_state` step: an open-loop batch that applies many
/// devices' joypad inputs in the same frame (concurrent button holds and axis
/// positions that persist across later steps). Every entry in `inputs` is
/// validated before anything is emitted at runtime. Each entry uses the same
/// `{device, motion|button}` shape as `drive_policy` results, but there is no
/// device allowlist: any non-negative device is accepted. `frames` (default `1`)
/// is the post-emit wait; `0` emits and continues in the same frame.
fn validate_input_joy_state_step(value: &Value, path: &Path) -> Result<(), ScenarioError> {
    const INPUT_JOY_STATE_FIELDS: &[&str] = &["inputs", "frames"];

    let Some(object) = value.as_object() else {
        return Err(validation_failure(
            path,
            "input_joy_state step value must be an object",
            json!("object"),
            json!(python_type_name(value)),
        ));
    };
    for key in object.keys() {
        if !INPUT_JOY_STATE_FIELDS.contains(&key.as_str()) {
            return Err(validation_failure(
                path,
                &format!("input_joy_state step uses unsupported field '{key}'"),
                json!(INPUT_JOY_STATE_FIELDS),
                json!({ "unknown_key": key }),
            ));
        }
    }
    let Some(inputs) = object.get("inputs").and_then(Value::as_array) else {
        return Err(validation_failure(
            path,
            "input_joy_state step field 'inputs' must be a non-empty list of {device, motion|button} objects",
            json!("non-empty list of {device, motion|button} objects"),
            object.get("inputs").cloned().unwrap_or(Value::Null),
        ));
    };
    if inputs.is_empty() {
        return Err(validation_failure(
            path,
            "input_joy_state step field 'inputs' must be a non-empty list of {device, motion|button} objects",
            json!("non-empty list of {device, motion|button} objects"),
            json!(inputs),
        ));
    }
    for (index, entry) in inputs.iter().enumerate() {
        validate_joy_input_entry(path, index, entry)?;
    }
    validate_optional_non_negative_int_field(path, "input_joy_state", object, "frames")?;
    Ok(())
}

/// Validate a `joy_connect`/`joy_disconnect` step: a setup-class step that
/// simulates joypad hardware CONNECTION state (not player input). It calls the
/// public `Input.joy_connection_changed` entry point so a lobby enumerating
/// `Input.get_connected_joypads()` or listening for `joy_connection_changed`
/// actually sees the pads. `devices` is required: a non-empty list of UNIQUE
/// non-negative integers. `joy_connect` also accepts an optional non-empty string
/// `name` (default "QA Virtual Pad" runtime-side); `joy_disconnect` takes no
/// `name`. Unknown top-level fields are rejected.
fn validate_joy_connection_step(
    value: &Value,
    path: &Path,
    step_name: &str,
    allow_name: bool,
) -> Result<(), ScenarioError> {
    let allowed_fields: &[&str] = if allow_name {
        &["devices", "name"]
    } else {
        &["devices"]
    };

    let Some(object) = value.as_object() else {
        return Err(validation_failure(
            path,
            &format!("{step_name} step value must be an object"),
            json!("object"),
            json!(python_type_name(value)),
        ));
    };
    for key in object.keys() {
        if !allowed_fields.contains(&key.as_str()) {
            return Err(validation_failure(
                path,
                &format!("{step_name} step uses unsupported field '{key}'"),
                json!(allowed_fields),
                json!({ "unknown_key": key }),
            ));
        }
    }

    let Some(devices) = object.get("devices").and_then(Value::as_array) else {
        return Err(validation_failure(
            path,
            &format!(
                "{step_name} step field 'devices' must be a non-empty list of device integers"
            ),
            json!("non-empty list of unique non-negative integers"),
            object.get("devices").cloned().unwrap_or(Value::Null),
        ));
    };
    if devices.is_empty() {
        return Err(validation_failure(
            path,
            &format!(
                "{step_name} step field 'devices' must be a non-empty list of device integers"
            ),
            json!("non-empty list of unique non-negative integers"),
            json!(devices),
        ));
    }
    let mut seen: Vec<i64> = Vec::new();
    for device in devices {
        if device.as_i64().is_none() || !device.is_i64() || device.as_i64().unwrap() < 0 {
            return Err(validation_failure(
                path,
                &format!("{step_name} step 'devices' entries must be non-negative integers"),
                json!("non-negative integer"),
                device.clone(),
            ));
        }
        let device_index = device.as_i64().unwrap();
        if seen.contains(&device_index) {
            return Err(validation_failure(
                path,
                &format!("{step_name} step 'devices' entries must be unique"),
                json!("unique non-negative integers"),
                json!(device_index),
            ));
        }
        seen.push(device_index);
    }

    if allow_name {
        if let Some(name) = object.get("name") {
            if !is_non_empty_string(name) {
                return Err(validation_failure(
                    path,
                    &format!("{step_name} step field 'name' must be a non-empty string"),
                    json!("non-empty string"),
                    name.clone(),
                ));
            }
        }
    }

    Ok(())
}

/// Validate one `{device, motion|button}` joypad input entry from an
/// `input_joy_state` step. `device` must be a non-negative integer, and exactly
/// one of `motion` (`{axis, value}` with a non-empty axis and numeric value in
/// `-1.0..=1.0`) or `button` (`{name, pressed}` with a non-empty name and
/// boolean `pressed`) must be present. Axis/button names are validated for
/// non-emptiness here; the runtime resolves them against the supported vocabulary.
fn validate_joy_input_entry(path: &Path, index: usize, entry: &Value) -> Result<(), ScenarioError> {
    let Some(object) = entry.as_object() else {
        return Err(validation_failure(
            path,
            &format!("input_joy_state input {index} must be an object"),
            json!("{device, motion|button}"),
            json!(python_type_name(entry)),
        ));
    };
    let device = object.get("device").unwrap_or(&Value::Null);
    if device.as_i64().is_none() || !device.is_i64() || device.as_i64().unwrap() < 0 {
        return Err(validation_failure(
            path,
            &format!("input_joy_state input {index} field 'device' must be a non-negative integer"),
            json!("non-negative integer"),
            device.clone(),
        ));
    }
    let has_motion = object.contains_key("motion");
    let has_button = object.contains_key("button");
    if has_motion == has_button {
        return Err(validation_failure(
            path,
            &format!("input_joy_state input {index} must use exactly one of 'motion' or 'button'"),
            json!("exactly one of 'motion' or 'button'"),
            entry.clone(),
        ));
    }
    if has_motion {
        let motion = object.get("motion").unwrap_or(&Value::Null);
        let Some(motion_object) = motion.as_object() else {
            return Err(validation_failure(
                path,
                &format!("input_joy_state input {index} field 'motion' must be an object"),
                json!("{axis, value}"),
                json!(python_type_name(motion)),
            ));
        };
        if !motion_object
            .get("axis")
            .map(is_non_empty_string)
            .unwrap_or(false)
        {
            return Err(validation_failure(
                path,
                &format!("input_joy_state input {index} motion 'axis' must be a non-empty string"),
                json!("non-empty string"),
                motion_object.get("axis").cloned().unwrap_or(Value::Null),
            ));
        }
        let raw_value = motion_object.get("value").unwrap_or(&Value::Null);
        let Some(axis_value) = raw_value.as_f64() else {
            return Err(validation_failure(
                path,
                &format!(
                    "input_joy_state input {index} motion 'value' must be a number between -1.0 and 1.0"
                ),
                json!("number between -1.0 and 1.0"),
                raw_value.clone(),
            ));
        };
        if !(-1.0..=1.0).contains(&axis_value) {
            return Err(validation_failure(
                path,
                &format!(
                    "input_joy_state input {index} motion 'value' must be between -1.0 and 1.0"
                ),
                json!("number between -1.0 and 1.0"),
                json!(axis_value),
            ));
        }
        return Ok(());
    }
    let button = object.get("button").unwrap_or(&Value::Null);
    let Some(button_object) = button.as_object() else {
        return Err(validation_failure(
            path,
            &format!("input_joy_state input {index} field 'button' must be an object"),
            json!("{name, pressed}"),
            json!(python_type_name(button)),
        ));
    };
    if !button_object
        .get("name")
        .map(is_non_empty_string)
        .unwrap_or(false)
    {
        return Err(validation_failure(
            path,
            &format!("input_joy_state input {index} button 'name' must be a non-empty string"),
            json!("non-empty string"),
            button_object.get("name").cloned().unwrap_or(Value::Null),
        ));
    }
    match button_object.get("pressed") {
        Some(Value::Bool(_)) => {}
        other => {
            return Err(validation_failure(
                path,
                &format!("input_joy_state input {index} button 'pressed' must be a boolean"),
                json!("boolean"),
                other.cloned().unwrap_or(Value::Null),
            ));
        }
    }
    Ok(())
}

fn validate_optional_non_negative_int_field(
    path: &Path,
    step_name: &str,
    object: &Map<String, Value>,
    field: &str,
) -> Result<(), ScenarioError> {
    let Some(value) = object.get(field) else {
        return Ok(());
    };
    if value.as_i64().is_none() || !value.is_i64() || value.as_i64().unwrap() < 0 {
        return Err(validation_failure(
            path,
            &format!("{step_name} step field '{field}' must be a non-negative integer"),
            json!("non-negative integer"),
            value.clone(),
        ));
    }
    Ok(())
}

fn validate_snapshot_filters(
    snapshots: &Map<String, Value>,
    path: &Path,
) -> Result<(), ScenarioError> {
    if let Some(value) = snapshots.get("target") {
        match value.as_str().map(str::trim) {
            Some(target) if !target.is_empty() => {}
            _ => {
                return Err(validation_failure(
                    path,
                    "snapshots.target must be a non-empty string",
                    json!("non-empty target locator"),
                    value.clone(),
                ));
            }
        }
    }
    if let Some(value) = snapshots.get("max_depth") {
        match value.as_i64() {
            Some(max_depth) if max_depth >= 0 => {}
            _ => {
                return Err(validation_failure(
                    path,
                    "snapshots.max_depth must be a non-negative integer",
                    json!("non-negative integer max_depth"),
                    value.clone(),
                ));
            }
        }
    }
    for field in ["qa_only", "include_metadata"] {
        if let Some(value) = snapshots.get(field) {
            if !value.is_boolean() {
                return Err(validation_failure(
                    path,
                    &format!("snapshots.{field} must be a boolean"),
                    json!("boolean"),
                    json!(python_type_name(value)),
                ));
            }
        }
    }
    if let Some(value) = snapshots.get("metadata_keys") {
        let Some(keys) = value.as_array() else {
            return Err(validation_failure(
                path,
                "snapshots.metadata_keys must be a list",
                json!("list of safe metadata keys"),
                json!(python_type_name(value)),
            ));
        };
        for key in keys {
            match key.as_str().map(str::trim) {
                Some(key) if is_safe_metadata_key(key) => {}
                _ => {
                    return Err(validation_failure(
                        path,
                        "snapshots.metadata_keys entries must be non-empty safe metadata keys",
                        json!("non-empty metadata key using [A-Za-z0-9._:-]"),
                        key.clone(),
                    ));
                }
            }
        }
    }
    Ok(())
}

fn normalize_snapshots(value: Option<&Value>) -> Value {
    let mut snapshots = clone_object_or_default(value);
    if let Some(object) = snapshots.as_object_mut() {
        if let Some(target) = object.get_mut("target")
            && let Some(raw) = target.as_str()
        {
            *target = json!(raw.trim());
        }
        if let Some(metadata_keys) = object
            .get_mut("metadata_keys")
            .and_then(Value::as_array_mut)
        {
            for key in metadata_keys {
                if let Some(raw) = key.as_str() {
                    *key = json!(raw.trim());
                }
            }
        }
    }
    snapshots
}

fn is_safe_metadata_key(key: &str) -> bool {
    !key.is_empty()
        && key
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | '.' | ':'))
}

fn trim_step_fields(step_value: &mut Value, fields: &[&str]) {
    let Some(step) = step_value.as_object_mut() else {
        return;
    };
    for field in fields {
        if let Some(value) = step.get_mut(*field) {
            if let Some(raw) = value.as_str() {
                *value = json!(raw.trim());
            }
        }
    }
}

fn trim_to_point_fields(step_value: &mut Value) {
    if let Some(to_point) = step_value
        .as_object_mut()
        .and_then(|step| step.get_mut("to_point"))
    {
        trim_step_fields(to_point, &["provider", "method", "id"]);
    }
}

fn trim_to_point_fields_named(value: &mut Value, fields: &[&str]) {
    let Some(object) = value.as_object_mut() else {
        return;
    };
    for field in fields {
        if let Some(point) = object.get_mut(*field) {
            trim_step_fields(point, &["provider", "method", "id"]);
        }
    }
}

fn normalize_assertion(assertion: &Value) -> Value {
    let Some(assertion) = assertion.as_object() else {
        return assertion.clone();
    };
    let mut normalized = assertion.clone();
    if let Some(layout_audit) = normalized.get_mut("layout_audit") {
        if let Some(mode) = layout_audit.as_str() {
            *layout_audit = json!(mode.trim());
        }
    }
    if let Some(screenshot_match) = normalized.get_mut("screenshot_match") {
        if let Some(screenshot_match) = screenshot_match.as_object_mut() {
            if let Some(snapshot) = screenshot_match.get_mut("snapshot") {
                if let Some(snapshot_name) = snapshot.as_str() {
                    *snapshot = json!(snapshot_name.trim());
                }
            }
            if let Some(baseline) = screenshot_match.get_mut("baseline") {
                if let Some(path) = baseline.as_str() {
                    *baseline = json!(path.trim());
                }
            }
        }
    }
    if let Some(text_equals) = normalized.get_mut("text_equals") {
        trim_assertion_fields(text_equals, &["target"]);
    }
    if let Some(text_contains) = normalized.get_mut("text_contains") {
        trim_assertion_fields(text_contains, &["target"]);
    }
    if let Some(max_delta_position) = normalized.get_mut("max_delta_position") {
        trim_assertion_fields(max_delta_position, &["target"]);
        if let Some(max_delta_position) = max_delta_position.as_object_mut() {
            if !max_delta_position.contains_key("max")
                && let Some(max_delta) = max_delta_position.get("max_delta").cloned()
            {
                max_delta_position.insert("max".to_string(), max_delta);
            }
            if !max_delta_position.contains_key("window_frames")
                && !max_delta_position.contains_key("window_seconds")
            {
                max_delta_position.insert("window_frames".to_string(), json!(1));
            }
        }
    }
    if let Some(follows) = normalized.get_mut("follows") {
        trim_assertion_fields(follows, &["leader", "follower"]);
    }
    if let Some(within_limits) = normalized.get_mut("within_limits") {
        trim_assertion_fields(
            within_limits,
            &[
                "target",
                "source",
                "provider",
                "provider_id",
                "method",
                "path",
            ],
        );
    }
    if let Some(trend) = normalized.get_mut("trend") {
        trim_assertion_fields(
            trend,
            &[
                "target",
                "trend",
                "property",
                "field",
                "source",
                "provider",
                "provider_id",
                "method",
                "path",
            ],
        );
    }
    if let Some(distance) = normalized.get_mut("distance") {
        trim_assertion_fields(distance, &["a", "b"]);
        trim_to_point_fields_named(
            distance,
            &["a_point", "b_point", "target_point", "other_point"],
        );
    }
    if let Some(distance_between) = normalized.remove("distance_between") {
        let mut distance = distance_between;
        trim_assertion_fields(&mut distance, &["a", "b"]);
        trim_to_point_fields_named(
            &mut distance,
            &["a_point", "b_point", "target_point", "other_point"],
        );
        if let Some(distance) = distance.as_object_mut() {
            if !distance.contains_key("min")
                && let Some(min_distance) = distance.get("min_distance").cloned()
            {
                distance.insert("min".to_string(), min_distance);
            }
            if !distance.contains_key("max")
                && let Some(max_distance) = distance.get("max_distance").cloned()
            {
                distance.insert("max".to_string(), max_distance);
            }
        }
        normalized.insert("distance".to_string(), distance);
    }
    if let Some(distance_changed) = normalized.get_mut("distance_changed") {
        trim_assertion_fields(distance_changed, &["a", "b"]);
    }
    if let Some(visible_pixels) = normalized.get_mut("visible_pixels") {
        trim_assertion_fields(visible_pixels, &["target", "snapshot"]);
    }
    if let Some(saved_snapshot_pixels) = normalized.get_mut("saved_snapshot_pixels") {
        trim_assertion_fields(saved_snapshot_pixels, &["target", "snapshot"]);
    }
    if let Some(not_occluded) = normalized.get_mut("not_occluded") {
        trim_assertion_fields(not_occluded, &["target", "occluder", "snapshot"]);
    }
    if let Some(renders_above) = normalized.get_mut("renders_above") {
        trim_assertion_fields(renders_above, &["target", "other", "snapshot", "a", "b"]);
        normalize_render_order_aliases(renders_above);
    }
    if let Some(renders_below) = normalized.get_mut("renders_below") {
        trim_assertion_fields(renders_below, &["target", "other", "snapshot", "a", "b"]);
        normalize_render_order_aliases(renders_below);
    }
    if let Some(never) = normalized.get_mut("never") {
        trim_assertion_fields(
            never,
            &[
                "target",
                "property",
                "text",
                "token",
                "source",
                "provider",
                "provider_id",
                "method",
                "path",
            ],
        );
        if let Some(never) = never.as_object_mut() {
            if let Some(forbidden) = never.get_mut("forbidden") {
                match forbidden {
                    Value::String(raw) => {
                        *forbidden = json!(raw.trim());
                    }
                    Value::Array(values) => {
                        for value in values {
                            if let Some(raw) = value.as_str() {
                                *value = json!(raw.trim());
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
    }
    if let Some(observe_until) = normalized.get_mut("observe_until") {
        trim_assertion_fields(observe_until, &["target", "method", "source", "property"]);
        if let Some(observe_until) = observe_until.as_object_mut()
            && let Some(matches) = observe_until
                .get_mut("matches")
                .and_then(Value::as_object_mut)
        {
            let trimmed_entries = matches
                .iter()
                .map(|(key, value)| (key.trim().to_string(), value.clone()))
                .collect::<Map<_, _>>();
            *matches = trimmed_entries;
        }
    }
    if let Some(surface_probe) = normalized.get_mut("surface_probe") {
        trim_assertion_fields(
            surface_probe,
            &[
                "target",
                "provider",
                "provider_id",
                "provider_method",
                "coordinate_space",
                "surface",
                "probe_id",
                "expected_surface",
            ],
        );
        if let Some(surface_probe) = surface_probe.as_object_mut()
            && let Some(points) = surface_probe
                .get_mut("points")
                .and_then(Value::as_array_mut)
        {
            for point in points {
                if let Some(point) = point.as_object_mut()
                    && let Some(expected) = point.get_mut("expected")
                    && let Some(raw) = expected.as_str()
                {
                    *expected = json!(raw.trim());
                }
            }
        }
    }
    if let Some(surface_matches) = normalized.get_mut("surface_matches") {
        trim_assertion_fields(surface_matches, &["target", "surface", "expected"]);
        if let Some(surface_matches) = surface_matches.as_object_mut()
            && !surface_matches.contains_key("surface")
            && let Some(expected) = surface_matches.get("expected").cloned()
        {
            surface_matches.insert("surface".to_string(), expected);
        }
    }
    Value::Object(normalized)
}

fn trim_assertion_fields(assertion_value: &mut Value, fields: &[&str]) {
    let Some(assertion) = assertion_value.as_object_mut() else {
        return;
    };
    for field in fields {
        if let Some(value) = assertion.get_mut(*field) {
            if let Some(raw) = value.as_str() {
                *value = json!(raw.trim());
            }
        }
    }
}

fn normalize_render_order_aliases(assertion_value: &mut Value) {
    let Some(assertion) = assertion_value.as_object_mut() else {
        return;
    };
    if !assertion.contains_key("target")
        && let Some(a) = assertion.get("a").cloned()
    {
        assertion.insert("target".to_string(), a);
    }
    if !assertion.contains_key("other")
        && let Some(b) = assertion.get("b").cloned()
    {
        assertion.insert("other".to_string(), b);
    }
}

fn require_non_empty_string_field(
    path: &Path,
    value: &Map<String, Value>,
    assertion_name: &str,
    field_name: &str,
) -> Result<(), ScenarioError> {
    if !value
        .get(field_name)
        .map(is_non_empty_string)
        .unwrap_or(false)
    {
        return Err(validation_failure(
            path,
            &format!("{assertion_name} assertion field '{field_name}' must be a non-empty string"),
            json!("non-empty string"),
            value.get(field_name).cloned().unwrap_or(Value::Null),
        ));
    }
    Ok(())
}

fn require_numeric_field(
    path: &Path,
    value: &Map<String, Value>,
    assertion_name: &str,
    field_name: &str,
) -> Result<(), ScenarioError> {
    if !value.get(field_name).map(Value::is_number).unwrap_or(false) {
        return Err(validation_failure(
            path,
            &format!("{assertion_name} assertion field '{field_name}' must be a number"),
            json!("number"),
            value.get(field_name).cloned().unwrap_or(Value::Null),
        ));
    }
    Ok(())
}

fn require_finite_non_negative_number(
    path: &Path,
    value: &Map<String, Value>,
    assertion_name: &str,
    field_name: &str,
) -> Result<f64, ScenarioError> {
    let raw = value.get(field_name).cloned().unwrap_or(Value::Null);
    let Some(number) = raw.as_f64() else {
        return Err(validation_failure(
            path,
            &format!(
                "{assertion_name} assertion field '{field_name}' must be a finite non-negative number"
            ),
            json!("finite non-negative number"),
            raw,
        ));
    };
    if !number.is_finite() || number < 0.0 {
        return Err(validation_failure(
            path,
            &format!(
                "{assertion_name} assertion field '{field_name}' must be a finite non-negative number"
            ),
            json!("finite non-negative number"),
            value.get(field_name).cloned().unwrap_or(Value::Null),
        ));
    }
    Ok(number)
}

fn validate_distance_assertion_payload(
    path: &Path,
    value: &Value,
    assertion_name: &str,
    min_field: &str,
    max_field: &str,
) -> Result<(), ScenarioError> {
    let Some(value) = value.as_object() else {
        return Err(validation_failure(
            path,
            &format!("{assertion_name} assertion value must be an object"),
            json!("object"),
            json!(python_type_name(value)),
        ));
    };
    if value.contains_key("a_point") || value.contains_key("target_point") {
        let point_value = value
            .get("a_point")
            .or_else(|| value.get("target_point"))
            .unwrap_or(&Value::Null);
        validate_to_point_object(path, assertion_name, point_value)?;
    } else {
        require_non_empty_string_field(path, value, assertion_name, "a")?;
    }
    if value.contains_key("b_point") || value.contains_key("other_point") {
        let point_value = value
            .get("b_point")
            .or_else(|| value.get("other_point"))
            .unwrap_or(&Value::Null);
        validate_to_point_object(path, assertion_name, point_value)?;
    } else {
        require_non_empty_string_field(path, value, assertion_name, "b")?;
    }
    let min = optional_finite_non_negative_number(path, value, assertion_name, min_field)?;
    let max = optional_finite_non_negative_number(path, value, assertion_name, max_field)?;
    if min.is_none() && max.is_none() {
        let message = if assertion_name == "distance" {
            "distance assertion requires at least one of 'min' or 'max'".to_string()
        } else {
            format!("{assertion_name} assertion requires at least one distance bound")
        };
        return Err(validation_failure(
            path,
            &message,
            json!(format!("at least one of '{min_field}' or '{max_field}'")),
            json!(value),
        ));
    }
    Ok(())
}

fn require_positive_number_field(
    path: &Path,
    value: &Map<String, Value>,
    assertion_name: &str,
    field_name: &str,
) -> Result<f64, ScenarioError> {
    let raw = value.get(field_name).cloned().unwrap_or(Value::Null);
    let Some(number) = raw.as_f64() else {
        return Err(validation_failure(
            path,
            &format!("{assertion_name} assertion field '{field_name}' must be a positive number"),
            json!("positive number"),
            raw,
        ));
    };
    if !number.is_finite() || number <= 0.0 {
        return Err(validation_failure(
            path,
            &format!("{assertion_name} assertion field '{field_name}' must be a positive number"),
            json!("positive number"),
            raw,
        ));
    }
    Ok(number)
}

fn optional_finite_non_negative_number(
    path: &Path,
    value: &Map<String, Value>,
    assertion_name: &str,
    field_name: &str,
) -> Result<Option<f64>, ScenarioError> {
    if !value.contains_key(field_name) {
        return Ok(None);
    }
    Ok(Some(require_finite_non_negative_number(
        path,
        value,
        assertion_name,
        field_name,
    )?))
}

fn validate_temporal_window(
    path: &Path,
    value: &Map<String, Value>,
    assertion_name: &str,
) -> Result<(), ScenarioError> {
    let has_frames = value.contains_key("window_frames");
    let has_seconds = value.contains_key("window_seconds");
    if has_frames == has_seconds {
        return Err(validation_failure(
            path,
            &format!(
                "{assertion_name} assertion must declare exactly one of 'window_frames' or 'window_seconds'"
            ),
            json!("exactly one window form"),
            Value::Object(value.clone()),
        ));
    }
    let field = if has_frames {
        "window_frames"
    } else {
        "window_seconds"
    };
    let Some(window) = value.get(field).and_then(Value::as_f64) else {
        return Err(validation_failure(
            path,
            &format!("{assertion_name} assertion field '{field}' must be a positive number"),
            json!("positive number"),
            value.get(field).cloned().unwrap_or(Value::Null),
        ));
    };
    if window <= 0.0 {
        return Err(validation_failure(
            path,
            &format!("{assertion_name} assertion field '{field}' must be a positive number"),
            json!("positive number"),
            value.get(field).cloned().unwrap_or(Value::Null),
        ));
    }
    Ok(())
}

/// Validate the optional state-expression observation shared by temporal
/// assertions (`within_limits`, `never`, `trend`). Absent `source` means the
/// assertion observes the resolved target node directly (legacy behavior) and
/// this returns `Ok(false)`. With `source` present the payload reuses the
/// `state_matches`/`wait_until` state-reading vocabulary: `provider` (registered
/// state provider) or `method` (`target` + read-only QA method), plus an
/// optional dot-separated `path` into the returned state.
fn validate_temporal_state_observation(
    path: &Path,
    value: &Map<String, Value>,
    assertion_name: &str,
) -> Result<bool, ScenarioError> {
    if !value.contains_key("source") {
        return Ok(false);
    }
    match value.get("source").and_then(Value::as_str).map(str::trim) {
        Some("provider") => {
            if value.contains_key("provider") || !value.contains_key("provider_id") {
                require_non_empty_string_field(path, value, assertion_name, "provider")?;
            } else {
                require_non_empty_string_field(path, value, assertion_name, "provider_id")?;
            }
        }
        Some("method") | Some("qa_method") => {
            require_non_empty_string_field(path, value, assertion_name, "target")?;
            require_non_empty_string_field(path, value, assertion_name, "method")?;
        }
        _ => {
            return Err(validation_failure(
                path,
                &format!(
                    "{assertion_name} assertion field 'source' must be 'provider' or 'method'"
                ),
                json!(["provider", "method"]),
                value.get("source").cloned().unwrap_or(Value::Null),
            ));
        }
    }
    if let Some(path_value) = value.get("path") {
        let Some(path_value) = path_value.as_str() else {
            return Err(validation_failure(
                path,
                &format!(
                    "{assertion_name} assertion field 'path' must use non-empty dot-separated state paths"
                ),
                json!("non-empty dot-separated state path"),
                path_value.clone(),
            ));
        };
        validate_state_path(path, path_value, assertion_name, "path")?;
    }
    Ok(true)
}

fn validate_temporal_sampling(
    determinism: &Map<String, Value>,
    path: &Path,
) -> Result<(), ScenarioError> {
    let Some(temporal_sampling) = determinism.get("temporal_sampling") else {
        return Ok(());
    };
    validate_temporal_sampling_value(path, "determinism.temporal_sampling", temporal_sampling)
}

fn validate_assertion_temporal_sampling(
    path: &Path,
    value: &Map<String, Value>,
    assertion_name: &str,
) -> Result<(), ScenarioError> {
    let Some(temporal_sampling) = value.get("temporal_sampling") else {
        return Ok(());
    };
    validate_temporal_sampling_value(
        path,
        &format!("{assertion_name}.temporal_sampling"),
        temporal_sampling,
    )
}

fn validate_temporal_sampling_value(
    path: &Path,
    field_path: &str,
    temporal_sampling: &Value,
) -> Result<(), ScenarioError> {
    let Some(temporal_sampling) = temporal_sampling.as_object() else {
        return Err(validation_failure(
            path,
            &format!("{field_path} must be an object"),
            json!("object"),
            json!(python_type_name(temporal_sampling)),
        ));
    };
    let Some(mode) = temporal_sampling.get("mode").and_then(Value::as_str) else {
        return Err(validation_failure(
            path,
            &format!("{field_path}.mode must be a supported mode"),
            json!(["fixed_delta", "max_frame_duration"]),
            temporal_sampling
                .get("mode")
                .cloned()
                .unwrap_or(Value::Null),
        ));
    };
    let expected_field = match mode.trim() {
        "fixed_delta" => "fixed_delta_seconds",
        "max_frame_duration" => "max_frame_duration_seconds",
        _ => {
            return Err(validation_failure(
                path,
                &format!("{field_path}.mode must be a supported mode"),
                json!(["fixed_delta", "max_frame_duration"]),
                json!(mode),
            ));
        }
    };
    let duration_fields = ["fixed_delta_seconds", "max_frame_duration_seconds"]
        .into_iter()
        .filter(|field| temporal_sampling.contains_key(*field))
        .collect::<Vec<_>>();
    if duration_fields.len() != 1 || duration_fields[0] != expected_field {
        return Err(validation_failure(
            path,
            &format!("{field_path} must declare exactly one duration field matching mode"),
            json!({
                "fixed_delta": "fixed_delta_seconds",
                "max_frame_duration": "max_frame_duration_seconds",
            }),
            Value::Object(temporal_sampling.clone()),
        ));
    }
    let Some(duration) = temporal_sampling
        .get(expected_field)
        .and_then(Value::as_f64)
        .filter(|value| value.is_finite() && *value > 0.0)
    else {
        return Err(validation_failure(
            path,
            &format!("{field_path}.{expected_field} must be a positive number"),
            json!("positive number"),
            temporal_sampling
                .get(expected_field)
                .cloned()
                .unwrap_or(Value::Null),
        ));
    };
    if duration <= 0.0 {
        return Err(validation_failure(
            path,
            &format!("{field_path}.{expected_field} must be a positive number"),
            json!("positive number"),
            temporal_sampling
                .get(expected_field)
                .cloned()
                .unwrap_or(Value::Null),
        ));
    }
    Ok(())
}

fn validate_observe_until_assertion(path: &Path, value: &Value) -> Result<(), ScenarioError> {
    let Some(value) = value.as_object() else {
        return Err(validation_failure(
            path,
            "observe_until assertion value must be an object",
            json!("object"),
            json!(python_type_name(value)),
        ));
    };
    require_non_empty_string_field(path, value, "observe_until", "target")?;
    validate_assertion_temporal_sampling(path, value, "observe_until")?;
    validate_temporal_window(path, value, "observe_until")?;
    validate_optional_positive_integer(path, value, "observe_until", "sample_every_frames")?;

    let has_property = value.contains_key("property")
        || value.contains_key("operator")
        || value.contains_key("expected");
    let has_visible = value.contains_key("visible");
    let has_state_matches = value.contains_key("source")
        || value.contains_key("method")
        || value.contains_key("matches");
    let family_count = [has_property, has_visible, has_state_matches]
        .into_iter()
        .filter(|present| *present)
        .count();
    if family_count != 1 {
        return Err(validation_failure(
            path,
            "observe_until assertion must declare exactly one condition family",
            json!([
                "property/operator/expected",
                "visible",
                "source/method/matches"
            ]),
            Value::Object(value.clone()),
        ));
    }

    if has_property {
        require_non_empty_string_field(path, value, "observe_until", "property")?;
        require_non_empty_string_field(path, value, "observe_until", "operator")?;
        if !value.contains_key("expected") {
            return Err(validation_failure(
                path,
                "observe_until assertion field 'expected' is required for property conditions",
                json!("expected value"),
                Value::Null,
            ));
        }
        validate_within_operator_tolerance(path, value, "observe_until assertion")?;
    } else if has_visible {
        if !value.get("visible").is_some_and(Value::is_boolean) {
            return Err(validation_failure(
                path,
                "observe_until assertion field 'visible' must be a boolean",
                json!("boolean"),
                value.get("visible").cloned().unwrap_or(Value::Null),
            ));
        }
    } else {
        require_non_empty_string_field(path, value, "observe_until", "source")?;
        require_non_empty_string_field(path, value, "observe_until", "method")?;
        let Some(matches) = value.get("matches").and_then(Value::as_object) else {
            return Err(validation_failure(
                path,
                "observe_until assertion field 'matches' must be an object",
                json!("object"),
                value.get("matches").cloned().unwrap_or(Value::Null),
            ));
        };
        if matches.is_empty() {
            return Err(validation_failure(
                path,
                "observe_until assertion field 'matches' must include at least one state path",
                json!("non-empty object"),
                Value::Object(matches.clone()),
            ));
        }
        for path_value in matches.keys() {
            validate_state_path(path, path_value, "observe_until", "matches")?;
        }
    }

    Ok(())
}

fn validate_state_path(
    path: &Path,
    path_value: &str,
    assertion_name: &str,
    field_name: &str,
) -> Result<(), ScenarioError> {
    let trimmed = path_value.trim();
    if trimmed.is_empty()
        || trimmed.split('.').any(|segment| segment.trim().is_empty())
        || trimmed != path_value
    {
        return Err(validation_failure(
            path,
            &format!(
                "{assertion_name} assertion field '{field_name}' must use non-empty dot-separated state paths"
            ),
            json!("non-empty dot-separated state path"),
            json!(path_value),
        ));
    }
    Ok(())
}

/// The `within` operator asserts `|actual - expected| <= tolerance` with an
/// absolute tolerance (distinct from `approx_equals`' `epsilon`). Wherever a
/// state check declares `operator: within`, `tolerance` must be a positive
/// number.
fn validate_within_operator_tolerance(
    path: &Path,
    value: &Map<String, Value>,
    context_name: &str,
) -> Result<(), ScenarioError> {
    let operator = value
        .get("operator")
        .or_else(|| value.get("op"))
        .and_then(Value::as_str)
        .map(str::trim)
        .unwrap_or_default();
    if operator != "within" {
        return Ok(());
    }
    let raw = value.get("tolerance").cloned().unwrap_or(Value::Null);
    let valid = raw
        .as_f64()
        .map(|number| number.is_finite() && number > 0.0)
        .unwrap_or(false);
    if !valid {
        return Err(validation_failure(
            path,
            &format!(
                "{context_name} field 'tolerance' must be a positive number for operator 'within'"
            ),
            json!("positive number"),
            raw,
        ));
    }
    Ok(())
}

/// Minimal structural validation for `state_matches` assertions. Historically
/// this assertion had no Rust-side validation, so a `matches` mis-shape sailed
/// through until it silently failed at runtime. We keep this lenient (consumers
/// use a wide field vocabulary and empty `path` with `where`/`value_path`) but
/// reject clearly malformed `source`/`matches` shapes early.
fn validate_state_matches_assertion(path: &Path, value: &Value) -> Result<(), ScenarioError> {
    let Some(value) = value.as_object() else {
        return Err(validation_failure(
            path,
            "state_matches assertion value must be an object",
            json!("object"),
            json!(python_type_name(value)),
        ));
    };
    if let Some(source) = value.get("source").and_then(Value::as_str) {
        let trimmed = source.trim();
        if !trimmed.is_empty() && !STATE_MATCH_SOURCES.contains(&trimmed) {
            return Err(validation_failure(
                path,
                "state_matches assertion field 'source' must be a supported source",
                json!(STATE_MATCH_SOURCES),
                json!(source),
            ));
        }
    }
    validate_within_operator_tolerance(path, value, "state_matches assertion")?;
    match value.get("matches") {
        None | Some(Value::Object(_)) => {}
        Some(Value::Array(entries)) => {
            for entry in entries {
                let Some(entry) = entry.as_object() else {
                    return Err(validation_failure(
                        path,
                        "state_matches assertion 'matches' entries must be objects",
                        json!("object"),
                        entry.clone(),
                    ));
                };
                validate_within_operator_tolerance(path, entry, "state_matches assertion check")?;
            }
        }
        Some(other) => {
            return Err(validation_failure(
                path,
                "state_matches assertion field 'matches' must be a list or object",
                json!("list or object"),
                json!(python_type_name(other)),
            ));
        }
    }
    Ok(())
}

/// Validate a `record_state` step: a read-only observation that captures a
/// state VALUE as run evidence under a scenario-unique `name`. It reuses the
/// exact `{source, provider | target+method, path}` state-source vocabulary that
/// `wait_until`/`state_matches` read from (same `require_non_empty_string_field`
/// and `validate_state_path` doc-side validators), but records the value instead
/// of asserting on it — so it carries no operator/expected fields. `name`
/// follows the same safe-artifact-segment charset as `snapshot` names because it
/// keys the `state_records` evidence map, and must be unique across the
/// scenario's record_state steps so no earlier record is silently overwritten.
fn validate_record_state_step(
    path: &Path,
    value: &Value,
    seen_names: &mut Vec<String>,
) -> Result<(), ScenarioError> {
    const RECORD_STATE_FIELDS: &[&str] = &[
        "name",
        "source",
        "provider",
        "provider_id",
        "method",
        "target",
        "path",
    ];

    let Some(value) = value.as_object() else {
        return Err(validation_failure(
            path,
            "record_state step value must be an object",
            json!("object"),
            json!(python_type_name(value)),
        ));
    };

    for key in value.keys() {
        if !RECORD_STATE_FIELDS.contains(&key.as_str()) {
            return Err(validation_failure(
                path,
                &format!("record_state step uses unsupported field '{key}'"),
                json!(RECORD_STATE_FIELDS),
                json!({ "unknown_key": key }),
            ));
        }
    }

    let Some(name) = value.get("name").and_then(Value::as_str) else {
        return Err(validation_failure(
            path,
            "record_state step field 'name' must be a non-empty string",
            json!("non-empty string"),
            value.get("name").cloned().unwrap_or(Value::Null),
        ));
    };
    let trimmed_name = name.trim();
    if !is_safe_artifact_segment(trimmed_name) {
        return Err(validation_failure(
            path,
            "record_state step field 'name' must be a safe artifact path segment",
            json!("letters, numbers, dot, underscore, or hyphen"),
            json!(name),
        ));
    }
    if seen_names.iter().any(|seen| seen == trimmed_name) {
        return Err(validation_failure(
            path,
            &format!(
                "record_state step name '{trimmed_name}' is already used by an earlier record_state step"
            ),
            json!("unique record_state name"),
            json!(trimmed_name),
        ));
    }
    seen_names.push(trimmed_name.to_string());

    match value.get("source").and_then(Value::as_str).map(str::trim) {
        Some("provider") => {
            require_non_empty_string_field(path, value, "record_state", "provider")?;
        }
        Some("method") | Some("qa_method") => {
            require_non_empty_string_field(path, value, "record_state", "target")?;
            require_non_empty_string_field(path, value, "record_state", "method")?;
        }
        _ => {
            return Err(validation_failure(
                path,
                "record_state step field 'source' must be 'provider' or 'method'",
                json!(["provider", "method"]),
                value.get("source").cloned().unwrap_or(Value::Null),
            ));
        }
    }

    if let Some(path_value) = value.get("path") {
        let path_str = path_value.as_str().unwrap_or_default();
        validate_state_path(path, path_str, "record_state", "path")?;
    }

    Ok(())
}

/// Validate a `wait_until` step: a bounded state predicate that reuses the
/// `state_matches` source/operator vocabulary.
fn validate_wait_until_step(path: &Path, value: &Value) -> Result<(), ScenarioError> {
    const WAIT_UNTIL_FIELDS: &[&str] = &[
        "source",
        "provider",
        "provider_id",
        "method",
        "target",
        "path",
        "operator",
        "expected",
        "epsilon",
        "tolerance",
        "where",
        "value_path",
        "timeout_frames",
        "poll_every_frames",
    ];

    let Some(value) = value.as_object() else {
        return Err(validation_failure(
            path,
            "wait_until step value must be an object",
            json!("object"),
            json!(python_type_name(value)),
        ));
    };

    for key in value.keys() {
        if !WAIT_UNTIL_FIELDS.contains(&key.as_str()) {
            return Err(validation_failure(
                path,
                &format!("wait_until step uses unsupported field '{key}'"),
                json!(WAIT_UNTIL_FIELDS),
                json!({ "unknown_key": key }),
            ));
        }
    }

    match value.get("source").and_then(Value::as_str).map(str::trim) {
        Some("provider") => {
            require_non_empty_string_field(path, value, "wait_until", "provider")?;
        }
        Some("method") | Some("qa_method") => {
            require_non_empty_string_field(path, value, "wait_until", "target")?;
            require_non_empty_string_field(path, value, "wait_until", "method")?;
        }
        _ => {
            return Err(validation_failure(
                path,
                "wait_until step field 'source' must be 'provider' or 'method'",
                json!(["provider", "method"]),
                value.get("source").cloned().unwrap_or(Value::Null),
            ));
        }
    }

    let path_value = value
        .get("path")
        .and_then(Value::as_str)
        .unwrap_or_default();
    validate_state_path(path, path_value, "wait_until", "path")?;

    let operator = value
        .get("operator")
        .and_then(Value::as_str)
        .map(str::trim)
        .unwrap_or_default();
    if !STATE_MATCH_OPERATORS.contains(&operator) {
        return Err(validation_failure(
            path,
            "wait_until step field 'operator' must be a supported comparison",
            json!(STATE_MATCH_OPERATORS),
            value.get("operator").cloned().unwrap_or(Value::Null),
        ));
    }
    if OPERATORS_REQUIRING_EXPECTED.contains(&operator) && !value.contains_key("expected") {
        return Err(validation_failure(
            path,
            &format!("wait_until step field 'expected' is required for operator '{operator}'"),
            json!("expected value"),
            Value::Null,
        ));
    }
    validate_within_operator_tolerance(path, value, "wait_until step")?;
    if let Some(epsilon) = value.get("epsilon")
        && epsilon
            .as_f64()
            .map(|number| !number.is_finite())
            .unwrap_or(true)
    {
        return Err(validation_failure(
            path,
            "wait_until step field 'epsilon' must be a finite number",
            json!("finite number"),
            epsilon.clone(),
        ));
    }

    let Some(timeout_frames) = value.get("timeout_frames") else {
        return Err(validation_failure(
            path,
            "wait_until step field 'timeout_frames' is required",
            json!("positive integer"),
            Value::Null,
        ));
    };
    if timeout_frames.as_i64().is_none()
        || !timeout_frames.is_i64()
        || timeout_frames.as_i64().unwrap() <= 0
    {
        return Err(validation_failure(
            path,
            "wait_until step field 'timeout_frames' must be a positive integer",
            json!("positive integer"),
            timeout_frames.clone(),
        ));
    }
    validate_optional_positive_integer(path, value, "wait_until", "poll_every_frames")?;
    Ok(())
}

/// Validate a `drive_policy` step: a bounded closed-loop input step that calls
/// a registered policy provider each tick and emits the joypad inputs it
/// returns. The optional `until` early-stop predicate reuses the
/// `wait_until`/`state_matches` source/operator vocabulary; its frame budget
/// comes from the step's own `max_frames`, so `timeout_frames` is not allowed.
fn validate_drive_policy_step(path: &Path, value: &Value) -> Result<(), ScenarioError> {
    const DRIVE_POLICY_FIELDS: &[&str] = &[
        "policy",
        "params",
        "devices",
        "max_frames",
        "tick_every_frames",
        "until",
    ];

    let Some(value) = value.as_object() else {
        return Err(validation_failure(
            path,
            "drive_policy step value must be an object",
            json!("object"),
            json!(python_type_name(value)),
        ));
    };

    for key in value.keys() {
        if !DRIVE_POLICY_FIELDS.contains(&key.as_str()) {
            return Err(validation_failure(
                path,
                &format!("drive_policy step uses unsupported field '{key}'"),
                json!(DRIVE_POLICY_FIELDS),
                json!({ "unknown_key": key }),
            ));
        }
    }

    if !value
        .get("policy")
        .map(is_non_empty_string)
        .unwrap_or(false)
    {
        return Err(validation_failure(
            path,
            "drive_policy step field 'policy' must be a non-empty string",
            json!("non-empty string"),
            value.get("policy").cloned().unwrap_or(Value::Null),
        ));
    }

    if let Some(params) = value.get("params") {
        if !params.is_object() {
            return Err(validation_failure(
                path,
                "drive_policy step field 'params' must be an object when present",
                json!("object"),
                json!(python_type_name(params)),
            ));
        }
    }

    let Some(devices) = value.get("devices").and_then(Value::as_array) else {
        return Err(validation_failure(
            path,
            "drive_policy step field 'devices' must be a non-empty list of device integers",
            json!("non-empty list of non-negative integers"),
            value.get("devices").cloned().unwrap_or(Value::Null),
        ));
    };
    if devices.is_empty() {
        return Err(validation_failure(
            path,
            "drive_policy step field 'devices' must be a non-empty list of device integers",
            json!("non-empty list of non-negative integers"),
            json!(devices),
        ));
    }
    for device in devices {
        if device.as_i64().is_none() || !device.is_i64() || device.as_i64().unwrap() < 0 {
            return Err(validation_failure(
                path,
                "drive_policy step 'devices' entries must be non-negative integers",
                json!("non-negative integer"),
                device.clone(),
            ));
        }
    }

    let Some(max_frames) = value.get("max_frames") else {
        return Err(validation_failure(
            path,
            "drive_policy step field 'max_frames' is required",
            json!("positive integer"),
            Value::Null,
        ));
    };
    if max_frames.as_i64().is_none() || !max_frames.is_i64() || max_frames.as_i64().unwrap() <= 0 {
        return Err(validation_failure(
            path,
            "drive_policy step field 'max_frames' must be a positive integer",
            json!("positive integer"),
            max_frames.clone(),
        ));
    }
    validate_optional_positive_integer(path, value, "drive_policy", "tick_every_frames")?;

    if let Some(until) = value.get("until") {
        validate_drive_policy_until(path, until)?;
    }
    Ok(())
}

/// Validate the `until` predicate of a `drive_policy` step: the state-reading
/// fields of `wait_until` without its own frame budget.
fn validate_drive_policy_until(path: &Path, value: &Value) -> Result<(), ScenarioError> {
    const DRIVE_POLICY_UNTIL_FIELDS: &[&str] = &[
        "source",
        "provider",
        "provider_id",
        "method",
        "target",
        "path",
        "operator",
        "expected",
        "epsilon",
        "tolerance",
        "where",
        "value_path",
    ];

    let Some(value) = value.as_object() else {
        return Err(validation_failure(
            path,
            "drive_policy step field 'until' must be an object",
            json!("object"),
            json!(python_type_name(value)),
        ));
    };

    for key in value.keys() {
        if !DRIVE_POLICY_UNTIL_FIELDS.contains(&key.as_str()) {
            return Err(validation_failure(
                path,
                &format!("drive_policy step 'until' uses unsupported field '{key}'"),
                json!(DRIVE_POLICY_UNTIL_FIELDS),
                json!({ "unknown_key": key }),
            ));
        }
    }

    match value.get("source").and_then(Value::as_str).map(str::trim) {
        Some("provider") => {
            require_non_empty_string_field(path, value, "drive_policy step 'until'", "provider")?;
        }
        Some("method") | Some("qa_method") => {
            require_non_empty_string_field(path, value, "drive_policy step 'until'", "target")?;
            require_non_empty_string_field(path, value, "drive_policy step 'until'", "method")?;
        }
        _ => {
            return Err(validation_failure(
                path,
                "drive_policy step 'until' field 'source' must be 'provider' or 'method'",
                json!(["provider", "method"]),
                value.get("source").cloned().unwrap_or(Value::Null),
            ));
        }
    }

    let path_value = value
        .get("path")
        .and_then(Value::as_str)
        .unwrap_or_default();
    validate_state_path(path, path_value, "drive_policy step 'until'", "path")?;

    let operator = value
        .get("operator")
        .and_then(Value::as_str)
        .map(str::trim)
        .unwrap_or_default();
    if !STATE_MATCH_OPERATORS.contains(&operator) {
        return Err(validation_failure(
            path,
            "drive_policy step 'until' field 'operator' must be a supported comparison",
            json!(STATE_MATCH_OPERATORS),
            value.get("operator").cloned().unwrap_or(Value::Null),
        ));
    }
    if OPERATORS_REQUIRING_EXPECTED.contains(&operator) && !value.contains_key("expected") {
        return Err(validation_failure(
            path,
            &format!(
                "drive_policy step 'until' field 'expected' is required for operator '{operator}'"
            ),
            json!("expected value"),
            Value::Null,
        ));
    }
    validate_within_operator_tolerance(path, value, "drive_policy step 'until'")?;
    if let Some(epsilon) = value.get("epsilon")
        && epsilon
            .as_f64()
            .map(|number| !number.is_finite())
            .unwrap_or(true)
    {
        return Err(validation_failure(
            path,
            "drive_policy step 'until' field 'epsilon' must be a finite number",
            json!("finite number"),
            epsilon.clone(),
        ));
    }
    Ok(())
}

fn validate_optional_positive_integer(
    path: &Path,
    value: &Map<String, Value>,
    assertion_name: &str,
    field_name: &str,
) -> Result<(), ScenarioError> {
    let Some(sample_every_frames) = value.get(field_name) else {
        return Ok(());
    };
    if sample_every_frames.as_i64().is_none()
        || !sample_every_frames.is_i64()
        || sample_every_frames.as_i64().unwrap() <= 0
    {
        return Err(validation_failure(
            path,
            &format!("{assertion_name} assertion field '{field_name}' must be a positive integer"),
            json!("positive integer"),
            sample_every_frames.clone(),
        ));
    }
    Ok(())
}

fn validate_visual_threshold_fields(
    path: &Path,
    value: &Map<String, Value>,
    assertion_name: &str,
) -> Result<(), ScenarioError> {
    if let Some(min_ratio) = value.get("min_ratio") {
        let Some(number) = min_ratio.as_f64() else {
            return Err(validation_failure(
                path,
                &format!(
                    "{assertion_name} assertion field 'min_ratio' must be between 0.0 and 1.0"
                ),
                json!("number between 0.0 and 1.0"),
                min_ratio.clone(),
            ));
        };
        if !(0.0..=1.0).contains(&number) {
            return Err(validation_failure(
                path,
                &format!(
                    "{assertion_name} assertion field 'min_ratio' must be between 0.0 and 1.0"
                ),
                json!("number between 0.0 and 1.0"),
                min_ratio.clone(),
            ));
        }
    }

    if let Some(min_count) = value.get("min_count") {
        if min_count.as_i64().is_none() || !min_count.is_i64() || min_count.as_i64().unwrap() < 0 {
            return Err(validation_failure(
                path,
                &format!(
                    "{assertion_name} assertion field 'min_count' must be a non-negative integer"
                ),
                json!("non-negative integer"),
                min_count.clone(),
            ));
        }
    }

    Ok(())
}

fn require_visual_threshold(
    path: &Path,
    value: &Map<String, Value>,
    assertion_name: &str,
) -> Result<(), ScenarioError> {
    if value.contains_key("min_ratio") || value.contains_key("min_count") {
        return Ok(());
    }
    Err(validation_failure(
        path,
        &format!("{assertion_name} assertion requires at least one of 'min_ratio' or 'min_count'"),
        json!("at least one of 'min_ratio' or 'min_count'"),
        Value::Object(value.clone()),
    ))
}

fn validate_never_observation(
    path: &Path,
    value: &Map<String, Value>,
) -> Result<(), ScenarioError> {
    let keys = ["property", "text", "token"];
    let present: Vec<&str> = keys
        .iter()
        .copied()
        .filter(|key| value.contains_key(*key))
        .collect();
    if present.len() != 1 {
        return Err(validation_failure(
            path,
            "never assertion must declare exactly one observation source: 'property', 'text', or 'token'",
            json!(["property", "text", "token"]),
            Value::Object(value.clone()),
        ));
    }
    require_non_empty_string_field(path, value, "never", present[0])
}

fn validate_never_forbidden(path: &Path, value: &Map<String, Value>) -> Result<(), ScenarioError> {
    let Some(forbidden) = value.get("forbidden") else {
        return Err(validation_failure(
            path,
            "never assertion field 'forbidden' must be a string or list of non-empty strings",
            json!("string or list of non-empty strings"),
            Value::Null,
        ));
    };
    match forbidden {
        Value::String(raw) => {
            if raw.trim().is_empty() {
                return Err(validation_failure(
                    path,
                    "never assertion field 'forbidden' must be a string or list of non-empty strings",
                    json!("string or list of non-empty strings"),
                    forbidden.clone(),
                ));
            }
        }
        Value::Array(values) => {
            if values.is_empty()
                || values.iter().any(|value| {
                    value
                        .as_str()
                        .map(|raw| raw.trim().is_empty())
                        .unwrap_or(true)
                })
            {
                return Err(validation_failure(
                    path,
                    "never assertion field 'forbidden' must be a string or list of non-empty strings",
                    json!("string or list of non-empty strings"),
                    forbidden.clone(),
                ));
            }
        }
        _ => {
            return Err(validation_failure(
                path,
                "never assertion field 'forbidden' must be a string or list of non-empty strings",
                json!("string or list of non-empty strings"),
                forbidden.clone(),
            ));
        }
    }
    Ok(())
}

fn clone_object_or_default(value: Option<&Value>) -> Value {
    match value {
        Some(Value::Object(map)) => Value::Object(map.clone()),
        _ => json!({}),
    }
}

fn clone_array_or_default(value: Option<&Value>) -> Value {
    match value {
        Some(Value::Array(items)) => Value::Array(items.clone()),
        _ => json!([]),
    }
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

fn scenario_parse_failure(path: &Path, message: &str, actual: Value) -> ScenarioError {
    ScenarioError {
        failure: FailureDetail {
            kind: "scenario_parse_failed".to_string(),
            path: Some(path.to_string_lossy().into_owned()),
            message: Some(message.to_string()),
            expected: Some(json!("valid scenario document")),
            actual: Some(actual),
            ..FailureDetail::default()
        },
    }
}

fn yaml_parse_failure(path: &Path, error: &godot_qa_core::yaml::YamlParseError) -> ScenarioError {
    ScenarioError {
        failure: FailureDetail {
            kind: "scenario_yaml_parse_failed".to_string(),
            path: Some(path.to_string_lossy().into_owned()),
            message: Some(error.detail()),
            expected: Some(json!("valid YAML scenario document")),
            actual: Some(json!(error.location_summary())),
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

fn strip_wrapping_quotes(value: &str) -> String {
    if value.len() >= 2 {
        let bytes = value.as_bytes();
        if (bytes[0] == b'"' && bytes[value.len() - 1] == b'"')
            || (bytes[0] == b'\'' && bytes[value.len() - 1] == b'\'')
        {
            return value[1..value.len() - 1].to_string();
        }
    }
    value.to_string()
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

fn python_type_name_from_error(_error: &std::io::Error) -> &'static str {
    "OSError"
}

trait PathStemExt {
    fn file_stem_string(&self) -> String;
}

impl PathStemExt for Path {
    fn file_stem_string(&self) -> String {
        self.file_stem()
            .and_then(|stem| stem.to_str())
            .unwrap_or_default()
            .to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::{
        ScenarioIndex, find_scenarios_by_name, list_scenarios, normalize_scenario_document,
        validate_scenario_document,
    };
    use serde_json::json;
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_scenarios_dir(name: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!("godot-qa-{name}-{unique}/qa/scenarios"));
        fs::create_dir_all(&path).unwrap();
        path
    }

    fn write_scenario(path: &Path, contents: &str) {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(path, contents).unwrap();
    }

    #[test]
    fn scenario_index_returns_every_duplicate_path_in_listing_order() {
        let dir = temp_scenarios_dir("index-duplicates");
        let second = dir.join("nested/b_second.yaml");
        let first = dir.join("a_first.yaml");
        let third = dir.join("z_third.json");
        write_scenario(&first, "name: shared\nsteps: []\n");
        write_scenario(&second, "name: shared\nsteps: []\n");
        write_scenario(&third, "{\"name\": \"shared\"}\n");

        let index = ScenarioIndex::load(&dir);
        assert_eq!(
            index.paths_for("shared"),
            find_scenarios_by_name(&dir, "shared").as_slice()
        );
        assert_eq!(
            index.paths_for("shared"),
            [first, second, third].as_slice(),
            "duplicate names resolve to every path, sorted by path"
        );
    }

    #[test]
    fn scenario_index_excludes_abstract_templates() {
        let dir = temp_scenarios_dir("index-abstract");
        let concrete = dir.join("concrete.yaml");
        write_scenario(&concrete, "name: shared\nsteps: []\n");
        write_scenario(
            &dir.join("template.yaml"),
            "abstract: true\nname: shared\nsteps: []\n",
        );
        write_scenario(
            &dir.join("template.json"),
            "{\"abstract\": true, \"name\": \"shared\"}\n",
        );

        let index = ScenarioIndex::load(&dir);
        assert_eq!(index.paths_for("shared"), [concrete].as_slice());
    }

    #[test]
    fn scenario_index_falls_back_to_file_stem_and_mixes_formats() {
        let dir = temp_scenarios_dir("index-fallback");
        let yaml_stem = dir.join("yaml_stem.yaml");
        let yml_stem = dir.join("yml_stem.yml");
        let json_stem = dir.join("json_stem.json");
        let json_named = dir.join("json_named.json");
        let broken = dir.join("broken.json");
        // No `name`, an empty `name`, and an unparsable document all fall back to
        // the file stem.
        write_scenario(&yaml_stem, "steps: []\n");
        write_scenario(&yml_stem, "name:\nsteps: []\n");
        write_scenario(&json_stem, "{}\n");
        write_scenario(&json_named, "{\"name\": \"Declared Name\"}\n");
        write_scenario(&broken, "{not json\n");

        let index = ScenarioIndex::load(&dir);
        assert_eq!(index.paths_for("yaml_stem"), [yaml_stem].as_slice());
        assert_eq!(index.paths_for("yml_stem"), [yml_stem].as_slice());
        assert_eq!(index.paths_for("json_stem"), [json_stem].as_slice());
        assert_eq!(index.paths_for("Declared Name"), [json_named].as_slice());
        assert_eq!(index.paths_for("broken"), [broken].as_slice());
        // Names match exactly; the stem of a declared-name file is not an alias.
        assert!(index.paths_for("json_named").is_empty());
        assert!(index.paths_for("missing").is_empty());
    }

    #[test]
    fn scenario_index_agrees_with_listing_for_every_name() {
        let dir = temp_scenarios_dir("index-parity");
        write_scenario(&dir.join("a.yaml"), "name: Alpha\nsteps: []\n");
        write_scenario(&dir.join("nested/a.yaml"), "name: Alpha\nsteps: []\n");
        write_scenario(&dir.join("b.json"), "{\"name\": \"Beta\"}\n");
        write_scenario(&dir.join("stem.yml"), "steps: []\n");
        write_scenario(&dir.join("tpl.yaml"), "abstract: true\nname: Gamma\n");

        let listed = list_scenarios(&dir);
        assert_eq!(
            listed.len(),
            4,
            "abstract template stays out of the listing"
        );
        for entry in listed {
            let name = entry.get("name").and_then(|v| v.as_str()).unwrap();
            assert_eq!(
                ScenarioIndex::load(&dir).paths_for(name),
                find_scenarios_by_name(&dir, name).as_slice(),
                "index disagrees with find_scenarios_by_name for {name}"
            );
        }
        assert!(ScenarioIndex::load(&dir).paths_for("Gamma").is_empty());
    }

    #[test]
    fn scenario_index_over_missing_directory_is_empty() {
        let dir = temp_scenarios_dir("index-missing").join("does_not_exist");
        let index = ScenarioIndex::load(&dir);
        assert!(index.paths_for("anything").is_empty());
        assert!(list_scenarios(&dir).is_empty());
    }

    #[test]
    fn screenshot_match_with_multi_viewport_matrix_is_rejected() {
        let doc = json!({
            "name": "shot_matrix",
            "scene": "res://ui/main_menu.tscn",
            "viewports": [[1280, 720], [1920, 1080]],
            "steps": [{"snapshot": "after"}],
            "assert": [
                {"screenshot_match": {"snapshot": "after", "baseline": "qa/baselines/a.png"}}
            ],
        });
        let failure = validate_scenario_document(&doc, Path::new("scenario.json"))
            .expect_err("screenshot_match + multi-viewport must fail statically");
        assert_eq!(failure.failure.kind, "scenario_validation_failed");
        assert!(
            failure
                .failure
                .message
                .as_deref()
                .unwrap_or_default()
                .contains("screenshot_match does not support multi-viewport")
        );
    }

    #[test]
    fn screenshot_match_with_single_viewport_is_accepted() {
        let doc = json!({
            "name": "shot_single",
            "scene": "res://ui/main_menu.tscn",
            "viewports": [[1280, 720]],
            "steps": [{"snapshot": "after"}],
            "assert": [
                {"screenshot_match": {"snapshot": "after", "baseline": "qa/baselines/a.png"}}
            ],
        });
        validate_scenario_document(&doc, Path::new("scenario.json"))
            .expect("screenshot_match with a single viewport should validate");
    }

    #[test]
    fn layout_audit_with_single_viewport_is_accepted() {
        let doc = json!({
            "name": "layout_single",
            "scene": "res://ui/main_menu.tscn",
            "viewports": [[1280, 720]],
            "steps": [{"snapshot": "after"}],
            "assert": [{"layout_audit": "strict"}],
        });
        validate_scenario_document(&doc, Path::new("scenario.json"))
            .expect("single-viewport layout_audit should validate");
    }

    #[test]
    fn validate_scenario_document_accepts_visual_assertions() {
        let doc = json!({
            "name": "visual_asserts_ok",
            "scene": "res://ui/main_menu.tscn",
            "steps": [],
            "assert": [
                { "visible_pixels": { "target": " qa:important_actor ", "snapshot": " occluded ", "min_ratio": 0.25, "min_count": 4 } },
                { "not_occluded": { "target": " qa:important_actor ", "snapshot": " occluded ", "occluder": " qa:hud_overlay ", "min_ratio": 0.1 } },
                { "renders_above": { "target": " qa:important_actor ", "other": " qa:background ", "snapshot": " order " } },
                { "renders_below": { "target": " qa:cursor ", "other": " qa:tooltip ", "snapshot": " order " } }
            ]
        });
        validate_scenario_document(&doc, Path::new("scenario.json"))
            .expect("visual assertions should validate");

        let normalized = normalize_scenario_document(&doc);
        let assertions = normalized
            .get("assert")
            .and_then(|value| value.as_array())
            .expect("normalized assertions array");
        assert_eq!(
            assertions[0]["visible_pixels"]["target"].as_str(),
            Some("qa:important_actor")
        );
        assert_eq!(
            assertions[0]["visible_pixels"]["snapshot"].as_str(),
            Some("occluded")
        );
        assert_eq!(
            assertions[1]["not_occluded"]["occluder"].as_str(),
            Some("qa:hud_overlay")
        );
        assert_eq!(
            assertions[2]["renders_above"]["other"].as_str(),
            Some("qa:background")
        );
        assert_eq!(
            assertions[3]["renders_below"]["target"].as_str(),
            Some("qa:cursor")
        );
    }

    #[test]
    fn validate_scenario_document_accepts_saved_snapshot_pixels() {
        let doc = json!({
            "name": "saved_snapshot_pixels_ok",
            "scene": "res://ui/main_menu.tscn",
            "steps": [{"snapshot": " after "}],
            "assert": [
                { "saved_snapshot_pixels": { "target": " qa:important_actor ", "snapshot": " after ", "min_ratio": 0.25, "min_count": 4 } }
            ]
        });
        validate_scenario_document(&doc, Path::new("scenario.json"))
            .expect("saved snapshot pixel assertions should validate");

        let normalized = normalize_scenario_document(&doc);
        let assertion = &normalized["assert"][0]["saved_snapshot_pixels"];
        assert_eq!(assertion["target"].as_str(), Some("qa:important_actor"));
        assert_eq!(assertion["snapshot"].as_str(), Some("after"));
        assert_eq!(assertion["min_ratio"].as_f64(), Some(0.25));
        assert_eq!(assertion["min_count"].as_i64(), Some(4));

        let missing_threshold = json!({
            "name": "saved_snapshot_pixels_missing_threshold",
            "scene": "res://ui/main_menu.tscn",
            "steps": [{"snapshot": "after"}],
            "assert": [{ "saved_snapshot_pixels": { "target": "qa:actor", "snapshot": "after" } }]
        });
        assert!(
            validate_scenario_document(&missing_threshold, Path::new("scenario.json")).is_err()
        );

        let bad_ratio = json!({
            "name": "saved_snapshot_pixels_bad_ratio",
            "scene": "res://ui/main_menu.tscn",
            "steps": [{"snapshot": "after"}],
            "assert": [{ "saved_snapshot_pixels": { "target": "qa:actor", "snapshot": "after", "min_ratio": 1.5 } }]
        });
        assert!(validate_scenario_document(&bad_ratio, Path::new("scenario.json")).is_err());
    }

    #[test]
    fn validate_and_normalize_step_local_assertions() {
        let doc = json!({
            "name": "step_assertions_ok",
            "scene": "res://ui/main_menu.tscn",
            "steps": [
                {"wait_frames": 1},
                {
                    "assert": [
                        {"visible": " qa:primary_start "},
                        {"text_contains": {"target": " qa:status ", "expected": "ready"}},
                        {"distance_between": {"a": " qa:a ", "b": " qa:b ", "min_distance": 12.0}},
                        {"max_delta_position": {"target": " qa:player ", "max_delta": 3.0}},
                        {"surface_matches": {"target": " qa:player ", "expected": "water"}},
                        {"surface_probe": {"provider": " qa:match_controller ", "probe_id": " sample ", "expected_surface": "water", "expected_walkable": true}},
                        {"renders_above": {"a": " qa:player ", "b": " qa:boat ", "snapshot": " water "}}
                    ]
                }
            ]
        });
        validate_scenario_document(&doc, Path::new("scenario.json"))
            .expect("step-local assertions should validate");

        let normalized = normalize_scenario_document(&doc);
        let assertions = normalized["steps"][1]["assert"].as_array().unwrap();
        assert_eq!(
            assertions[0]["visible"].as_str(),
            Some(" qa:primary_start ")
        );
        assert_eq!(
            assertions[1]["text_contains"]["target"].as_str(),
            Some("qa:status")
        );
        assert_eq!(assertions[2]["distance"]["min"].as_f64(), Some(12.0));
        assert_eq!(
            assertions[3]["max_delta_position"]["max"].as_f64(),
            Some(3.0)
        );
        assert_eq!(
            assertions[3]["max_delta_position"]["window_frames"].as_i64(),
            Some(1)
        );
        assert_eq!(
            assertions[4]["surface_matches"]["surface"].as_str(),
            Some("water")
        );
        assert_eq!(
            assertions[5]["surface_probe"]["probe_id"].as_str(),
            Some("sample")
        );
        assert_eq!(
            assertions[6]["renders_above"]["target"].as_str(),
            Some("qa:player")
        );
        assert_eq!(
            assertions[6]["renders_above"]["other"].as_str(),
            Some("qa:boat")
        );
    }

    #[test]
    fn validate_and_normalize_node_inspect_step() {
        let doc = json!({
            "name": "node_inspect_step_ok",
            "scene": "res://ui/main_menu.tscn",
            "steps": [{"node_inspect": " qa:start_button "}],
        });
        validate_scenario_document(&doc, Path::new("scenario.json"))
            .expect("node_inspect step should validate");

        let normalized = normalize_scenario_document(&doc);
        assert_eq!(
            normalized["steps"][0]["node_inspect"].as_str(),
            Some("qa:start_button")
        );

        let invalid = json!({
            "name": "node_inspect_step_invalid",
            "scene": "res://ui/main_menu.tscn",
            "steps": [{"node_inspect": "  "}],
        });
        let failure = validate_scenario_document(&invalid, Path::new("scenario.json"))
            .expect_err("empty node_inspect target should fail");
        assert_eq!(failure.failure.kind, "scenario_validation_failed");
        assert!(
            failure
                .failure
                .message
                .as_deref()
                .unwrap_or_default()
                .contains("node_inspect step value")
        );
    }

    #[test]
    fn audio_scenario_document_accepts_and_normalizes_valid_audio_steps() {
        let doc = json!({
            "name": "audio_steps_ok",
            "scene": "res://ui/main_menu.tscn",
            "steps": [
                {
                    "audio_capture": {
                        "name": " dodgeball_music ",
                        "duration_seconds": 4.0,
                        "bus": " Master ",
                        "analyze": true
                    }
                },
                {
                    "audio_export": {
                        "name": " generated_loop ",
                        "target": " qa:audio_controller ",
                        "method": " qa_export_audio_track ",
                        "args": ["dodgeball_rush_loop"],
                        "analyze": true
                    }
                }
            ]
        });
        validate_scenario_document(&doc, Path::new("scenario.json"))
            .expect("well-formed audio steps should validate");

        let normalized = normalize_scenario_document(&doc);
        assert_eq!(
            normalized["steps"][0]["audio_capture"]["name"].as_str(),
            Some("dodgeball_music")
        );
        assert_eq!(
            normalized["steps"][0]["audio_capture"]["bus"].as_str(),
            Some("Master")
        );
        assert_eq!(
            normalized["steps"][1]["audio_export"]["name"].as_str(),
            Some("generated_loop")
        );
        assert_eq!(
            normalized["steps"][1]["audio_export"]["target"].as_str(),
            Some("qa:audio_controller")
        );
        assert_eq!(
            normalized["steps"][1]["audio_export"]["method"].as_str(),
            Some("qa_export_audio_track")
        );
    }

    #[test]
    fn audio_scenario_document_rejects_unsafe_names() {
        let doc = json!({
            "name": "audio_bad_name",
            "scene": "res://ui/main_menu.tscn",
            "steps": [
                {"audio_capture": {"name": "../music", "duration_seconds": 1.0}}
            ]
        });
        assert!(validate_scenario_document(&doc, Path::new("scenario.json")).is_err());
    }

    #[test]
    fn audio_scenario_document_rejects_non_qa_export_methods() {
        let doc = json!({
            "name": "audio_bad_method",
            "scene": "res://ui/main_menu.tscn",
            "steps": [
                {
                    "audio_export": {
                        "name": "generated_loop",
                        "target": "qa:audio_controller",
                        "method": "export_audio_track"
                    }
                }
            ]
        });
        assert!(validate_scenario_document(&doc, Path::new("scenario.json")).is_err());
    }

    #[test]
    fn audio_scenario_document_rejects_invalid_durations() {
        for duration in [json!(0.0), json!(-1.0), json!(61.0), json!("1.0")] {
            let doc = json!({
                "name": "audio_bad_duration",
                "scene": "res://ui/main_menu.tscn",
                "steps": [
                    {"audio_capture": {"name": "music", "duration_seconds": duration}}
                ]
            });
            assert!(validate_scenario_document(&doc, Path::new("scenario.json")).is_err());
        }
    }

    #[test]
    fn audio_scenario_document_rejects_non_array_export_args() {
        let doc = json!({
            "name": "audio_bad_args",
            "scene": "res://ui/main_menu.tscn",
            "steps": [
                {
                    "audio_export": {
                        "name": "generated_loop",
                        "target": "qa:audio_controller",
                        "method": "qa_export_audio_track",
                        "args": "dodgeball_rush_loop"
                    }
                }
            ]
        });
        assert!(validate_scenario_document(&doc, Path::new("scenario.json")).is_err());
    }

    #[test]
    fn validate_scenario_document_rejects_invalid_visual_assertions() {
        let bad_ratio = json!({
            "name": "bad_ratio",
            "scene": "res://ui/main_menu.tscn",
            "steps": [],
            "assert": [{ "visible_pixels": { "target": "qa:actor", "snapshot": "after", "min_ratio": 1.2 } }]
        });
        assert!(validate_scenario_document(&bad_ratio, Path::new("scenario.json")).is_err());

        let missing_target = json!({
            "name": "missing_target",
            "scene": "res://ui/main_menu.tscn",
            "steps": [],
            "assert": [{ "visible_pixels": { "min_count": 2 } }]
        });
        assert!(validate_scenario_document(&missing_target, Path::new("scenario.json")).is_err());

        let bad_count = json!({
            "name": "bad_count",
            "scene": "res://ui/main_menu.tscn",
            "steps": [],
            "assert": [{ "not_occluded": { "target": "qa:actor", "snapshot": "after", "occluder": "qa:overlay", "min_count": -1 } }]
        });
        assert!(validate_scenario_document(&bad_count, Path::new("scenario.json")).is_err());

        let missing_snapshot = json!({
            "name": "missing_snapshot",
            "scene": "res://ui/main_menu.tscn",
            "steps": [],
            "assert": [{ "visible_pixels": { "target": "qa:actor", "min_ratio": 0.25 } }]
        });
        assert!(validate_scenario_document(&missing_snapshot, Path::new("scenario.json")).is_err());

        let missing_threshold = json!({
            "name": "missing_threshold",
            "scene": "res://ui/main_menu.tscn",
            "steps": [],
            "assert": [{ "visible_pixels": { "target": "qa:actor", "snapshot": "after" } }]
        });
        assert!(
            validate_scenario_document(&missing_threshold, Path::new("scenario.json")).is_err()
        );

        let missing_peer = json!({
            "name": "missing_peer",
            "scene": "res://ui/main_menu.tscn",
            "steps": [],
            "assert": [{ "renders_above": { "target": "qa:actor", "snapshot": "after" } }]
        });
        assert!(validate_scenario_document(&missing_peer, Path::new("scenario.json")).is_err());
    }

    #[test]
    fn validate_and_normalize_snapshot_filters() {
        let doc = json!({
            "name": "snapshot_filters",
            "scene": "res://ui/main_menu.tscn",
            "steps": [{"snapshot": "after"}],
            "snapshots": {
                "save_tree": true,
                "save_ui": true,
                "save_screenshot": true,
                "save_logs": true,
                "fail_on_low_information": false,
                "target": " qa:start_button ",
                "max_depth": 1,
                "qa_only": true,
                "include_metadata": true,
                "metadata_keys": [" qa_id ", "state:current"]
            }
        });
        validate_scenario_document(&doc, Path::new("scenario.json"))
            .expect("snapshot filters should validate");
        let normalized = normalize_scenario_document(&doc);
        assert_eq!(
            normalized["snapshots"]["target"].as_str(),
            Some("qa:start_button")
        );
        assert_eq!(
            normalized["snapshots"]["metadata_keys"],
            json!(["qa_id", "state:current"])
        );
        assert_eq!(normalized["snapshots"]["save_tree"], true);
    }

    #[test]
    fn validate_scenario_document_rejects_malformed_snapshot_filters() {
        let bad_depth = json!({
            "name": "bad_snapshot_depth",
            "scene": "res://ui/main_menu.tscn",
            "steps": [],
            "snapshots": {"max_depth": -1}
        });
        assert!(validate_scenario_document(&bad_depth, Path::new("scenario.json")).is_err());

        let bad_key = json!({
            "name": "bad_snapshot_key",
            "scene": "res://ui/main_menu.tscn",
            "steps": [],
            "snapshots": {"metadata_keys": ["qa_id", "../secret"]}
        });
        assert!(validate_scenario_document(&bad_key, Path::new("scenario.json")).is_err());
    }

    #[test]
    fn validate_scenario_document_accepts_unfiltered_snapshot_compatibility() {
        let doc = json!({
            "name": "snapshot_compat",
            "scene": "res://ui/main_menu.tscn",
            "steps": [{"snapshot": "after"}],
            "snapshots": {
                "save_tree": true,
                "save_ui": true,
                "save_screenshot": false,
                "save_logs": true,
                "fail_on_low_information": true
            }
        });
        validate_scenario_document(&doc, Path::new("scenario.json"))
            .expect("legacy snapshot config should validate");
        let normalized = normalize_scenario_document(&doc);
        assert!(normalized["snapshots"].get("target").is_none());
        assert_eq!(normalized["snapshots"]["save_tree"], true);
    }

    #[test]
    fn validate_scenario_document_accepts_and_preserves_execution_isolation() {
        let doc = json!({
            "name": "cold_only",
            "scene": "res://ui/main_menu.tscn",
            "steps": [],
            "execution": {"isolation": "cold"}
        });
        validate_scenario_document(&doc, Path::new("scenario.json"))
            .expect("execution.isolation: cold should validate");
        let normalized = normalize_scenario_document(&doc);
        assert_eq!(normalized["execution"]["isolation"], "cold");
    }

    #[test]
    fn validate_scenario_document_rejects_invalid_execution_isolation() {
        let doc = json!({
            "name": "bad",
            "scene": "res://ui/main_menu.tscn",
            "steps": [],
            "execution": {"isolation": "hot"}
        });
        assert!(validate_scenario_document(&doc, Path::new("scenario.json")).is_err());
    }
}
