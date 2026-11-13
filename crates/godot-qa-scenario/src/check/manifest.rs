use super::artifacts::{normalize_existing_or_join, slug_value};
use super::result::python_type_name;
use super::{
    CHECK_STARTUP_FRAMES, CheckTestSelectionRequest, DEFAULT_SLOW_SUMMARY_THRESHOLD_MS,
    ManifestProfile, ManifestTestEntry, ResolvedCheckTestSelection, TEST_MANIFEST_RELATIVE_PATH,
    TestManifest,
};
use crate::frame_pacing::manifest_frame_pacing;
use crate::time_control::validate_time_control_for_manifest;
use godot_qa_core::config::ProjectConfig;
use godot_qa_core::results::FailureDetail;
use serde_json::{Map, Value, json};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

pub(super) fn resolve_check_test_selection(
    command: &str,
    manifest_path: &Path,
    manifest: &TestManifest,
    request: CheckTestSelectionRequest,
) -> Result<ResolvedCheckTestSelection, FailureDetail> {
    let total_test_count = manifest.entries.len();
    let requested_tests = normalize_selection_values(
        command,
        manifest_path,
        request.test_ids,
        "check --tests --test values must be non-empty strings",
        false,
    )?;
    let requested_tags = normalize_selection_values(
        command,
        manifest_path,
        request.tags,
        "check --tests --tag values must be non-empty strings",
        true,
    )?;
    let requested_profile = request
        .profile
        .map(|profile| profile.trim().to_string())
        .filter(|profile| !profile.is_empty());

    if requested_tests.is_empty() && requested_tags.is_empty() && requested_profile.is_none() {
        return Ok(ResolvedCheckTestSelection {
            selected_indices: (0..total_test_count).collect(),
            total_test_count,
            selection: json!({
                "mode": "all",
                "tests": [],
                "tags": [],
                "profile": Value::Null,
                "selected_ids": manifest.entries.iter().map(|entry| entry.id.clone()).collect::<Vec<_>>(),
            }),
        });
    }

    let mut id_to_index = HashMap::new();
    let mut known_tags = HashSet::new();
    for (index, entry) in manifest.entries.iter().enumerate() {
        id_to_index.insert(entry.id.as_str(), index);
        for tag in &entry.tags {
            known_tags.insert(tag.as_str());
        }
    }

    let mut selected_indices = Vec::new();
    let mut selected_seen = HashSet::new();
    let mut profile_tags = Vec::new();
    let mut profile_tests = Vec::new();

    if let Some(profile_name) = requested_profile.as_deref() {
        let profile = manifest.profiles.get(profile_name).ok_or_else(|| {
            selection_failure(
                command,
                manifest_path,
                "unknown_profile",
                "check --tests profile must exist in qa/tests/manifest.json",
                json!("known profile name"),
                json!(profile_name),
                Some(profile_name.to_string()),
            )
        })?;
        profile_tags = profile.tags.clone();
        profile_tests = profile.tests.clone();
        let mut profile_seen = HashSet::new();
        for test_id in &profile_tests {
            let index = *id_to_index.get(test_id.as_str()).ok_or_else(|| {
                selection_failure(
                    command,
                    manifest_path,
                    "unknown_test",
                    "manifest profile expanded to an unknown test id",
                    json!("known test id"),
                    json!(test_id),
                    Some(profile_name.to_string()),
                )
            })?;
            if !profile_seen.insert(index) {
                return Err(selection_failure(
                    command,
                    manifest_path,
                    "ambiguous_selection",
                    "manifest profile selects the same test id more than once",
                    json!("unique profile-derived test ids"),
                    json!(test_id),
                    Some(profile_name.to_string()),
                ));
            }
            push_selected_index(&mut selected_indices, &mut selected_seen, index);
        }
        for tag in &profile_tags {
            let matching_indices = indices_for_tag(&manifest.entries, tag);
            for index in matching_indices {
                if !profile_seen.insert(index) {
                    return Err(selection_failure(
                        command,
                        manifest_path,
                        "ambiguous_selection",
                        "manifest profile tags and tests select the same test id",
                        json!("unique profile-derived test ids"),
                        json!(manifest.entries[index].id),
                        Some(profile_name.to_string()),
                    ));
                }
                push_selected_index(&mut selected_indices, &mut selected_seen, index);
            }
        }
    }

    for test_id in &requested_tests {
        let index = *id_to_index.get(test_id.as_str()).ok_or_else(|| {
            selection_failure(
                command,
                manifest_path,
                "unknown_test",
                "check --tests --test value must match a manifest test id",
                json!("known test id"),
                json!(test_id),
                Some(test_id.to_string()),
            )
        })?;
        push_selected_index(&mut selected_indices, &mut selected_seen, index);
    }

    for tag in &requested_tags {
        if !known_tags.contains(tag.as_str()) {
            return Err(selection_failure(
                command,
                manifest_path,
                "unknown_tag",
                "check --tests --tag value must match at least one manifest test tag",
                json!("known tag"),
                json!(tag),
                Some(tag.to_string()),
            ));
        }
        for index in indices_for_tag(&manifest.entries, tag) {
            push_selected_index(&mut selected_indices, &mut selected_seen, index);
        }
    }

    selected_indices.sort_unstable();
    if selected_indices.is_empty() {
        return Err(selection_failure(
            command,
            manifest_path,
            "empty_selection",
            "check --tests selection matched no manifest tests",
            json!("at least one selected test"),
            json!("empty_selection"),
            requested_profile.clone(),
        ));
    }
    let selected_ids = selected_indices
        .iter()
        .map(|index| manifest.entries[*index].id.clone())
        .collect::<Vec<_>>();
    Ok(ResolvedCheckTestSelection {
        selected_indices,
        total_test_count,
        selection: json!({
            "mode": "filtered",
            "tests": requested_tests,
            "tags": requested_tags,
            "profile": requested_profile,
            "profile_tests": profile_tests,
            "profile_tags": profile_tags,
            "selected_ids": selected_ids,
        }),
    })
}

