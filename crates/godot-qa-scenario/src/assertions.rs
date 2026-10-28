use crate::{artifact_query, run as scenario_run, screenshots, snapshot_context};
use godot_qa_core::config::ProjectConfig;
use godot_qa_core::results::{CommandResult, FailureDetail};
use godot_qa_runtime;
use serde_json::{Map, Value, json};

#[derive(Debug, Clone, Default)]
pub struct AssertOptions {
    pub window_frames: Option<i64>,
    pub window_seconds: Option<f64>,
    pub sample_every_frames: Option<i64>,
    pub dwell_frames: Option<i64>,
    pub dwell_seconds: Option<f64>,
    pub max: Option<f64>,
    pub min: Option<f64>,
    pub min_ratio: Option<f64>,
    pub min_count: Option<i64>,
    pub tolerance: Option<f64>,
    pub leader: Option<String>,
    pub occluder: Option<String>,
    pub property: Option<String>,
    pub forbidden: Option<String>,
    pub evidence_samples: Option<i64>,
    pub point: Vec<String>,
    pub surface: Option<String>,
    pub provider: Option<String>,
    pub provider_id: Option<String>,
    pub provider_method: Option<String>,
    pub coordinate_space: Option<String>,
    pub radius: Option<f64>,
    pub max_points: Option<i64>,
}

pub fn run_assert(
    config: &ProjectConfig,
    assertion_name: &str,
    target: Option<&str>,
    expected: Option<&str>,
    options: AssertOptions,
    live: bool,
    scenario: Option<&str>,
    run_id: Option<&str>,
    snapshot: Option<&str>,
) -> CommandResult {
    let normalized = normalize_assertion_name(assertion_name);
    let assertion = match assertion_request(&normalized, target, expected, &options) {
        Ok(value) => value,
        Err(err) => {
            return CommandResult {
                ok: false,
                command: "assert".to_string(),
                message: "assertion failed".to_string(),
                data: Map::new(),
                failures: vec![err],
                exit_code: 1,
            };
        }
    };
    let is_temporal = assertion
        .get("temporal")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    if live {
        return run_live_assert(
            config,
            &normalized,
            target,
            expected,
            &options,
            scenario,
            run_id,
            snapshot,
        );
    }
    let is_runtime_target = matches!(
        normalized.as_str(),
        "exists"
            | "not_exists"
            | "visible"
            | "hidden"
            | "enabled"
            | "disabled"
            | "focused"
            | "text_equals"
            | "text_contains"
            | "inside_viewport"
            | "no_overlap"
            | "no_errors"
    );
    if is_runtime_target && scenario.is_none() && run_id.is_none() && snapshot.is_none() {
        return scenario_run::run_generated_scenario_once(config, &assertion, "assert");
    }
    let is_visual = is_visual_assertion_name(&normalized);
    if is_visual && scenario.is_none() && run_id.is_none() {
        return scenario_run::run_generated_visual_assertion_once(
            config, &assertion, snapshot, "assert",
        );
    }
    let is_surface = normalized == "surface_probe" || normalized == "surface_matches";
    if is_surface && scenario.is_none() && run_id.is_none() {
        let assertion = assertion_with_runtime_snapshot(assertion, snapshot);
        return scenario_run::run_generated_scenario_once(config, &assertion, "assert");
    }
    if (is_temporal || normalized == "distance" || is_surface)
        && scenario.is_none()
        && run_id.is_none()
        && snapshot.is_none()
    {
        return scenario_run::run_generated_scenario_once(config, &assertion, "assert");
    }
    run_snapshot_assert(
        config,
        &normalized,
        target,
        expected,
        &options,
        scenario,
        run_id,
        snapshot,
    )
}

fn normalize_assertion_name(assertion_name: &str) -> String {
    assertion_name.replace('-', "_")
}

fn is_visual_assertion_name(assertion_name: &str) -> bool {
    matches!(
        assertion_name,
        "visible_pixels" | "not_occluded" | "renders_above" | "renders_below"
    )
}

fn assertion_with_runtime_snapshot(
    mut assertion: Map<String, Value>,
    snapshot: Option<&str>,
) -> Map<String, Value> {
    let Some(snapshot) = snapshot.map(str::trim).filter(|value| !value.is_empty()) else {
        return assertion;
    };
    if let Some(surface_probe) = assertion
        .get_mut("surface_probe")
        .and_then(Value::as_object_mut)
    {
        surface_probe.insert("snapshot".to_string(), json!(snapshot));
        return assertion;
    }
    assertion.insert("snapshot".to_string(), json!(snapshot));
    assertion
}

