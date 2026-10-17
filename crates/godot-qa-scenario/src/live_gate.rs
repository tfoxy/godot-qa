use godot_qa_core::config::ProjectConfig;
use godot_qa_core::results::{CommandResult, FailureDetail};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use crate::scenario_doc::is_safe_artifact_segment;

const UNSUPPORTED_KEYS: [&str; 8] = [
    "commands",
    "script",
    "env",
    "if",
    "then",
    "include",
    "bundles",
    "scenarios",
];

#[derive(Debug, Clone, PartialEq)]
pub struct LiveGateManifest {
    pub name: String,
    pub scene: String,
    pub overlay: Option<String>,
    pub required_anchors: Vec<String>,
    pub optional_anchors: Vec<String>,
    pub methods: Vec<String>,
    pub probes: Vec<String>,
    pub method_checks: Vec<LiveGateMethodCheck>,
    pub probe_checks: Vec<LiveGateProbeCheck>,
    pub path: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LiveGateMethodCheck {
    pub anchor: Option<String>,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct LiveGateProbeCheck {
    pub anchor: Option<String>,
    pub name: String,
    pub params: Map<String, Value>,
}

#[derive(Debug, Default)]
pub struct LiveGateListResult {
    pub gates: Vec<LiveGateManifest>,
    pub failures: Vec<FailureDetail>,
}

#[derive(Debug, Clone, Default)]
pub struct LiveGateRunOptions {
    pub strict: bool,
    pub attach: bool,
    pub scene: Option<String>,
    pub show_overlay: Option<bool>,
    pub viewport: String,
    pub headless: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AnchorDiagnostic {
    pub name: String,
    pub locator: String,
    pub required: bool,
    pub resolved: Option<String>,
    pub status: String,
    pub methods: Vec<Value>,
    pub probes: Vec<Value>,
    pub failures: Vec<Value>,
    pub warnings: Vec<Value>,
}

pub struct LiveGateRunHooks<'a> {
    pub session_start: &'a dyn Fn(&ProjectConfig, Option<&str>, &str, bool) -> CommandResult,
    pub session_stop: &'a dyn Fn(&ProjectConfig) -> CommandResult,
    pub load_session: &'a dyn Fn(&ProjectConfig, &str) -> Result<Value, FailureDetail>,
    pub protocol: &'a dyn Fn(&ProjectConfig, &str, &str, Map<String, Value>) -> CommandResult,
}

impl LiveGateManifest {
    pub fn overlay_summary(&self) -> String {
        self.overlay
            .clone()
            .unwrap_or_else(|| "unspecified".to_string())
    }
}

pub fn live_gates_dir(config: &ProjectConfig) -> PathBuf {
    config.project_root.join("qa/live_gates")
}

pub fn load_live_gate_manifest(path: &Path) -> Result<Value, FailureDetail> {
    let raw_text = fs::read_to_string(path).map_err(|error| FailureDetail {
        kind: "live_gate_parse_failed".to_string(),
        path: Some(path.to_string_lossy().into_owned()),
        message: Some(error.to_string()),
        expected: Some(json!("valid live gate manifest")),
        actual: Some(json!("io_error")),
        ..FailureDetail::default()
    })?;

    let data = match path.extension().and_then(|ext| ext.to_str()) {
        Some("json") => {
            serde_json::from_str::<Value>(&raw_text).map_err(|error| FailureDetail {
                kind: "live_gate_parse_failed".to_string(),
                path: Some(path.to_string_lossy().into_owned()),
                message: Some(error.to_string()),
                expected: Some(json!("valid live gate manifest")),
                actual: Some(json!("JSONDecodeError")),
                ..FailureDetail::default()
            })?
        }
        _ => {
            godot_qa_core::yaml::parse_yaml_document(&raw_text).map_err(|error| FailureDetail {
                kind: "live_gate_parse_failed".to_string(),
                path: Some(path.to_string_lossy().into_owned()),
                message: Some(error.detail()),
                expected: Some(json!("valid live gate manifest")),
                actual: Some(json!(error.location_summary())),
                ..FailureDetail::default()
            })?
        }
    };

    if !data.is_object() {
        return Err(FailureDetail {
            kind: "live_gate_parse_failed".to_string(),
            path: Some(path.to_string_lossy().into_owned()),
            message: Some("live gate manifest root must be an object".to_string()),
            expected: Some(json!("object")),
            actual: Some(json!(json_type_name(&data))),
            ..FailureDetail::default()
        });
    }

    Ok(data)
}

pub fn validate_live_gate_manifest(data: &Value, path: &Path) -> Result<(), FailureDetail> {
    let Some(document) = data.as_object() else {
        return Err(validation_failure(
            path,
            "live gate manifest root must be an object",
            json!("object"),
            json!(json_type_name(data)),
        ));
    };

    for key in UNSUPPORTED_KEYS {
        if document.contains_key(key) {
            return Err(FailureDetail {
                kind: "live_gate_unsupported_key".to_string(),
                path: Some(path.to_string_lossy().into_owned()),
                message: Some(format!("live gate manifest field '{key}' is not supported")),
                expected: Some(json!("declarative live gate manifest fields only")),
                actual: Some(json!(key)),
                ..FailureDetail::default()
            });
        }
    }

    for key in document.keys() {
        if !matches!(
            key.as_str(),
            "name" | "scene" | "overlay" | "anchors" | "methods" | "probes"
        ) {
            return Err(FailureDetail {
                kind: "live_gate_unsupported_key".to_string(),
                path: Some(path.to_string_lossy().into_owned()),
                message: Some(format!("live gate manifest field '{key}' is not supported")),
                expected: Some(json!("name, scene, overlay, anchors, methods, or probes")),
                actual: Some(json!(key)),
                ..FailureDetail::default()
            });
        }
    }

    let name = require_non_empty_string(document.get("name"), path, "name")?;
    if !is_safe_artifact_segment(name.trim()) {
        return Err(validation_failure(
            path,
            "live gate field 'name' must be a safe artifact path segment",
            json!("letters, numbers, dot, underscore, or hyphen"),
            json!(name),
        ));
    }

    require_non_empty_string(document.get("scene"), path, "scene")?;
    require_overlay(document.get("overlay"), path)?;
    let anchors = document.get("anchors");
    require_anchor_refs(anchors, path, "required")?;
    require_anchor_refs(anchors, path, "optional")?;
    collect_method_checks(document.get("methods"), path)?;
    collect_probe_checks(document.get("probes"), path)?;
    Ok(())
}

pub fn list_live_gates(config: &ProjectConfig) -> LiveGateListResult {
    let mut paths = Vec::new();
    collect_manifest_paths(&live_gates_dir(config), &mut paths);

    let mut gates = Vec::new();
    let mut failures = Vec::new();

    for path in paths {
        match load_live_gate_manifest(&path).and_then(|value| {
            validate_live_gate_manifest(&value, &path)?;
            live_gate_from_value(&value, &path)
        }) {
            Ok(gate) => gates.push(gate),
            Err(failure) => failures.push(failure),
        }
    }

    gates.sort_by(|a, b| a.name.cmp(&b.name).then_with(|| a.path.cmp(&b.path)));

    let mut seen: HashMap<String, PathBuf> = HashMap::new();
    for gate in &gates {
        if let Some(original_path) = seen.get(&gate.name) {
            failures.push(FailureDetail {
                kind: "duplicate_live_gate_name".to_string(),
                path: Some(gate.path.to_string_lossy().into_owned()),
                message: Some(format!("live gate name '{}' is duplicated", gate.name)),
                expected: Some(json!("unique live gate names")),
                actual: Some(json!({"name": gate.name, "first_path": original_path, "duplicate_path": gate.path})),
                ..FailureDetail::default()
            });
        } else {
            seen.insert(gate.name.clone(), gate.path.clone());
        }
    }

    LiveGateListResult { gates, failures }
}

pub fn run_live_gate(
    config: &ProjectConfig,
    name: &str,
    options: LiveGateRunOptions,
) -> CommandResult {
    let hooks = LiveGateRunHooks {
        session_start: &|config, scene, viewport, headless| {
            // Live gate keeps the pre-audio-axis behavior: real audio only when
            // windowed (`--headless` already forces the Dummy driver).
            godot_qa_runtime::session_start_result(
                config,
                scene,
                viewport,
                headless,
                !headless,
                &[],
                None,
                false,
            )
        },
        session_stop: &|config| godot_qa_runtime::session_stop_result(config),
        load_session: &|config, command| {
            godot_qa_runtime::load_session_record(config, command)
                .map(|record| record.to_json_value())
        },
        protocol: &|config, command, protocol_command, params| {
            godot_qa_runtime::run_live_protocol_command(config, command, protocol_command, params)
        },
    };
    run_live_gate_with_hooks(config, name, options, &hooks)
}

pub fn run_live_gate_with_hooks(
    config: &ProjectConfig,
    name: &str,
    options: LiveGateRunOptions,
    hooks: &LiveGateRunHooks<'_>,
) -> CommandResult {
    let command = "live-gate run";
    let mut preflight_failures = Vec::new();
    if let Some(mut failure) = config.project_config_failure_for_command(command) {
        failure.command = Some(command.to_string());
        preflight_failures.push(failure);
    }
    if !config.addon_dir.is_dir() && !config.repo_addon_dir.is_dir() {
        preflight_failures.push(FailureDetail {
            kind: "missing_addon".to_string(),
            command: Some(command.to_string()),
            path: Some(config.addon_dir.to_string_lossy().into_owned()),
            message: Some("Godot QA addon directory was not found".to_string()),
            expected: Some(json!(
                "addons/godot_qa available to the project or repository"
            )),
            actual: Some(json!("missing")),
            ..FailureDetail::default()
        });
    }
    if !options.attach && config.godot_binary.is_none() {
        preflight_failures.push(FailureDetail {
            kind: "missing_godot".to_string(),
            command: Some(command.to_string()),
            message: Some("Godot binary not found in PATH or GODOT_BIN".to_string()),
            expected: Some(json!(
                "discoverable Godot binary via --godot-bin, GODOT_BIN, or PATH"
            )),
            actual: Some(json!("missing")),
            ..FailureDetail::default()
        });
    }
    if !preflight_failures.is_empty() {
        return run_failure(
            command,
            "live gate preflight failed",
            preflight_failures,
            Map::new(),
        );
    }

    let listed = list_live_gates(config);
    if !listed.failures.is_empty() {
        return run_failure(
            command,
            "unable to resolve live gate",
            listed
                .failures
                .into_iter()
                .map(|mut failure| {
                    failure.command.get_or_insert_with(|| command.to_string());
                    failure
                })
                .collect(),
            Map::new(),
        );
    }
    let Some(gate) = listed.gates.into_iter().find(|gate| gate.name == name) else {
        return run_failure(
            command,
            "unable to resolve live gate",
            vec![FailureDetail {
                kind: "live_gate_not_found".to_string(),
                command: Some(command.to_string()),
                path: Some(live_gates_dir(config).to_string_lossy().into_owned()),
                message: Some(format!("live gate '{name}' was not found")),
                expected: Some(json!("named live gate manifest")),
                actual: Some(json!(name)),
                ..FailureDetail::default()
            }],
            Map::new(),
        );
    };

    let scene = options.scene.as_deref().unwrap_or(&gate.scene);
    let mut started = false;
    let session_result = if options.attach {
        match (hooks.load_session)(config, command) {
            Ok(session) => CommandResult {
                ok: true,
                command: command.to_string(),
                message: "attached live session loaded".to_string(),
                data: session.as_object().cloned().unwrap_or_default(),
                failures: Vec::new(),
                exit_code: 0,
            },
            Err(failure) => run_failure(
                command,
                "unable to attach live session",
                vec![failure],
                Map::new(),
            ),
        }
    } else {
        let result =
            (hooks.session_start)(config, Some(scene), &options.viewport, options.headless);
        if result.ok {
            started = true;
        }
        result
    };
    if !session_result.ok {
        return session_result_as_live_gate_failure(command, session_result);
    }

    let mut session = session_result.data.clone();
    let mut failures = Vec::new();
    let mut warnings = Vec::new();
    let mut overlay = verify_overlay(config, &gate, &options, hooks);
    if !overlay.ok {
        failures.extend(overlay.failures.clone());
    }
    if !overlay.warnings.is_empty() {
        warnings.append(&mut overlay.warnings);
    }

    let status = (hooks.protocol)(
        config,
        command,
        godot_qa_runtime::SESSION_STATUS,
        Map::new(),
    );
    if status.ok {
        session.insert("status".to_string(), Value::Object(status.data));
    } else {
        failures.extend(status.failures.iter().map(failure_to_value_failure));
        session.insert("status_failure".to_string(), json!(status.failures));
    }

    let anchors = inspect_anchors(config, &gate, options.strict, hooks);
    for anchor in &anchors {
        if anchor.status == "failed" {
            failures.extend(anchor.failures.iter().map(value_to_failure));
        }
        warnings.extend(anchor.warnings.iter().cloned());
    }

    let errors = (hooks.protocol)(config, command, godot_qa_runtime::ERRORS_READ, Map::new());
    if errors.ok {
        session.insert("errors".to_string(), Value::Object(errors.data.clone()));
        if errors
            .data
            .get("errors")
            .and_then(Value::as_array)
            .map(|items| !items.is_empty())
            .unwrap_or(false)
        {
            failures.push(FailureDetail {
                kind: "runtime_errors_present".to_string(),
                command: Some(command.to_string()),
                message: Some("runtime bridge reported errors during live gate run".to_string()),
                actual: Some(Value::Object(errors.data)),
                ..FailureDetail::default()
            });
        }
    } else {
        failures.extend(errors.failures.iter().map(failure_to_value_failure));
        session.insert("errors_failure".to_string(), json!(errors.failures));
    }

    let ok = failures.is_empty();
    let artifact_result = prepare_live_gate_artifacts(
        config,
        &gate,
        &session,
        &overlay.evidence,
        &anchors,
        &warnings,
        &failures,
        ok,
    );
    let mut artifacts = Map::new();
    match artifact_result {
        Ok(paths) => artifacts = paths,
        Err(mut failure) => {
            failure.command = Some(command.to_string());
            failures.push(failure);
        }
    }

    if started {
        let stop = (hooks.session_stop)(config);
        if !stop.ok {
            failures.extend(stop.failures);
        }
    }

    let ok = failures.is_empty();
    let data = Map::from_iter([
        ("gate".to_string(), json!(gate.name)),
        ("scene".to_string(), json!(scene)),
        ("strict".to_string(), json!(options.strict)),
        ("attached".to_string(), json!(options.attach)),
        ("session".to_string(), Value::Object(session)),
        ("overlay".to_string(), overlay.evidence),
        ("anchors".to_string(), json!(anchors)),
        ("warnings".to_string(), Value::Array(warnings)),
        ("artifacts".to_string(), Value::Object(artifacts)),
    ]);
    CommandResult {
        ok,
        command: command.to_string(),
        message: if ok {
            "live gate passed".to_string()
        } else {
            "live gate failed".to_string()
        },
        data,
        failures,
        exit_code: if ok { 0 } else { 1 },
    }
}

fn inspect_anchors(
    config: &ProjectConfig,
    gate: &LiveGateManifest,
    strict: bool,
    hooks: &LiveGateRunHooks<'_>,
) -> Vec<AnchorDiagnostic> {
    let mut anchors = Vec::new();
    for (name, required) in gate
        .required_anchors
        .iter()
        .map(|name| (name, true))
        .chain(gate.optional_anchors.iter().map(|name| (name, false)))
    {
        let locator = format!("qa:{name}");
        let mut diagnostic = AnchorDiagnostic {
            name: name.clone(),
            locator: locator.clone(),
            required,
            resolved: None,
            status: "passed".to_string(),
            methods: Vec::new(),
            probes: Vec::new(),
            failures: Vec::new(),
            warnings: Vec::new(),
        };
        let inspect = (hooks.protocol)(
            config,
            "live-gate run",
            godot_qa_runtime::NODE_INSPECT,
            Map::from_iter([("target".to_string(), json!(locator))]),
        );
        if inspect.ok {
            diagnostic.resolved = inspect
                .data
                .get("resolved")
                .and_then(Value::as_str)
                .map(str::to_string);
        } else {
            let details: Vec<Value> = inspect.failures.iter().map(failure_to_value).collect();
            if required || strict {
                diagnostic.failures.extend(details);
                diagnostic.status = "failed".to_string();
            } else {
                diagnostic.warnings.extend(details);
                diagnostic.status = "warning".to_string();
                anchors.push(diagnostic);
                continue;
            }
        }
        if diagnostic.status != "failed" {
            for method in gate
                .method_checks
                .iter()
                .filter(|method| check_applies_to_anchor(method.anchor.as_deref(), name))
            {
                let result = (hooks.protocol)(
                    config,
                    "live-gate run",
                    godot_qa_runtime::ASSERT_STATE_MATCHES,
                    Map::from_iter([
                        ("target".to_string(), json!(locator)),
                        ("source".to_string(), json!("method")),
                        ("method".to_string(), json!(method.name)),
                        (
                            "matches".to_string(),
                            json!([{"path": "", "operator": "present"}]),
                        ),
                    ]),
                );
                diagnostic.methods.push(json!({
                    "name": method.name,
                    "status": if result.ok { "passed" } else { "failed" },
                    "data": result.data,
                    "failures": result.failures,
                }));
                if !result.ok {
                    diagnostic
                        .failures
                        .extend(result.failures.iter().map(failure_to_value));
                    diagnostic.status = "failed".to_string();
                }
            }
            for probe in gate
                .probe_checks
                .iter()
                .filter(|probe| check_applies_to_anchor(probe.anchor.as_deref(), name))
            {
                let mut params = probe.params.clone();
                params.insert("target".to_string(), json!(locator));
                params.insert("provider_method".to_string(), json!(probe.name));
                params
                    .entry("points".to_string())
                    .or_insert_with(|| json!([{"x": 120.0, "y": 420.0, "expected": "walkable"}]));
                let result = (hooks.protocol)(
                    config,
                    "live-gate run",
                    godot_qa_runtime::ASSERT_SURFACE_PROBE,
                    params,
                );
                diagnostic.probes.push(json!({
                    "name": probe.name,
                    "status": if result.ok { "passed" } else { "failed" },
                    "data": result.data,
                    "failures": result.failures,
                }));
                if !result.ok {
                    diagnostic
                        .failures
                        .extend(result.failures.iter().map(failure_to_value));
                    diagnostic.status = "failed".to_string();
                }
            }
        }
        anchors.push(diagnostic);
    }
    anchors
}

struct OverlayCheck {
    ok: bool,
    evidence: Value,
    failures: Vec<FailureDetail>,
    warnings: Vec<Value>,
}

fn verify_overlay(
    config: &ProjectConfig,
    gate: &LiveGateManifest,
    options: &LiveGateRunOptions,
    hooks: &LiveGateRunHooks<'_>,
) -> OverlayCheck {
    let required = options
        .show_overlay
        .map(|show| {
            if show {
                "visible".to_string()
            } else {
                "hidden".to_string()
            }
        })
        .or_else(|| gate.overlay.clone());
    let Some(required) = required else {
        return OverlayCheck {
            ok: true,
            evidence: json!({"required": Value::Null, "status": "unspecified"}),
            failures: Vec::new(),
            warnings: Vec::new(),
        };
    };
    if required != "hidden" {
        return OverlayCheck {
            ok: true,
            evidence: json!({"required": required, "status": "not_checked"}),
            failures: Vec::new(),
            warnings: Vec::new(),
        };
    }
    let result = (hooks.protocol)(
        config,
        "live-gate run",
        godot_qa_runtime::ASSERT_STATE_MATCHES,
        Map::from_iter([
            ("target".to_string(), json!("qa:qa_debug_overlay")),
            ("source".to_string(), json!("method")),
            ("method".to_string(), json!("get_hidden_overlay_qa_state")),
            (
                "matches".to_string(),
                json!([
                    {"path": "hidden", "operator": "equals", "expected": true},
                    {"path": "visible_in_tree", "operator": "equals", "expected": false}
                ]),
            ),
        ]),
    );
    if result.ok {
        OverlayCheck {
            ok: true,
            evidence: json!({"required": "hidden", "status": "passed", "data": result.data}),
            failures: Vec::new(),
            warnings: Vec::new(),
        }
    } else {
        let failures = result.failures;
        OverlayCheck {
            ok: false,
            evidence: json!({"required": "hidden", "status": "failed", "failures": failures}),
            failures: failures
                .into_iter()
                .map(|mut failure| {
                    failure.kind = format!("overlay_{}", failure.kind);
                    failure
                })
                .collect(),
            warnings: Vec::new(),
        }
    }
}

pub fn prepare_live_gate_artifacts(
    config: &ProjectConfig,
    gate: &LiveGateManifest,
    session: &Map<String, Value>,
    overlay: &Value,
    anchors: &[AnchorDiagnostic],
    warnings: &[Value],
    failures: &[FailureDetail],
    ok: bool,
) -> Result<Map<String, Value>, FailureDetail> {
    let run_id = unique_run_id();
    let root = config
        .artifacts_dir
        .join("live-gates")
        .join(&gate.name)
        .join(&run_id);
    fs::create_dir_all(&root).map_err(|error| artifact_failure(&root, error))?;
    let meta_path = root.join("meta.json");
    let summary_path = root.join("summary.json");
    let result_path = root.join("result.json");
    let meta = json!({
        "gate": gate.name,
        "manifest": gate.path,
        "scene": gate.scene,
        "run_id": run_id,
        "session": session,
    });
    let summary = json!({
        "ok": ok,
        "gate": gate.name,
        "overlay": overlay,
        "anchor_count": anchors.len(),
        "failed_anchor_count": anchors.iter().filter(|anchor| anchor.status == "failed").count(),
        "warning_count": warnings.len(),
        "failure_count": failures.len(),
    });
    let result = json!({
        "ok": ok,
        "gate": gate.name,
        "overlay": overlay,
        "anchors": anchors,
        "session": session,
        "warnings": warnings,
        "failures": failures,
        "artifacts": {
            "root": root,
            "meta": meta_path,
            "summary": summary_path,
            "result": result_path,
        }
    });
    crate::artifacts::write_json(&meta_path, &meta)
        .map_err(|error| artifact_failure(&meta_path, error))?;
    crate::artifacts::write_json(&summary_path, &summary)
        .map_err(|error| artifact_failure(&summary_path, error))?;
    crate::artifacts::write_json(&result_path, &result)
        .map_err(|error| artifact_failure(&result_path, error))?;
    Ok(Map::from_iter([
        ("root".to_string(), json!(root)),
        ("meta".to_string(), json!(meta_path)),
        ("summary".to_string(), json!(summary_path)),
        ("result".to_string(), json!(result_path)),
    ]))
}

fn run_failure(
    command: &str,
    message: &str,
    failures: Vec<FailureDetail>,
    data: Map<String, Value>,
) -> CommandResult {
    CommandResult {
        ok: false,
        command: command.to_string(),
        message: message.to_string(),
        data,
        failures,
        exit_code: 1,
    }
}

fn session_result_as_live_gate_failure(command: &str, result: CommandResult) -> CommandResult {
    let mut data = Map::new();
    data.insert("session".to_string(), Value::Object(result.data));
    run_failure(
        command,
        "live gate session setup failed",
        result.failures,
        data,
    )
}

fn failure_to_value(failure: &FailureDetail) -> Value {
    serde_json::to_value(failure).unwrap_or_else(|_| json!({"type": failure.kind}))
}

fn failure_to_value_failure(failure: &FailureDetail) -> FailureDetail {
    failure.clone()
}

fn value_to_failure(value: &Value) -> FailureDetail {
    serde_json::from_value(value.clone()).unwrap_or_else(|_| FailureDetail {
        kind: "live_gate_anchor_failed".to_string(),
        command: Some("live-gate run".to_string()),
        actual: Some(value.clone()),
        ..FailureDetail::default()
    })
}

fn artifact_failure(path: &Path, error: std::io::Error) -> FailureDetail {
    FailureDetail {
        kind: "artifact_write_failed".to_string(),
        path: Some(path.to_string_lossy().into_owned()),
        message: Some(error.to_string()),
        expected: Some(json!("writable live gate artifact path")),
        actual: Some(json!("io_error")),
        ..FailureDetail::default()
    }
}

fn unique_run_id() -> String {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or(0);
    format!("run-{nanos}")
}

fn live_gate_from_value(data: &Value, path: &Path) -> Result<LiveGateManifest, FailureDetail> {
    let document = data.as_object().ok_or_else(|| {
        validation_failure(
            path,
            "live gate manifest root must be an object",
            json!("object"),
            json!(json_type_name(data)),
        )
    })?;

    Ok(LiveGateManifest {
        name: require_non_empty_string(document.get("name"), path, "name")?
            .trim()
            .to_string(),
        scene: require_non_empty_string(document.get("scene"), path, "scene")?
            .trim()
            .to_string(),
        overlay: collect_overlay(document.get("overlay"), path)?,
        required_anchors: collect_anchor_refs(document.get("anchors"), path, "required")?,
        optional_anchors: collect_anchor_refs(document.get("anchors"), path, "optional")?,
        methods: collect_check_names(document.get("methods"), path, "methods")?,
        probes: collect_check_names(document.get("probes"), path, "probes")?,
        method_checks: collect_method_checks(document.get("methods"), path)?,
        probe_checks: collect_probe_checks(document.get("probes"), path)?,
        path: path.to_path_buf(),
    })
}

fn check_applies_to_anchor(anchor: Option<&str>, name: &str) -> bool {
    anchor
        .map(|anchor| anchor.trim_start_matches("qa:") == name)
        .unwrap_or(true)
}

fn collect_manifest_paths(dir: &Path, paths: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_manifest_paths(&path, paths);
            continue;
        }
        match path.extension().and_then(|ext| ext.to_str()) {
            Some("json") | Some("yaml") | Some("yml") => paths.push(path),
            _ => {}
        }
    }
}

