//! Portable evidence export from an explicit, allowlisted selection.
//! Source JSON is projected into small scalar summaries; raw connection data
//! and absolute artifact paths are never copied into the exported directory.

use crate::ArtifactExportArgs;
use godot_qa_core::hash::{Sha256, to_lower_hex};
use godot_qa_core::results::{CommandResult, FailureDetail};
use image::GenericImageView;
use serde_json::{Map, Value, json};
use std::collections::HashSet;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Component, Path, PathBuf};

const SCHEMA: u64 = 1;
const MAX_JSON_BYTES: u64 = 32 * 1024 * 1024;

pub(crate) fn artifact_export_result(args: &ArtifactExportArgs) -> CommandResult {
    let command = "artifact export";
    let selection_path = Path::new(&args.selection);
    let output = PathBuf::from(&args.out);
    if output.exists() || output.symlink_metadata().is_ok() {
        return failed(
            &output,
            "export_output_exists",
            "export output must be a new directory",
        );
    }
    let selection = match read_json(selection_path) {
        Ok(value) => value,
        Err(message) => return failed(selection_path, "export_selection_invalid", &message),
    };
    if selection.get("schema_version").and_then(Value::as_u64) != Some(SCHEMA) {
        return failed(
            selection_path,
            "export_selection_invalid",
            "selection schema_version must be 1",
        );
    }
    let Some(title) = selection
        .get("title")
        .and_then(Value::as_str)
        .filter(|value| safe_text(value, 200))
    else {
        return failed(
            selection_path,
            "export_selection_invalid",
            "title must be short printable text",
        );
    };
    let Some(sources) = selection
        .get("sources")
        .and_then(Value::as_array)
        .filter(|items| !items.is_empty())
    else {
        return failed(
            selection_path,
            "export_selection_invalid",
            "sources must be a nonempty array",
        );
    };
    let selection_parent = selection_path.parent().unwrap_or_else(|| Path::new("."));
    let mut prepared = Vec::new();
    let mut seen_sources = HashSet::new();
    for source in sources {
        match prepare_source(source, selection_parent, &mut seen_sources) {
            Ok(source) => prepared.push(source),
            Err((path, kind, message)) => return failed(&path, kind, &message),
        }
    }
    if let Some(parent) = output.parent()
        && let Err(error) = fs::create_dir_all(if parent.as_os_str().is_empty() {
            Path::new(".")
        } else {
            parent
        })
    {
        return failed(parent, "export_write_failed", &error.to_string());
    }
    if let Err(error) = fs::create_dir(&output) {
        return failed(&output, "export_output_exists", &error.to_string());
    }
    let mut exported_sources = Vec::new();
    let mut incomplete = false;
    for source in prepared {
        match write_source(&output, source) {
            Ok((summary, missing)) => {
                exported_sources.push(summary);
                incomplete |= missing;
            }
            Err((path, kind, message)) => return failed(&path, kind, &message),
        }
    }
    let manifest = json!({
        "schema_version": SCHEMA,
        "title": title,
        "portable": true,
        "sources": exported_sources,
    });
    let manifest_path = output.join("evidence.json");
    if let Err(error) = write_json_new(&manifest_path, &manifest) {
        return failed(&manifest_path, "export_write_failed", &error.to_string());
    }
    let index_path = output.join("index.html");
    if let Err(error) = write_new(&index_path, render_index(title, &manifest).as_bytes()) {
        return failed(&index_path, "export_write_failed", &error.to_string());
    }
    let mut result = CommandResult::success(
        command,
        if incomplete {
            "evidence exported with incomplete sources"
        } else {
            "evidence exported"
        },
    );
    result.data = Map::from_iter([
        ("output".to_owned(), json!(output)),
        ("manifest".to_owned(), json!(manifest_path)),
        ("index".to_owned(), json!(index_path)),
        ("source_count".to_owned(), json!(sources.len())),
        ("incomplete".to_owned(), json!(incomplete)),
    ]);
    if incomplete {
        result.ok = false;
        result.exit_code = 1;
        result.failures.push(FailureDetail {
            kind: "export_source_incomplete".to_owned(),
            command: Some(command.to_owned()),
            path: Some(output.to_string_lossy().into_owned()),
            message: Some(
                "one or more selected source results were missing or malformed; see evidence.json"
                    .to_owned(),
            ),
            ..FailureDetail::default()
        });
    }
    result
}