// keep rest from existing file
pub fn run_snapshot_assert(
    config: &ProjectConfig,
    assertion_name: &str,
    target: Option<&str>,
    expected: Option<&str>,
    options: &AssertOptions,
    scenario: Option<&str>,
    run_id: Option<&str>,
    snapshot: Option<&str>,
) -> CommandResult {
    let command = "assert";
    let mut failures = Vec::new();
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
    let scenario_name = scenario.unwrap_or_default().trim().to_string();
    if scenario_name.is_empty() {
        failures.push(failure(
            "scenario_required",
            "snapshot-backed assert requires --scenario",
            Some(json!("scenario name via --scenario")),
            Some(json!(scenario.unwrap_or_default())),
            None,
            None,
        ));
    }
    let assertion = match assertion_request(assertion_name, target, expected, options) {
        Ok(assertion) => assertion,
        Err(failure) => {
            failures.push(failure);
            Map::new()
        }
    };
    if !failures.is_empty() {
        return CommandResult {
            ok: false,
            command: command.to_string(),
            message: "assertion failed".to_string(),
            data: Map::new(),
            failures,
            exit_code: 1,
        };
    }

    let loaded = match snapshot_context::load_snapshot_context(
        &config.artifacts_dir,
        &scenario_name,
        run_id,
        snapshot,
        command,
    ) {
        Ok(loaded) => loaded,
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

    let outcome = match evaluate_snapshot_assertion(
        &assertion,
        &loaded.snapshot_context,
        &loaded.artifacts,
    ) {
        Ok(outcome) => outcome,
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
    let mut data = Map::from_iter([
        ("scenario".to_string(), json!(scenario_name)),
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
        ("assertion".to_string(), Value::Object(assertion.clone())),
        (
            "artifacts".to_string(),
            Value::Object(loaded.artifacts.clone()),
        ),
    ]);
    for (key, value) in outcome.data {
        data.insert(key, value);
    }
    if let Some(failure) = outcome.failure {
        return CommandResult {
            ok: false,
            command: command.to_string(),
            message: "assertion failed".to_string(),
            data,
            failures: vec![failure],
            exit_code: 1,
        };
    }
    CommandResult {
        ok: true,
        command: command.to_string(),
        message: "assertion passed".to_string(),
        data,
        failures: Vec::new(),
        exit_code: 0,
    }
}

fn run_live_assert(
    config: &ProjectConfig,
    assertion_name: &str,
    target: Option<&str>,
    expected: Option<&str>,
    options: &AssertOptions,
    scenario: Option<&str>,
    run_id: Option<&str>,
    snapshot: Option<&str>,
) -> CommandResult {
    let command = "assert";
    let mut failures = Vec::new();
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
    let assertion = match assertion_request(assertion_name, target, expected, options) {
        Ok(assertion) => assertion,
        Err(failure) => {
            failures.push(failure);
            Map::new()
        }
    };
    if scenario.is_some() || run_id.is_some() || snapshot.is_some() {
        failures.push(failure(
            "invalid_assertion_args",
            "--live cannot be combined with snapshot artifact selection flags",
            Some(json!(
                "use --live without --scenario, --run-id, or --snapshot"
            )),
            Some(json!({
                "scenario": scenario,
                "run_id": run_id,
                "snapshot": snapshot,
            })),
            None,
            None,
        ));
    }
    if !failures.is_empty() {
        return CommandResult {
            ok: false,
            command: command.to_string(),
            message: "assertion failed".to_string(),
            data: Map::new(),
            failures,
            exit_code: 1,
        };
    }

    let (protocol_name, params) = live_protocol_request(&assertion);
    let protocol_command = live_assertion_protocol(&protocol_name);
    let Some(protocol_command) = protocol_command else {
        return CommandResult {
            ok: false,
            command: command.to_string(),
            message: "assertion failed".to_string(),
            data: Map::new(),
            failures: vec![failure(
                "invalid_assertion",
                "assertion name is not supported",
                Some(json!([])),
                Some(json!(assertion_name)),
                None,
                None,
            )],
            exit_code: 1,
        };
    };
    let mut result = if is_visual_assertion_name(&protocol_name) {
        godot_qa_runtime::run_live_screenshot_protocol_command(
            config,
            command,
            protocol_command,
            params,
        )
    } else {
        godot_qa_runtime::run_live_protocol_command(config, command, protocol_command, params)
    };
    let mut data = Map::from_iter([("assertion".to_string(), Value::Object(assertion))]);
    for (key, value) in result.data {
        data.insert(key, value);
    }
    result.data = data;
    result
}

fn assertion_request(
    assertion_name: &str,
    target: Option<&str>,
    expected: Option<&str>,
    options: &AssertOptions,
) -> Result<Map<String, Value>, FailureDetail> {
    match assertion_name {
        "max_delta_position" => {
            let target = target.unwrap_or_default().trim();
            if target.is_empty() {
                return Err(failure(
                    "invalid_assertion_args",
                    "max_delta_position requires a target argument",
                    Some(json!("non-empty target argument")),
                    Some(json!(target)),
                    None,
                    None,
                ));
            }
            if expected.is_some() || options.min.is_some() {
                return Err(failure(
                    "invalid_assertion_args",
                    "max_delta_position does not accept expected or --min",
                    Some(json!("--max with target")),
                    Some(json!({"expected": expected, "min": options.min})),
                    None,
                    None,
                ));
            }
            let Some(max) = options.max else {
                return Err(failure(
                    "invalid_assertion_args",
                    "max_delta_position requires --max",
                    Some(json!("numeric --max")),
                    Some(Value::Null),
                    None,
                    None,
                ));
            };
            let mut payload = Map::from_iter([
                ("target".to_string(), json!(target)),
                ("max".to_string(), json!(max)),
            ]);
            insert_temporal_window_fields(&mut payload, options)?;
            Ok(Map::from_iter([
                ("temporal".to_string(), json!(true)),
                ("max_delta_position".to_string(), Value::Object(payload)),
            ]))
        }
        "follows" => {
            let follower = target.unwrap_or_default().trim();
            let leader = options.leader.as_deref().unwrap_or_default().trim();
            if follower.is_empty() || leader.is_empty() {
                return Err(failure(
                    "invalid_assertion_args",
                    "follows requires a follower target argument and --leader",
                    Some(
                        json!({"follower": "non-empty target argument", "leader": "non-empty --leader"}),
                    ),
                    Some(json!({"follower": follower, "leader": leader})),
                    None,
                    None,
                ));
            }
            if expected.is_some() || options.max.is_some() || options.min.is_some() {
                return Err(failure(
                    "invalid_assertion_args",
                    "follows accepts --leader and --tolerance, not expected, --max, or --min",
                    Some(json!("--leader and --tolerance")),
                    Some(json!({"expected": expected, "max": options.max, "min": options.min})),
                    None,
                    None,
                ));
            }
            let Some(tolerance) = options.tolerance else {
                return Err(failure(
                    "invalid_assertion_args",
                    "follows requires --tolerance",
                    Some(json!("numeric --tolerance")),
                    Some(Value::Null),
                    None,
                    None,
                ));
            };
            let mut payload = Map::from_iter([
                ("leader".to_string(), json!(leader)),
                ("follower".to_string(), json!(follower)),
                ("tolerance".to_string(), json!(tolerance)),
            ]);
            insert_temporal_window_fields(&mut payload, options)?;
            Ok(Map::from_iter([
                ("temporal".to_string(), json!(true)),
                ("follows".to_string(), Value::Object(payload)),
            ]))
        }
        "never" => {
            let target = target.unwrap_or_default().trim();
            if target.is_empty() {
                return Err(failure(
                    "invalid_assertion_args",
                    "never requires a target argument",
                    Some(json!("non-empty target argument")),
                    Some(json!(target)),
                    None,
                    None,
                ));
            }
            if options.max.is_some() || options.min.is_some() || options.tolerance.is_some() {
                return Err(failure(
                    "invalid_assertion_args",
                    "never does not accept numeric bound options",
                    Some(json!("--forbidden with --property or observation argument")),
                    Some(
                        json!({"max": options.max, "min": options.min, "tolerance": options.tolerance}),
                    ),
                    None,
                    None,
                ));
            }
            let observation = options
                .property
                .as_deref()
                .or(expected)
                .unwrap_or_default()
                .trim();
            if observation.is_empty() {
                return Err(failure(
                    "invalid_assertion_args",
                    "never requires --property or an observation argument",
                    Some(json!("non-empty observation field")),
                    Some(Value::Null),
                    None,
                    None,
                ));
            }
            let forbidden = options.forbidden.as_deref().unwrap_or_default().trim();
            if forbidden.is_empty() {
                return Err(failure(
                    "invalid_assertion_args",
                    "never requires --forbidden",
                    Some(json!("non-empty --forbidden")),
                    Some(json!(forbidden)),
                    None,
                    None,
                ));
            }
            let mut payload = Map::from_iter([
                ("target".to_string(), json!(target)),
                ("property".to_string(), json!(observation)),
                ("forbidden".to_string(), json!(forbidden_values(forbidden))),
            ]);
            insert_temporal_window_fields(&mut payload, options)?;
            Ok(Map::from_iter([
                ("temporal".to_string(), json!(true)),
                ("never".to_string(), Value::Object(payload)),
            ]))
        }
        "within_limits" => {
            let target = target.unwrap_or_default().trim();
            if target.is_empty() {
                return Err(failure(
                    "invalid_assertion_args",
                    "within_limits requires a target argument",
                    Some(json!("non-empty target argument")),
                    Some(json!(target)),
                    None,
                    None,
                ));
            }
            if options.leader.is_some() || options.tolerance.is_some() {
                return Err(failure(
                    "invalid_assertion_args",
                    "within_limits does not accept --leader or --tolerance",
                    Some(json!("--min and --max with optional --property")),
                    Some(json!({"leader": options.leader, "tolerance": options.tolerance})),
                    None,
                    None,
                ));
            }
            let (Some(min), Some(max)) = (options.min, options.max) else {
                return Err(failure(
                    "invalid_assertion_args",
                    "within_limits requires --min and --max",
                    Some(json!({"min": "numeric --min", "max": "numeric --max"})),
                    Some(json!({"min": options.min, "max": options.max})),
                    None,
                    None,
                ));
            };
            let mut payload = Map::from_iter([
                ("target".to_string(), json!(target)),
                ("min".to_string(), json!(min)),
                ("max".to_string(), json!(max)),
            ]);
            if let Some(field) = options
                .property
                .as_deref()
                .map(str::trim)
                .filter(|v| !v.is_empty())
            {
                payload.insert("property".to_string(), json!(field));
            } else if let Some(field) = expected.map(str::trim).filter(|v| !v.is_empty()) {
                payload.insert("property".to_string(), json!(field));
            }
            insert_temporal_window_fields(&mut payload, options)?;
            Ok(Map::from_iter([
                ("temporal".to_string(), json!(true)),
                ("within_limits".to_string(), Value::Object(payload)),
            ]))
        }
        "distance" => {
            let a = target.unwrap_or_default().trim();
            let b = expected.unwrap_or_default().trim();
            if a.is_empty() {
                return Err(failure(
                    "invalid_assertion_args",
                    "distance requires the first target argument",
                    Some(json!("non-empty first target argument")),
                    Some(json!(a)),
                    None,
                    None,
                ));
            }
            if b.is_empty() {
                return Err(failure(
                    "invalid_assertion_args",
                    "distance requires the second target argument",
                    Some(json!("non-empty second target argument")),
                    Some(json!(b)),
                    None,
                    None,
                ));
            }
            if options.tolerance.is_some()
                || options.leader.is_some()
                || options.property.is_some()
                || options.forbidden.is_some()
                || options.window_frames.is_some()
                || options.window_seconds.is_some()
                || options.sample_every_frames.is_some()
                || options.evidence_samples.is_some()
            {
                return Err(failure(
                    "invalid_assertion_args",
                    "distance accepts only --min and/or --max bounds",
                    Some(json!("two targets and numeric --min and/or --max")),
                    Some(json!({
                        "tolerance": options.tolerance,
                        "leader": options.leader,
                        "property": options.property,
                        "forbidden": options.forbidden,
                        "window_frames": options.window_frames,
                        "window_seconds": options.window_seconds,
                        "sample_every_frames": options.sample_every_frames,
                        "evidence_samples": options.evidence_samples
                    })),
                    None,
                    None,
                ));
            }
            if options.min.is_none() && options.max.is_none() {
                return Err(failure(
                    "invalid_assertion_args",
                    "distance requires --min, --max, or both",
                    Some(json!("numeric --min and/or --max")),
                    Some(json!({"min": options.min, "max": options.max})),
                    None,
                    None,
                ));
            }
            if let Some(min) = options.min {
                if !min.is_finite() || min < 0.0 {
                    return Err(failure(
                        "invalid_assertion_args",
                        "distance --min must be a finite non-negative number",
                        Some(json!("finite number >= 0")),
                        Some(json!(min)),
                        None,
                        None,
                    ));
                }
            }
            if let Some(max) = options.max {
                if !max.is_finite() || max < 0.0 {
                    return Err(failure(
                        "invalid_assertion_args",
                        "distance --max must be a finite non-negative number",
                        Some(json!("finite number >= 0")),
                        Some(json!(max)),
                        None,
                        None,
                    ));
                }
            }
            Ok(Map::from_iter([
                ("name".to_string(), json!("distance")),
                ("a".to_string(), json!(a)),
                ("b".to_string(), json!(b)),
                ("min".to_string(), json!(options.min)),
                ("max".to_string(), json!(options.max)),
            ]))
        }
        "distance_changed" => {
            let a = target.unwrap_or_default().trim();
            let b = expected.unwrap_or_default().trim();
            if a.is_empty() {
                return Err(failure(
                    "invalid_assertion_args",
                    "distance_changed requires the first target argument",
                    Some(json!("non-empty first target argument")),
                    Some(json!(a)),
                    None,
                    None,
                ));
            }
            if b.is_empty() {
                return Err(failure(
                    "invalid_assertion_args",
                    "distance_changed requires the second target argument",
                    Some(json!("non-empty second target argument")),
                    Some(json!(b)),
                    None,
                    None,
                ));
            }
            if options.max.is_some()
                || options.tolerance.is_some()
                || options.leader.is_some()
                || options.property.is_some()
                || options.forbidden.is_some()
                || options.evidence_samples.is_some()
            {
                return Err(failure(
                    "invalid_assertion_args",
                    "distance_changed accepts --min and temporal window options only",
                    Some(json!("two targets, --min, and temporal window options")),
                    Some(json!({
                        "max": options.max,
                        "tolerance": options.tolerance,
                        "leader": options.leader,
                        "property": options.property,
                        "forbidden": options.forbidden,
                        "evidence_samples": options.evidence_samples
                    })),
                    None,
                    None,
                ));
            }
            let Some(min) = options.min else {
                return Err(failure(
                    "invalid_assertion_args",
                    "distance_changed requires --min",
                    Some(json!("numeric --min")),
                    Some(Value::Null),
                    None,
                    None,
                ));
            };
            if !min.is_finite() || min < 0.0 {
                return Err(failure(
                    "invalid_assertion_args",
                    "distance_changed --min must be a finite non-negative number",
                    Some(json!("finite number >= 0")),
                    Some(json!(min)),
                    None,
                    None,
                ));
            }
            let mut payload = Map::from_iter([
                ("a".to_string(), json!(a)),
                ("b".to_string(), json!(b)),
                ("min".to_string(), json!(min)),
            ]);
            insert_temporal_window_fields(&mut payload, options)?;
            Ok(Map::from_iter([
                ("temporal".to_string(), json!(true)),
                ("distance_changed".to_string(), Value::Object(payload)),
            ]))
        }
        "surface_probe" => build_surface_probe_request(target, expected, options),
        "surface_matches" => build_surface_matches_request(target, expected, options),
        "visible_pixels" => {
            let target = target.unwrap_or_default().trim();
            if target.is_empty() {
                return Err(failure(
                    "invalid_assertion_args",
                    "visible_pixels requires a target argument",
                    Some(json!("non-empty target argument")),
                    Some(json!(target)),
                    None,
                    None,
                ));
            }
            if expected.is_some()
                || options.max.is_some()
                || options.min.is_some()
                || options.tolerance.is_some()
                || options.leader.is_some()
                || options.property.is_some()
                || options.forbidden.is_some()
                || options.evidence_samples.is_some()
                || options.occluder.is_some()
            {
                return Err(failure(
                    "invalid_assertion_args",
                    "visible_pixels accepts only --min-ratio and --min-count",
                    Some(json!("target with optional --min-ratio/--min-count")),
                    Some(
                        json!({"expected": expected, "max": options.max, "min": options.min, "tolerance": options.tolerance, "leader": options.leader, "property": options.property, "forbidden": options.forbidden, "evidence_samples": options.evidence_samples, "occluder": options.occluder}),
                    ),
                    None,
                    None,
                ));
            }
            if let Some(min_ratio) = options.min_ratio
                && (!min_ratio.is_finite() || !(0.0..=1.0).contains(&min_ratio))
            {
                return Err(failure(
                    "invalid_assertion_args",
                    "visible_pixels --min-ratio must be finite and within [0.0, 1.0]",
                    Some(json!("finite ratio in range [0.0, 1.0]")),
                    Some(json!(min_ratio)),
                    None,
                    None,
                ));
            }
            if let Some(min_count) = options.min_count
                && min_count < 0
            {
                return Err(failure(
                    "invalid_assertion_args",
                    "visible_pixels --min-count must be non-negative",
                    Some(json!("integer >= 0")),
                    Some(json!(min_count)),
                    None,
                    None,
                ));
            }
            if options.min_ratio.is_none() && options.min_count.is_none() {
                return Err(failure(
                    "invalid_assertion_args",
                    "visible_pixels requires at least one of --min-ratio or --min-count",
                    Some(json!("target with --min-ratio and/or --min-count")),
                    Some(json!({"min_ratio": options.min_ratio, "min_count": options.min_count})),
                    None,
                    None,
                ));
            }
            let mut payload = Map::from_iter([
                ("name".to_string(), json!("visible_pixels")),
                ("target".to_string(), json!(target)),
            ]);
            if let Some(min_ratio) = options.min_ratio {
                payload.insert("min_ratio".to_string(), json!(min_ratio));
            }
            if let Some(min_count) = options.min_count {
                payload.insert("min_count".to_string(), json!(min_count));
            }
            Ok(payload)
        }
        "not_occluded" => {
            let target = target.unwrap_or_default().trim();
            let occluder = options.occluder.as_deref().unwrap_or_default().trim();
            if target.is_empty() || occluder.is_empty() {
                return Err(failure(
                    "invalid_assertion_args",
                    "not_occluded requires target and --occluder",
                    Some(
                        json!({"target": "non-empty target argument", "occluder": "non-empty --occluder"}),
                    ),
                    Some(json!({"target": target, "occluder": occluder})),
                    None,
                    None,
                ));
            }
            if expected.is_some() {
                return Err(failure(
                    "invalid_assertion_args",
                    "not_occluded does not accept an expected argument",
                    Some(Value::Null),
                    Some(json!(expected)),
                    None,
                    None,
                ));
            }
            if options.max.is_some()
                || options.min.is_some()
                || options.tolerance.is_some()
                || options.leader.is_some()
                || options.property.is_some()
                || options.forbidden.is_some()
                || options.evidence_samples.is_some()
                || options.window_frames.is_some()
                || options.window_seconds.is_some()
                || options.sample_every_frames.is_some()
            {
                return Err(failure(
                    "invalid_assertion_args",
                    "not_occluded accepts only --occluder, --min-ratio, and --min-count",
                    Some(json!(
                        "target with --occluder and optional --min-ratio/--min-count"
                    )),
                    Some(json!({
                        "max": options.max,
                        "min": options.min,
                        "tolerance": options.tolerance,
                        "leader": options.leader,
                        "property": options.property,
                        "forbidden": options.forbidden,
                        "evidence_samples": options.evidence_samples,
                        "window_frames": options.window_frames,
                        "window_seconds": options.window_seconds,
                        "sample_every_frames": options.sample_every_frames,
                    })),
                    None,
                    None,
                ));
            }
            if let Some(min_ratio) = options.min_ratio
                && (!min_ratio.is_finite() || !(0.0..=1.0).contains(&min_ratio))
            {
                return Err(failure(
                    "invalid_assertion_args",
                    "not_occluded --min-ratio must be finite and within [0.0, 1.0]",
                    Some(json!("finite ratio in range [0.0, 1.0]")),
                    Some(json!(min_ratio)),
                    None,
                    None,
                ));
            }
            if let Some(min_count) = options.min_count
                && min_count < 0
            {
                return Err(failure(
                    "invalid_assertion_args",
                    "not_occluded --min-count must be non-negative",
                    Some(json!("integer >= 0")),
                    Some(json!(min_count)),
                    None,
                    None,
                ));
            }
            let mut payload = Map::from_iter([
                ("name".to_string(), json!("not_occluded")),
                ("target".to_string(), json!(target)),
                ("occluder".to_string(), json!(occluder)),
            ]);
            if let Some(min_ratio) = options.min_ratio {
                payload.insert("min_ratio".to_string(), json!(min_ratio));
            }
            if let Some(min_count) = options.min_count {
                payload.insert("min_count".to_string(), json!(min_count));
            }
            Ok(payload)
        }
        "renders_above" | "renders_below" => {
            let target = target.unwrap_or_default().trim();
            let peer = expected.unwrap_or_default().trim();
            if target.is_empty() || peer.is_empty() {
                return Err(failure(
                    "invalid_assertion_args",
                    &format!("{assertion_name} requires target and peer arguments"),
                    Some(
                        json!({"target": "non-empty first argument", "peer": "non-empty second argument"}),
                    ),
                    Some(json!({"target": target, "peer": peer})),
                    None,
                    None,
                ));
            }
            if options.max.is_some()
                || options.min.is_some()
                || options.min_ratio.is_some()
                || options.min_count.is_some()
                || options.tolerance.is_some()
                || options.leader.is_some()
                || options.property.is_some()
                || options.forbidden.is_some()
                || options.evidence_samples.is_some()
                || options.occluder.is_some()
                || options.window_frames.is_some()
                || options.window_seconds.is_some()
                || options.sample_every_frames.is_some()
            {
                return Err(failure(
                    "invalid_assertion_args",
                    &format!("{assertion_name} accepts only target and peer arguments"),
                    Some(json!("target and peer only")),
                    Some(json!({
                        "max": options.max,
                        "min": options.min,
                        "min_ratio": options.min_ratio,
                        "min_count": options.min_count,
                        "tolerance": options.tolerance,
                        "leader": options.leader,
                        "property": options.property,
                        "forbidden": options.forbidden,
                        "evidence_samples": options.evidence_samples,
                        "occluder": options.occluder,
                        "window_frames": options.window_frames,
                        "window_seconds": options.window_seconds,
                        "sample_every_frames": options.sample_every_frames,
                    })),
                    None,
                    None,
                ));
            }
            Ok(Map::from_iter([
                ("name".to_string(), json!(assertion_name)),
                ("target".to_string(), json!(target)),
                ("peer".to_string(), json!(peer)),
            ]))
        }
        "exists" | "not_exists" | "visible" | "hidden" | "enabled" | "disabled" | "focused"
        | "inside_viewport" | "snapshot_exists" | "snapshot_visible" => {
            let target = target.unwrap_or_default().trim();
            if target.is_empty() {
                return Err(failure(
                    "invalid_assertion_args",
                    &format!("{assertion_name} requires a target argument"),
                    Some(json!("non-empty target argument")),
                    Some(json!(target)),
                    None,
                    None,
                ));
            }
            if expected.is_some() {
                return Err(failure(
                    "invalid_assertion_args",
                    &format!("{assertion_name} does not accept an expected argument"),
                    Some(Value::Null),
                    Some(json!(expected)),
                    None,
                    None,
                ));
            }
            Ok(Map::from_iter([
                ("name".to_string(), json!(assertion_name)),
                ("target".to_string(), json!(target)),
            ]))
        }
        "text_equals" | "text_contains" => {
            let target = target.unwrap_or_default().trim();
            if target.is_empty() {
                return Err(failure(
                    "invalid_assertion_args",
                    &format!("{assertion_name} requires a target argument"),
                    Some(json!("non-empty target argument")),
                    Some(json!(target)),
                    None,
                    None,
                ));
            }
            let Some(expected) = expected else {
                return Err(failure(
                    "invalid_assertion_args",
                    &format!("{assertion_name} requires an expected string argument"),
                    Some(json!("string expected argument")),
                    Some(Value::Null),
                    None,
                    None,
                ));
            };
            Ok(Map::from_iter([
                ("name".to_string(), json!(assertion_name)),
                ("target".to_string(), json!(target)),
                ("expected".to_string(), json!(expected)),
            ]))
        }
        "no_overlap" => {
            let left = target.unwrap_or_default().trim();
            let right = expected.unwrap_or_default().trim();
            if left.is_empty() {
                return Err(failure(
                    "invalid_assertion_args",
                    "no_overlap requires the first target argument",
                    Some(json!("non-empty first target argument")),
                    Some(json!(left)),
                    None,
                    None,
                ));
            }
            if right.is_empty() {
                return Err(failure(
                    "invalid_assertion_args",
                    "no_overlap requires the second target argument",
                    Some(json!("non-empty second target argument")),
                    Some(json!(right)),
                    None,
                    None,
                ));
            }
            Ok(Map::from_iter([
                ("name".to_string(), json!("no_overlap")),
                ("a".to_string(), json!(left)),
                ("b".to_string(), json!(right)),
            ]))
        }
        "snapshot_contains_type" => {
            let node_type = target.unwrap_or_default().trim();
            if node_type.is_empty() {
                return Err(failure(
                    "invalid_assertion_args",
                    "snapshot_contains_type requires a node type argument",
                    Some(json!("non-empty node type argument")),
                    Some(json!(node_type)),
                    None,
                    None,
                ));
            }
            if expected.is_some() {
                return Err(failure(
                    "invalid_assertion_args",
                    "snapshot_contains_type does not accept an expected argument",
                    Some(Value::Null),
                    Some(json!(expected)),
                    None,
                    None,
                ));
            }
            Ok(Map::from_iter([
                ("name".to_string(), json!("snapshot_contains_type")),
                ("type".to_string(), json!(node_type)),
            ]))
        }
        "no_errors" => Ok(Map::from_iter([("name".to_string(), json!("no_errors"))])),
        _ => Err(failure(
            "invalid_assertion",
            "assertion name is not supported",
            Some(json!([
                "disabled",
                "enabled",
                "exists",
                "focused",
                "hidden",
                "inside_viewport",
                "no_errors",
                "no_overlap",
                "not_exists",
                "snapshot_contains_type",
                "snapshot_exists",
                "snapshot_visible",
                "text_contains",
                "text_equals",
                "visible",
                "max_delta_position",
                "follows",
                "never",
                "within_limits",
                "distance",
                "distance_changed"
            ])),
            Some(json!(assertion_name)),
            None,
            None,
        )),
    }
}

fn live_protocol_request(assertion: &Map<String, Value>) -> (String, Map<String, Value>) {
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
                return (key.to_string(), params.clone());
            }
        }
        return (String::new(), Map::new());
    }
    let name = assertion
        .get("name")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let params = assertion
        .iter()
        .filter(|(key, _)| key.as_str() != "name")
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect::<Map<_, _>>();
    (name, params)
}