fn normalize_selection_values(
    command: &str,
    manifest_path: &Path,
    values: Vec<String>,
    message: &str,
    reject_duplicates: bool,
) -> Result<Vec<String>, FailureDetail> {
    let mut normalized = Vec::new();
    let mut seen = HashSet::new();
    for value in values {
        let trimmed = value.trim();
        if trimmed.is_empty() {
            return Err(selection_failure(
                command,
                manifest_path,
                "invalid_selection",
                message,
                json!("non-empty string"),
                json!(value),
                None,
            ));
        }
        if reject_duplicates && !seen.insert(trimmed.to_string()) {
            return Err(selection_failure(
                command,
                manifest_path,
                "duplicate_tag",
                "check --tests --tag values must not contain duplicates",
                json!("unique tag filters"),
                json!(trimmed),
                Some(trimmed.to_string()),
            ));
        }
        if !reject_duplicates {
            seen.insert(trimmed.to_string());
        }
        normalized.push(trimmed.to_string());
    }
    if !reject_duplicates {
        normalized.sort();
        normalized.dedup();
    }
    Ok(normalized)
}

fn indices_for_tag(entries: &[ManifestTestEntry], tag: &str) -> Vec<usize> {
    entries
        .iter()
        .enumerate()
        .filter_map(|(index, entry)| {
            entry
                .tags
                .iter()
                .any(|entry_tag| entry_tag == tag)
                .then_some(index)
        })
        .collect()
}

pub(super) fn selected_batch_opt_out_failures(
    command: &str,
    manifest: &TestManifest,
    selected_indices: &[usize],
) -> Vec<FailureDetail> {
    selected_indices
        .iter()
        .filter_map(|index| manifest.entries.get(*index))
        .filter(|entry| !entry.batchable)
        .map(|entry| FailureDetail {
            kind: "test_batch_opt_out".to_string(),
            command: Some(command.to_string()),
            target: Some(entry.id.clone()),
            message: Some("manifest test entry is not batchable".to_string()),
            expected: Some(json!("selected tests are batchable")),
            actual: Some(json!({"batch": false})),
            ..FailureDetail::default()
        })
        .collect()
}

