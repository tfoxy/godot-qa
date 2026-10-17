// Bundle manifests: the on-disk `qa/gates/*.yaml` shape, loading and validating
// one, listing every manifest in a project, and expanding a manifest (plus the
// bundles it includes) into the flat child list a bundle run dispatches.
//
// Included into `bundle.rs`, so every item keeps its `crate::bundle::*` path.

const UNSUPPORTED_KEYS: [&str; 8] = [
    "commands",
    "script",
    "env",
    "if",
    "then",
    "include",
    "bundles",
    "optional_bundles",
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BundleManifest {
    pub name: String,
    pub purpose: Option<String>,
    pub required_bundles: Vec<String>,
    pub required_scenarios: Vec<String>,
    pub optional_scenarios: Vec<String>,
    pub recommended_manual_checks: Vec<String>,
    pub path: PathBuf,
    pub warnings: Vec<String>,
}

#[derive(Debug, Default)]
pub struct BundleListResult {
    pub bundles: Vec<BundleManifest>,
    pub failures: Vec<FailureDetail>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BundleReferenceSource {
    Direct,
    Included,
}

impl BundleReferenceSource {
    fn as_str(&self) -> &'static str {
        match self {
            Self::Direct => "direct",
            Self::Included => "included",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExpandedBundleChild {
    pub scenario: String,
    pub required: bool,
    pub source_bundle: String,
    pub bundle_chain: Vec<String>,
    pub source_kind: BundleReferenceSource,
    pub manifest_path: PathBuf,
}

pub fn load_bundle_manifest(path: &Path) -> Result<Value, FailureDetail> {
    let raw_text = fs::read_to_string(path).map_err(|error| FailureDetail {
        kind: "bundle_parse_failed".to_string(),
        path: Some(path.to_string_lossy().into_owned()),
        message: Some(error.to_string()),
        expected: Some(json!("valid bundle manifest")),
        actual: Some(json!("io_error")),
        ..FailureDetail::default()
    })?;

    let data = match path.extension().and_then(|ext| ext.to_str()) {
        Some("json") => {
            serde_json::from_str::<Value>(&raw_text).map_err(|error| FailureDetail {
                kind: "bundle_parse_failed".to_string(),
                path: Some(path.to_string_lossy().into_owned()),
                message: Some(error.to_string()),
                expected: Some(json!("valid bundle manifest")),
                actual: Some(json!("JSONDecodeError")),
                ..FailureDetail::default()
            })?
        }
        _ => {
            godot_qa_core::yaml::parse_yaml_document(&raw_text).map_err(|error| FailureDetail {
                kind: "bundle_parse_failed".to_string(),
                path: Some(path.to_string_lossy().into_owned()),
                message: Some(error.detail()),
                expected: Some(json!("valid bundle manifest")),
                actual: Some(json!(error.location_summary())),
                ..FailureDetail::default()
            })?
        }
    };

    if !data.is_object() {
        return Err(FailureDetail {
            kind: "bundle_parse_failed".to_string(),
            path: Some(path.to_string_lossy().into_owned()),
            message: Some("bundle manifest root must be an object".to_string()),
            expected: Some(json!("object")),
            actual: Some(json!(python_type_name(&data))),
            ..FailureDetail::default()
        });
    }

    Ok(data)
}

pub fn validate_bundle_manifest(data: &Value, path: &Path) -> Result<(), FailureDetail> {
    let Some(document) = data.as_object() else {
        return Err(validation_failure(
            path,
            "bundle manifest root must be an object",
            json!("object"),
            json!(python_type_name(data)),
        ));
    };

    for key in UNSUPPORTED_KEYS {
        if document.contains_key(key) {
            let kind = if key.contains("bundle") {
                "bundle_recursive_reference_unsupported"
            } else {
                "bundle_unsupported_key"
            };
            return Err(FailureDetail {
                kind: kind.to_string(),
                path: Some(path.to_string_lossy().into_owned()),
                message: Some(format!("bundle manifest field '{key}' is not supported")),
                expected: Some(json!("declarative bundle manifest fields only")),
                actual: Some(json!(key)),
                ..FailureDetail::default()
            });
        }
    }

    let name = require_non_empty_string(document.get("name"), path, "name")?;
    if !is_safe_artifact_segment(name.trim()) {
        return Err(validation_failure(
            path,
            "bundle field 'name' must be a safe artifact path segment",
            json!("letters, numbers, dot, underscore, or hyphen"),
            json!(name),
        ));
    }

    if let Some(value) = document.get("purpose") {
        if !value
            .as_str()
            .map(|s| !s.trim().is_empty())
            .unwrap_or(false)
        {
            return Err(validation_failure(
                path,
                "bundle field 'purpose' must be a non-empty string",
                json!("non-empty string"),
                value.clone(),
            ));
        }
    }

    require_scenario_refs(
        document.get("required_scenarios"),
        path,
        "required_scenarios",
    )?;
    require_bundle_refs(document.get("required_bundles"), path, "required_bundles")?;
    require_scenario_refs(
        document.get("optional_scenarios"),
        path,
        "optional_scenarios",
    )?;
    require_string_list(
        document.get("recommended_manual_checks"),
        path,
        "recommended_manual_checks",
    )?;
    Ok(())
}

pub fn list_bundles(config: &ProjectConfig) -> BundleListResult {
    let mut paths = Vec::new();
    let gates_dir = config.project_root.join("qa/gates");
    collect_manifest_paths(&gates_dir, &mut paths);

    let mut bundles = Vec::new();
    let mut failures = Vec::new();

    for path in paths {
        match load_bundle_manifest(&path).and_then(|value| {
            validate_bundle_manifest(&value, &path)?;
            bundle_from_value(&value, &path)
        }) {
            Ok(bundle) => bundles.push(bundle),
            Err(failure) => failures.push(failure),
        }
    }

    bundles.sort_by(|a, b| a.name.cmp(&b.name).then_with(|| a.path.cmp(&b.path)));

    let mut seen: HashMap<String, PathBuf> = HashMap::new();
    for bundle in &bundles {
        if let Some(original_path) = seen.get(&bundle.name) {
            failures.push(FailureDetail {
                kind: "duplicate_bundle_name".to_string(),
                path: Some(bundle.path.to_string_lossy().into_owned()),
                message: Some(format!("bundle name '{}' is duplicated", bundle.name)),
                expected: Some(json!("unique bundle names")),
                actual: Some(json!({"name": bundle.name, "first_path": original_path, "duplicate_path": bundle.path})),
                ..FailureDetail::default()
            });
        } else {
            seen.insert(bundle.name.clone(), bundle.path.clone());
        }
    }

    BundleListResult { bundles, failures }
}

fn bundle_from_value(data: &Value, path: &Path) -> Result<BundleManifest, FailureDetail> {
    let document = data.as_object().ok_or_else(|| {
        validation_failure(
            path,
            "bundle manifest root must be an object",
            json!("object"),
            json!(python_type_name(data)),
        )
    })?;

    Ok(BundleManifest {
        name: require_non_empty_string(document.get("name"), path, "name")?
            .trim()
            .to_string(),
        purpose: document
            .get("purpose")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string),
        required_scenarios: collect_scenario_refs(
            document.get("required_scenarios"),
            path,
            "required_scenarios",
        )?,
        required_bundles: collect_bundle_refs(
            document.get("required_bundles"),
            path,
            "required_bundles",
        )?,
        optional_scenarios: collect_scenario_refs(
            document.get("optional_scenarios"),
            path,
            "optional_scenarios",
        )?,
        recommended_manual_checks: collect_string_list(
            document.get("recommended_manual_checks"),
            path,
            "recommended_manual_checks",
        )?,
        path: path.to_path_buf(),
        warnings: Vec::new(),
    })
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
                &format!("bundle field '{field}' must be a non-empty string"),
                json!("non-empty string"),
                value.cloned().unwrap_or(Value::Null),
            )
        })
}