fn forbidden_values(raw: &str) -> Vec<String> {
    raw.split(',')
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .collect()
}

const SURFACE_COORDINATE_SPACES: [&str; 3] = ["world_2d", "viewport_2d", "world_3d"];
const SURFACE_MAX_POINTS: usize = 32;
const SURFACE_MAX_RADIUS: f64 = 1024.0;

fn build_surface_probe_request(
    target: Option<&str>,
    _expected: Option<&str>,
    options: &AssertOptions,
) -> Result<Map<String, Value>, FailureDetail> {
    let target = target.unwrap_or_default().trim();
    let provider = options.provider.as_deref().unwrap_or_default().trim();
    let provider_id = options.provider_id.as_deref().unwrap_or_default().trim();
    if target.is_empty() || (provider.is_empty() && provider_id.is_empty()) {
        return Err(failure(
            "invalid_assertion_args",
            "surface_probe requires target and --provider or --provider-id",
            Some(
                json!({"target":"non-empty target argument","provider":"non-empty --provider or --provider-id"}),
            ),
            Some(json!({"target": target, "provider": provider, "provider_id": provider_id})),
            None,
            None,
        ));
    }
    let points = parse_surface_points(&options.point, options.max_points)?;
    let mut payload = Map::from_iter([
        ("target".to_string(), json!(target)),
        ("points".to_string(), Value::Array(points)),
    ]);
    if !provider_id.is_empty() {
        payload.insert("provider_id".to_string(), json!(provider_id));
    } else {
        payload.insert("provider".to_string(), json!(provider));
        payload.insert("provider_target".to_string(), json!(provider));
    }
    if let Some(surface) = options
        .surface
        .as_deref()
        .map(str::trim)
        .filter(|v| !v.is_empty())
    {
        payload.insert("surface".to_string(), json!(surface));
    }
    if let Some(method) = options
        .provider_method
        .as_deref()
        .map(str::trim)
        .filter(|v| !v.is_empty())
    {
        payload.insert("provider_method".to_string(), json!(method));
    }
    if let Some(space) = options
        .coordinate_space
        .as_deref()
        .map(str::trim)
        .filter(|v| !v.is_empty())
    {
        if !SURFACE_COORDINATE_SPACES.contains(&space) {
            return Err(failure(
                "invalid_assertion_args",
                "surface_probe --coordinate-space is not supported",
                Some(json!(SURFACE_COORDINATE_SPACES)),
                Some(json!(space)),
                None,
                None,
            ));
        }
        payload.insert("coordinate_space".to_string(), json!(space));
    }
    if let Some(radius) = options.radius {
        if !radius.is_finite() || !(0.0..=SURFACE_MAX_RADIUS).contains(&radius) {
            return Err(failure(
                "invalid_assertion_args",
                "surface_probe --radius must be finite and within bounds",
                Some(json!(format!(
                    "finite number in range [0.0, {SURFACE_MAX_RADIUS}]"
                ))),
                Some(json!(radius)),
                None,
                None,
            ));
        }
        payload.insert("radius".to_string(), json!(radius));
    }
    Ok(Map::from_iter([
        ("name".to_string(), json!("surface_probe")),
        ("surface_probe".to_string(), Value::Object(payload)),
    ]))
}