struct PreparedSource {
    id: String,
    root: PathBuf,
    result: Option<(PathBuf, Value, String)>,
    result_missing: bool,
    declared_run_id: Option<String>,
    revision: Value,
    environment: Value,
    images: Vec<PreparedImage>,
    metrics: Vec<PreparedMetric>,
}

struct PreparedImage {
    id: String,
    source: PathBuf,
    source_relative: String,
    role: String,
    snapshot: Option<String>,
    source_hash: String,
    dimensions: (u32, u32),
    review: Value,
}

struct PreparedMetric {
    id: String,
    source_relative: String,
    source_hash: String,
    fields: Map<String, Value>,
}

type ExportError = (PathBuf, &'static str, String);

fn prepare_source(
    value: &Value,
    selection_parent: &Path,
    seen: &mut HashSet<String>,
) -> Result<PreparedSource, ExportError> {
    let id = required_slug(value, "id", selection_parent)?;
    if !seen.insert(id.clone()) {
        return Err((
            selection_parent.to_path_buf(),
            "export_selection_invalid",
            format!("duplicate source id {id}"),
        ));
    }
    let raw_root = value
        .get("run_root")
        .and_then(Value::as_str)
        .ok_or_else(|| {
            (
                selection_parent.to_path_buf(),
                "export_selection_invalid",
                format!("source {id} needs run_root"),
            )
        })?;
    let root_input = Path::new(raw_root);
    let root = fs::canonicalize(if root_input.is_absolute() {
        root_input.to_path_buf()
    } else {
        selection_parent.join(root_input)
    })
    .map_err(|error| {
        (
            root_input.to_path_buf(),
            "export_source_invalid",
            error.to_string(),
        )
    })?;
    if !root.is_dir() {
        return Err((
            root,
            "export_source_invalid",
            "run_root must be a directory".to_owned(),
        ));
    }
    let declared_run_id = value
        .get("run_id")
        .and_then(Value::as_str)
        .map(str::to_owned);
    if declared_run_id.as_ref().is_some_and(|id| !slug(id)) {
        return Err((
            root,
            "export_selection_invalid",
            "run_id must be a slug".to_owned(),
        ));
    }
    let result_rel = value.get("result").and_then(Value::as_str).ok_or_else(|| {
        (
            root.clone(),
            "export_selection_invalid",
            "source needs result path".to_owned(),
        )
    })?;
    if result_rel != "result.json" && result_rel != "verdict.json" {
        return Err((
            root,
            "export_selection_invalid",
            "result path must be result.json or verdict.json".to_owned(),
        ));
    }
    let result_path = root.join(result_rel);
    let (result, result_missing) = if result_path.exists() {
        let path = confined_file(&root, result_rel)?;
        let source_hash = hash_file(&path)
            .map_err(|error| (path.clone(), "export_source_invalid", error.to_string()))?;
        match read_json(&path) {
            Ok(json) => (Some((path, json, source_hash)), false),
            Err(_) => (None, true),
        }
    } else {
        (None, true)
    };
    let revision = selected_map(value.get("revision"), &["tool", "project", "build"], &root)?;
    let environment = selected_environment(value.get("environment"), &root)?;
    let mut images = Vec::new();
    let mut seen_images = HashSet::new();
    if let Some(items) = value.get("images") {
        let items = items.as_array().ok_or_else(|| {
            (
                root.clone(),
                "export_selection_invalid",
                "images must be an array".to_owned(),
            )
        })?;
        for item in items {
            let image_id = required_slug(item, "id", &root)?;
            if !seen_images.insert(image_id.clone()) {
                return Err((
                    root,
                    "export_selection_invalid",
                    format!("duplicate image id {image_id}"),
                ));
            }
            let relative = item.get("path").and_then(Value::as_str).ok_or_else(|| {
                (
                    root.clone(),
                    "export_selection_invalid",
                    "image needs path".to_owned(),
                )
            })?;
            let path = confined_file(&root, relative)?;
            if path.extension().and_then(|value| value.to_str()) != Some("png") {
                return Err((
                    path,
                    "export_image_invalid",
                    "selected image must be PNG".to_owned(),
                ));
            }
            let mut signature = [0_u8; 8];
            File::open(&path)
                .and_then(|mut file| file.read_exact(&mut signature))
                .map_err(|error| (path.clone(), "export_image_invalid", error.to_string()))?;
            if signature != [137, 80, 78, 71, 13, 10, 26, 10] {
                return Err((
                    path,
                    "export_image_invalid",
                    "selected image is not a PNG".to_owned(),
                ));
            }
            let dimensions = image::image_dimensions(&path)
                .map_err(|error| (path.clone(), "export_image_invalid", error.to_string()))?;
            if dimensions.0 == 0
                || dimensions.1 == 0
                || dimensions.0 > 8192
                || dimensions.1 > 8192
                || u64::from(dimensions.0) * u64::from(dimensions.1) > 16_000_000
            {
                return Err((
                    path,
                    "export_image_invalid",
                    "selected PNG dimensions exceed review limit".to_owned(),
                ));
            }
            let decoded_dimensions = image::open(&path)
                .map_err(|error| (path.clone(), "export_image_invalid", error.to_string()))?
                .dimensions();
            if decoded_dimensions != dimensions {
                return Err((
                    path,
                    "export_image_invalid",
                    "PNG header and decoded dimensions differ".to_owned(),
                ));
            }
            let role = item.get("role").and_then(Value::as_str).ok_or_else(|| {
                (
                    path.clone(),
                    "export_selection_invalid",
                    "image needs role".to_owned(),
                )
            })?;
            if role != "original" && role != "contact_sheet" {
                return Err((
                    path,
                    "export_selection_invalid",
                    "image role must be original or contact_sheet".to_owned(),
                ));
            }
            let snapshot = item
                .get("snapshot")
                .and_then(Value::as_str)
                .map(str::to_owned);
            if snapshot
                .as_ref()
                .is_some_and(|value| !safe_text(value, 128))
            {
                return Err((
                    path,
                    "export_selection_invalid",
                    "snapshot is not safe printable text".to_owned(),
                ));
            }
            let hash = hash_file(&path)
                .map_err(|error| (path.clone(), "export_image_invalid", error.to_string()))?;
            let review = selected_review(item.get("review"), &hash, &path)?;
            images.push(PreparedImage {
                id: image_id,
                source: path,
                source_relative: relative.to_owned(),
                role: role.to_owned(),
                snapshot,
                source_hash: hash,
                dimensions,
                review,
            });
        }
    }
    let mut metrics = Vec::new();
    let mut seen_metrics = HashSet::new();
    if let Some(items) = value.get("metrics") {
        let items = items.as_array().ok_or_else(|| {
            (
                root.clone(),
                "export_selection_invalid",
                "metrics must be an array".to_owned(),
            )
        })?;
        for item in items {
            let metric_id = required_slug(item, "id", &root)?;
            if !seen_metrics.insert(metric_id.clone()) {
                return Err((
                    root,
                    "export_selection_invalid",
                    format!("duplicate metric id {metric_id}"),
                ));
            }
            let relative = item.get("path").and_then(Value::as_str).ok_or_else(|| {
                (
                    root.clone(),
                    "export_selection_invalid",
                    "metric needs path".to_owned(),
                )
            })?;
            if !relative.ends_with(".json") || forbidden_source_path(relative) {
                return Err((
                    root,
                    "export_metric_invalid",
                    "metric source must be an allowlisted JSON artifact".to_owned(),
                ));
            }
            let path = confined_file(&root, relative)?;
            let source = read_json(&path)
                .map_err(|message| (path.clone(), "export_metric_invalid", message))?;
            let source_hash = hash_file(&path)
                .map_err(|error| (path.clone(), "export_metric_invalid", error.to_string()))?;
            let fields = item
                .get("fields")
                .and_then(Value::as_object)
                .filter(|fields| !fields.is_empty())
                .ok_or_else(|| {
                    (
                        path.clone(),
                        "export_selection_invalid",
                        "metric fields must be a nonempty object".to_owned(),
                    )
                })?;
            let mut projected = Map::new();
            for (name, pointer) in fields {
                if !slug(name) || sensitive_metric_segment(name) {
                    return Err((
                        path,
                        "export_selection_invalid",
                        "metric field names must be non-sensitive slugs".to_owned(),
                    ));
                }
                let pointer = pointer
                    .as_str()
                    .filter(|p| {
                        p.starts_with('/')
                            && p.len() <= 200
                            && !p.split('/').any(sensitive_metric_segment)
                    })
                    .ok_or_else(|| {
                        (
                            path.clone(),
                            "export_selection_invalid",
                            "metric fields must be JSON pointers".to_owned(),
                        )
                    })?;
                let selected = source.pointer(pointer).ok_or_else(|| {
                    (
                        path.clone(),
                        "export_metric_missing",
                        format!("metric pointer {pointer} is missing"),
                    )
                })?;
                if !(selected.is_null() || selected.is_boolean() || selected.is_number()) {
                    return Err((
                        path,
                        "export_metric_invalid",
                        format!(
                            "metric pointer {pointer} must resolve to a number, boolean, or null"
                        ),
                    ));
                }
                projected.insert(name.clone(), selected.clone());
            }
            metrics.push(PreparedMetric {
                id: metric_id,
                source_relative: relative.to_owned(),
                source_hash,
                fields: projected,
            });
        }
    }
    Ok(PreparedSource {
        id,
        root,
        result,
        result_missing,
        declared_run_id,
        revision,
        environment,
        images,
        metrics,
    })
}

fn write_source(output: &Path, source: PreparedSource) -> Result<(Value, bool), ExportError> {
    let dir = output.join("sources").join(&source.id);
    fs::create_dir_all(&dir)
        .map_err(|error| (dir.clone(), "export_write_failed", error.to_string()))?;
    let result_value = source.result.as_ref().map(|(_, value, _)| value);
    let run_id = result_value
        .and_then(|value| {
            value
                .pointer("/data/run_identity/run_id")
                .or_else(|| value.pointer("/data/run_id"))
        })
        .and_then(Value::as_str)
        .filter(|value| slug(value))
        .map(str::to_owned)
        .or_else(|| read_external_run_id(&source.root))
        .or_else(|| {
            source
                .root
                .file_name()
                .and_then(|name| name.to_str())
                .filter(|id| slug(id))
                .map(str::to_owned)
        });
    if let (Some(expected), Some(actual)) = (&source.declared_run_id, &run_id) {
        if expected != actual {
            return Err((
                source.root,
                "export_run_identity_mismatch",
                format!("declared run_id {expected} differs from source result {actual}"),
            ));
        }
    }
    let mut failure_types = Vec::new();
    let mut reported_ok = None;
    let (ok, command, outcome) = if let Some(value) = result_value {
        reported_ok = value.get("ok").and_then(Value::as_bool);
        if let Some(items) = value.get("failures").and_then(Value::as_array) {
            for (index, item) in items.iter().enumerate() {
                let kind = item
                    .get("type")
                    .and_then(Value::as_str)
                    .filter(|kind| slug(kind))
                    .unwrap_or("redacted_failure_type");
                failure_types
                    .push(json!({"type":kind,"source_pointer":format!("/failures/{index}")}));
            }
            if reported_ok == Some(true) && !items.is_empty() {
                failure_types
                    .push(json!({"type":"result_status_inconsistent","source_pointer":"/ok"}));
            }
            if reported_ok == Some(false) && items.is_empty() {
                failure_types.push(
                    json!({"type":"reported_failure_without_details","source_pointer":"/ok"}),
                );
            }
        } else {
            failure_types
                .push(json!({"type":"source_result_malformed","source_pointer":"/failures"}));
        }
        (
            if reported_ok.is_some() && value.get("failures").and_then(Value::as_array).is_some() {
                Some(reported_ok == Some(true) && failure_types.is_empty())
            } else {
                None
            },
            safe_command(value),
            value
                .pointer("/data/outcome/status")
                .and_then(Value::as_str)
                .filter(|s| safe_text(s, 64))
                .map(str::to_owned),
        )
    } else {
        failure_types.push(json!({"type":"source_result_missing_or_malformed"}));
        (None, None, None)
    };
    let incomplete = source.result_missing || ok.is_none();
    let summary = json!({
        "schema_version": SCHEMA,
        "source_id": source.id,
        "run_id": run_id,
        "command": command,
        "ok": ok,
        "reported_ok": reported_ok,
        "outcome": outcome,
        "incomplete": incomplete,
        "failure_count": failure_types.len(),
        "failures": failure_types,
    });
    let summary_rel = format!("sources/{}/result-summary.json", source.id);
    let summary_path = output.join(&summary_rel);
    write_json_new(&summary_path, &summary).map_err(|error| {
        (
            summary_path.clone(),
            "export_write_failed",
            error.to_string(),
        )
    })?;
    let mut image_entries = Vec::new();
    for image in source.images {
        let rel = format!("sources/{}/images/{}.png", source.id, image.id);
        let destination = output.join(&rel);
        fs::create_dir_all(destination.parent().unwrap()).map_err(|error| {
            (
                destination.clone(),
                "export_write_failed",
                error.to_string(),
            )
        })?;
        fs::copy(&image.source, &destination).map_err(|error| {
            (
                destination.clone(),
                "export_write_failed",
                error.to_string(),
            )
        })?;
        let exported_hash = hash_file(&destination).map_err(|error| {
            (
                destination.clone(),
                "export_write_failed",
                error.to_string(),
            )
        })?;
        if exported_hash != image.source_hash {
            return Err((
                destination,
                "export_hash_mismatch",
                "copied image differs from source".to_owned(),
            ));
        }
        image_entries.push(json!({
            "id": image.id,
            "path": rel,
            "source_relative_path": image.source_relative,
            "role": image.role,
            "snapshot": image.snapshot,
            "capture_status": if image.role == "original" { "captured" } else { "derived" },
            "source_artifact_kind": if image.role == "original" { "original_capture" } else { "derived_contact_sheet" },
            "dimensions": [image.dimensions.0,image.dimensions.1],
            "source_sha256": image.source_hash,
            "export_sha256": exported_hash,
            "transformation": "byte_exact_copy",
            "visual_review": image.review,
        }));
    }
    let mut metric_entries = Vec::new();
    for metric in source.metrics {
        let rel = format!("sources/{}/metrics/{}.json", source.id, metric.id);
        let path = output.join(&rel);
        fs::create_dir_all(path.parent().unwrap())
            .map_err(|error| (path.clone(), "export_write_failed", error.to_string()))?;
        write_json_new(&path, &json!({"fields":metric.fields}))
            .map_err(|error| (path.clone(), "export_write_failed", error.to_string()))?;
        let export_hash = hash_file(&path)
            .map_err(|error| (path.clone(), "export_write_failed", error.to_string()))?;
        metric_entries.push(json!({"id":metric.id,"path":rel,"source_relative_path":metric.source_relative,"source_sha256":metric.source_hash,"export_sha256":export_hash,"transformation":"scalar_projection_v1"}));
    }
    let source_hash = source.result.as_ref().map(|(_, _, hash)| hash.clone());
    let summary_hash = hash_file(&summary_path).map_err(|error| {
        (
            summary_path.clone(),
            "export_write_failed",
            error.to_string(),
        )
    })?;
    let entry = json!({
        "id": source.id,
        "run_id": run_id,
        "revision": source.revision,
        "environment": source.environment,
        "automatic_check_status": match ok { Some(true) => "passed", Some(false) => "failed", None => "incomplete" },
        "result_summary": {"path":summary_rel,"source_sha256":source_hash,"export_sha256":summary_hash,"transformation":"allowlisted_result_v1"},
        "failure_count": failure_types.len(),
        "failures": failure_types,
        "images": image_entries,
        "metrics": metric_entries,
    });
    Ok((entry, incomplete))
}

fn read_external_run_id(root: &Path) -> Option<String> {
    let value = read_json(&root.join("run.json")).ok()?;
    value
        .get("run_id")
        .and_then(Value::as_str)
        .filter(|value| slug(value))
        .map(str::to_owned)
}

fn safe_command(value: &Value) -> Option<String> {
    let command = value.get("command")?.as_str()?;
    [
        "scenario run",
        "scenario profile",
        "scenario bundle run",
        "artifact validate-result",
        "screenshot assert-startup-presented",
        "check",
        "session profile",
    ]
    .contains(&command)
    .then(|| command.to_owned())
}

fn selected_review(
    review: Option<&Value>,
    source_hash: &str,
    path: &Path,
) -> Result<Value, ExportError> {
    let Some(review) = review else {
        return Ok(json!({"status":"unreviewed"}));
    };
    let status = review
        .get("status")
        .and_then(Value::as_str)
        .ok_or_else(|| {
            (
                path.to_path_buf(),
                "export_review_invalid",
                "review needs status".to_owned(),
            )
        })?;
    if status == "unreviewed" {
        return Ok(json!({"status":"unreviewed"}));
    }
    if status != "reviewed" {
        return Err((
            path.to_path_buf(),
            "export_review_invalid",
            "review status must be reviewed or unreviewed".to_owned(),
        ));
    }
    let mut selected = Map::from_iter([("status".to_owned(), json!("reviewed"))]);
    for key in ["reviewer", "method", "scale", "scope", "reviewed_at"] {
        let text = review
            .get(key)
            .and_then(Value::as_str)
            .filter(|text| safe_text(text, 200))
            .ok_or_else(|| {
                (
                    path.to_path_buf(),
                    "export_review_invalid",
                    format!("review needs safe {key}"),
                )
            })?;
        selected.insert(key.to_owned(), json!(text));
    }
    if review.get("sha256").and_then(Value::as_str) != Some(source_hash) {
        return Err((
            path.to_path_buf(),
            "export_review_hash_mismatch",
            "review sha256 must match the selected image bytes".to_owned(),
        ));
    }
    selected.insert("sha256".to_owned(), json!(source_hash));
    Ok(Value::Object(selected))
}

fn selected_map(
    value: Option<&Value>,
    allowed: &[&str],
    path: &Path,
) -> Result<Value, ExportError> {
    let Some(value) = value else {
        return Ok(json!({}));
    };
    let object = value.as_object().ok_or_else(|| {
        (
            path.to_path_buf(),
            "export_selection_invalid",
            "metadata must be an object".to_owned(),
        )
    })?;
    let mut selected = Map::new();
    for (key, value) in object {
        if !allowed.contains(&key.as_str()) {
            return Err((
                path.to_path_buf(),
                "export_selection_invalid",
                format!("metadata key {key} is not allowed"),
            ));
        }
        let text = value
            .as_str()
            .filter(|text| safe_metadata(text))
            .ok_or_else(|| {
                (
                    path.to_path_buf(),
                    "export_selection_invalid",
                    format!("metadata {key} must use safe revision characters"),
                )
            })?;
        selected.insert(key.clone(), json!(text));
    }
    Ok(Value::Object(selected))
}

fn selected_environment(value: Option<&Value>, path: &Path) -> Result<Value, ExportError> {
    let Some(value) = value else {
        return Ok(json!({}));
    };
    let object = value.as_object().ok_or_else(|| {
        (
            path.to_path_buf(),
            "export_selection_invalid",
            "environment must be an object".to_owned(),
        )
    })?;
    let mut selected = Map::new();
    for (key, value) in object {
        match key.as_str() {
            "renderer" | "display_backend" | "vsync" | "pacing" => {
                let text = value
                    .as_str()
                    .filter(|text| safe_metadata(text))
                    .ok_or_else(|| {
                        (
                            path.to_path_buf(),
                            "export_selection_invalid",
                            format!("environment {key} must use safe setting characters"),
                        )
                    })?;
                selected.insert(key.clone(), json!(text));
            }
            "instrumented" => {
                let flag = value.as_bool().ok_or_else(|| {
                    (
                        path.to_path_buf(),
                        "export_selection_invalid",
                        "instrumented must be boolean".to_owned(),
                    )
                })?;
                selected.insert(key.clone(), json!(flag));
            }
            "resolution" => {
                let dims = value
                    .as_array()
                    .filter(|dims| {
                        dims.len() == 2
                            && dims
                                .iter()
                                .all(|n| n.as_u64().is_some_and(|n| n > 0 && n <= 16384))
                    })
                    .ok_or_else(|| {
                        (
                            path.to_path_buf(),
                            "export_selection_invalid",
                            "resolution must be two positive pixel dimensions".to_owned(),
                        )
                    })?;
                selected.insert(key.clone(), json!(dims));
            }
            _ => {
                return Err((
                    path.to_path_buf(),
                    "export_selection_invalid",
                    format!("environment key {key} is not allowed"),
                ));
            }
        }
    }
    Ok(Value::Object(selected))
}

fn confined_file(root: &Path, relative: &str) -> Result<PathBuf, ExportError> {
    let path = Path::new(relative);
    if path.as_os_str().is_empty()
        || !path
            .components()
            .all(|part| matches!(part, Component::Normal(_)))
        || forbidden_source_path(relative)
    {
        return Err((
            root.join(path),
            "export_path_rejected",
            "source path must be a safe relative artifact path".to_owned(),
        ));
    }
    let joined = root.join(path);
    let canonical = fs::canonicalize(&joined)
        .map_err(|error| (joined.clone(), "export_source_missing", error.to_string()))?;
    if !canonical.starts_with(root) || !canonical.is_file() {
        return Err((
            joined,
            "export_path_rejected",
            "source file resolves outside the run root or is not a regular file".to_owned(),
        ));
    }
    Ok(canonical)
}

fn forbidden_source_path(path: &str) -> bool {
    path.split('/').any(|part| {
        let part = part.to_ascii_lowercase();
        part.contains("request")
            || part.contains("session")
            || part == "logs"
            || part.starts_with("logs.")
            || part.starts_with("log.")
            || part.ends_with(".log")
            || part.contains("-log.")
            || part.contains("receipt")
            || part == "run.json"
            || part == "child-status.json"
            || part.contains("token")
            || part.contains("credential")
            || part.contains("secret")
            || part.contains("connection")
    })
}

fn sensitive_metric_segment(segment: &str) -> bool {
    let lower = segment.to_ascii_lowercase();
    [
        "token",
        "credential",
        "secret",
        "connection",
        "password",
        "passwd",
        "auth",
        "bearer",
        "api_key",
        "apikey",
        "pin",
    ]
    .iter()
    .any(|word| lower.contains(word))
}

fn required_slug(value: &Value, field: &str, path: &Path) -> Result<String, ExportError> {
    value
        .get(field)
        .and_then(Value::as_str)
        .filter(|value| slug(value))
        .map(str::to_owned)
        .ok_or_else(|| {
            (
                path.to_path_buf(),
                "export_selection_invalid",
                format!("{field} must be an ASCII slug"),
            )
        })
}

fn slug(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value
            .bytes()
            .next()
            .is_some_and(|byte| byte.is_ascii_alphanumeric())
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
}

fn safe_text(value: &str, max: usize) -> bool {
    !value.is_empty() && value.len() <= max && !value.chars().any(char::is_control)
}

fn safe_metadata(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'+' | b'-'))
}