fn require_non_empty_string<'a>(
    value: Option<&'a Value>,
    path: &Path,
    field: &str,
) -> Result<&'a str, FailureDetail> {
    value
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| {
            validation_failure(
                path,
                &format!("live gate field '{field}' must be a non-empty string"),
                json!("non-empty string"),
                value.cloned().unwrap_or(Value::Null),
            )
        })
}

fn require_overlay(value: Option<&Value>, path: &Path) -> Result<(), FailureDetail> {
    collect_overlay(value, path).map(|_| ())
}

fn collect_overlay(value: Option<&Value>, path: &Path) -> Result<Option<String>, FailureDetail> {
    let Some(value) = value else {
        return Ok(None);
    };
    match value {
        Value::String(text) if !text.trim().is_empty() => Ok(Some(text.trim().to_string())),
        Value::Bool(hidden) => Ok(Some(if *hidden { "hidden" } else { "visible" }.to_string())),
        Value::Object(map) => {
            for key in map.keys() {
                if key != "required" {
                    return Err(FailureDetail {
                        kind: "live_gate_unsupported_key".to_string(),
                        path: Some(path.to_string_lossy().into_owned()),
                        message: Some(format!(
                            "live gate manifest field 'overlay.{key}' is not supported"
                        )),
                        expected: Some(json!("overlay.required only")),
                        actual: Some(json!(key)),
                        ..FailureDetail::default()
                    });
                }
            }
            match map.get("required") {
                Some(Value::String(text)) if !text.trim().is_empty() => {
                    Ok(Some(text.trim().to_string()))
                }
                Some(Value::Bool(hidden)) => {
                    Ok(Some(if *hidden { "hidden" } else { "visible" }.to_string()))
                }
                Some(actual) => Err(validation_failure(
                    path,
                    "live gate field 'overlay.required' must be a non-empty string or bool",
                    json!("non-empty string or bool"),
                    actual.clone(),
                )),
                None => Ok(None),
            }
        }
        _ => Err(validation_failure(
            path,
            "live gate field 'overlay' must be a non-empty string, bool, or object",
            json!("non-empty string, bool, or object"),
            value.clone(),
        )),
    }
}