fn push_selected_index(
    selected_indices: &mut Vec<usize>,
    selected_seen: &mut HashSet<usize>,
    index: usize,
) {
    if selected_seen.insert(index) {
        selected_indices.push(index);
    }
}

fn selection_failure(
    command: &str,
    manifest_path: &Path,
    actual_kind: &str,
    message: &str,
    expected: Value,
    actual: Value,
    target: Option<String>,
) -> FailureDetail {
    FailureDetail {
        kind: "test_selection_invalid".to_string(),
        command: Some(command.to_string()),
        path: Some(manifest_path.to_string_lossy().into_owned()),
        target,
        message: Some(message.to_string()),
        expected: Some(expected),
        actual: Some(json!(actual_kind)),
        details: Map::from_iter([("value".to_string(), actual)]),
        ..FailureDetail::default()
    }
}

pub(super) fn load_test_manifest(
    config: &ProjectConfig,
    command: &str,
) -> (PathBuf, Option<TestManifest>, Option<FailureDetail>) {
    let manifest_path = config.project_root.join(TEST_MANIFEST_RELATIVE_PATH);
    if !manifest_path.exists() {
        return (
            manifest_path.clone(),
            None,
            Some(manifest_failure(
                command,
                &manifest_path,
                "test_manifest_missing",
                "qa/tests/manifest.json is required for check --tests",
                Some(json!("existing manifest file")),
                Some(json!("missing")),
                None,
            )),
        );
    }

    let raw = match fs::read_to_string(&manifest_path) {
        Ok(raw) => raw,
        Err(error) => {
            return (
                manifest_path.clone(),
                None,
                Some(manifest_failure(
                    command,
                    &manifest_path,
                    "test_manifest_invalid",
                    &error.to_string(),
                    Some(json!("readable manifest file")),
                    Some(json!("OSError")),
                    None,
                )),
            );
        }
    };
    let payload: Value = match serde_json::from_str(&raw) {
        Ok(value) => value,
        Err(error) => {
            return (
                manifest_path.clone(),
                None,
                Some(manifest_failure(
                    command,
                    &manifest_path,
                    "test_manifest_invalid",
                    "manifest.json must be valid JSON",
                    Some(json!("JSON object")),
                    Some(json!(error.to_string())),
                    None,
                )),
            );
        }
    };
    let Some(payload_object) = payload.as_object() else {
        return (
            manifest_path.clone(),
            None,
            Some(manifest_failure(
                command,
                &manifest_path,
                "test_manifest_invalid",
                "manifest.json root must be an object",
                Some(json!("JSON object")),
                Some(json!(python_type_name(&payload))),
                None,
            )),
        );
    };
    if payload_object.get("version").and_then(Value::as_i64) != Some(1) {
        return (
            manifest_path.clone(),
            None,
            Some(manifest_failure(
                command,
                &manifest_path,
                "test_manifest_invalid",
                "manifest version must be 1",
                Some(json!(1)),
                payload_object.get("version").cloned(),
                None,
            )),
        );
    }
    let raw_tests = payload_object.get("tests");
    let Some(raw_tests) = raw_tests else {
        return (
            manifest_path.clone(),
            None,
            Some(manifest_failure(
                command,
                &manifest_path,
                "test_manifest_invalid",
                "manifest tests must be an array",
                Some(json!("array")),
                Some(Value::Null),
                None,
            )),
        );
    };
    let Some(raw_tests) = raw_tests.as_array() else {
        return (
            manifest_path.clone(),
            None,
            Some(manifest_failure(
                command,
                &manifest_path,
                "test_manifest_invalid",
                "manifest tests must be an array",
                Some(json!("array")),
                Some(json!(python_type_name(raw_tests))),
                None,
            )),
        );
    };
    let default_timeout = match parse_manifest_timeout(
        command,
        &manifest_path,
        payload_object.get("timeout_seconds"),
        payload_object.get("timeout"),
        None,
    ) {
        Ok(timeout) => timeout,
        Err(failure) => return (manifest_path.clone(), None, Some(failure)),
    };
    let default_slow_summary_threshold_ms =
        match parse_slow_summary_threshold_ms(payload_object.get("slow_summary_threshold_ms")) {
            Ok(threshold) => threshold.unwrap_or(DEFAULT_SLOW_SUMMARY_THRESHOLD_MS),
            Err(failure) => return (manifest_path.clone(), None, Some(failure)),
        };

    let mut seen_ids = HashSet::new();
    let mut normalized = Vec::new();
    for (index, raw_entry) in raw_tests.iter().enumerate() {
        let Some(raw_entry) = raw_entry.as_object() else {
            return (
                manifest_path.clone(),
                None,
                Some(manifest_failure(
                    command,
                    &manifest_path,
                    "test_manifest_invalid",
                    "manifest test entries must be objects",
                    Some(json!("JSON object")),
                    Some(json!(python_type_name(raw_entry))),
                    Some(index.to_string()),
                )),
            );
        };
        let Some(test_id) = raw_entry.get("id").and_then(Value::as_str).map(str::trim) else {
            return (
                manifest_path.clone(),
                None,
                Some(manifest_failure(
                    command,
                    &manifest_path,
                    "test_manifest_invalid",
                    "manifest test id must be a non-empty string",
                    Some(json!("non-empty string")),
                    raw_entry.get("id").cloned(),
                    None,
                )),
            );
        };
        if test_id.is_empty() {
            return (
                manifest_path.clone(),
                None,
                Some(manifest_failure(
                    command,
                    &manifest_path,
                    "test_manifest_invalid",
                    "manifest test id must be a non-empty string",
                    Some(json!("non-empty string")),
                    raw_entry.get("id").cloned(),
                    None,
                )),
            );
        }
        if !seen_ids.insert(test_id.to_string()) {
            return (
                manifest_path.clone(),
                None,
                Some(manifest_failure(
                    command,
                    &manifest_path,
                    "test_manifest_invalid",
                    "manifest test ids must be unique",
                    Some(json!("unique test id")),
                    Some(json!("duplicate")),
                    Some(test_id.to_string()),
                )),
            );
        }
        let kind = raw_entry
            .get("kind")
            .and_then(Value::as_str)
            .unwrap_or_default();
        if kind != "scene" && kind != "script" {
            return (
                manifest_path.clone(),
                None,
                Some(manifest_failure(
                    command,
                    &manifest_path,
                    "test_manifest_invalid",
                    "manifest test kind must be scene or script",
                    Some(json!(["scene", "script"])),
                    raw_entry.get("kind").cloned(),
                    Some(test_id.to_string()),
                )),
            );
        }
        let Some(target) = raw_entry.get("target").and_then(Value::as_str) else {
            return (
                manifest_path.clone(),
                None,
                Some(manifest_failure(
                    command,
                    &manifest_path,
                    "test_manifest_invalid",
                    "manifest test target must be a non-empty string",
                    Some(json!("non-empty string")),
                    raw_entry.get("target").cloned(),
                    Some(test_id.to_string()),
                )),
            );
        };
        if target.is_empty() {
            return (
                manifest_path.clone(),
                None,
                Some(manifest_failure(
                    command,
                    &manifest_path,
                    "test_manifest_invalid",
                    "manifest test target must be a non-empty string",
                    Some(json!("non-empty string")),
                    raw_entry.get("target").cloned(),
                    Some(test_id.to_string()),
                )),
            );
        }

        let startup_frames = raw_entry.get("startup_frames").cloned();
        let time_control = match raw_entry.get("time_control") {
            Some(time_control) => match validate_time_control_for_manifest(time_control) {
                Ok(settings) => Some(settings.to_normalized_value()),
                Err(mut failure) => {
                    failure.command = Some(command.to_string());
                    failure.path = Some(manifest_path.to_string_lossy().into_owned());
                    failure.target = Some(test_id.to_string());
                    return (manifest_path.clone(), None, Some(failure));
                }
            },
            None => None,
        };
        let frame_pacing = match manifest_frame_pacing(
            raw_entry.get("fixed_fps"),
            time_control.is_some(),
            &config.project_root,
            &manifest_path,
            test_id,
        ) {
            Ok(frame_pacing) => frame_pacing,
            Err(mut failure) => {
                failure.command = Some(command.to_string());
                return (manifest_path.clone(), None, Some(failure));
            }
        };
        let timeout = match parse_manifest_timeout(
            command,
            &manifest_path,
            raw_entry.get("timeout_seconds"),
            raw_entry.get("timeout"),
            Some(test_id.to_string()),
        ) {
            Ok(timeout) => timeout.or(default_timeout),
            Err(failure) => return (manifest_path.clone(), None, Some(failure)),
        };
        let slow_summary_threshold_ms =
            match parse_slow_summary_threshold_ms(raw_entry.get("slow_summary_threshold_ms")) {
                Ok(threshold) => threshold.unwrap_or(default_slow_summary_threshold_ms),
                Err(mut failure) => {
                    failure.target = Some(test_id.to_string());
                    return (manifest_path.clone(), None, Some(failure));
                }
            };
        let tags =
            match parse_manifest_tags(command, &manifest_path, raw_entry.get("tags"), test_id) {
                Ok(tags) => tags,
                Err(failure) => return (manifest_path.clone(), None, Some(failure)),
            };
        let normalized_entry = ManifestTestEntry {
            id: test_id.to_string(),
            artifact_slug: slug_value(test_id),
            kind: kind.to_string(),
            target: target.to_string(),
            tags,
            batchable: match raw_entry.get("batch") {
                Some(Value::Bool(batchable)) => *batchable,
                Some(raw_batch) => {
                    return (
                        manifest_path.clone(),
                        None,
                        Some(manifest_failure(
                            command,
                            &manifest_path,
                            "test_manifest_invalid",
                            "manifest test batch must be a boolean",
                            Some(json!("boolean")),
                            Some(raw_batch.clone()),
                            Some(test_id.to_string()),
                        )),
                    );
                }
                None => true,
            },
            startup_frames: startup_frames.as_ref().and_then(Value::as_i64),
            time_control,
            frame_pacing,
            timeout,
            slow_summary_threshold_ms,
        };
        normalized.push((normalized_entry, startup_frames));
    }

    let mut entries = Vec::new();
    for (mut entry, raw_startup_frames) in normalized {
        if let Some(failure) = validate_project_relative_target(
            command,
            &manifest_path,
            &config.project_root,
            &entry.id,
            &entry.target,
        ) {
            return (manifest_path.clone(), None, Some(failure));
        }
        if entry.kind == "scene" {
            let startup_frames = raw_startup_frames.unwrap_or_else(|| json!(CHECK_STARTUP_FRAMES));
            let Some(startup_frames) = startup_frames.as_i64() else {
                return (
                    manifest_path.clone(),
                    None,
                    Some(manifest_failure(
                        command,
                        &manifest_path,
                        "test_manifest_invalid",
                        "scene test startup_frames must be a non-negative integer",
                        Some(json!("non-negative integer")),
                        Some(startup_frames),
                        Some(entry.id.clone()),
                    )),
                );
            };
            if startup_frames < 0 {
                return (
                    manifest_path.clone(),
                    None,
                    Some(manifest_failure(
                        command,
                        &manifest_path,
                        "test_manifest_invalid",
                        "scene test startup_frames must be a non-negative integer",
                        Some(json!("non-negative integer")),
                        Some(json!(startup_frames)),
                        Some(entry.id.clone()),
                    )),
                );
            }
            entry.startup_frames = Some(startup_frames);
        } else {
            entry.startup_frames = None;
        }
        entries.push(entry);
    }

    let mut slug_counts = HashMap::new();
    for entry in &entries {
        *slug_counts.entry(entry.artifact_slug.clone()).or_insert(0) += 1;
    }
    for (index, entry) in entries.iter_mut().enumerate() {
        if slug_counts[&entry.artifact_slug] > 1 {
            // A dot cannot occur in a slug, so suffixes cannot collide with other slugs.
            entry.artifact_slug = format!("{}.{index}", entry.artifact_slug);
        }
    }

    let profiles = match parse_manifest_profiles(
        command,
        &manifest_path,
        payload_object.get("profiles"),
        &entries,
    ) {
        Ok(profiles) => profiles,
        Err(failure) => return (manifest_path.clone(), None, Some(failure)),
    };

    (
        manifest_path,
        Some(TestManifest {
            entries,
            profiles,
            slow_summary_threshold_ms: default_slow_summary_threshold_ms,
        }),
        None,
    )
}

