use super::{ScenarioError, python_type_name};
use godot_qa_core::results::FailureDetail;
use serde_json::{Value, json};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

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
/// `name` (`None` when absent, empty, or unavailable, so the caller
/// falls back to the file stem) and whether it is an abstract template.
struct ScenarioHeader {
    name: Option<String>,
    is_abstract: bool,
}

/// Read a scenario file once and derive both header facts from that read.
///
/// Unreadable/invalid JSON files use the stem fallback. Malformed YAML retains
/// the legacy line scan so lint can still address broken files by name.
fn read_scenario_header(path: &Path) -> ScenarioHeader {
    let Ok(raw_text) = fs::read_to_string(path) else {
        return ScenarioHeader {
            name: None,
            is_abstract: false,
        };
    };
    let header_text = (path.extension().and_then(|ext| ext.to_str()) != Some("json"))
        .then(|| unambiguous_yaml_header(&raw_text))
        .flatten();
    let data = match parse_scenario_value(path, header_text.as_deref().unwrap_or(&raw_text)) {
        Ok(data) => data,
        Err(_) => {
            return if path.extension().and_then(|ext| ext.to_str()) == Some("json") {
                ScenarioHeader {
                    name: None,
                    is_abstract: false,
                }
            } else {
                yaml_scenario_header(&raw_text)
            };
        }
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

/// Parse only header entries for simple block mappings. Exclude syntax that can
/// hide keys or change their meaning (quotes, flow collections, anchors, tags,
/// block scalars, document markers and multiline header values). Everything
/// else goes through the full shared parser, including quoted/flow headers.
fn unambiguous_yaml_header(raw_text: &str) -> Option<String> {
    let mut header = String::new();
    let mut seen_keys = [false; 2];
    let mut after_header = false;
    let mut has_mapping = false;
    for line in raw_text.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        if !line.bytes().all(|byte| {
            byte.is_ascii_alphanumeric() || byte.is_ascii_whitespace() || b"_-.:/#".contains(&byte)
        }) {
            return None;
        }
        if line.starts_with([' ', '\t']) {
            if after_header {
                return None;
            }
            continue;
        }
        let (key, value) = line.split_once(':')?;
        if key.is_empty()
            || !key
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"_-".contains(&b))
            || (!value.is_empty() && !value.starts_with(char::is_whitespace))
        {
            return None;
        }
        has_mapping = true;
        after_header = false;
        if let Some(index) = ["name", "abstract"]
            .iter()
            .position(|candidate| *candidate == key)
        {
            if seen_keys[index] {
                return None;
            }
            seen_keys[index] = true;
            after_header = true;
            header.push_str(line);
            header.push('\n');
        }
    }
    (has_mapping && !header.is_empty()).then_some(header)
}

/// Best-effort fallback for malformed YAML: keep broken scenarios discoverable.
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

/// Detect whether a scenario file declares `abstract: true` at the top level.
/// Abstract templates are excluded from listing.
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