fn require_anchor_refs(
    anchors: Option<&Value>,
    path: &Path,
    field: &str,
) -> Result<(), FailureDetail> {
    let refs = collect_anchor_refs(anchors, path, field)?;
    for item in refs {
        if !is_safe_artifact_segment(item.trim()) {
            return Err(FailureDetail {
                kind: "unsafe_live_gate_anchor".to_string(),
                path: Some(path.to_string_lossy().into_owned()),
                message: Some(format!(
                    "live gate field 'anchors.{field}' contains an unsafe anchor"
                )),
                expected: Some(json!("letters, numbers, dot, underscore, or hyphen")),
                actual: Some(json!(item)),
                ..FailureDetail::default()
            });
        }
    }
    Ok(())
}

fn collect_anchor_refs(
    anchors: Option<&Value>,
    path: &Path,
    field: &str,
) -> Result<Vec<String>, FailureDetail> {
    let Some(anchors) = anchors else {
        return Ok(Vec::new());
    };
    let Some(map) = anchors.as_object() else {
        return Err(validation_failure(
            path,
            "live gate field 'anchors' must be an object",
            json!("object"),
            json!(json_type_name(anchors)),
        ));
    };
    for key in map.keys() {
        if key != "required" && key != "optional" {
            return Err(FailureDetail {
                kind: "live_gate_unsupported_key".to_string(),
                path: Some(path.to_string_lossy().into_owned()),
                message: Some(format!(
                    "live gate manifest field 'anchors.{key}' is not supported"
                )),
                expected: Some(json!("anchors.required or anchors.optional")),
                actual: Some(json!(key)),
                ..FailureDetail::default()
            });
        }
    }
    collect_string_list(map.get(field), path, &format!("anchors.{field}"))
}