fn parse_slow_summary_threshold_ms(raw: Option<&Value>) -> Result<Option<u64>, FailureDetail> {
    let Some(raw) = raw else {
        return Ok(None);
    };
    let Some(value) = raw.as_u64() else {
        return Err(FailureDetail {
            kind: "test_manifest_invalid".to_string(),
            command: Some("check".to_string()),
            message: Some("slow_summary_threshold_ms must be a non-negative integer".to_string()),
            expected: Some(json!("non-negative integer milliseconds")),
            actual: Some(raw.clone()),
            ..FailureDetail::default()
        });
    };
    Ok(Some(value))
}

fn parse_manifest_tags(
    command: &str,
    manifest_path: &Path,
    raw_tags: Option<&Value>,
    test_id: &str,
) -> Result<Vec<String>, FailureDetail> {
    let Some(raw_tags) = raw_tags else {
        return Ok(Vec::new());
    };
    let Some(raw_tags) = raw_tags.as_array() else {
        return Err(manifest_failure(
            command,
            manifest_path,
            "test_manifest_invalid",
            "manifest test tags must be an array of non-empty strings",
            Some(json!("array of non-empty strings")),
            Some(json!(python_type_name(raw_tags))),
            Some(test_id.to_string()),
        ));
    };
    parse_string_array(
        command,
        manifest_path,
        raw_tags,
        Some(test_id.to_string()),
        "manifest test tags must be an array of non-empty strings",
        true,
    )
}

