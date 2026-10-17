//! Scenario templating: `extends` + `vars` + `abstract`.
//!
//! Every scenario consumer funnels through
//! [`crate::scenario_doc::load_scenario_document`], which calls
//! [`resolve_scenario_document`] right after the root-object check. Files without
//! any of the template keys (`extends`, `vars`, `abstract`) take a fast path and
//! are returned untouched, so downstream validation/normalization keeps seeing
//! plain resolved documents with zero behavior change.
//!
//! Resolution pipeline (for files that DO carry template keys):
//!
//! 1. An entry document with `abstract: true` is rejected with
//!    `scenario_abstract` (abstract templates are not runnable on their own).
//! 2. The `extends` chain is resolved recursively. `extends` is a path relative
//!    to the current file's directory (absolute paths rejected), limited to
//!    `.yaml`/`.yml`/`.json`. Cycles (`scenario_extends_cycle`) and depths past
//!    [`MAX_EXTENDS_DEPTH`] (`scenario_extends_invalid`) are rejected. A missing
//!    or unreadable base is `scenario_extends_missing`; a base that parses badly
//!    propagates the base's own parse-failure kind with the chain in its message.
//! 3. Bodies merge base-first, child-wins: objects deep-merge key-by-key, while
//!    arrays and scalars replace wholesale. A post-merge key whose value is
//!    literal `null` is deleted at any depth (the author's deletion mechanism).
//!    `name` and `abstract` are never inherited from a base. `vars` maps merge
//!    per-key with the child winning.
//! 4. After the whole chain merges, `${var}` placeholders are substituted ONCE
//!    over the merged document using the effective vars.

use crate::scenario_doc::{self, ScenarioError};
use godot_qa_core::results::FailureDetail;
use serde_json::{Map, Value, json};
use std::fs;
use std::path::{Path, PathBuf};

/// Top-level keys that drive templating and are stripped before merging.
pub(crate) const TEMPLATE_KEYS: &[&str] = &["extends", "vars", "abstract"];

/// Maximum number of chained `extends` hops before resolution is rejected.
pub(crate) const MAX_EXTENDS_DEPTH: usize = 8;

/// Resolve scenario templating for a freshly parsed root-object document.
///
/// The caller guarantees `entry_data` is a JSON object. Documents with no
/// template keys are returned unchanged (fast path).
pub fn resolve_scenario_document(
    entry_path: &Path,
    entry_data: Value,
) -> Result<Value, ScenarioError> {
    let Some(entry_obj) = entry_data.as_object() else {
        // Defensive: the loader already enforced a root object.
        return Ok(entry_data);
    };

    if is_abstract_true(entry_obj) {
        return Err(abstract_entry_failure(entry_path));
    }

    if !has_template_keys(entry_obj) {
        return Ok(entry_data);
    }

    let chain = collect_chain(entry_path, entry_data)?;
    let (body, vars) = merge_chain(&chain)?;
    substitute_document(body, &vars, entry_path)
}