fn collect_check_names(
    value: Option<&Value>,
    path: &Path,
    field: &str,
) -> Result<Vec<String>, FailureDetail> {
    let Some(value) = value else {
        return Ok(Vec::new());
    };
    let Some(items) = value.as_array() else {
        return Err(validation_failure(
            path,
            &format!("live gate field '{field}' must be a list"),
            json!("list"),
            json!(json_type_name(value)),
        ));
    };

    let mut result = Vec::new();
    for item in items {
        match item {
            Value::String(text) if !text.trim().is_empty() => {
                let (_, name) = parse_targeted_check_string(text.trim());
                result.push(name.to_string());
            }
            Value::Object(map) => {
                let name =
                    require_non_empty_string(map.get("name"), path, &format!("{field}.name"))?;
                result.push(name.trim().to_string());
            }
            _ => {
                return Err(validation_failure(
                    path,
                    &format!(
                        "live gate field '{field}' entries must be non-empty strings or objects with name"
                    ),
                    json!("non-empty string or object"),
                    item.clone(),
                ));
            }
        }
    }
    Ok(result)
}

fn collect_method_checks(
    value: Option<&Value>,
    path: &Path,
) -> Result<Vec<LiveGateMethodCheck>, FailureDetail> {
    let Some(value) = value else {
        return Ok(Vec::new());
    };
    let Some(items) = value.as_array() else {
        return Err(validation_failure(
            path,
            "live gate field 'methods' must be a list",
            json!("list"),
            json!(json_type_name(value)),
        ));
    };

    let mut result = Vec::new();
    for item in items {
        match item {
            Value::String(text) if !text.trim().is_empty() => {
                let (anchor, name) = parse_targeted_check_string(text.trim());
                result.push(LiveGateMethodCheck {
                    anchor: anchor.map(str::to_string),
                    name: name.to_string(),
                });
            }
            Value::Object(map) => {
                reject_unknown_check_keys(map, path, "methods", &["name", "anchor"])?;
                result.push(LiveGateMethodCheck {
                    anchor: optional_non_empty_string(map.get("anchor"), path, "methods.anchor")?,
                    name: require_non_empty_string(map.get("name"), path, "methods.name")?
                        .trim()
                        .to_string(),
                });
            }
            _ => {
                return Err(validation_failure(
                    path,
                    "live gate field 'methods' entries must be non-empty strings or objects",
                    json!("non-empty string or object"),
                    item.clone(),
                ));
            }
        }
    }
    Ok(result)
}