fn read_json(path: &Path) -> Result<Value, String> {
    let metadata = fs::metadata(path).map_err(|error| error.to_string())?;
    if metadata.len() > MAX_JSON_BYTES {
        return Err("JSON artifact exceeds the 32 MiB review limit".to_owned());
    }
    let bytes = fs::read(path).map_err(|error| error.to_string())?;
    serde_json::from_slice(&bytes).map_err(|error| error.to_string())
}

fn hash_file(path: &Path) -> std::io::Result<String> {
    let mut file = File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let count = file.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        hasher.update(&buffer[..count]);
    }
    Ok(to_lower_hex(&hasher.finish()))
}

fn write_new(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let mut file = OpenOptions::new().write(true).create_new(true).open(path)?;
    file.write_all(bytes)
}

fn write_json_new(path: &Path, value: &Value) -> std::io::Result<()> {
    let mut bytes = serde_json::to_vec_pretty(value)?;
    bytes.push(b'\n');
    write_new(path, &bytes)
}

fn render_index(title: &str, manifest: &Value) -> String {
    let mut html = format!(
        "<!doctype html><html lang=\"en\"><meta charset=\"utf-8\"><title>{}</title><style>body{{font:16px system-ui;max-width:1000px;margin:2rem auto;padding:0 1rem;background:#f6f6f3;color:#222}}article{{background:white;border:1px solid #ccc;padding:1rem;margin:1rem 0}}img{{max-width:100%;height:auto}}.failed{{color:#ad1919}}.passed{{color:#176c38}}</style><h1>{}</h1><p>Portable evidence index · <a href=\"evidence.json\">machine-readable manifest</a></p>",
        escape_html(title),
        escape_html(title)
    );
    if let Some(sources) = manifest.get("sources").and_then(Value::as_array) {
        for source in sources {
            let id = source.get("id").and_then(Value::as_str).unwrap_or("");
            let status = source
                .get("automatic_check_status")
                .and_then(Value::as_str)
                .unwrap_or("incomplete");
            let summary = source
                .pointer("/result_summary/path")
                .and_then(Value::as_str)
                .unwrap_or("");
            html.push_str(&format!("<article><h2>{}</h2><p class=\"{}\">Automatic check: {} · {} failure(s)</p><p><a href=\"{}\">Result summary</a></p>", escape_html(id), escape_html(status), escape_html(status), source.get("failure_count").and_then(Value::as_u64).unwrap_or(0), escape_html(summary)));
            if let Some(images) = source.get("images").and_then(Value::as_array) {
                for image in images {
                    let path = image.get("path").and_then(Value::as_str).unwrap_or("");
                    let role = image.get("role").and_then(Value::as_str).unwrap_or("");
                    let review = image
                        .pointer("/visual_review/status")
                        .and_then(Value::as_str)
                        .unwrap_or("unreviewed");
                    html.push_str(&format!("<figure><a href=\"{}\"><img src=\"{}\" alt=\"{}\"></a><figcaption>{} · visual review: {}</figcaption></figure>", escape_html(path), escape_html(path), escape_html(role), escape_html(role), escape_html(review)));
                }
            }
            html.push_str("</article>");
        }
    }
    html.push_str("</html>");
    html
}

fn escape_html(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

fn failed(path: &Path, kind: &str, message: &str) -> CommandResult {
    CommandResult {
        ok: false,
        command: "artifact export".to_owned(),
        message: "artifact export failed".to_owned(),
        data: Map::new(),
        failures: vec![FailureDetail {
            kind: kind.to_owned(),
            command: Some("artifact export".to_owned()),
            path: Some(path.to_string_lossy().into_owned()),
            message: Some(message.to_owned()),
            expected: Some(json!(
                "valid explicit evidence selection and new portable output"
            )),
            actual: Some(json!(message)),
            ..FailureDetail::default()
        }],
        exit_code: 1,
    }
}