fn parse_manifest_timeout(
    command: &str,
    manifest_path: &Path,
    raw_timeout_seconds: Option<&Value>,
    raw_timeout: Option<&Value>,
    target: Option<String>,
) -> Result<Option<Duration>, FailureDetail> {
    if raw_timeout_seconds.is_some() && raw_timeout.is_some() {
        return Err(manifest_failure(
            command,
            manifest_path,
            "test_manifest_invalid",
            "manifest timeout must use either timeout_seconds or timeout",
            Some(json!("one timeout field")),
            Some(json!("multiple timeout fields")),
            target,
        ));
    }
    if let Some(value) = raw_timeout_seconds {
        let Some(seconds) = value.as_f64() else {
            return Err(invalid_timeout_failure(
                command,
                manifest_path,
                value.clone(),
                target,
            ));
        };
        if !seconds.is_finite() || seconds <= 0.0 {
            return Err(invalid_timeout_failure(
                command,
                manifest_path,
                value.clone(),
                target,
            ));
        }
        return Ok(Some(Duration::from_secs_f64(seconds)));
    }
    if let Some(value) = raw_timeout {
        let Some(raw) = value.as_str().map(str::trim) else {
            return Err(invalid_timeout_failure(
                command,
                manifest_path,
                value.clone(),
                target,
            ));
        };
        let (number, multiplier) = if let Some(number) = raw.strip_suffix("ms") {
            (number, 0.001)
        } else if let Some(number) = raw.strip_suffix('s') {
            (number, 1.0)
        } else if let Some(number) = raw.strip_suffix('m') {
            (number, 60.0)
        } else {
            return Err(invalid_timeout_failure(
                command,
                manifest_path,
                value.clone(),
                target,
            ));
        };
        let Ok(amount) = number.trim().parse::<f64>() else {
            return Err(invalid_timeout_failure(
                command,
                manifest_path,
                value.clone(),
                target,
            ));
        };
        let seconds = amount * multiplier;
        if !seconds.is_finite() || seconds <= 0.0 {
            return Err(invalid_timeout_failure(
                command,
                manifest_path,
                value.clone(),
                target,
            ));
        }
        return Ok(Some(Duration::from_secs_f64(seconds)));
    }
    Ok(None)
}