fn collect_probe_checks(
    value: Option<&Value>,
    path: &Path,
) -> Result<Vec<LiveGateProbeCheck>, FailureDetail> {
    let Some(value) = value else {
        return Ok(Vec::new());
    };
    let Some(items) = value.as_array() else {
        return Err(validation_failure(
            path,
            "live gate field 'probes' must be a list",
            json!("list"),
            json!(json_type_name(value)),
        ));
    };

    let mut result = Vec::new();
    for item in items {
        match item {
            Value::String(text) if !text.trim().is_empty() => {
                let (anchor, name) = parse_targeted_check_string(text.trim());
                result.push(LiveGateProbeCheck {
                    anchor: anchor.map(str::to_string),
                    name: name.to_string(),
                    params: Map::new(),
                });
            }
            Value::Object(map) => {
                reject_unknown_check_keys(
                    map,
                    path,
                    "probes",
                    &[
                        "name",
                        "anchor",
                        "points",
                        "coordinate_space",
                        "radius",
                        "max_radius",
                        "surfaces",
                    ],
                )?;
                let mut params = Map::new();
                for key in [
                    "points",
                    "coordinate_space",
                    "radius",
                    "max_radius",
                    "surfaces",
                ] {
                    if let Some(value) = map.get(key) {
                        params.insert(key.to_string(), value.clone());
                    }
                }
                result.push(LiveGateProbeCheck {
                    anchor: optional_non_empty_string(map.get("anchor"), path, "probes.anchor")?,
                    name: require_non_empty_string(map.get("name"), path, "probes.name")?
                        .trim()
                        .to_string(),
                    params,
                });
            }
            _ => {
                return Err(validation_failure(
                    path,
                    "live gate field 'probes' entries must be non-empty strings or objects",
                    json!("non-empty string or object"),
                    item.clone(),
                ));
            }
        }
    }
    Ok(result)
}