fn require_scenario_refs(
    value: Option<&Value>,
    path: &Path,
    field: &str,
) -> Result<(), FailureDetail> {
    let refs = collect_scenario_refs(value, path, field)?;
    for item in refs {
        if !is_safe_artifact_segment(item.trim()) {
            return Err(FailureDetail {
                kind: "unsafe_scenario_reference".to_string(),
                path: Some(path.to_string_lossy().into_owned()),
                message: Some(format!(
                    "bundle field '{field}' contains an unsafe scenario reference"
                )),
                expected: Some(json!("letters, numbers, dot, underscore, or hyphen")),
                actual: Some(json!(item)),
                ..FailureDetail::default()
            });
        }
    }
    Ok(())
}

fn require_bundle_refs(
    value: Option<&Value>,
    path: &Path,
    field: &str,
) -> Result<(), FailureDetail> {
    let refs = collect_bundle_refs(value, path, field)?;
    for item in refs {
        if !is_safe_artifact_segment(item.trim()) {
            return Err(FailureDetail {
                kind: "unsafe_bundle_reference".to_string(),
                path: Some(path.to_string_lossy().into_owned()),
                message: Some(format!(
                    "bundle field '{field}' contains an unsafe bundle reference"
                )),
                expected: Some(json!("letters, numbers, dot, underscore, or hyphen")),
                actual: Some(json!(item)),
                ..FailureDetail::default()
            });
        }
    }
    Ok(())
}