fn invalid_timeout_failure(
    command: &str,
    manifest_path: &Path,
    actual: Value,
    target: Option<String>,
) -> FailureDetail {
    manifest_failure(
        command,
        manifest_path,
        "test_manifest_invalid",
        "manifest timeout must be a positive finite duration",
        Some(json!(
            "positive number of seconds or duration string ending in ms, s, or m"
        )),
        Some(actual),
        target,
    )
}

fn parse_manifest_profiles(
    command: &str,
    manifest_path: &Path,
    raw_profiles: Option<&Value>,
    entries: &[ManifestTestEntry],
) -> Result<HashMap<String, ManifestProfile>, FailureDetail> {
    let Some(raw_profiles) = raw_profiles else {
        return Ok(HashMap::new());
    };
    let Some(raw_profiles) = raw_profiles.as_object() else {
        return Err(manifest_failure(
            command,
            manifest_path,
            "test_manifest_invalid",
            "manifest profiles must be an object",
            Some(json!("object")),
            Some(json!(python_type_name(raw_profiles))),
            None,
        ));
    };

    let test_ids: HashSet<&str> = entries.iter().map(|entry| entry.id.as_str()).collect();
    let mut profiles = HashMap::new();
    for (raw_name, raw_profile) in raw_profiles {
        let profile_name = raw_name.trim();
        if profile_name.is_empty() || profile_name == "all" || !is_safe_profile_name(profile_name) {
            return Err(manifest_failure(
                command,
                manifest_path,
                "test_manifest_invalid",
                "manifest profile names must be non-empty safe names and must not be reserved",
                Some(json!(
                    "profile name using letters, numbers, dot, underscore, or hyphen"
                )),
                Some(json!(raw_name)),
                Some(raw_name.to_string()),
            ));
        }
        let Some(profile_object) = raw_profile.as_object() else {
            return Err(manifest_failure(
                command,
                manifest_path,
                "test_manifest_invalid",
                "manifest profile definitions must be objects",
                Some(json!("object with tags and/or tests arrays")),
                Some(json!(python_type_name(raw_profile))),
                Some(profile_name.to_string()),
            ));
        };
        let tags = match profile_object.get("tags") {
            Some(Value::Array(tags)) => parse_string_array(
                command,
                manifest_path,
                tags,
                Some(profile_name.to_string()),
                "manifest profile tags must be an array of non-empty strings",
                false,
            )?,
            Some(value) => {
                return Err(manifest_failure(
                    command,
                    manifest_path,
                    "test_manifest_invalid",
                    "manifest profile tags must be an array of non-empty strings",
                    Some(json!("array of non-empty strings")),
                    Some(json!(python_type_name(value))),
                    Some(profile_name.to_string()),
                ));
            }
            None => Vec::new(),
        };
        let tests = match profile_object.get("tests") {
            Some(Value::Array(tests)) => parse_string_array(
                command,
                manifest_path,
                tests,
                Some(profile_name.to_string()),
                "manifest profile tests must be an array of known test ids",
                false,
            )?,
            Some(value) => {
                return Err(manifest_failure(
                    command,
                    manifest_path,
                    "test_manifest_invalid",
                    "manifest profile tests must be an array of known test ids",
                    Some(json!("array of known test ids")),
                    Some(json!(python_type_name(value))),
                    Some(profile_name.to_string()),
                ));
            }
            None => Vec::new(),
        };
        for test in &tests {
            if !test_ids.contains(test.as_str()) {
                return Err(manifest_failure(
                    command,
                    manifest_path,
                    "test_manifest_invalid",
                    "manifest profile tests must reference known test ids",
                    Some(json!("known test id")),
                    Some(json!(test)),
                    Some(profile_name.to_string()),
                ));
            }
        }
        profiles.insert(profile_name.to_string(), ManifestProfile { tags, tests });
    }
    Ok(profiles)
}