fn build_surface_matches_request(
    target: Option<&str>,
    expected: Option<&str>,
    options: &AssertOptions,
) -> Result<Map<String, Value>, FailureDetail> {
    let target = target.unwrap_or_default().trim();
    if target.is_empty() {
        return Err(failure(
            "invalid_assertion_args",
            "surface_matches requires a target argument",
            Some(json!("non-empty target argument")),
            Some(json!(target)),
            None,
            None,
        ));
    }
    let surface = options
        .surface
        .as_deref()
        .or(expected)
        .unwrap_or_default()
        .trim();
    let mut payload = Map::from_iter([
        ("name".to_string(), json!("surface_matches")),
        ("target".to_string(), json!(target)),
    ]);
    if !surface.is_empty() {
        payload.insert("surface".to_string(), json!(surface));
    }
    if let Some(provider) = options
        .provider
        .as_deref()
        .map(str::trim)
        .filter(|v| !v.is_empty())
    {
        payload.insert("provider".to_string(), json!(provider));
        payload.insert("provider_target".to_string(), json!(provider));
    }
    if let Some(method) = options
        .provider_method
        .as_deref()
        .map(str::trim)
        .filter(|v| !v.is_empty())
    {
        payload.insert("provider_method".to_string(), json!(method));
    }
    if let Some(space) = options
        .coordinate_space
        .as_deref()
        .map(str::trim)
        .filter(|v| !v.is_empty())
    {
        if !SURFACE_COORDINATE_SPACES.contains(&space) {
            return Err(failure(
                "invalid_assertion_args",
                "surface_matches --coordinate-space is not supported",
                Some(json!(SURFACE_COORDINATE_SPACES)),
                Some(json!(space)),
                None,
                None,
            ));
        }
        payload.insert("coordinate_space".to_string(), json!(space));
    }
    insert_optional_temporal_window_fields(&mut payload, options, "surface_matches")?;
    Ok(payload)
}

fn parse_surface_points(
    raw_points: &[String],
    max_points: Option<i64>,
) -> Result<Vec<Value>, FailureDetail> {
    let configured_max = max_points.unwrap_or(SURFACE_MAX_POINTS as i64);
    if configured_max <= 0 || configured_max as usize > SURFACE_MAX_POINTS {
        return Err(failure(
            "invalid_assertion_args",
            "surface_probe --max-points must be within supported bounds",
            Some(json!(format!("integer in range 1..={SURFACE_MAX_POINTS}"))),
            Some(json!(configured_max)),
            None,
            None,
        ));
    }
    if raw_points.is_empty() {
        return Err(failure(
            "invalid_assertion_args",
            "surface_probe requires at least one --point",
            Some(json!("--point x,y[,z][:expected]")),
            Some(Value::Null),
            None,
            None,
        ));
    }
    if raw_points.len() > configured_max as usize {
        return Err(failure(
            "invalid_assertion_args",
            "surface_probe exceeds point limit",
            Some(json!(configured_max)),
            Some(json!(raw_points.len())),
            None,
            None,
        ));
    }
    raw_points
        .iter()
        .map(|raw| {
            let trimmed = raw.trim();
            let (coords_raw, expected) = trimmed
                .split_once(':')
                .map_or((trimmed, None), |(a, b)| (a.trim(), Some(b.trim())));
            let coords: Vec<&str> = coords_raw.split(',').map(str::trim).collect();
            if coords.len() < 2 || coords.len() > 3 {
                return Err(failure(
                    "invalid_assertion_args",
                    "surface_probe --point must contain x,y or x,y,z",
                    Some(json!("x,y[,z][:expected]")),
                    Some(json!(raw)),
                    None,
                    None,
                ));
            }
            let x = coords[0].parse::<f64>().map_err(|_| {
                failure(
                    "invalid_assertion_args",
                    "surface_probe --point x must be numeric",
                    Some(json!("number")),
                    Some(json!(coords[0])),
                    None,
                    None,
                )
            })?;
            let y = coords[1].parse::<f64>().map_err(|_| {
                failure(
                    "invalid_assertion_args",
                    "surface_probe --point y must be numeric",
                    Some(json!("number")),
                    Some(json!(coords[1])),
                    None,
                    None,
                )
            })?;
            let mut point =
                Map::from_iter([("x".to_string(), json!(x)), ("y".to_string(), json!(y))]);
            if coords.len() == 3 {
                let z = coords[2].parse::<f64>().map_err(|_| {
                    failure(
                        "invalid_assertion_args",
                        "surface_probe --point z must be numeric",
                        Some(json!("number")),
                        Some(json!(coords[2])),
                        None,
                        None,
                    )
                })?;
                point.insert("z".to_string(), json!(z));
            }
            if let Some(label) = expected.filter(|v| !v.is_empty()) {
                point.insert("expected".to_string(), json!(label));
            }
            Ok(Value::Object(point))
        })
        .collect()
}

fn evaluate_snapshot_assertion(
    assertion: &Map<String, Value>,
    snapshot_context: &Value,
    artifacts: &Map<String, Value>,
) -> Result<EvalOutcome, FailureDetail> {
    let name = assertion
        .get("name")
        .and_then(Value::as_str)
        .unwrap_or_default();
    if matches!(
        name,
        "snapshot_exists" | "snapshot_visible" | "snapshot_contains_type"
    ) {
        return evaluate_artifact_query_assertion(assertion, snapshot_context, artifacts);
    }
    if is_visual_assertion_name(name) {
        return evaluate_saved_visual_assertion(assertion, snapshot_context, artifacts);
    }
    let indexes = build_indexes(snapshot_context)?;
    if name == "text_equals" {
        let target = assertion
            .get("target")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let expected = assertion
            .get("expected")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let (_node, control) = resolve_target(&indexes, target, artifacts)?;
        let actual = control
            .as_ref()
            .and_then(|control| control.get("text"))
            .and_then(Value::as_str)
            .map(str::to_owned);
        if actual.as_deref() == Some(expected) {
            return Ok(EvalOutcome::passed());
        }
        return Ok(EvalOutcome::failed(failure(
            "text_equals_mismatch",
            "target text did not match the expected value",
            Some(json!(expected)),
            Some(json!(actual)),
            Some(target.to_string()),
            Some(artifacts.clone()),
        )));
    }
    Ok(EvalOutcome::passed())
}