/// Structurally validate the `extends`/`vars` blocks of a template file.
///
/// Used by `scenario lint` to check abstract templates without resolving or
/// fully validating them (templates may be intentionally incomplete). This does
/// not read base files or verify that a referenced base exists.
pub fn validate_template_structure(value: &Value, path: &Path) -> Result<(), ScenarioError> {
    let Some(obj) = value.as_object() else {
        return Err(template_failure(
            "invalid_scenario_file",
            path,
            "scenario template root must be an object".to_string(),
            json!("object"),
            Value::Null,
        ));
    };

    if let Some(extends_val) = obj.get("extends") {
        match extends_val {
            Value::String(text) => {
                resolve_extends_path(path, text)?;
            }
            other => {
                return Err(extends_invalid_failure(
                    path,
                    "scenario field 'extends' must be a string path",
                    other.clone(),
                ));
            }
        }
    }

    if let Some(vars_val) = obj.get("vars") {
        validate_vars_block(path, vars_val, &mut Map::new())?;
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Chain collection
// ---------------------------------------------------------------------------

fn collect_chain(
    entry_path: &Path,
    entry_data: Value,
) -> Result<Vec<(PathBuf, Value)>, ScenarioError> {
    let mut chain: Vec<(PathBuf, Value)> = Vec::new();
    let mut visited: Vec<PathBuf> = Vec::new();
    let mut hops = 0usize;
    let mut current_path = entry_path.to_path_buf();
    let mut current_data = entry_data;

    loop {
        let canonical = fs::canonicalize(&current_path).unwrap_or_else(|_| current_path.clone());
        if visited.contains(&canonical) {
            let mut display = chain_display(&chain);
            display.push(current_path.to_string_lossy().into_owned());
            return Err(extends_cycle_failure(entry_path, display));
        }
        visited.push(canonical);

        let obj = current_data
            .as_object()
            .expect("scenario document root is an object");
        let next_extends = match obj.get("extends") {
            None => None,
            Some(Value::String(text)) => Some(text.clone()),
            Some(other) => {
                return Err(extends_invalid_failure(
                    &current_path,
                    "scenario field 'extends' must be a string path",
                    other.clone(),
                ));
            }
        };

        chain.push((current_path.clone(), current_data));

        let Some(extends_text) = next_extends else {
            break;
        };

        hops += 1;
        if hops > MAX_EXTENDS_DEPTH {
            return Err(extends_depth_failure(&current_path, chain_display(&chain)));
        }

        let base_path = resolve_extends_path(&current_path, &extends_text)?;
        let base_data = load_base_document(&base_path, &chain)?;
        current_path = base_path;
        current_data = base_data;
    }

    Ok(chain)
}

fn load_base_document(
    base_path: &Path,
    chain: &[(PathBuf, Value)],
) -> Result<Value, ScenarioError> {
    if !base_path.is_file() {
        return Err(extends_missing_failure(base_path, chain));
    }
    let raw = match fs::read_to_string(base_path) {
        Ok(raw) => raw,
        Err(_) => return Err(extends_missing_failure(base_path, chain)),
    };
    scenario_doc::parse_scenario_value(base_path, &raw)
        .map_err(|error| augment_with_chain(error, chain))
}

fn resolve_extends_path(current_file: &Path, extends: &str) -> Result<PathBuf, ScenarioError> {
    let trimmed = extends.trim();
    if trimmed.is_empty() {
        return Err(extends_invalid_failure(
            current_file,
            "scenario field 'extends' must be a non-empty relative path",
            json!(extends),
        ));
    }
    let candidate = Path::new(trimmed);
    if candidate.is_absolute() {
        return Err(extends_invalid_failure(
            current_file,
            "scenario field 'extends' must be a relative path (absolute paths are rejected)",
            json!(extends),
        ));
    }
    let extension_ok = matches!(
        candidate.extension().and_then(|ext| ext.to_str()),
        Some("yaml" | "yml" | "json")
    );
    if !extension_ok {
        return Err(extends_invalid_failure(
            current_file,
            "scenario field 'extends' must reference a .yaml, .yml, or .json file",
            json!(extends),
        ));
    }
    let dir = current_file.parent().unwrap_or_else(|| Path::new("."));
    Ok(dir.join(candidate))
}

// ---------------------------------------------------------------------------
// Merge
// ---------------------------------------------------------------------------

fn merge_chain(chain: &[(PathBuf, Value)]) -> Result<(Value, Map<String, Value>), ScenarioError> {
    let mut merged: Option<Value> = None;
    let mut vars: Map<String, Value> = Map::new();

    // Iterate base-first so more-derived levels win on both body and vars.
    for (file_path, data) in chain.iter().rev() {
        let obj = data
            .as_object()
            .expect("scenario document root is an object");

        if let Some(vars_val) = obj.get("vars") {
            validate_vars_block(file_path, vars_val, &mut vars)?;
        }

        let body = strip_template_keys(obj);
        merged = Some(match merged {
            None => Value::Object(body),
            Some(Value::Object(mut base_obj)) => {
                // `name` is never inherited from a base.
                base_obj.remove("name");
                deep_merge(Value::Object(base_obj), Value::Object(body))
            }
            Some(other) => deep_merge(other, Value::Object(body)),
        });
    }

    let mut merged = merged.expect("scenario extends chain is never empty");
    remove_null_keys(&mut merged);
    Ok((merged, vars))
}

/// Deep-merge `child` over `base`. Objects merge key-by-key with the child
/// winning; arrays and scalars replace wholesale.
pub(crate) fn deep_merge(base: Value, child: Value) -> Value {
    match (base, child) {
        (Value::Object(mut base_obj), Value::Object(child_obj)) => {
            for (key, child_value) in child_obj {
                let merged_value = match base_obj.remove(&key) {
                    Some(base_value) => deep_merge(base_value, child_value),
                    None => child_value,
                };
                base_obj.insert(key, merged_value);
            }
            Value::Object(base_obj)
        }
        (_, child) => child,
    }
}

/// Delete object keys whose value is literal `null`, at any depth (recursing
/// into array elements as well). This is the author's deletion mechanism.
fn remove_null_keys(value: &mut Value) {
    match value {
        Value::Object(map) => {
            map.retain(|_, entry| !entry.is_null());
            for entry in map.values_mut() {
                remove_null_keys(entry);
            }
        }
        Value::Array(items) => {
            for entry in items.iter_mut() {
                remove_null_keys(entry);
            }
        }
        _ => {}
    }
}

fn strip_template_keys(obj: &Map<String, Value>) -> Map<String, Value> {
    obj.iter()
        .filter(|(key, _)| !TEMPLATE_KEYS.contains(&key.as_str()))
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect()
}

// ---------------------------------------------------------------------------
// Vars + substitution
// ---------------------------------------------------------------------------

fn validate_vars_block(
    file_path: &Path,
    vars_val: &Value,
    into: &mut Map<String, Value>,
) -> Result<(), ScenarioError> {
    let Some(vars_obj) = vars_val.as_object() else {
        return Err(vars_invalid_failure(
            file_path,
            "scenario field 'vars' must be a top-level object".to_string(),
            vars_val.clone(),
        ));
    };
    for (name, value) in vars_obj {
        if !is_valid_var_name(name) {
            return Err(vars_invalid_failure(
                file_path,
                format!("scenario var name '{name}' is invalid"),
                json!(name),
            ));
        }
        if value.is_null() {
            return Err(vars_invalid_failure(
                file_path,
                format!("scenario var '{name}' must not be null"),
                json!(name),
            ));
        }
        into.insert(name.clone(), value.clone());
    }
    Ok(())
}

fn substitute_document(
    mut body: Value,
    vars: &Map<String, Value>,
    path: &Path,
) -> Result<Value, ScenarioError> {
    let mut unresolved: Vec<Value> = Vec::new();
    substitute_value(&mut body, vars, path, "", &mut unresolved)?;
    if !unresolved.is_empty() {
        return Err(var_unresolved_failure(path, unresolved));
    }
    Ok(body)
}

fn substitute_value(
    value: &mut Value,
    vars: &Map<String, Value>,
    path: &Path,
    location: &str,
    unresolved: &mut Vec<Value>,
) -> Result<(), ScenarioError> {
    match value {
        Value::String(text) => {
            *value = substitute_string(text, vars, path, location, unresolved)?;
        }
        Value::Array(items) => {
            for (index, item) in items.iter_mut().enumerate() {
                let item_location = format!("{location}[{index}]");
                substitute_value(item, vars, path, &item_location, unresolved)?;
            }
        }
        Value::Object(map) => {
            for (key, entry) in map.iter_mut() {
                let entry_location = if location.is_empty() {
                    key.clone()
                } else {
                    format!("{location}.{key}")
                };
                substitute_value(entry, vars, path, &entry_location, unresolved)?;
            }
        }
        _ => {}
    }
    Ok(())
}

fn substitute_string(
    input: &str,
    vars: &Map<String, Value>,
    path: &Path,
    location: &str,
    unresolved: &mut Vec<Value>,
) -> Result<Value, ScenarioError> {
    // A string that is EXACTLY `${name}` is replaced by the var's raw typed
    // JSON value (preserving int/bool/array/object).
    if let Some(name) = exact_placeholder(input) {
        if let Some(value) = vars.get(name) {
            return Ok(value.clone());
        }
        unresolved.push(json!({ "placeholder": input, "location": location }));
        return Ok(Value::String(input.to_string()));
    }

    // Otherwise scan for embedded `${name}` (scalar interpolation) and `$$`
    // escapes.
    let chars: Vec<char> = input.chars().collect();
    let mut out = String::new();
    let mut i = 0;
    while i < chars.len() {
        let ch = chars[i];
        if ch == '$' {
            if i + 1 < chars.len() && chars[i + 1] == '$' {
                // `$$` escapes to a literal `$`.
                out.push('$');
                i += 2;
                continue;
            }
            if i + 1 < chars.len() && chars[i + 1] == '{' {
                if let Some(close) = find_closing_brace(&chars, i + 2) {
                    let inner: String = chars[i + 2..close].iter().collect();
                    let placeholder: String = chars[i..=close].iter().collect();
                    if is_valid_var_name(&inner) {
                        if let Some(value) = vars.get(&inner) {
                            match scalar_to_string(value) {
                                Some(text) => out.push_str(&text),
                                None => {
                                    return Err(vars_invalid_failure(
                                        path,
                                        format!(
                                            "var '{inner}' is not a scalar and cannot be interpolated into a string at {location}"
                                        ),
                                        json!({ "var": inner, "location": location }),
                                    ));
                                }
                            }
                        } else {
                            unresolved
                                .push(json!({ "placeholder": placeholder, "location": location }));
                            out.push_str(&placeholder);
                        }
                    } else {
                        unresolved
                            .push(json!({ "placeholder": placeholder, "location": location }));
                        out.push_str(&placeholder);
                    }
                    i = close + 1;
                    continue;
                }
                // `${` with no closing brace: record the remainder as unresolved.
                let placeholder: String = chars[i..].iter().collect();
                unresolved.push(json!({ "placeholder": placeholder, "location": location }));
                out.push_str(&placeholder);
                break;
            }
            out.push('$');
            i += 1;
            continue;
        }
        out.push(ch);
        i += 1;
    }
    Ok(Value::String(out))
}

fn exact_placeholder(input: &str) -> Option<&str> {
    let inner = input.strip_prefix("${")?.strip_suffix('}')?;
    is_valid_var_name(inner).then_some(inner)
}

fn find_closing_brace(chars: &[char], start: usize) -> Option<usize> {
    (start..chars.len()).find(|&index| chars[index] == '}')
}

fn scalar_to_string(value: &Value) -> Option<String> {
    match value {
        Value::String(text) => Some(text.clone()),
        Value::Number(number) => Some(number.to_string()),
        Value::Bool(boolean) => Some(boolean.to_string()),
        Value::Null | Value::Array(_) | Value::Object(_) => None,
    }
}

fn is_valid_var_name(name: &str) -> bool {
    let mut chars = name.chars();
    match chars.next() {
        Some(first) if first.is_ascii_alphabetic() || first == '_' => {}
        _ => return false,
    }
    chars.all(|ch| ch.is_ascii_alphanumeric() || ch == '_')
}

fn is_abstract_true(obj: &Map<String, Value>) -> bool {
    matches!(obj.get("abstract"), Some(Value::Bool(true)))
}

fn has_template_keys(obj: &Map<String, Value>) -> bool {
    TEMPLATE_KEYS.iter().any(|key| obj.contains_key(*key))
}

// ---------------------------------------------------------------------------
// Failure constructors
// ---------------------------------------------------------------------------

fn template_failure(
    kind: &str,
    path: &Path,
    message: String,
    expected: Value,
    actual: Value,
) -> ScenarioError {
    ScenarioError {
        failure: FailureDetail {
            kind: kind.to_string(),
            path: Some(path.to_string_lossy().into_owned()),
            message: Some(message),
            expected: Some(expected),
            actual: Some(actual),
            ..FailureDetail::default()
        },
    }
}

fn abstract_entry_failure(path: &Path) -> ScenarioError {
    template_failure(
        "scenario_abstract",
        path,
        "abstract scenario templates cannot be loaded or run directly".to_string(),
        json!("concrete scenario"),
        json!(path.to_string_lossy()),
    )
}

fn extends_cycle_failure(entry_path: &Path, chain: Vec<String>) -> ScenarioError {
    template_failure(
        "scenario_extends_cycle",
        entry_path,
        "scenario 'extends' chain contains a cycle".to_string(),
        json!("an acyclic extends chain"),
        json!(chain),
    )
}

fn extends_depth_failure(path: &Path, chain: Vec<String>) -> ScenarioError {
    template_failure(
        "scenario_extends_invalid",
        path,
        format!("scenario 'extends' chain exceeds the maximum depth of {MAX_EXTENDS_DEPTH}"),
        json!(format!("at most {MAX_EXTENDS_DEPTH} chained extends")),
        json!(chain),
    )
}

fn extends_invalid_failure(path: &Path, message: &str, actual: Value) -> ScenarioError {
    template_failure(
        "scenario_extends_invalid",
        path,
        message.to_string(),
        json!("a relative .yaml, .yml, or .json path"),
        actual,
    )
}

fn extends_missing_failure(base_path: &Path, chain: &[(PathBuf, Value)]) -> ScenarioError {
    template_failure(
        "scenario_extends_missing",
        base_path,
        "scenario base referenced by 'extends' was not found or is unreadable".to_string(),
        json!("an existing base scenario file"),
        json!({
            "resolved": base_path.to_string_lossy(),
            "chain": chain_display(chain),
        }),
    )
}

fn vars_invalid_failure(path: &Path, message: String, actual: Value) -> ScenarioError {
    template_failure(
        "scenario_vars_invalid",
        path,
        message,
        json!("a vars object with valid names and non-null values"),
        actual,
    )
}

fn var_unresolved_failure(path: &Path, unresolved: Vec<Value>) -> ScenarioError {
    template_failure(
        "scenario_var_unresolved",
        path,
        "scenario contains unresolved ${...} template placeholders".to_string(),
        json!("all ${...} placeholders resolved by vars"),
        Value::Array(unresolved),
    )
}

fn augment_with_chain(mut error: ScenarioError, chain: &[(PathBuf, Value)]) -> ScenarioError {
    let chain_str = chain_display(chain).join(" -> ");
    let base_message = error.failure.message.clone().unwrap_or_default();
    error.failure.message = Some(format!("{base_message} (via extends chain: {chain_str})"));
    error
}

fn chain_display(chain: &[(PathBuf, Value)]) -> Vec<String> {
    chain
        .iter()
        .map(|(path, _)| path.to_string_lossy().into_owned())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chain_entry(path: &str, value: Value) -> (PathBuf, Value) {
        (PathBuf::from(path), value)
    }

    #[test]
    fn deep_merge_merges_objects_and_replaces_arrays_and_scalars() {
        let base = json!({
            "viewport": {"width": 1920, "height": 1080},
            "steps": [{"wait_frames": 5}],
            "scalar": 1,
        });
        let child = json!({
            "viewport": {"height": 720},
            "steps": [{"wait_frames": 9}, {"snapshot": "after"}],
            "scalar": 2,
        });
        let merged = deep_merge(base, child);
        assert_eq!(
            merged,
            json!({
                // objects deep-merge key-by-key (width preserved, height overridden)
                "viewport": {"width": 1920, "height": 720},
                // arrays replace wholesale
                "steps": [{"wait_frames": 9}, {"snapshot": "after"}],
                // scalars replace wholesale
                "scalar": 2,
            })
        );
    }

    #[test]
    fn remove_null_keys_deletes_keys_at_any_depth() {
        let mut value = json!({
            "keep": 1,
            "drop": null,
            "nested": {"keep": 2, "drop": null},
            "list": [{"keep": 3, "drop": null}],
        });
        remove_null_keys(&mut value);
        assert_eq!(
            value,
            json!({
                "keep": 1,
                "nested": {"keep": 2},
                "list": [{"keep": 3}],
            })
        );
    }

    #[test]
    fn merge_chain_does_not_inherit_name_or_abstract_and_merges_vars_child_wins() {
        let chain = vec![
            chain_entry(
                "child.yaml",
                json!({
                    "name": "child",
                    "vars": {"frames": 20},
                    "viewport": {"height": 720},
                }),
            ),
            chain_entry(
                "base.yaml",
                json!({
                    "abstract": true,
                    "name": "base",
                    "vars": {"frames": 5, "label": "hi"},
                    "scene": "res://ui/main_menu.tscn",
                    "viewport": {"width": 1920, "height": 1080},
                    "steps": [{"wait_frames": 1}],
                }),
            ),
        ];
        let (body, vars) = merge_chain(&chain).unwrap();
        assert_eq!(body.get("name").and_then(Value::as_str), Some("child"));
        assert!(body.get("abstract").is_none());
        assert_eq!(
            body.get("scene").and_then(Value::as_str),
            Some("res://ui/main_menu.tscn")
        );
        // deep object merge across the chain
        assert_eq!(body["viewport"], json!({"width": 1920, "height": 720}));
        // vars: child wins on frames, base's label survives
        assert_eq!(vars.get("frames"), Some(&json!(20)));
        assert_eq!(vars.get("label"), Some(&json!("hi")));
    }

    #[test]
    fn substitute_exact_match_preserves_typed_values() {
        let vars = json!({"frames": 5, "flag": true, "list": [1, 2], "obj": {"a": 1}})
            .as_object()
            .unwrap()
            .clone();
        let body = json!({
            "steps": [{"wait_frames": "${frames}"}],
            "flag": "${flag}",
            "list": "${list}",
            "obj": "${obj}",
        });
        let resolved = substitute_document(body, &vars, Path::new("s.yaml")).unwrap();
        assert_eq!(resolved["steps"][0]["wait_frames"], json!(5));
        assert!(resolved["steps"][0]["wait_frames"].is_i64());
        assert_eq!(resolved["flag"], json!(true));
        assert_eq!(resolved["list"], json!([1, 2]));
        assert_eq!(resolved["obj"], json!({"a": 1}));
    }

    #[test]
    fn substitute_embeds_scalars_and_escapes_double_dollar() {
        let vars = json!({"slot": "qa-profile", "n": 3})
            .as_object()
            .unwrap()
            .clone();
        let body = json!({
            "scene": "res://saves/${slot}/slot_${n}.tres",
            "literal": "$${slot} stays literal",
        });
        let resolved = substitute_document(body, &vars, Path::new("s.yaml")).unwrap();
        assert_eq!(
            resolved["scene"].as_str(),
            Some("res://saves/qa-profile/slot_3.tres")
        );
        assert_eq!(resolved["literal"].as_str(), Some("${slot} stays literal"));
    }

    #[test]
    fn substitute_rejects_non_scalar_embed() {
        let vars = json!({"list": [1, 2]}).as_object().unwrap().clone();
        let body = json!({"scene": "prefix-${list}"});
        let error = substitute_document(body, &vars, Path::new("s.yaml")).unwrap_err();
        assert_eq!(error.failure.kind, "scenario_vars_invalid");
    }

    #[test]
    fn substitute_reports_unresolved_placeholder_with_location() {
        let vars = Map::new();
        let body = json!({"steps": [{"tap_action": "unused"}, {"tap_action": "${missing}"}]});
        let error = substitute_document(body, &vars, Path::new("s.yaml")).unwrap_err();
        assert_eq!(error.failure.kind, "scenario_var_unresolved");
        let actual = error.failure.actual.unwrap();
        let entries = actual.as_array().unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0]["placeholder"], json!("${missing}"));
        assert_eq!(entries[0]["location"], json!("steps[1].tap_action"));
    }

    #[test]
    fn merge_before_substitute_child_literal_beats_base_placeholder() {
        let chain = vec![
            chain_entry(
                "child.yaml",
                json!({"name": "child", "scene": "res://real.tscn"}),
            ),
            chain_entry(
                "base.yaml",
                json!({"abstract": true, "vars": {"scene": "res://ignored.tscn"}, "scene": "${scene}"}),
            ),
        ];
        let (body, vars) = merge_chain(&chain).unwrap();
        let resolved = substitute_document(body, &vars, Path::new("child.yaml")).unwrap();
        // The child's literal replaces the base placeholder before substitution.
        assert_eq!(resolved["scene"].as_str(), Some("res://real.tscn"));
    }

    #[test]
    fn merge_before_substitute_child_var_reparameterizes_base_placeholder() {
        let chain = vec![
            chain_entry(
                "child.yaml",
                json!({"name": "child", "vars": {"frames": 20}}),
            ),
            chain_entry(
                "base.yaml",
                json!({
                    "abstract": true,
                    "vars": {"frames": 5},
                    "steps": [{"wait_frames": "${frames}"}],
                }),
            ),
        ];
        let (body, vars) = merge_chain(&chain).unwrap();
        let resolved = substitute_document(body, &vars, Path::new("child.yaml")).unwrap();
        assert_eq!(resolved["steps"][0]["wait_frames"], json!(20));
    }

    #[test]
    fn merge_chain_rejects_null_var() {
        let chain = vec![chain_entry(
            "child.yaml",
            json!({"name": "child", "vars": {"frames": null}}),
        )];
        let error = merge_chain(&chain).unwrap_err();
        assert_eq!(error.failure.kind, "scenario_vars_invalid");
    }

    #[test]
    fn merge_chain_rejects_bad_var_name() {
        let chain = vec![chain_entry(
            "child.yaml",
            json!({"name": "child", "vars": {"bad-name": 1}}),
        )];
        let error = merge_chain(&chain).unwrap_err();
        assert_eq!(error.failure.kind, "scenario_vars_invalid");
    }

    #[test]
    fn merge_chain_rejects_non_object_vars() {
        let chain = vec![chain_entry(
            "child.yaml",
            json!({"name": "child", "vars": ["not", "an", "object"]}),
        )];
        let error = merge_chain(&chain).unwrap_err();
        assert_eq!(error.failure.kind, "scenario_vars_invalid");
    }

    #[test]
    fn null_child_value_deletes_inherited_key() {
        let chain = vec![
            chain_entry("child.yaml", json!({"name": "child", "godot_args": null})),
            chain_entry(
                "base.yaml",
                json!({"abstract": true, "godot_args": ["--single-window"], "scene": "res://x.tscn"}),
            ),
        ];
        let (body, _vars) = merge_chain(&chain).unwrap();
        assert!(body.get("godot_args").is_none());
        assert_eq!(
            body.get("scene").and_then(Value::as_str),
            Some("res://x.tscn")
        );
    }
}
