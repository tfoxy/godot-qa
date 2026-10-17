use crate::scenario_doc;
use godot_qa_core::config::ProjectConfig;
use godot_qa_core::results::{CommandResult, FailureDetail};
use serde::Serialize;
use serde_json::{Map, Value, json};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

const COMMAND: &str = "scenario lint";
const RULE_FIXTURE_MASKING: &str = "GQA-LINT-001";
const RULE_DEBUG_ONLY_ASSERTIONS: &str = "GQA-LINT-002";
const RULE_MISSING_VISUAL_CAPTURE: &str = "GQA-LINT-003";
const RULE_MISSING_NEGATIVE_ASSERTION: &str = "GQA-LINT-004";
const RULE_SCREENSHOT_OBSTRUCTION_RISK: &str = "GQA-LINT-005";
const RULE_HIGH_CARDINALITY_QA_LOCATOR: &str = "GQA-LINT-006";
const RULE_METADATA_STATE_ASSERTION: &str = "GQA-LINT-007";
const RULE_PROJECT_FLAG_WITHOUT_PROJECT_ARGS: &str = "GQA-LINT-008";
const RULE_MALFORMED_SUPPRESSION: &str = "malformed_suppression";

#[derive(Debug, Clone, Default)]
pub struct LintOptions {
    pub scenario: Option<String>,
    pub strict: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct LintFinding {
    pub rule_id: String,
    pub severity: String,
    pub scenario: Option<String>,
    pub scenario_path: Option<String>,
    pub target: Option<String>,
    pub step_index: Option<usize>,
    pub assertion_index: Option<usize>,
    pub reason: String,
    pub suggested_fix: Option<String>,
    pub suppressible: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub suppression: Option<Value>,
}

#[derive(Debug, Clone)]
struct LintSuppression {
    rule_id: String,
    reason: String,
    target: Option<String>,
}

#[derive(Debug, Clone, Default)]
struct StaticProjectIndex {
    scenario_names: HashMap<String, Vec<String>>,
    qa_ids: HashSet<String>,
}

pub fn run_scenario_lint(config: &ProjectConfig, options: LintOptions) -> CommandResult {
    let mut findings = Vec::<LintFinding>::new();
    let mut failures = Vec::<FailureDetail>::new();

    if !config.project_root.exists() {
        push_contract_error(
            &mut findings,
            &mut failures,
            "project_missing",
            None,
            Some(config.project_root.as_path()),
            format!(
                "project root does not exist: {}",
                config.project_root.display()
            ),
            Some("create the project directory or pass --project pointing at a valid project"),
            "path_missing",
            Some(json!("existing project root directory")),
            Some(json!(config.project_root)),
        );
        return build_result(findings, failures, options.strict);
    }
    if !config.scenarios_dir.exists() {
        push_contract_error(
            &mut findings,
            &mut failures,
            "scenarios_dir_missing",
            None,
            Some(config.scenarios_dir.as_path()),
            format!(
                "scenario directory does not exist: {}",
                config.scenarios_dir.display()
            ),
            Some("create the scenarios directory or run scenario init"),
            "path_missing",
            Some(json!("existing scenarios directory")),
            Some(json!(config.scenarios_dir)),
        );
        return build_result(findings, failures, options.strict);
    }

    let static_index = build_static_project_index(config);
    for (name, paths) in static_index
        .scenario_names
        .iter()
        .filter(|(_, paths)| paths.len() > 1)
    {
        push_contract_error(
            &mut findings,
            &mut failures,
            "duplicate_scenario_name",
            Some(name.to_string()),
            None,
            "multiple scenario files resolve to the same scenario name".to_string(),
            Some("rename scenarios so each declared name is unique"),
            "scenario_validation_failed",
            Some(json!("unique scenario names")),
            Some(json!(paths)),
        );
    }

    let listed = scenario_doc::list_scenarios(&config.scenarios_dir);
    let paths_to_lint = if let Some(selected) = options.scenario.as_deref().map(str::trim) {
        let matches = scenario_doc::find_scenarios_by_name(&config.scenarios_dir, selected);
        if matches.is_empty() {
            push_contract_error(
                &mut findings,
                &mut failures,
                "scenario_not_found",
                Some(selected.to_string()),
                None,
                format!("scenario '{selected}' was not found"),
                Some("use `scenario list` to see available names"),
                "scenario_not_found",
                Some(json!("existing scenario name")),
                Some(json!(selected)),
            );
            Vec::new()
        } else if matches.len() > 1 {
            push_contract_error(
                &mut findings,
                &mut failures,
                "scenario_ambiguous",
                Some(selected.to_string()),
                None,
                format!("scenario '{selected}' is ambiguous"),
                Some("choose a unique scenario name"),
                "scenario_ambiguous",
                Some(json!("single scenario match")),
                Some(json!(matches)),
            );
            Vec::new()
        } else {
            matches
        }
    } else {
        listed
            .iter()
            .filter_map(|entry| entry.get("path").and_then(Value::as_str).map(PathBuf::from))
            .collect()
    };

    for path in paths_to_lint {
        lint_one(&path, &static_index, &mut findings, &mut failures);
    }

    // Abstract templates are excluded from `list_scenarios`, so they never reach
    // `lint_one`. Validate them structurally here when linting the whole tree:
    // templates may be intentionally incomplete, so only their `extends`/`vars`
    // blocks are checked and GQA rules are skipped.
    if options.scenario.is_none() {
        for path in scenario_doc::list_abstract_scenario_files(&config.scenarios_dir) {
            lint_template(&path, &mut findings, &mut failures);
        }
    }

    build_result(findings, failures, options.strict)
}

fn lint_template(path: &Path, findings: &mut Vec<LintFinding>, failures: &mut Vec<FailureDetail>) {
    let value = match scenario_doc::parse_scenario_file(path) {
        Ok(value) => value,
        Err(error) => {
            let failure = error.failure;
            push_contract_error(
                findings,
                failures,
                "invalid_scenario_file",
                None,
                Some(path),
                failure
                    .message
                    .clone()
                    .unwrap_or_else(|| "failed to load scenario template".to_string()),
                Some("fix the scenario template syntax/format"),
                &failure.kind,
                failure.expected.clone(),
                failure.actual.clone(),
            );
            return;
        }
    };
    if let Err(error) = crate::template::validate_template_structure(&value, path) {
        let failure = error.failure;
        push_contract_error(
            findings,
            failures,
            "invalid_scenario_file",
            value
                .get("name")
                .and_then(Value::as_str)
                .map(str::to_string),
            Some(path),
            failure
                .message
                .clone()
                .unwrap_or_else(|| "invalid scenario template".to_string()),
            Some("fix the template's extends/vars block"),
            &failure.kind,
            failure.expected.clone(),
            failure.actual.clone(),
        );
    }
}

fn lint_one(
    path: &Path,
    static_index: &StaticProjectIndex,
    findings: &mut Vec<LintFinding>,
    failures: &mut Vec<FailureDetail>,
) {
    let loaded = match scenario_doc::load_scenario_document(path) {
        Ok(v) => v,
        Err(error) => {
            let failure = error.failure;
            push_contract_error(
                findings,
                failures,
                "invalid_scenario_file",
                None,
                Some(path),
                failure
                    .message
                    .clone()
                    .unwrap_or_else(|| "failed to load scenario".to_string()),
                Some("fix the scenario syntax/format"),
                &failure.kind,
                failure.expected.clone(),
                failure.actual.clone(),
            );
            return;
        }
    };
    if let Err(error) = scenario_doc::validate_scenario_document(&loaded, path) {
        let failure = error.failure;
        push_contract_error(
            findings,
            failures,
            "invalid_scenario_file",
            loaded
                .get("name")
                .and_then(Value::as_str)
                .map(str::to_string),
            Some(path),
            failure
                .message
                .clone()
                .unwrap_or_else(|| "failed to validate scenario".to_string()),
            Some("fix the validation error in the scenario"),
            &failure.kind,
            failure.expected.clone(),
            failure.actual.clone(),
        );
        return;
    }

    let normalized = scenario_doc::normalize_scenario_document(&loaded);
    if normalized != loaded {
        findings.push(LintFinding {
            rule_id: "scenario_document_normalized".to_string(),
            severity: "warning".to_string(),
            scenario: loaded
                .get("name")
                .and_then(Value::as_str)
                .map(str::to_string),
            scenario_path: Some(path.to_string_lossy().into_owned()),
            target: None,
            step_index: None,
            assertion_index: None,
            reason: "scenario contains fields or values that are normalized or dropped".to_string(),
            suggested_fix: Some(
                "align the file with normalized scenario shape to avoid hidden drift".to_string(),
            ),
            suppressible: true,
            suppression: None,
        });
    }

    let mut lint_doc = normalized.clone();
    if let Some(lint_meta) = loaded.get("lint")
        && let Some(doc) = lint_doc.as_object_mut()
    {
        doc.insert("lint".to_string(), lint_meta.clone());
    }

    let suppressions = parse_suppressions(path, &loaded, findings, failures);
    lint_fixture_masking(path, &lint_doc, findings);
    lint_debug_only_assertions(path, &lint_doc, findings);
    lint_visual_capture(path, &lint_doc, findings);
    lint_missing_negative_assertion(path, &lint_doc, findings);
    lint_screenshot_obstruction_risk(path, &lint_doc, findings);
    lint_high_cardinality_qa_locators(path, &lint_doc, findings);
    lint_metadata_state_assertions(path, &lint_doc, findings);
    lint_project_flag_contract(path, &lint_doc, findings);
    lint_missing_references(path, &loaded, static_index, findings);
    apply_suppressions(findings, &suppressions, path);
}

fn build_static_project_index(config: &ProjectConfig) -> StaticProjectIndex {
    let mut idx = StaticProjectIndex::default();
    for entry in scenario_doc::list_scenarios(&config.scenarios_dir) {
        if let (Some(name), Some(path)) = (
            entry.get("name").and_then(Value::as_str),
            entry.get("path").and_then(Value::as_str),
        ) {
            idx.scenario_names
                .entry(name.to_string())
                .or_default()
                .push(path.to_string());
        }
    }
    collect_static_qa_ids(&config.project_root, &mut idx.qa_ids);
    idx
}
fn collect_static_qa_ids(root: &Path, qa_ids: &mut HashSet<String>) {
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                let rel = path
                    .strip_prefix(root)
                    .ok()
                    .and_then(|p| p.to_str())
                    .unwrap_or("");
                if rel.starts_with(".godot")
                    || rel.starts_with(".qa")
                    || rel.starts_with("qa/artifacts")
                {
                    continue;
                }
                stack.push(path);
                continue;
            }
            let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
            if !matches!(ext, "gd" | "tscn" | "scn") {
                continue;
            }
            let Ok(text) = fs::read_to_string(&path) else {
                continue;
            };
            for caps in text.match_indices("qa:") {
                let rest = &text[caps.0 + 3..];
                let id: String = rest
                    .chars()
                    .take_while(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '/'))
                    .collect();
                if !id.is_empty() {
                    qa_ids.insert(id);
                }
            }
            for token in [
                "set_meta(\"qa_id\"",
                "set_meta('qa_id'",
                "metadata/qa_id",
                "qa_id",
            ] {
                if text.contains(token) {
                    for quoted in text.split(['\"', '\'']) {
                        if quoted.contains(':')
                            || quoted.is_empty()
                            || quoted == "qa_id"
                            || quoted == "metadata/qa_id"
                        {
                            continue;
                        }
                        if quoted
                            .chars()
                            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '/'))
                            && (quoted.contains('_')
                                || quoted.contains('-')
                                || quoted.contains('/'))
                        {
                            qa_ids.insert(quoted.to_string());
                        }
                    }
                }
            }
        }
    }
}
fn collect_qa_locators(doc: &Value) -> HashSet<String> {
    let mut out = HashSet::new();
    if let Some(steps) = doc.get("steps").and_then(Value::as_array) {
        for step in steps {
            if let Some(s) = step.get("click").and_then(Value::as_str) {
                push_qa_locator(&mut out, s);
            }
            for key in [
                "fixture_teleport",
                "teleport_node",
                "set_property",
                "call_method",
                "spawn_scene",
                "wait_until",
            ] {
                if let Some(o) = step.get(key).and_then(Value::as_object) {
                    for field in ["target", "destination"] {
                        if let Some(s) = o.get(field).and_then(Value::as_str) {
                            push_qa_locator(&mut out, s);
                        }
                    }
                }
            }
        }
    }
    if let Some(asserts) = doc.get("assert").and_then(Value::as_array) {
        for a in asserts {
            if let Some(o) = a.as_object() {
                for key in [
                    "exists",
                    "visible",
                    "hidden",
                    "text_equals",
                    "text_contains",
                    "text_not_contains",
                    "text_matches",
                    "text_not_equals",
                    "visible_pixels",
                    "not_occluded",
                    "renders_above",
                    "renders_below",
                    "within_seconds",
                    "never",
                    "near",
                    "distance_within",
                    "surface_probe",
                    "surface_type",
                    "screenshot_match",
                ] {
                    if let Some(v) = o.get(key) {
                        if let Some(s) = v.as_str() {
                            push_qa_locator(&mut out, s);
                        }
                        if let Some(vo) = v.as_object() {
                            for field in ["target", "source", "other", "from", "to"] {
                                if let Some(s) = vo.get(field).and_then(Value::as_str) {
                                    push_qa_locator(&mut out, s);
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    out
}
fn push_qa_locator(out: &mut HashSet<String>, s: &str) {
    if let Some(id) = s.strip_prefix("qa:") {
        out.insert(id.to_string());
    }
}
fn lint_missing_references(
    path: &Path,
    scenario: &Value,
    idx: &StaticProjectIndex,
    findings: &mut Vec<LintFinding>,
) {
    // NOTE: the obsolete `missing_scenario_reference` rule (which treated a
    // top-level `extends` as a scenario NAME) was removed when scenario
    // templating landed. `extends` is now a base-file path resolved at load
    // time; a missing base fails loading with `scenario_extends_missing`.
    let mut spawned = HashSet::new();
    if let Some(steps) = scenario.get("steps").and_then(Value::as_array) {
        for step in steps {
            if let Some(spawn) = step.get("spawn_scene").and_then(Value::as_object)
                && let Some(qa_id) = spawn.get("qa_id").and_then(Value::as_str)
            {
                spawned.insert(qa_id.to_string());
            }
        }
    }
    for locator in collect_qa_locators(scenario) {
        if idx.qa_ids.contains(&locator) || spawned.contains(&locator) {
            continue;
        }
        findings.push(LintFinding {
            rule_id: "missing_qa_id_reference".to_string(),
            severity: "warning".to_string(),
            scenario: scenario
                .get("name")
                .and_then(Value::as_str)
                .map(str::to_string),
            scenario_path: Some(path.to_string_lossy().into_owned()),
            target: Some(format!("qa:{locator}")),
            step_index: None,
            assertion_index: None,
            reason: format!("qa locator 'qa:{locator}' was not found in static project sources"),
            suggested_fix: Some(
                "add/set the qa_id in scene/script metadata or update the locator".to_string(),
            ),
            suppressible: true,
            suppression: None,
        });
    }
}

fn parse_suppressions(
    path: &Path,
    loaded: &Value,
    findings: &mut Vec<LintFinding>,
    failures: &mut Vec<FailureDetail>,
) -> Vec<LintSuppression> {
    let Some(suppressions_value) = loaded
        .get("lint")
        .and_then(Value::as_object)
        .and_then(|lint| lint.get("suppressions"))
    else {
        return Vec::new();
    };
    let Some(suppressions_obj) = suppressions_value.as_object() else {
        malformed_suppression(
            findings,
            failures,
            path,
            loaded
                .get("name")
                .and_then(Value::as_str)
                .map(str::to_string),
            None,
            "lint.suppressions must be an object keyed by rule id".to_string(),
        );
        return Vec::new();
    };
    let mut parsed = Vec::new();
    for (rule_id, payload) in suppressions_obj {
        if !is_known_rule(rule_id) {
            malformed_suppression(
                findings,
                failures,
                path,
                loaded
                    .get("name")
                    .and_then(Value::as_str)
                    .map(str::to_string),
                Some(rule_id.clone()),
                format!("unknown suppression rule id '{rule_id}'"),
            );
            continue;
        }
        if !is_rule_suppressible(rule_id) {
            malformed_suppression(
                findings,
                failures,
                path,
                loaded
                    .get("name")
                    .and_then(Value::as_str)
                    .map(str::to_string),
                Some(rule_id.clone()),
                format!("rule '{rule_id}' is not suppressible"),
            );
            continue;
        }
        let (reason, target) = if let Some(reason) = payload.as_str() {
            (reason.trim().to_string(), None)
        } else if let Some(obj) = payload.as_object() {
            let reason = obj
                .get("reason")
                .and_then(Value::as_str)
                .unwrap_or("")
                .trim()
                .to_string();
            let target = obj
                .get("target")
                .and_then(Value::as_str)
                .map(str::to_string);
            (reason, target)
        } else {
            malformed_suppression(findings, failures, path, loaded.get("name").and_then(Value::as_str).map(str::to_string), Some(rule_id.clone()), "suppression payload must be an object with reason/target or a non-empty reason string".to_string());
            continue;
        };
        if reason.is_empty() {
            malformed_suppression(
                findings,
                failures,
                path,
                loaded
                    .get("name")
                    .and_then(Value::as_str)
                    .map(str::to_string),
                Some(rule_id.clone()),
                "suppression reason must be non-empty".to_string(),
            );
            continue;
        }
        parsed.push(LintSuppression {
            rule_id: rule_id.clone(),
            reason,
            target,
        });
    }
    parsed
}

fn is_known_rule(rule_id: &str) -> bool {
    matches!(
        rule_id,
        RULE_FIXTURE_MASKING
            | RULE_DEBUG_ONLY_ASSERTIONS
            | RULE_MISSING_VISUAL_CAPTURE
            | RULE_MISSING_NEGATIVE_ASSERTION
            | RULE_SCREENSHOT_OBSTRUCTION_RISK
            | RULE_HIGH_CARDINALITY_QA_LOCATOR
            | RULE_METADATA_STATE_ASSERTION
            | RULE_PROJECT_FLAG_WITHOUT_PROJECT_ARGS
            | "scenario_document_normalized"
            | "duplicate_scenario_name"
            | "scenario_not_found"
            | "scenario_ambiguous"
            | "invalid_scenario_file"
            | "project_missing"
            | "scenarios_dir_missing"
            | "missing_qa_id_reference"
    )
}
fn is_rule_suppressible(rule_id: &str) -> bool {
    matches!(
        rule_id,
        RULE_FIXTURE_MASKING
            | RULE_DEBUG_ONLY_ASSERTIONS
            | RULE_MISSING_VISUAL_CAPTURE
            | RULE_MISSING_NEGATIVE_ASSERTION
            | RULE_SCREENSHOT_OBSTRUCTION_RISK
            | RULE_HIGH_CARDINALITY_QA_LOCATOR
            | RULE_METADATA_STATE_ASSERTION
            | RULE_PROJECT_FLAG_WITHOUT_PROJECT_ARGS
            | "scenario_document_normalized"
            | "missing_qa_id_reference"
    )
}

fn apply_suppressions(
    findings: &mut Vec<LintFinding>,
    suppressions: &[LintSuppression],
    declaring_path: &Path,
) {
    let declaring_path = declaring_path.to_string_lossy();
    let mut kept = Vec::new();
    let mut suppressed = Vec::new();
    for mut finding in findings.drain(..) {
        let matched = if finding.severity == "warning" {
            suppressions.iter().find(|s| {
                finding.scenario_path.as_deref() == Some(declaring_path.as_ref())
                    && s.rule_id == finding.rule_id
                    && (s.target.is_none() || s.target == finding.target)
            })
        } else {
            None
        };
        if let Some(s) = matched {
            let suppression = json!({"reason": s.reason, "target": s.target});
            finding.suppression = Some(suppression);
            suppressed.push(finding);
        } else {
            kept.push(finding);
        }
    }
    kept.extend(suppressed.iter().cloned());
    *findings = kept;
}
fn malformed_suppression(
    findings: &mut Vec<LintFinding>,
    failures: &mut Vec<FailureDetail>,
    path: &Path,
    scenario: Option<String>,
    target: Option<String>,
    reason: String,
) {
    findings.push(LintFinding {
        rule_id: RULE_MALFORMED_SUPPRESSION.to_string(),
        severity: "error".to_string(),
        scenario,
        scenario_path: Some(path.to_string_lossy().into_owned()),
        target,
        step_index: None,
        assertion_index: None,
        reason: reason.clone(),
        suggested_fix: Some(
            "use lint.suppressions.<rule_id>: { reason: <non-empty>, target?: <finding target> }"
                .to_string(),
        ),
        suppressible: false,
        suppression: None,
    });
    failures.push(FailureDetail {
        kind: "scenario_validation_failed".to_string(),
        command: Some(COMMAND.to_string()),
        path: Some(path.to_string_lossy().into_owned()),
        message: Some(reason),
        severity: Some("error".to_string()),
        ..FailureDetail::default()
    });
}

fn lint_fixture_masking(path: &Path, scenario: &Value, findings: &mut Vec<LintFinding>) {
    let steps = scenario
        .get("steps")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let assertions = scenario
        .get("assert")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    if steps.is_empty() || assertions.is_empty() {
        return;
    }
    let interactive_index = steps.iter().position(is_player_input_step);
    let assertion_targets: Vec<String> = assertions.iter().filter_map(assertion_target).collect();
    for (idx, step) in steps.iter().enumerate() {
        if let Some(mutation_target) = mutation_step_target(step) {
            let after_input = interactive_index.map(|i| idx > i).unwrap_or(false);
            let in_final_window = idx + 1 >= steps.len();
            let same_target = assertion_targets.iter().any(|t| t == &mutation_target);
            if after_input || (in_final_window && same_target) {
                findings.push(rule_warning(path, scenario, RULE_FIXTURE_MASKING, Some(format!("step[{idx}]")), Some(idx), None, format!("fixture/state mutation step '{}' can mask player-driven behavior", mutation_step_name(step).unwrap_or("mutation")), "prefer player-facing input for behavior under test; keep fixtures before interactive flow"));
            }
        }
    }
}
fn lint_debug_only_assertions(path: &Path, scenario: &Value, findings: &mut Vec<LintFinding>) {
    if !is_interactive_scenario(scenario) {
        return;
    }
    let assertions = scenario
        .get("assert")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    if assertions.is_empty() {
        return;
    }
    let filtered: Vec<(usize, &Value)> = assertions
        .iter()
        .enumerate()
        .filter(|(_, a)| !a.get("no_errors").and_then(Value::as_bool).unwrap_or(false))
        .collect();
    if filtered.is_empty() {
        return;
    }
    if filtered.iter().all(|(_, a)| is_debug_text_assertion(a)) {
        findings.push(rule_warning(path, scenario, RULE_DEBUG_ONLY_ASSERTIONS, None, None, None, "interactive scenario only asserts debug/status text tokens".to_string(), "add at least one player-visible state assertion (visibility, focus, occlusion, ordering, or geometry)"));
    }
}
fn lint_visual_capture(path: &Path, scenario: &Value, findings: &mut Vec<LintFinding>) {
    let has_snapshot_steps = scenario
        .get("steps")
        .and_then(Value::as_array)
        .map(|s| s.iter().any(|v| v.get("snapshot").is_some()))
        .unwrap_or(false);
    if !has_snapshot_steps || !claims_visual_coverage(scenario) {
        return;
    }
    let save = scenario
        .get("snapshots")
        .and_then(Value::as_object)
        .and_then(|s| s.get("save_screenshot"))
        .and_then(Value::as_bool)
        .unwrap_or(false);
    if !save {
        findings.push(rule_warning(
            path,
            scenario,
            RULE_MISSING_VISUAL_CAPTURE,
            Some("snapshots.save_screenshot".to_string()),
            None,
            None,
            "scenario claims visual coverage but snapshot steps do not save screenshots"
                .to_string(),
            "set snapshots.save_screenshot: true for scenarios asserting visual outcomes",
        ));
    }
}
fn lint_missing_negative_assertion(path: &Path, scenario: &Value, findings: &mut Vec<LintFinding>) {
    if !is_interactive_scenario(scenario) {
        return;
    }
    let assertions = scenario
        .get("assert")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let has_negative = assertions.iter().any(is_negative_assertion);
    if !has_negative {
        findings.push(rule_warning(path, scenario, RULE_MISSING_NEGATIVE_ASSERTION, None, None, None, "interactive scenario has no negative guard assertion".to_string(), "add a negative check such as hidden/not_exists/disabled/never/no_overlap/not_occluded/no_errors"));
    }
}
fn lint_screenshot_obstruction_risk(
    path: &Path,
    scenario: &Value,
    findings: &mut Vec<LintFinding>,
) {
    if !claims_visual_coverage(scenario) {
        return;
    }
    let save = scenario
        .get("snapshots")
        .and_then(Value::as_object)
        .and_then(|s| s.get("save_screenshot"))
        .and_then(Value::as_bool)
        .unwrap_or(false);
    if !save {
        return;
    }
    let assertions = scenario
        .get("assert")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let has_guard = assertions.iter().any(|a| {
        a.get("visible_pixels").is_some()
            || a.get("not_occluded").is_some()
            || a.get("renders_above").is_some()
            || a.get("renders_below").is_some()
    });
    if !has_guard {
        findings.push(rule_warning(path, scenario, RULE_SCREENSHOT_OBSTRUCTION_RISK, None, None, None, "visual screenshots are saved without occlusion/pixel/order guards".to_string(), "add visible_pixels, not_occluded, renders_above, or renders_below for the important target"));
    }
}

fn lint_high_cardinality_qa_locators(
    path: &Path,
    scenario: &Value,
    findings: &mut Vec<LintFinding>,
) {
    for locator in collect_qa_locators(scenario) {
        if !looks_like_high_cardinality_locator(&locator) {
            continue;
        }
        findings.push(rule_warning(
            path,
            scenario,
            RULE_HIGH_CARDINALITY_QA_LOCATOR,
            Some(format!("qa:{locator}")),
            None,
            None,
            format!("scenario targets generated or high-cardinality locator 'qa:{locator}'"),
            "replace per-cell/per-object locators with durable anchors plus to_point, state_matches selectors, or QA probes",
        ));
    }
}

fn lint_metadata_state_assertions(path: &Path, scenario: &Value, findings: &mut Vec<LintFinding>) {
    for assertion in collect_assertions(scenario) {
        let Some(state_matches) = assertion.get("state_matches").and_then(Value::as_object) else {
            continue;
        };
        if state_matches.get("source").and_then(Value::as_str) != Some("metadata") {
            continue;
        }
        let target = state_matches
            .get("target")
            .and_then(Value::as_str)
            .unwrap_or("state_matches");
        findings.push(rule_warning(
            path,
            scenario,
            RULE_METADATA_STATE_ASSERTION,
            Some(target.to_string()),
            None,
            None,
            "state_matches reads metadata as dynamic state".to_string(),
            "prefer source: method with get_qa_*_state()/get_*_qa_state(), keeping metadata for static identity only",
        ));
    }
}

fn lint_project_flag_contract(path: &Path, scenario: &Value, findings: &mut Vec<LintFinding>) {
    if !claims_project_flag_coverage(scenario) {
        return;
    }
    if has_non_empty_project_args(scenario) || has_equivalent_project_args_fixture(scenario) {
        return;
    }
    findings.push(rule_warning(
        path,
        scenario,
        RULE_PROJECT_FLAG_WITHOUT_PROJECT_ARGS,
        Some("lint.validates_project_args".to_string()),
        None,
        None,
        "scenario claims project-flag validation but does not provide project_args or documented equivalent fixture metadata".to_string(),
        "add non-empty project_args or document the equivalent fixture with lint.project_args_fixture/lint.equivalent_project_args_fixture",
    ));
}

fn claims_project_flag_coverage(scenario: &Value) -> bool {
    scenario
        .get("lint")
        .and_then(Value::as_object)
        .map(|lint| {
            lint.get("validates_project_flag")
                .and_then(Value::as_bool)
                .unwrap_or(false)
                || lint
                    .get("validates_project_args")
                    .and_then(Value::as_bool)
                    .unwrap_or(false)
        })
        .unwrap_or(false)
}

fn has_non_empty_project_args(scenario: &Value) -> bool {
    scenario
        .get("project_args")
        .and_then(Value::as_array)
        .map(|args| !args.is_empty())
        .unwrap_or(false)
}

fn has_equivalent_project_args_fixture(scenario: &Value) -> bool {
    scenario
        .get("lint")
        .and_then(Value::as_object)
        .map(|lint| {
            lint.get("project_args_fixture")
                .is_some_and(is_non_empty_fixture_marker)
                || lint
                    .get("equivalent_project_args_fixture")
                    .is_some_and(is_non_empty_fixture_marker)
        })
        .unwrap_or(false)
}

fn is_non_empty_fixture_marker(value: &Value) -> bool {
    match value {
        Value::Bool(true) => true,
        Value::String(s) => !s.trim().is_empty(),
        Value::Array(values) => !values.is_empty(),
        Value::Object(map) => !map.is_empty(),
        _ => false,
    }
}

fn looks_like_high_cardinality_locator(locator: &str) -> bool {
    let lower = locator.to_ascii_lowercase();
    lower.contains("_floor_")
        || lower.contains("_module_cell_")
        || lower.contains("_preview_cell_")
        || lower.ends_with("_extraction_area")
        || lower.ends_with("_landing_area")
        || lower.ends_with("_spawn")
        || lower.ends_with("_station")
        || lower.contains("_water_boat_affordance_pixels")
}

fn collect_assertions(scenario: &Value) -> Vec<&Value> {
    let mut assertions = Vec::new();
    if let Some(top_level) = scenario.get("assert").and_then(Value::as_array) {
        assertions.extend(top_level.iter());
    }
    if let Some(steps) = scenario.get("steps").and_then(Value::as_array) {
        for step in steps {
            if let Some(step_assertions) = step.get("assert").and_then(Value::as_array) {
                assertions.extend(step_assertions.iter());
            }
        }
    }
    assertions
}

fn is_interactive_scenario(s: &Value) -> bool {
    s.get("steps")
        .and_then(Value::as_array)
        .map(|steps| steps.iter().any(is_player_input_step))
        .unwrap_or(false)
        || s.get("lint")
            .and_then(Value::as_object)
            .and_then(|m| m.get("interactive"))
            .and_then(Value::as_bool)
            .unwrap_or(false)
}
fn is_player_input_step(step: &Value) -> bool {
    step.get("click").is_some()
        || step.get("tap_action").is_some()
        || step.get("hold_action").is_some()
        || step.get("release_action").is_some()
        || step.get("input_joy_button").is_some()
        || step.get("input_joy_motion").is_some()
        // input_joy_state emits an open-loop batch of per-device joypad input
        // (concurrent holds) in one frame; it is player-facing input, same as
        // the other joypad steps.
        || step.get("input_joy_state").is_some()
        // drive_policy emits real per-device joypad input computed by a
        // registered policy each tick; it is player-facing input, not a
        // fixture/state mutation, so GQA-LINT-001 must not flag it.
        || step.get("drive_policy").is_some()
    // NOTE: joy_connect/joy_disconnect are deliberately NOT player input. They
    // simulate joypad hardware CONNECTION state (Input.joy_connection_changed) —
    // scenario setup, like spawn_scene/fixture_call — not a button/axis press. So
    // they neither count as the interactive marker here nor as a mutation below.
    // This is intentional: joy_disconnect mid-scenario (after input) is the
    // canonical controller-drop use case and must not be flagged by GQA-LINT-001.
}
fn mutation_step_name(step: &Value) -> Option<&str> {
    // joy_connect/joy_disconnect are intentionally absent: treating them as
    // mutations would flag the canonical joy_disconnect-after-input (mid-match
    // controller-drop) scenario. They are neutral setup, like spawn_scene.
    [
        "fixture_teleport",
        "teleport_node",
        "set_property",
        "call_method",
    ]
    .into_iter()
    .find(|k| step.get(*k).is_some())
}
fn mutation_step_target(step: &Value) -> Option<String> {
    [
        "fixture_teleport",
        "teleport_node",
        "set_property",
        "call_method",
    ]
    .into_iter()
    .find_map(|name| {
        step.get(name)
            .and_then(Value::as_object)
            .and_then(|o| o.get("target"))
            .and_then(Value::as_str)
            .map(str::to_string)
    })
}
fn assertion_target(assertion: &Value) -> Option<String> {
    let obj = assertion.as_object()?;
    for key in [
        "visible",
        "hidden",
        "disabled",
        "focused",
        "text_equals",
        "text_contains",
        "not_exists",
        "no_overlap",
        "not_occluded",
        "visible_pixels",
        "renders_above",
        "renders_below",
    ] {
        if let Some(v) = obj.get(key) {
            if let Some(t) = v.as_str() {
                return Some(t.to_string());
            }
            if let Some(m) = v.as_object() {
                if let Some(t) = m.get("target").and_then(Value::as_str) {
                    return Some(t.to_string());
                }
            }
        }
    }
    None
}
fn is_debug_text_assertion(a: &Value) -> bool {
    let locator = a
        .get("text_equals")
        .or_else(|| a.get("text_contains"))
        .and_then(Value::as_object)
        .and_then(|m| m.get("target"))
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_lowercase();
    let expected = a
        .get("text_equals")
        .or_else(|| a.get("text_contains"))
        .and_then(Value::as_object)
        .and_then(|m| m.get("expected"))
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_lowercase();
    (a.get("text_equals").is_some() || a.get("text_contains").is_some())
        && ["debug", "fixture", "qa_", "token", "state"]
            .iter()
            .any(|t| locator.contains(t) || expected.contains(t))
}
fn claims_visual_coverage(s: &Value) -> bool {
    s.get("lint")
        .and_then(Value::as_object)
        .and_then(|m| m.get("visual_coverage"))
        .and_then(Value::as_bool)
        .unwrap_or(false)
        || s.get("assert")
            .and_then(Value::as_array)
            .map(|arr| {
                arr.iter().any(|a| {
                    a.get("layout_audit").is_some()
                        || a.get("screenshot_match").is_some()
                        || a.get("visible_pixels").is_some()
                        || a.get("not_occluded").is_some()
                        || a.get("renders_above").is_some()
                        || a.get("renders_below").is_some()
                })
            })
            .unwrap_or(false)
}
fn is_negative_assertion(a: &Value) -> bool {
    a.get("not_exists").is_some()
        || a.get("hidden").is_some()
        || a.get("disabled").is_some()
        || a.get("never").is_some()
        || a.get("no_overlap").is_some()
        || a.get("not_occluded").is_some()
        || a.get("no_errors").is_some()
}

fn rule_warning(
    path: &Path,
    scenario: &Value,
    rule_id: &str,
    target: Option<String>,
    step_index: Option<usize>,
    assertion_index: Option<usize>,
    reason: String,
    suggested_fix: &str,
) -> LintFinding {
    LintFinding {
        rule_id: rule_id.to_string(),
        severity: "warning".to_string(),
        scenario: scenario
            .get("name")
            .and_then(Value::as_str)
            .map(str::to_string),
        scenario_path: Some(path.to_string_lossy().into_owned()),
        target,
        step_index,
        assertion_index,
        reason,
        suggested_fix: Some(suggested_fix.to_string()),
        suppressible: true,
        suppression: None,
    }
}
fn push_contract_error(
    findings: &mut Vec<LintFinding>,
    failures: &mut Vec<FailureDetail>,
    rule_id: &str,
    scenario: Option<String>,
    scenario_path: Option<&Path>,
    reason: String,
    suggested_fix: Option<&str>,
    failure_kind: &str,
    expected: Option<Value>,
    actual: Option<Value>,
) {
    findings.push(LintFinding {
        rule_id: rule_id.to_string(),
        severity: "error".to_string(),
        scenario,
        scenario_path: scenario_path.map(|p| p.to_string_lossy().into_owned()),
        target: None,
        step_index: None,
        assertion_index: None,
        reason: reason.clone(),
        suggested_fix: suggested_fix.map(str::to_string),
        suppressible: false,
        suppression: None,
    });
    failures.push(FailureDetail {
        kind: failure_kind.to_string(),
        command: Some(COMMAND.to_string()),
        path: scenario_path.map(|p| p.to_string_lossy().into_owned()),
        message: Some(reason),
        severity: Some("error".to_string()),
        expected,
        actual,
        ..FailureDetail::default()
    });
}
fn build_result(
    findings: Vec<LintFinding>,
    failures: Vec<FailureDetail>,
    strict: bool,
) -> CommandResult {
    let suppressed_findings: Vec<LintFinding> = findings
        .iter()
        .filter(|f| f.suppression.is_some())
        .cloned()
        .collect();
    let active_findings: Vec<LintFinding> = findings
        .iter()
        .filter(|f| f.suppression.is_none())
        .cloned()
        .collect();
    let warning_count = active_findings
        .iter()
        .filter(|f| f.severity == "warning")
        .count();
    let error_count = active_findings
        .iter()
        .filter(|f| f.severity == "error")
        .count();
    let ok = error_count == 0 && (!strict || warning_count == 0);
    let exit_code = if ok { 0 } else { 1 };
    let mut data = Map::new();
    data.insert("summary".to_string(), json!({ "total": active_findings.len(), "warnings": warning_count, "errors": error_count, "strict": strict, "suppressed": suppressed_findings.len() }));
    data.insert(
        "findings".to_string(),
        serde_json::to_value(&active_findings).unwrap_or_else(|_| json!([])),
    );
    data.insert(
        "suppressed_findings".to_string(),
        serde_json::to_value(&suppressed_findings).unwrap_or_else(|_| json!([])),
    );
    CommandResult {
        ok,
        command: COMMAND.to_string(),
        message: if ok {
            "scenario lint passed".to_string()
        } else {
            "scenario lint found issues".to_string()
        },
        data,
        failures,
        exit_code,
    }
}