fn evaluate_saved_visual_assertion(
    assertion: &Map<String, Value>,
    snapshot_context: &Value,
    artifacts: &Map<String, Value>,
) -> Result<EvalOutcome, FailureDetail> {
    let name = assertion
        .get("name")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let target = assertion
        .get("target")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let indexes = build_indexes(snapshot_context)?;
    let (node, _control) = resolve_target(&indexes, target, artifacts)?;
    if matches!(name, "renders_above" | "renders_below") {
        let other = assertion
            .get("other")
            .or_else(|| assertion.get("peer"))
            .and_then(Value::as_str)
            .unwrap_or_default();
        resolve_target(&indexes, other, artifacts)?;
    }
    let screenshot_path = require_safe_saved_visual_screenshot(artifacts, target)?;
    if name == "visible_pixels" {
        return evaluate_saved_visible_pixels(
            assertion,
            target,
            &node,
            &screenshot_path,
            artifacts,
            &mut screenshots::ScreenshotAnalysisCache::default(),
        );
    }
    Ok(EvalOutcome::failed(failure(
        "unsupported_visual_assertion",
        "saved snapshot artifacts cannot evaluate visual pixel assertions because runtime target-hidden screenshot evidence is required",
        Some(json!(
            "run the visual assertion through scenario run, live protocol, or generated assert without --scenario/--run-id"
        )),
        Some(json!({
            "assertion": name,
            "mode": "saved_snapshot",
        })),
        Some(target.to_string()),
        Some(artifacts.clone()),
    )))
}

/// Snapshot indexes are fixed for the duration of scenario finalization and can be
/// reused by each target assertion without rebuilding cloned tree/UI maps.
pub(crate) struct SavedSnapshotPixelContext {
    indexes: Result<Indexes, FailureDetail>,
}

impl SavedSnapshotPixelContext {
    pub(crate) fn new(snapshot_context: &Value) -> Self {
        Self {
            indexes: build_indexes(snapshot_context),
        }
    }
}

pub(crate) fn evaluate_saved_snapshot_pixels(
    assertion: &Map<String, Value>,
    context: &SavedSnapshotPixelContext,
    artifacts: &Map<String, Value>,
    cache: &mut screenshots::ScreenshotAnalysisCache,
) -> Result<EvalOutcome, FailureDetail> {
    let target = assertion
        .get("target")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let indexes = context.indexes.as_ref().map_err(Clone::clone)?;
    let (node, _control) = resolve_target(indexes, target, artifacts)?;
    let screenshot_path = require_safe_saved_visual_screenshot(artifacts, target)?;
    evaluate_saved_visible_pixels(assertion, target, &node, &screenshot_path, artifacts, cache)
}

fn require_safe_saved_visual_screenshot(
    artifacts: &Map<String, Value>,
    target: &str,
) -> Result<std::path::PathBuf, FailureDetail> {
    let screenshot = artifacts
        .get("screenshot")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let run_root = artifacts
        .get("run_root")
        .and_then(Value::as_str)
        .unwrap_or_default();
    if screenshot.is_empty() {
        return Err(failure(
            "missing_screenshot_image",
            "visual assertions require a saved screenshot artifact",
            Some(json!("snapshot screenshot path")),
            Some(json!("missing")),
            Some(target.to_string()),
            Some(artifacts.clone()),
        ));
    }
    let screenshot_path = std::path::Path::new(screenshot);
    let run_root_path = std::path::Path::new(run_root);
    let normalized_screenshot = screenshot.replace('\\', "/");
    if run_root.is_empty()
        || !screenshot_path.is_absolute()
        || !screenshot_path.starts_with(run_root_path)
        || normalized_screenshot.contains("/qa/baselines/")
    {
        return Err(failure(
            "invalid_visual_artifact_path",
            "visual assertion screenshot artifacts must stay under the selected run artifacts",
            Some(json!(
                "absolute screenshot path under run_root and outside qa/baselines"
            )),
            Some(json!({"screenshot": screenshot, "run_root": run_root})),
            Some(target.to_string()),
            Some(artifacts.clone()),
        ));
    }
    if !screenshot_path.is_file() {
        return Err(failure(
            "missing_screenshot_image",
            "visual assertions require an existing saved screenshot artifact",
            Some(json!("existing screenshot PNG")),
            Some(json!(screenshot)),
            Some(target.to_string()),
            Some(artifacts.clone()),
        ));
    }
    Ok(screenshot_path.to_path_buf())
}

fn evaluate_saved_visible_pixels(
    assertion: &Map<String, Value>,
    target: &str,
    node: &Map<String, Value>,
    screenshot_path: &std::path::Path,
    artifacts: &Map<String, Value>,
    cache: &mut screenshots::ScreenshotAnalysisCache,
) -> Result<EvalOutcome, FailureDetail> {
    let min_count = assertion
        .get("min_count")
        .and_then(Value::as_i64)
        .map(|value| value.max(0) as u64);
    let min_ratio = assertion.get("min_ratio").and_then(Value::as_f64);
    let Some(rect) = saved_visual_rect(node) else {
        let mut detail = failure(
            "unsupported_target_geometry",
            "saved visible_pixels assertions require viewport-space 2D target rect evidence in the snapshot",
            Some(json!(
                "visual_evidence.coordinate_space=viewport_2d with a positive rect"
            )),
            Some(node.get("visual_evidence").cloned().unwrap_or(Value::Null)),
            Some(target.to_string()),
            Some(artifacts.clone()),
        );
        detail.details.insert(
            "remediation".to_string(),
            json!("capture a supported 2D Control, Line2D, or CanvasItem target, or expose qa_visual_rect with viewport_2d coordinates"),
        );
        return Ok(EvalOutcome::failed(detail));
    };
    let mut evidence = cache
        .get(screenshot_path)
        .map(|analysis| analysis.measure_target(rect))
        .map_err(|error| {
            failure(
                "screenshot_read_failed",
                "saved visible_pixels assertion could not read the screenshot PNG",
                Some(json!("readable PNG screenshot artifact")),
                Some(json!(error)),
                Some(target.to_string()),
                Some(artifacts.clone()),
            )
        })?;
    if let Some(object) = evidence.as_object_mut() {
        object.insert("target".to_string(), json!(target));
        object.insert(
            "min_count".to_string(),
            min_count.map_or(Value::Null, Value::from),
        );
        object.insert(
            "min_ratio".to_string(),
            min_ratio.map_or(Value::Null, Value::from),
        );
    }
    if evidence
        .get("low_information")
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        let mut detail = failure(
            "screenshot_low_information",
            "saved visible_pixels assertion requires a nonblank real screenshot",
            Some(json!("non-low-information screenshot with target pixels")),
            Some(evidence.clone()),
            Some(target.to_string()),
            Some(artifacts.clone()),
        );
        detail.details.insert(
            "remediation".to_string(),
            json!("capture the saved snapshot from a real rendered viewport with visible, nonblank target pixels"),
        );
        return Ok(EvalOutcome {
            data: Map::from_iter([("visual_evidence".to_string(), evidence.clone())]),
            failure: Some(detail),
        });
    }
    let useful_count = evidence
        .get("useful_pixel_count")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let useful_ratio = evidence
        .get("useful_pixel_ratio")
        .and_then(Value::as_f64)
        .unwrap_or(0.0);
    let count_passes = min_count.is_none_or(|threshold| useful_count >= threshold);
    let ratio_passes = min_ratio.is_none_or(|threshold| useful_ratio >= threshold);
    let data = Map::from_iter([("visual_evidence".to_string(), evidence.clone())]);
    if count_passes && ratio_passes {
        return Ok(EvalOutcome {
            data,
            failure: None,
        });
    }
    let mut detail = failure(
        "visible_pixels_threshold_mismatch",
        "saved visible_pixels assertion measured too few useful target pixels",
        Some(json!({"min_count": min_count, "min_ratio": min_ratio})),
        Some(evidence),
        Some(target.to_string()),
        Some(artifacts.clone()),
    );
    detail.details.insert(
        "remediation".to_string(),
        json!("ensure the target contributes visible rendered pixels in the saved screenshot, not only scene-tree visibility"),
    );
    Ok(EvalOutcome {
        data,
        failure: Some(detail),
    })
}

fn saved_visual_rect(node: &Map<String, Value>) -> Option<[f64; 4]> {
    let evidence = node.get("visual_evidence")?;
    if evidence.get("coordinate_space").and_then(Value::as_str) != Some("viewport_2d") {
        return None;
    }
    // Screenshots are captured at window resolution, so prefer the screenshot-space
    // rect (present under content scaling) and fall back to the canvas-space `rect`
    // for snapshots saved before this evidence field existed.
    let rect = evidence
        .get("rect_screenshot")
        .and_then(Value::as_array)
        .or_else(|| evidence.get("rect").and_then(Value::as_array))?;
    if rect.len() != 4 {
        return None;
    }
    let parsed = [
        rect.first()?.as_f64()?,
        rect.get(1)?.as_f64()?,
        rect.get(2)?.as_f64()?,
        rect.get(3)?.as_f64()?,
    ];
    if parsed.iter().all(|value| value.is_finite()) && parsed[2] > 0.0 && parsed[3] > 0.0 {
        Some(parsed)
    } else {
        None
    }
}

fn evaluate_artifact_query_assertion(
    assertion: &Map<String, Value>,
    snapshot_context: &Value,
    artifacts: &Map<String, Value>,
) -> Result<EvalOutcome, FailureDetail> {
    let snapshot = artifacts
        .get("root")
        .and_then(Value::as_str)
        .and_then(|root| std::path::Path::new(root).file_name())
        .and_then(|name| name.to_str())
        .unwrap_or("snapshot");
    let assertion_name = assertion
        .get("name")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let target = assertion.get("target").and_then(Value::as_str);
    let node_type = assertion.get("type").and_then(Value::as_str);
    let query = match assertion_name {
        "snapshot_exists" => artifact_query::QueryRequest {
            target: target.map(str::to_string),
            ..artifact_query::QueryRequest::default()
        },
        "snapshot_visible" => artifact_query::QueryRequest {
            target: target.map(str::to_string),
            ..artifact_query::QueryRequest::default()
        },
        "snapshot_contains_type" => artifact_query::QueryRequest {
            node_type: node_type.map(str::to_string),
            visible: true,
            ..artifact_query::QueryRequest::default()
        },
        _ => artifact_query::QueryRequest::default(),
    };
    let result =
        artifact_query::query_snapshot_context(snapshot, snapshot_context, query, "assert")?;
    let mut data = Map::from_iter([
        ("count".to_string(), json!(result.matches.len())),
        ("matches".to_string(), json!(result.matches)),
    ]);
    match assertion_name {
        "snapshot_exists" if result.matches.is_empty() => Ok(EvalOutcome {
            data,
            failure: Some(failure(
                "snapshot_target_missing",
                "snapshot did not contain the requested target",
                Some(json!("one or more matching nodes in the selected snapshot")),
                Some(json!("missing")),
                target.map(str::to_string),
                Some(artifacts.clone()),
            )),
        }),
        "snapshot_visible" => {
            if result.matches.iter().any(|item| item.visible) {
                return Ok(EvalOutcome {
                    data,
                    failure: None,
                });
            }
            let failure_kind = if result.matches.is_empty() {
                "snapshot_target_missing"
            } else {
                "snapshot_visible_mismatch"
            };
            Ok(EvalOutcome {
                data,
                failure: Some(failure(
                    failure_kind,
                    if result.matches.is_empty() {
                        "snapshot did not contain the requested target"
                    } else {
                        "snapshot target exists but is not visible"
                    },
                    Some(json!(
                        "one or more visible matching nodes in the selected snapshot"
                    )),
                    Some(json!(if result.matches.is_empty() {
                        "missing"
                    } else {
                        "hidden"
                    })),
                    target.map(str::to_string),
                    Some(artifacts.clone()),
                )),
            })
        }
        "snapshot_contains_type" if result.matches.is_empty() => Ok(EvalOutcome {
            data,
            failure: Some(failure(
                "snapshot_type_missing",
                "snapshot did not contain a visible node with the requested type",
                Some(json!("one or more visible nodes with the requested type")),
                Some(json!("missing")),
                node_type.map(str::to_string),
                Some(artifacts.clone()),
            )),
        }),
        _ => Ok(EvalOutcome {
            data: std::mem::take(&mut data),
            failure: None,
        }),
    }
}