fn parse_string_array(
    command: &str,
    manifest_path: &Path,
    values: &[Value],
    target: Option<String>,
    message: &str,
    reject_reserved_tags: bool,
) -> Result<Vec<String>, FailureDetail> {
    let mut seen = HashSet::new();
    let mut normalized = Vec::new();
    for value in values {
        let Some(raw_value) = value.as_str() else {
            return Err(manifest_failure(
                command,
                manifest_path,
                "test_manifest_invalid",
                message,
                Some(json!("array of non-empty strings")),
                Some(json!(python_type_name(value))),
                target.clone(),
            ));
        };
        let trimmed = raw_value.trim();
        if trimmed.is_empty() {
            return Err(manifest_failure(
                command,
                manifest_path,
                "test_manifest_invalid",
                message,
                Some(json!("non-empty string")),
                Some(json!(raw_value)),
                target.clone(),
            ));
        }
        if reject_reserved_tags && is_reserved_manifest_tag(trimmed) {
            return Err(manifest_failure(
                command,
                manifest_path,
                "test_manifest_invalid",
                "manifest test tags must not use reserved names",
                Some(json!("non-reserved tag name")),
                Some(json!(trimmed)),
                target.clone(),
            ));
        }
        if !seen.insert(trimmed.to_string()) {
            return Err(manifest_failure(
                command,
                manifest_path,
                "test_manifest_invalid",
                "manifest string arrays must not contain duplicate values",
                Some(json!("unique values")),
                Some(json!(trimmed)),
                target.clone(),
            ));
        }
        normalized.push(trimmed.to_string());
    }
    Ok(normalized)
}