fn reject_unknown_check_keys(
    map: &Map<String, Value>,
    path: &Path,
    field: &str,
    allowed: &[&str],
) -> Result<(), FailureDetail> {
    for key in map.keys() {
        if !allowed.contains(&key.as_str()) {
            return Err(FailureDetail {
                kind: "live_gate_unsupported_key".to_string(),
                path: Some(path.to_string_lossy().into_owned()),
                message: Some(format!(
                    "live gate manifest field '{field}.{key}' is not supported"
                )),
                expected: Some(json!(allowed)),
                actual: Some(json!(key)),
                ..FailureDetail::default()
            });
        }
    }
    Ok(())
}

fn parse_targeted_check_string(value: &str) -> (Option<&str>, &str) {
    if let Some((anchor, name)) = value.split_once('.') {
        if !anchor.trim().is_empty() && !name.trim().is_empty() {
            return (Some(anchor.trim()), name.trim());
        }
    }
    (None, value)
}

fn optional_non_empty_string(
    value: Option<&Value>,
    path: &Path,
    field: &str,
) -> Result<Option<String>, FailureDetail> {
    let Some(value) = value else {
        return Ok(None);
    };
    let Some(text) = value.as_str().filter(|value| !value.trim().is_empty()) else {
        return Err(validation_failure(
            path,
            &format!("live gate field '{field}' must be a non-empty string"),
            json!("non-empty string"),
            value.clone(),
        ));
    };
    Ok(Some(text.trim().to_string()))
}