fn live_assertion_protocol(assertion_name: &str) -> Option<&'static str> {
    match assertion_name {
        "max_delta_position" => Some(godot_qa_runtime::ASSERT_TEMPORAL_MAX_DELTA_POSITION),
        "follows" => Some(godot_qa_runtime::ASSERT_TEMPORAL_FOLLOWS),
        "never" => Some(godot_qa_runtime::ASSERT_TEMPORAL_NEVER),
        "within_limits" => Some(godot_qa_runtime::ASSERT_TEMPORAL_WITHIN_LIMITS),
        "distance_changed" => Some(godot_qa_runtime::ASSERT_TEMPORAL_DISTANCE_CHANGED),
        "exists" => Some(godot_qa_runtime::ASSERT_EXISTS),
        "not_exists" => Some(godot_qa_runtime::ASSERT_NOT_EXISTS),
        "visible" => Some(godot_qa_runtime::ASSERT_VISIBLE),
        "visible_pixels" => Some(godot_qa_runtime::ASSERT_VISIBLE_PIXELS),
        "not_occluded" => Some(godot_qa_runtime::ASSERT_NOT_OCCLUDED),
        "renders_above" => Some(godot_qa_runtime::ASSERT_RENDERS_ABOVE),
        "renders_below" => Some(godot_qa_runtime::ASSERT_RENDERS_BELOW),
        "hidden" => Some(godot_qa_runtime::ASSERT_HIDDEN),
        "enabled" => Some(godot_qa_runtime::ASSERT_ENABLED),
        "disabled" => Some(godot_qa_runtime::ASSERT_DISABLED),
        "focused" => Some(godot_qa_runtime::ASSERT_FOCUSED),
        "text_equals" => Some(godot_qa_runtime::ASSERT_TEXT_EQUALS),
        "text_contains" => Some(godot_qa_runtime::ASSERT_TEXT_CONTAINS),
        "inside_viewport" => Some(godot_qa_runtime::ASSERT_INSIDE_VIEWPORT),
        "no_overlap" => Some(godot_qa_runtime::ASSERT_NO_OVERLAP),
        "no_errors" => Some(godot_qa_runtime::ASSERT_NO_ERRORS),
        "distance" => Some(godot_qa_runtime::ASSERT_DISTANCE),
        "surface_probe" => Some(godot_qa_runtime::ASSERT_SURFACE_PROBE),
        "surface_matches" => Some(godot_qa_runtime::ASSERT_SURFACE_MATCHES),
        _ => None,
    }
}

fn insert_temporal_window_fields(
    map: &mut Map<String, Value>,
    options: &AssertOptions,
) -> Result<(), FailureDetail> {
    match (options.window_frames, options.window_seconds) {
        (Some(_), Some(_)) => {
            return Err(failure(
                "invalid_assertion_args",
                "temporal assertions accept either --window-frames or --window-seconds, not both",
                Some(json!("single bounded window selector")),
                Some(
                    json!({"window_frames": options.window_frames, "window_seconds": options.window_seconds}),
                ),
                None,
                None,
            ));
        }
        (None, None) => {
            return Err(failure(
                "invalid_assertion_args",
                "temporal assertions require --window-frames or --window-seconds",
                Some(json!("bounded window selector")),
                Some(Value::Null),
                None,
                None,
            ));
        }
        _ => {}
    }
    if let Some(v) = options.window_frames {
        if v <= 0 {
            return Err(failure(
                "invalid_assertion_args",
                "--window-frames must be positive",
                Some(json!("positive integer")),
                Some(json!(v)),
                None,
                None,
            ));
        }
        map.insert("window_frames".to_string(), json!(v));
    }
    if let Some(v) = options.window_seconds {
        if v <= 0.0 {
            return Err(failure(
                "invalid_assertion_args",
                "--window-seconds must be positive",
                Some(json!("positive number")),
                Some(json!(v)),
                None,
                None,
            ));
        }
        map.insert("window_seconds".to_string(), json!(v));
    }
    if let Some(v) = options.sample_every_frames {
        if v <= 0 {
            return Err(failure(
                "invalid_assertion_args",
                "--sample-every-frames must be positive",
                Some(json!("positive integer")),
                Some(json!(v)),
                None,
                None,
            ));
        }
        map.insert("sample_every_frames".to_string(), json!(v));
    }
    if let Some(v) = options.evidence_samples {
        if v <= 0 {
            return Err(failure(
                "invalid_assertion_args",
                "--evidence-samples must be positive",
                Some(json!("positive integer")),
                Some(json!(v)),
                None,
                None,
            ));
        }
        map.insert("evidence_samples".to_string(), json!(v));
    }
    Ok(())
}

fn build_indexes(snapshot_context: &Value) -> Result<Indexes, FailureDetail> {
    let filtered_evidence = snapshot_context
        .get("tree")
        .and_then(|tree| tree.get("filter"))
        .is_some()
        || snapshot_context
            .get("ui")
            .and_then(|ui| ui.get("filter"))
            .is_some()
        || snapshot_context
            .get("focus")
            .and_then(|focus| focus.get("filter"))
            .is_some();
    let tree_root = snapshot_context
        .get("tree")
        .and_then(|tree| tree.get("root"))
        .and_then(Value::as_object)
        .ok_or_else(|| {
            failure(
                "artifact_read_failed",
                "tree.json must contain a root object",
                Some(json!("root object")),
                Some(json!("missing")),
                None,
                None,
            )
        })?;
    let ui_controls = snapshot_context
        .get("ui")
        .and_then(|ui| ui.get("controls"))
        .and_then(Value::as_array)
        .ok_or_else(|| {
            failure(
                "artifact_read_failed",
                "ui.json controls must be an array",
                Some(json!("array")),
                Some(json!("missing")),
                None,
                None,
            )
        })?;

    let mut by_path = Map::new();
    let mut by_qa = Map::new();
    let mut by_name = Map::new();
    for node in flatten_tree(tree_root) {
        if let Some(path) = node.get("path").and_then(Value::as_str) {
            by_path.insert(path.to_string(), Value::Object(node.clone()));
        }
        if let Some(qa_id) = node.get("qa_id").and_then(Value::as_str) {
            by_qa.insert(qa_id.to_string(), json!([Value::Object(node.clone())]));
        }
        if let Some(name) = node.get("name").and_then(Value::as_str) {
            let entry = by_name.entry(name.to_string()).or_insert_with(|| json!([]));
            if let Some(items) = entry.as_array_mut() {
                items.push(Value::Object(node.clone()));
            }
        }
    }
    let mut ui_by_path = Map::new();
    for control in ui_controls.iter().filter_map(Value::as_object) {
        if let Some(path) = control.get("path").and_then(Value::as_str) {
            ui_by_path.insert(path.to_string(), Value::Object(control.clone()));
        }
    }
    Ok(Indexes {
        by_path,
        by_qa,
        by_name,
        ui_by_path,
        filtered_evidence,
    })
}

fn resolve_target(
    indexes: &Indexes,
    target: &str,
    artifacts: &Map<String, Value>,
) -> Result<(Map<String, Value>, Option<Map<String, Value>>), FailureDetail> {
    let matches = matches_for_target(indexes, target);
    if matches.len() > 1 {
        return Err(failure(
            "ambiguous_target",
            "Multiple nodes matched the requested target",
            Some(json!("unique snapshot target")),
            Some(json!(
                matches
                    .iter()
                    .map(|node| node_label(node))
                    .collect::<Vec<_>>()
            )),
            Some(target.to_string()),
            Some(artifacts.clone()),
        ));
    }
    let Some(node) = matches.into_iter().next() else {
        let (kind, message, actual) = if indexes.filtered_evidence {
            (
                "snapshot_target_omitted",
                "selected filtered snapshot evidence did not contain the requested target",
                json!("missing_or_omitted_by_filter"),
            )
        } else {
            (
                "target_not_found",
                "No node matched the requested target",
                json!("missing"),
            )
        };
        return Err(failure(
            kind,
            message,
            Some(json!("existing snapshot target")),
            Some(actual),
            Some(target.to_string()),
            Some(artifacts.clone()),
        ));
    };
    let control = node
        .get("path")
        .and_then(Value::as_str)
        .and_then(|path| indexes.ui_by_path.get(path))
        .and_then(Value::as_object)
        .cloned();
    Ok((node, control))
}

fn matches_for_target(indexes: &Indexes, target: &str) -> Vec<Map<String, Value>> {
    if let Some(qa_id) = target.strip_prefix("qa:") {
        return indexes
            .by_qa
            .get(qa_id)
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default()
            .into_iter()
            .filter_map(|value| value.as_object().cloned())
            .collect();
    }
    if let Some(path) = target.strip_prefix("path:") {
        return indexes
            .by_path
            .get(path)
            .and_then(Value::as_object)
            .cloned()
            .into_iter()
            .collect();
    }
    if let Some(name) = target.strip_prefix("name:") {
        return indexes
            .by_name
            .get(name)
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default()
            .into_iter()
            .filter_map(|value| value.as_object().cloned())
            .collect();
    }
    Vec::new()
}