fn collect_scenario_refs(
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
            &format!("bundle field '{field}' must be a list"),
            json!("list"),
            json!(python_type_name(value)),
        ));
    };

    let mut result = Vec::new();
    for item in items {
        let Some(name) = item.as_str() else {
            return Err(validation_failure(
                path,
                &format!("bundle field '{field}' entries must be non-empty strings"),
                json!("non-empty string"),
                item.clone(),
            ));
        };
        let trimmed = name.trim();
        if trimmed.is_empty() {
            return Err(validation_failure(
                path,
                &format!("bundle field '{field}' entries must be non-empty strings"),
                json!("non-empty string"),
                item.clone(),
            ));
        }
        result.push(trimmed.to_string());
    }
    Ok(result)
}

fn collect_bundle_refs(
    value: Option<&Value>,
    path: &Path,
    field: &str,
) -> Result<Vec<String>, FailureDetail> {
    collect_scenario_refs(value, path, field)
}

fn require_string_list(
    value: Option<&Value>,
    path: &Path,
    field: &str,
) -> Result<(), FailureDetail> {
    collect_string_list(value, path, field).map(|_| ())
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
            &format!("bundle field '{field}' must be a list"),
            json!("list"),
            json!(python_type_name(value)),
        ));
    };

    let mut result = Vec::new();
    for item in items {
        let Some(text) = item.as_str() else {
            return Err(validation_failure(
                path,
                &format!("bundle field '{field}' entries must be non-empty strings"),
                json!("non-empty string"),
                item.clone(),
            ));
        };
        if text.trim().is_empty() {
            return Err(validation_failure(
                path,
                &format!("bundle field '{field}' entries must be non-empty strings"),
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
        kind: "bundle_validation_failed".to_string(),
        path: Some(path.to_string_lossy().into_owned()),
        message: Some(message.to_string()),
        expected: Some(expected),
        actual: Some(actual),
        ..FailureDetail::default()
    }
}

fn python_type_name(value: &Value) -> &'static str {
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

pub fn expand_bundle(
    root: &BundleManifest,
    manifests: &[BundleManifest],
    command: &str,
) -> Result<Vec<ExpandedBundleChild>, FailureDetail> {
    let mut by_name: HashMap<&str, &BundleManifest> = HashMap::new();
    for manifest in manifests {
        if by_name.insert(manifest.name.as_str(), manifest).is_some() {
            return Err(FailureDetail {
                kind: "duplicate_bundle_name".to_string(),
                command: Some(command.to_string()),
                path: Some(manifest.path.to_string_lossy().into_owned()),
                message: Some(format!("bundle name '{}' is duplicated", manifest.name)),
                expected: Some(json!("unique bundle names")),
                actual: Some(json!(manifest.name)),
                target: Some(root.name.clone()),
                ..FailureDetail::default()
            });
        }
    }

    let mut children = Vec::new();
    let mut stack = Vec::new();
    let mut expanded_bundles = HashSet::new();
    expand_bundle_inner(
        root,
        &by_name,
        command,
        BundleReferenceSource::Direct,
        &mut stack,
        &mut expanded_bundles,
        &mut children,
    )?;
    reject_duplicate_required_scenarios(root, command, &children)?;
    Ok(children)
}

fn expand_bundle_inner<'a>(
    bundle: &'a BundleManifest,
    by_name: &HashMap<&'a str, &'a BundleManifest>,
    command: &str,
    source_kind: BundleReferenceSource,
    stack: &mut Vec<String>,
    expanded_bundles: &mut HashSet<String>,
    children: &mut Vec<ExpandedBundleChild>,
) -> Result<(), FailureDetail> {
    if stack.iter().any(|name| name == &bundle.name) {
        let mut chain = stack.clone();
        chain.push(bundle.name.clone());
        return Err(bundle_cycle_failure(
            command,
            bundle,
            stack.last().map(String::as_str),
            chain,
        ));
    }
    if source_kind == BundleReferenceSource::Included
        && expanded_bundles.contains(bundle.name.as_str())
    {
        return Ok(());
    }
    stack.push(bundle.name.clone());
    expanded_bundles.insert(bundle.name.clone());

    for required_bundle in &bundle.required_bundles {
        if !is_safe_artifact_segment(required_bundle) {
            return Err(FailureDetail {
                kind: "unsafe_bundle_reference".to_string(),
                command: Some(command.to_string()),
                path: Some(bundle.path.to_string_lossy().into_owned()),
                message: Some(
                    "bundle required_bundles contains an unsafe bundle reference".to_string(),
                ),
                expected: Some(json!("letters, numbers, dot, underscore, or hyphen")),
                actual: Some(json!(required_bundle)),
                target: Some(required_bundle.clone()),
                details: serde_json::Map::from_iter([(
                    "source_bundle".to_string(),
                    json!(bundle.name),
                )]),
                ..FailureDetail::default()
            });
        }
        if stack.iter().any(|name| name == required_bundle) {
            let mut chain = stack.clone();
            chain.push(required_bundle.clone());
            return Err(bundle_cycle_failure(
                command,
                bundle,
                Some(bundle.name.as_str()),
                chain,
            ));
        }
        let Some(included) = by_name.get(required_bundle.as_str()).copied() else {
            return Err(FailureDetail {
                kind: "missing_required_bundle".to_string(),
                command: Some(command.to_string()),
                path: Some(bundle.path.to_string_lossy().into_owned()),
                message: Some(format!(
                    "required bundle '{required_bundle}' referenced by '{}' was not found",
                    bundle.name
                )),
                expected: Some(json!("existing bundle by unique name")),
                actual: Some(json!(required_bundle)),
                target: Some(required_bundle.clone()),
                details: serde_json::Map::from_iter([(
                    "source_bundle".to_string(),
                    json!(bundle.name),
                )]),
                ..FailureDetail::default()
            });
        };
        expand_bundle_inner(
            included,
            by_name,
            command,
            BundleReferenceSource::Included,
            stack,
            expanded_bundles,
            children,
        )?;
    }

    for scenario in &bundle.required_scenarios {
        children.push(ExpandedBundleChild {
            scenario: scenario.clone(),
            required: true,
            source_bundle: bundle.name.clone(),
            bundle_chain: stack.clone(),
            source_kind: source_kind.clone(),
            manifest_path: bundle.path.clone(),
        });
    }
    for scenario in &bundle.optional_scenarios {
        children.push(ExpandedBundleChild {
            scenario: scenario.clone(),
            required: false,
            source_bundle: bundle.name.clone(),
            bundle_chain: stack.clone(),
            source_kind: source_kind.clone(),
            manifest_path: bundle.path.clone(),
        });
    }

    stack.pop();
    Ok(())
}

fn reject_duplicate_required_scenarios(
    root: &BundleManifest,
    command: &str,
    children: &[ExpandedBundleChild],
) -> Result<(), FailureDetail> {
    let mut seen: HashMap<&str, &ExpandedBundleChild> = HashMap::new();
    for child in children.iter().filter(|child| child.required) {
        if let Some(first) = seen.get(child.scenario.as_str()) {
            return Err(FailureDetail {
                kind: "duplicate_required_scenario".to_string(),
                command: Some(command.to_string()),
                path: Some(child.manifest_path.to_string_lossy().into_owned()),
                message: Some(format!(
                    "required scenario '{}' appears in more than one expanded bundle entry",
                    child.scenario
                )),
                expected: Some(json!(
                    "each required scenario appears once after bundle expansion"
                )),
                actual: Some(json!({
                    "scenario": child.scenario,
                    "first_source_bundle": first.source_bundle,
                    "duplicate_source_bundle": child.source_bundle
                })),
                target: Some(root.name.clone()),
                details: serde_json::Map::from_iter([(
                    "source_bundle".to_string(),
                    json!(child.source_bundle),
                )]),
                ..FailureDetail::default()
            });
        }
        seen.insert(child.scenario.as_str(), child);
    }
    Ok(())
}

fn bundle_cycle_failure(
    command: &str,
    bundle: &BundleManifest,
    source_bundle: Option<&str>,
    chain: Vec<String>,
) -> FailureDetail {
    FailureDetail {
        kind: "bundle_cycle_detected".to_string(),
        command: Some(command.to_string()),
        path: Some(bundle.path.to_string_lossy().into_owned()),
        message: Some("bundle required_bundles contains a cycle".to_string()),
        expected: Some(json!("acyclic bundle references")),
        actual: Some(json!({ "chain": chain })),
        target: Some(bundle.name.clone()),
        details: source_bundle
            .map(|source| {
                serde_json::Map::from_iter([("source_bundle".to_string(), json!(source))])
            })
            .unwrap_or_default(),
        ..FailureDetail::default()
    }
}

fn invalid_manifest_failure_for_name(config: &ProjectConfig, name: &str) -> Option<FailureDetail> {
    let mut paths = Vec::new();
    collect_manifest_paths(&config.project_root.join("qa/gates"), &mut paths);
    paths.sort();

    for path in paths {
        match load_bundle_manifest(&path) {
            Ok(value) => {
                let matches_name = value
                    .as_object()
                    .and_then(|document| document.get("name"))
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .is_some_and(|manifest_name| manifest_name == name);
                if matches_name {
                    if let Err(failure) = validate_bundle_manifest(&value, &path) {
                        return Some(failure);
                    }
                }
            }
            Err(failure) => {
                let matches_stem = path
                    .file_stem()
                    .and_then(|stem| stem.to_str())
                    .is_some_and(|stem| stem == name);
                if matches_stem {
                    return Some(failure);
                }
            }
        }
    }

    None
}