fn is_reserved_manifest_tag(tag: &str) -> bool {
    matches!(tag, "all" | "full")
}

fn is_safe_profile_name(name: &str) -> bool {
    name.chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
}

fn validate_project_relative_target(
    command: &str,
    manifest_path: &Path,
    project_root: &Path,
    test_id: &str,
    target: &str,
) -> Option<FailureDetail> {
    if !target.starts_with("res://") {
        return Some(manifest_failure(
            command,
            manifest_path,
            "test_manifest_invalid",
            "manifest test target must be a res:// path",
            Some(json!("res:// path under the selected project root")),
            Some(json!(target)),
            Some(test_id.to_string()),
        ));
    }
    let target_path = project_root.join(target.trim_start_matches("res://"));
    let resolved_target = normalize_existing_or_join(&target_path);
    let resolved_root = normalize_existing_or_join(project_root);
    if !resolved_target.starts_with(&resolved_root) {
        return Some(manifest_failure(
            command,
            manifest_path,
            "test_manifest_invalid",
            "manifest test target must stay within the selected project root",
            Some(json!("path under the selected project root")),
            Some(json!("outside_project")),
            Some(test_id.to_string()),
        ));
    }
    if !resolved_target.exists() {
        return Some(manifest_failure(
            command,
            manifest_path,
            "test_manifest_invalid",
            "manifest test target must exist",
            Some(json!("existing file under the selected project root")),
            Some(json!("missing")),
            Some(test_id.to_string()),
        ));
    }
    if !resolved_target.is_file() {
        return Some(manifest_failure(
            command,
            manifest_path,
            "test_manifest_invalid",
            "manifest test target must resolve to a file",
            Some(json!("existing file under the selected project root")),
            Some(json!("not_file")),
            Some(test_id.to_string()),
        ));
    }
    None
}

fn manifest_failure(
    command: &str,
    manifest_path: &Path,
    failure_type: &str,
    message: &str,
    expected: Option<Value>,
    actual: Option<Value>,
    target: Option<String>,
) -> FailureDetail {
    FailureDetail {
        kind: failure_type.to_string(),
        command: Some(command.to_string()),
        path: Some(manifest_path.to_string_lossy().into_owned()),
        target,
        message: Some(message.to_string()),
        expected,
        actual,
        ..FailureDetail::default()
    }
}