fn collect_string_list(
    value: Option<&Value>,
    path: &Path,
    field: &str,
) -> Result<Vec<String>, FailureDetail> {
    let Some(value) = value else {
        return Ok(Vec::new());
    };
    let Some(items) = value.as_array() else {
        return Err(validation_failure(
            path,
            &format!("live gate field '{field}' must be a list"),
            json!("list"),
            json!(json_type_name(value)),
        ));
    };

    let mut result = Vec::new();
    for item in items {
        let Some(text) = item.as_str() else {
            return Err(validation_failure(
                path,
                &format!("live gate field '{field}' entries must be non-empty strings"),
                json!("non-empty string"),
                item.clone(),
            ));
        };
        if text.trim().is_empty() {
            return Err(validation_failure(
                path,
                &format!("live gate field '{field}' entries must be non-empty strings"),
                json!("non-empty string"),
                item.clone(),
            ));
        }
        result.push(text.trim().to_string());
    }
    Ok(result)
}

fn validation_failure(path: &Path, message: &str, expected: Value, actual: Value) -> FailureDetail {
    FailureDetail {
        kind: "live_gate_validation_failed".to_string(),
        path: Some(path.to_string_lossy().into_owned()),
        message: Some(message.to_string()),
        expected: Some(expected),
        actual: Some(actual),
        ..FailureDetail::default()
    }
}

fn json_type_name(value: &Value) -> &'static str {
    match value {
        Value::Null => "NoneType",
        Value::Bool(_) => "bool",
        Value::Number(number) => {
            if number.is_i64() || number.is_u64() {
                "int"
            } else {
                "float"
            }
        }
        Value::String(_) => "str",
        Value::Array(_) => "list",
        Value::Object(_) => "dict",
    }
}