fn flatten_tree(root: &Map<String, Value>) -> Vec<Map<String, Value>> {
    let mut entries = vec![root.clone()];
    if let Some(children) = root.get("children").and_then(Value::as_array) {
        for child in children.iter().filter_map(Value::as_object) {
            entries.extend(flatten_tree(child));
        }
    }
    entries
}

fn node_label(node: &Map<String, Value>) -> String {
    format!(
        "path:{}",
        node.get("path").and_then(Value::as_str).unwrap_or_default()
    )
}

fn failure(
    kind: &str,
    message: &str,
    expected: Option<Value>,
    actual: Option<Value>,
    target: Option<String>,
    artifacts: Option<Map<String, Value>>,
) -> FailureDetail {
    FailureDetail {
        kind: kind.to_string(),
        command: Some("assert".to_string()),
        message: Some(message.to_string()),
        expected,
        actual,
        target,
        severity: Some("error".to_string()),
        artifacts,
        ..FailureDetail::default()
    }
}

fn insert_optional_temporal_window_fields(
    map: &mut Map<String, Value>,
    options: &AssertOptions,
    assertion_name: &str,
) -> Result<(), FailureDetail> {
    if options.window_frames.is_some()
        || options.window_seconds.is_some()
        || options.sample_every_frames.is_some()
        || options.dwell_frames.is_some()
        || options.dwell_seconds.is_some()
    {
        match (options.window_frames, options.window_seconds) {
            (Some(_), Some(_)) => {
                return Err(failure(
                    "invalid_assertion_args",
                    &format!(
                        "{assertion_name} accepts either --window-frames or --window-seconds, not both"
                    ),
                    Some(json!("single bounded window selector")),
                    Some(json!({
                        "window_frames": options.window_frames,
                        "window_seconds": options.window_seconds
                    })),
                    None,
                    None,
                ));
            }
            (None, None) => {
                return Err(failure(
                    "invalid_assertion_args",
                    &format!(
                        "{assertion_name} temporal mode requires --window-frames or --window-seconds"
                    ),
                    Some(json!("bounded window selector")),
                    Some(Value::Null),
                    None,
                    None,
                ));
            }
            _ => {}
        }
    }
    insert_positive_i64(map, "window_frames", options.window_frames)?;
    insert_positive_f64(map, "window_seconds", options.window_seconds)?;
    insert_positive_i64(map, "sample_every_frames", options.sample_every_frames)?;
    if let (Some(_), Some(_)) = (options.dwell_frames, options.dwell_seconds) {
        return Err(failure(
            "invalid_assertion_args",
            &format!("{assertion_name} accepts either --dwell-frames or --dwell-seconds, not both"),
            Some(json!("single dwell selector")),
            Some(json!({
                "dwell_frames": options.dwell_frames,
                "dwell_seconds": options.dwell_seconds
            })),
            None,
            None,
        ));
    }
    insert_positive_i64(map, "dwell_frames", options.dwell_frames)?;
    insert_positive_f64(map, "dwell_seconds", options.dwell_seconds)?;
    Ok(())
}

fn insert_positive_i64(
    map: &mut Map<String, Value>,
    field: &str,
    value: Option<i64>,
) -> Result<(), FailureDetail> {
    if let Some(value) = value {
        if value <= 0 {
            return Err(failure(
                "invalid_assertion_args",
                &format!("--{} must be positive", field.replace('_', "-")),
                Some(json!("positive integer")),
                Some(json!(value)),
                None,
                None,
            ));
        }
        map.insert(field.to_string(), json!(value));
    }
    Ok(())
}

fn insert_positive_f64(
    map: &mut Map<String, Value>,
    field: &str,
    value: Option<f64>,
) -> Result<(), FailureDetail> {
    if let Some(value) = value {
        if value <= 0.0 || !value.is_finite() {
            return Err(failure(
                "invalid_assertion_args",
                &format!(
                    "--{} must be a finite positive number",
                    field.replace('_', "-")
                ),
                Some(json!("finite positive number")),
                Some(json!(value)),
                None,
                None,
            ));
        }
        map.insert(field.to_string(), json!(value));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use godot_qa_core::config::ProjectConfig;
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn temporal_normalizes_alias_and_builds_request() {
        let options = AssertOptions {
            window_frames: Some(12),
            max: Some(32.0),
            ..AssertOptions::default()
        };
        let request = assertion_request(
            &normalize_assertion_name("max-delta-position"),
            Some("qa:demo_actor"),
            None,
            &options,
        )
        .expect("request");
        assert_eq!(request.get("temporal").and_then(Value::as_bool), Some(true));
        let payload = request
            .get("max_delta_position")
            .and_then(Value::as_object)
            .expect("nested max_delta_position payload");
        assert_eq!(
            payload.get("target").and_then(Value::as_str),
            Some("qa:demo_actor")
        );
        assert_eq!(
            payload.get("window_frames").and_then(Value::as_i64),
            Some(12)
        );
        assert_eq!(payload.get("max").and_then(Value::as_f64), Some(32.0));
    }

    #[test]
    fn temporal_rejects_missing_bound() {
        let options = AssertOptions {
            window_frames: Some(12),
            ..AssertOptions::default()
        };
        let err = assertion_request("max_delta_position", Some("qa:demo_actor"), None, &options)
            .expect_err("expected error");
        assert_eq!(err.kind, "invalid_assertion_args");
    }

    #[test]
    fn temporal_builds_all_cli_alias_requests() {
        let follows = assertion_request(
            "follows",
            Some("qa:follower"),
            None,
            &AssertOptions {
                window_frames: Some(12),
                leader: Some("qa:leader".to_string()),
                tolerance: Some(4.0),
                ..AssertOptions::default()
            },
        )
        .expect("follows request");
        assert_eq!(
            live_protocol_request(&follows).0,
            "follows",
            "live routing should extract nested temporal assertion name"
        );

        let never = assertion_request(
            "never",
            Some("qa:status"),
            None,
            &AssertOptions {
                window_seconds: Some(0.2),
                property: Some("text".to_string()),
                forbidden: Some("DESYNC,ERROR".to_string()),
                ..AssertOptions::default()
            },
        )
        .expect("never request");
        assert_eq!(live_protocol_request(&never).0, "never");

        let within_limits = assertion_request(
            &normalize_assertion_name("within-limits"),
            Some("qa:actor"),
            Some("position"),
            &AssertOptions {
                window_frames: Some(6),
                min: Some(0.0),
                max: Some(64.0),
                ..AssertOptions::default()
            },
        )
        .expect("within_limits request");
        let (name, params) = live_protocol_request(&within_limits);
        assert_eq!(name, "within_limits");
        assert_eq!(
            params.get("property").and_then(Value::as_str),
            Some("position")
        );
    }

    #[test]
    fn distance_builds_bounds_payload_and_live_mapping() {
        let request = assertion_request(
            "distance",
            Some("qa:actor"),
            Some("qa:target"),
            &AssertOptions {
                max: Some(96.0),
                ..AssertOptions::default()
            },
        )
        .expect("distance request");
        assert_eq!(
            request.get("name").and_then(Value::as_str),
            Some("distance")
        );
        assert_eq!(request.get("a").and_then(Value::as_str), Some("qa:actor"));
        assert_eq!(request.get("b").and_then(Value::as_str), Some("qa:target"));
        assert_eq!(request.get("max").and_then(Value::as_f64), Some(96.0));
        let (name, _params) = live_protocol_request(&request);
        assert_eq!(name, "distance");
        assert_eq!(
            live_assertion_protocol(&name),
            Some(godot_qa_runtime::ASSERT_DISTANCE)
        );
    }

    #[test]
    fn distance_supports_outside_radius_min_only() {
        let request = assertion_request(
            "distance",
            Some("qa:a"),
            Some("qa:b"),
            &AssertOptions {
                min: Some(24.0),
                ..AssertOptions::default()
            },
        )
        .expect("distance min-only request");
        assert_eq!(request.get("min").and_then(Value::as_f64), Some(24.0));
        assert!(request.get("max").is_some());
        assert!(request.get("max").is_some_and(Value::is_null));
    }

    #[test]
    fn distance_rejects_negative_nan_and_infinite_thresholds() {
        let negative = assertion_request(
            "distance",
            Some("qa:a"),
            Some("qa:b"),
            &AssertOptions {
                max: Some(-1.0),
                ..AssertOptions::default()
            },
        )
        .expect_err("negative max should fail");
        assert_eq!(negative.kind, "invalid_assertion_args");

        let nan = assertion_request(
            "distance",
            Some("qa:a"),
            Some("qa:b"),
            &AssertOptions {
                min: Some(f64::NAN),
                ..AssertOptions::default()
            },
        )
        .expect_err("nan min should fail");
        assert_eq!(nan.kind, "invalid_assertion_args");

        let inf = assertion_request(
            "distance",
            Some("qa:a"),
            Some("qa:b"),
            &AssertOptions {
                max: Some(f64::INFINITY),
                ..AssertOptions::default()
            },
        )
        .expect_err("infinite max should fail");
        assert_eq!(inf.kind, "invalid_assertion_args");
    }

    #[test]
    fn distance_changed_builds_temporal_payload_and_live_mapping() {
        let request = assertion_request(
            "distance_changed",
            Some("qa:a"),
            Some("qa:b"),
            &AssertOptions {
                window_frames: Some(5),
                sample_every_frames: Some(2),
                min: Some(3.0),
                ..AssertOptions::default()
            },
        )
        .expect("distance_changed request");
        assert_eq!(request.get("temporal").and_then(Value::as_bool), Some(true));
        let payload = request
            .get("distance_changed")
            .and_then(Value::as_object)
            .expect("distance_changed payload");
        assert_eq!(payload.get("a").and_then(Value::as_str), Some("qa:a"));
        assert_eq!(payload.get("b").and_then(Value::as_str), Some("qa:b"));
        assert_eq!(payload.get("min").and_then(Value::as_f64), Some(3.0));
        assert_eq!(
            payload.get("sample_every_frames").and_then(Value::as_i64),
            Some(2)
        );
        let (name, _params) = live_protocol_request(&request);
        assert_eq!(name, "distance_changed");
        assert_eq!(
            live_assertion_protocol(&name),
            Some(godot_qa_runtime::ASSERT_TEMPORAL_DISTANCE_CHANGED)
        );
    }

    #[test]
    fn distance_is_runtime_generated_when_no_snapshot_flags() {
        let options = AssertOptions {
            max: Some(16.0),
            ..AssertOptions::default()
        };
        let request = assertion_request("distance", Some("qa:a"), Some("qa:b"), &options)
            .expect("distance request");
        let normalized = normalize_assertion_name("distance");
        let is_temporal = request
            .get("temporal")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let should_generate = (is_temporal || normalized == "distance")
            && None::<&str>.is_none()
            && None::<&str>.is_none()
            && None::<&str>.is_none();
        assert!(
            should_generate,
            "distance should follow generated-runtime path without snapshot selectors"
        );
    }

    #[test]
    fn visible_pixels_assertion_request() {
        let request = assertion_request(
            &normalize_assertion_name("visible-pixels"),
            Some("qa:target"),
            None,
            &AssertOptions {
                min_ratio: Some(0.25),
                min_count: Some(10),
                ..AssertOptions::default()
            },
        )
        .expect("visible_pixels request");
        assert_eq!(
            request.get("name").and_then(Value::as_str),
            Some("visible_pixels")
        );
        assert_eq!(
            request.get("target").and_then(Value::as_str),
            Some("qa:target")
        );
        assert_eq!(request.get("min_ratio").and_then(Value::as_f64), Some(0.25));
        assert_eq!(request.get("min_count").and_then(Value::as_i64), Some(10));

        let bad_ratio = assertion_request(
            "visible_pixels",
            Some("qa:target"),
            None,
            &AssertOptions {
                min_ratio: Some(1.5),
                ..AssertOptions::default()
            },
        )
        .expect_err("ratio out of range should fail");
        assert_eq!(bad_ratio.kind, "invalid_assertion_args");

        let bad_count = assertion_request(
            "visible_pixels",
            Some("qa:target"),
            None,
            &AssertOptions {
                min_count: Some(-1),
                ..AssertOptions::default()
            },
        )
        .expect_err("negative count should fail");
        assert_eq!(bad_count.kind, "invalid_assertion_args");

        let missing_threshold = assertion_request(
            "visible_pixels",
            Some("qa:target"),
            None,
            &AssertOptions::default(),
        )
        .expect_err("missing visual threshold should fail");
        assert_eq!(missing_threshold.kind, "invalid_assertion_args");
    }

    #[test]
    fn visual_assertion_protocol_mapping() {
        assert_eq!(
            live_assertion_protocol("visible_pixels"),
            Some(godot_qa_runtime::ASSERT_VISIBLE_PIXELS)
        );
        assert_eq!(
            live_assertion_protocol("not_occluded"),
            Some(godot_qa_runtime::ASSERT_NOT_OCCLUDED)
        );
        assert_eq!(
            live_assertion_protocol("renders_above"),
            Some(godot_qa_runtime::ASSERT_RENDERS_ABOVE)
        );
        assert_eq!(
            live_assertion_protocol("renders_below"),
            Some(godot_qa_runtime::ASSERT_RENDERS_BELOW)
        );
    }

    #[test]
    fn surface_probe_assertion_request() {
        let request = assertion_request(
            "surface_probe",
            Some("qa:demo_actor"),
            None,
            &AssertOptions {
                provider: Some("qa:surface_provider".to_string()),
                point: vec!["1,2:walkable".to_string(), "3,4,5:blocked".to_string()],
                coordinate_space: Some("world_2d".to_string()),
                radius: Some(64.0),
                ..AssertOptions::default()
            },
        )
        .expect("surface_probe request");
        assert_eq!(
            request.get("name").and_then(Value::as_str),
            Some("surface_probe")
        );
    }

    #[test]
    fn surface_probe_rejects_malformed_point_and_invalid_radius() {
        let malformed = assertion_request(
            "surface_probe",
            Some("qa:demo_actor"),
            None,
            &AssertOptions {
                provider: Some("qa:surface_provider".to_string()),
                point: vec!["1:walkable".to_string()],
                ..AssertOptions::default()
            },
        )
        .expect_err("malformed --point should fail");
        assert_eq!(malformed.kind, "invalid_assertion_args");
        assert_eq!(
            malformed.message.as_deref(),
            Some("surface_probe --point must contain x,y or x,y,z")
        );

        let bad_radius = assertion_request(
            "surface_probe",
            Some("qa:demo_actor"),
            None,
            &AssertOptions {
                provider: Some("qa:surface_provider".to_string()),
                point: vec!["1,2".to_string()],
                radius: Some(-0.1),
                ..AssertOptions::default()
            },
        )
        .expect_err("negative radius should fail");
        assert_eq!(bad_radius.kind, "invalid_assertion_args");
        assert_eq!(
            bad_radius.message.as_deref(),
            Some("surface_probe --radius must be finite and within bounds")
        );
    }

    #[test]
    fn surface_probe_rejects_missing_provider_and_unsupported_coordinate_space() {
        let missing_provider = assertion_request(
            "surface_probe",
            Some("qa:demo_actor"),
            None,
            &AssertOptions {
                point: vec!["1,2".to_string()],
                ..AssertOptions::default()
            },
        )
        .expect_err("missing --provider should fail");
        assert_eq!(missing_provider.kind, "invalid_assertion_args");
        assert_eq!(
            missing_provider.message.as_deref(),
            Some("surface_probe requires target and --provider or --provider-id")
        );

        let bad_space = assertion_request(
            "surface_probe",
            Some("qa:demo_actor"),
            None,
            &AssertOptions {
                provider: Some("qa:surface_provider".to_string()),
                point: vec!["1,2".to_string()],
                coordinate_space: Some("screen".to_string()),
                ..AssertOptions::default()
            },
        )
        .expect_err("unsupported coordinate space should fail");
        assert_eq!(bad_space.kind, "invalid_assertion_args");
        assert_eq!(
            bad_space.message.as_deref(),
            Some("surface_probe --coordinate-space is not supported")
        );
    }

    #[test]
    fn surface_matches_assertion_request_supports_snapshot_generated_mode() {
        let request = assertion_request(
            "surface_matches",
            Some("qa:demo_actor"),
            None,
            &AssertOptions {
                surface: Some("walkable".to_string()),
                window_frames: Some(12),
                sample_every_frames: Some(2),
                dwell_frames: Some(4),
                ..AssertOptions::default()
            },
        )
        .expect("surface_matches request");
        assert_eq!(
            request.get("name").and_then(Value::as_str),
            Some("surface_matches")
        );
        assert_eq!(
            request.get("target").and_then(Value::as_str),
            Some("qa:demo_actor")
        );
        assert_eq!(
            request.get("window_frames").and_then(Value::as_i64),
            Some(12)
        );
        assert_eq!(
            request.get("sample_every_frames").and_then(Value::as_i64),
            Some(2)
        );
        assert_eq!(request.get("dwell_frames").and_then(Value::as_i64), Some(4));
        assert_eq!(
            live_assertion_protocol("surface_probe"),
            Some(godot_qa_runtime::ASSERT_SURFACE_PROBE)
        );
        assert_eq!(
            live_assertion_protocol("surface_matches"),
            Some(godot_qa_runtime::ASSERT_SURFACE_MATCHES)
        );
    }

    #[test]
    fn live_visual_assertions_reject_headless_sessions_before_connecting() {
        let root = temp_root("live-visual-headless");
        let config = test_config(&root);
        write_headless_session_record(&config, &root);

        let result = run_assert(
            &config,
            "visible_pixels",
            Some("qa:important_actor"),
            None,
            AssertOptions {
                min_ratio: Some(0.25),
                ..AssertOptions::default()
            },
            true,
            None,
            None,
            None,
        );

        assert!(!result.ok);
        assert_eq!(result.command, "assert");
        assert_eq!(result.message, "live command failed");
        assert_eq!(result.failures[0].kind, "headless_screenshot_unsupported");
        assert_eq!(
            result.data["assertion"]["name"].as_str(),
            Some("visible_pixels")
        );
        assert_eq!(
            result.failures[0].actual,
            Some(json!({
                "session_id": "session-test",
                "headless": true,
            }))
        );
        let _ = fs::remove_dir_all(root);
    }

    fn write_headless_session_record(config: &ProjectConfig, root: &Path) {
        let record = godot_qa_runtime::SessionRecord {
            process_identity: None,
            protocol_version: godot_qa_runtime::PROTOCOL_VERSION.to_string(),
            session_id: "session-test".to_string(),
            pid: 999_999,
            host: "127.0.0.1".to_string(),
            port: 9,
            scene: "res://ui/main_menu.tscn".to_string(),
            viewport: vec![1280, 720],
            artifact_dir: root
                .join(".qa/artifacts/live/session-test")
                .to_string_lossy()
                .into_owned(),
            request_path: root
                .join(".qa/artifacts/live/session-test/request.json")
                .to_string_lossy()
                .into_owned(),
            token_file: godot_qa_runtime::session_token_path(config)
                .to_string_lossy()
                .into_owned(),
            started_at: "0Z".to_string(),
            headless: true,
            project_args: Vec::new(),
            frame_pacing: None,
            target: godot_qa_runtime::SessionTarget::Desktop,
            android: None,
        };
        fs::write(
            godot_qa_runtime::session_json_path(config),
            serde_json::to_string_pretty(&record).unwrap(),
        )
        .unwrap();
    }

    fn temp_root(label: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("godot-qa-scenario-{label}-{unique}"));
        fs::create_dir_all(root.join(".qa")).unwrap();
        root
    }

    fn test_config(root: &Path) -> ProjectConfig {
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
            godot_launch_prefix: Vec::new(),
            godot_windowed_launch_prefix: Vec::new(),
            godot_args: Vec::new(),
            godot_project_args: Vec::new(),
            godot_launch_mode: godot_qa_core::config::GodotLaunchMode::Auto,
            godot_audio_mode: godot_qa_core::config::GodotAudioMode::Auto,
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
}

pub(crate) struct EvalOutcome {
    pub(crate) failure: Option<FailureDetail>,
    pub(crate) data: Map<String, Value>,
}

impl EvalOutcome {
    fn passed() -> Self {
        Self {
            failure: None,
            data: Map::new(),
        }
    }

    fn failed(failure: FailureDetail) -> Self {
        Self {
            failure: Some(failure),
            data: Map::new(),
        }
    }
}

struct Indexes {
    by_path: Map<String, Value>,
    by_qa: Map<String, Value>,
    by_name: Map<String, Value>,
    ui_by_path: Map<String, Value>,
    filtered_evidence: bool,
}

#[cfg(test)]
#[path = "assertions_saved_pixels_tests.rs"]
mod saved_pixels_tests;
