use serde::Serialize;
use serde_json::{Map, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

const LARGE_TEXT_LIMIT_BYTES: u64 = 256 * 1024;
const MAX_IMPORTANT_FIELDS: usize = 24;
const MAX_FINDINGS_PER_ARTIFACT: usize = 12;
const MAX_LOG_LINES: usize = 200;

const SUMMARY_TYPES: [&str; 21] = [
    "scenario_results",
    "snapshots",
    "screenshots",
    "screenshot_sequences",
    "contact_sheets",
    "screenshot_diffs",
    "layout_audits",
    "traces",
    "physics_inspections",
    "physics_overlays",
    "collision_reports",
    "surface_audits",
    "audio_artifacts",
    "audio_analyses",
    "event_traces",
    "visual_audits",
    "profiles",
    "logs",
    "errors",
    "warnings",
    "unknown_artifacts",
];

#[derive(Debug, Clone)]
pub struct ArtifactSummaryOptions {
    pub type_filters: Vec<String>,
    pub warnings_only: bool,
    pub expected_types: Vec<String>,
    pub write_summary: bool,
    pub summary_filename: String,
    pub markdown_run_label: Option<String>,
}

impl Default for ArtifactSummaryOptions {
    fn default() -> Self {
        Self {
            type_filters: Vec::new(),
            warnings_only: false,
            expected_types: Vec::new(),
            write_summary: true,
            summary_filename: "artifact_summary.json".to_string(),
            markdown_run_label: None,
        }
    }
}

impl ArtifactSummaryOptions {
    pub fn normalized_type_filters(&self) -> BTreeSet<String> {
        self.type_filters
            .iter()
            .filter_map(|value| normalize_artifact_type(value))
            .collect()
    }

    pub fn normalized_expected_types(&self) -> BTreeSet<String> {
        self.expected_types
            .iter()
            .filter_map(|value| normalize_artifact_type(value))
            .collect()
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct ArtifactSummary {
    pub run_root: String,
    pub summary_path: Option<String>,
    pub counts: BTreeMap<String, usize>,
    pub important_findings: Vec<ArtifactFinding>,
    pub scenario_results: Vec<ArtifactEntry>,
    pub snapshots: Vec<ArtifactEntry>,
    pub screenshots: Vec<ArtifactEntry>,
    pub screenshot_sequences: Vec<ArtifactEntry>,
    pub contact_sheets: Vec<ArtifactEntry>,
    pub screenshot_diffs: Vec<ArtifactEntry>,
    pub layout_audits: Vec<ArtifactEntry>,
    pub traces: Vec<ArtifactEntry>,
    pub physics_inspections: Vec<ArtifactEntry>,
    pub physics_overlays: Vec<ArtifactEntry>,
    pub collision_reports: Vec<ArtifactEntry>,
    pub surface_audits: Vec<ArtifactEntry>,
    pub audio_artifacts: Vec<ArtifactEntry>,
    pub audio_analyses: Vec<ArtifactEntry>,
    pub event_traces: Vec<ArtifactEntry>,
    pub visual_audits: Vec<ArtifactEntry>,
    pub profiles: Vec<ArtifactEntry>,
    pub logs: Vec<ArtifactEntry>,
    pub errors: Vec<ArtifactEntry>,
    pub warnings: Vec<ArtifactEntry>,
    pub unknown_artifacts: Vec<ArtifactEntry>,
}

impl ArtifactSummary {
    fn empty(run_root: &Path, summary_path: Option<String>) -> Self {
        Self {
            run_root: run_root.to_string_lossy().into_owned(),
            summary_path,
            counts: SUMMARY_TYPES
                .iter()
                .map(|key| ((*key).to_string(), 0))
                .collect(),
            important_findings: Vec::new(),
            scenario_results: Vec::new(),
            snapshots: Vec::new(),
            screenshots: Vec::new(),
            screenshot_sequences: Vec::new(),
            contact_sheets: Vec::new(),
            screenshot_diffs: Vec::new(),
            layout_audits: Vec::new(),
            traces: Vec::new(),
            physics_inspections: Vec::new(),
            physics_overlays: Vec::new(),
            collision_reports: Vec::new(),
            surface_audits: Vec::new(),
            audio_artifacts: Vec::new(),
            audio_analyses: Vec::new(),
            event_traces: Vec::new(),
            visual_audits: Vec::new(),
            profiles: Vec::new(),
            logs: Vec::new(),
            errors: Vec::new(),
            warnings: Vec::new(),
            unknown_artifacts: Vec::new(),
        }
    }

    fn push(&mut self, category: &str, entry: ArtifactEntry) {
        match category {
            "scenario_results" => self.scenario_results.push(entry),
            "snapshots" => self.snapshots.push(entry),
            "screenshots" => self.screenshots.push(entry),
            "screenshot_sequences" => self.screenshot_sequences.push(entry),
            "contact_sheets" => self.contact_sheets.push(entry),
            "screenshot_diffs" => self.screenshot_diffs.push(entry),
            "layout_audits" => self.layout_audits.push(entry),
            "traces" => self.traces.push(entry),
            "physics_inspections" => self.physics_inspections.push(entry),
            "physics_overlays" => self.physics_overlays.push(entry),
            "collision_reports" => self.collision_reports.push(entry),
            "surface_audits" => self.surface_audits.push(entry),
            "audio_artifacts" => self.audio_artifacts.push(entry),
            "audio_analyses" => self.audio_analyses.push(entry),
            "event_traces" => self.event_traces.push(entry),
            "visual_audits" => self.visual_audits.push(entry),
            "profiles" => self.profiles.push(entry),
            "logs" => self.logs.push(entry),
            "errors" => self.errors.push(entry),
            "warnings" => self.warnings.push(entry),
            _ => self.unknown_artifacts.push(entry),
        }
    }

    fn refresh_counts(&mut self) {
        self.counts
            .insert("scenario_results".to_string(), self.scenario_results.len());
        self.counts
            .insert("snapshots".to_string(), self.snapshots.len());
        self.counts
            .insert("screenshots".to_string(), self.screenshots.len());
        self.counts.insert(
            "screenshot_sequences".to_string(),
            self.screenshot_sequences.len(),
        );
        self.counts
            .insert("contact_sheets".to_string(), self.contact_sheets.len());
        self.counts
            .insert("screenshot_diffs".to_string(), self.screenshot_diffs.len());
        self.counts
            .insert("layout_audits".to_string(), self.layout_audits.len());
        self.counts.insert("traces".to_string(), self.traces.len());
        self.counts.insert(
            "physics_inspections".to_string(),
            self.physics_inspections.len(),
        );
        self.counts
            .insert("physics_overlays".to_string(), self.physics_overlays.len());
        self.counts.insert(
            "collision_reports".to_string(),
            self.collision_reports.len(),
        );
        self.counts
            .insert("surface_audits".to_string(), self.surface_audits.len());
        self.counts
            .insert("audio_artifacts".to_string(), self.audio_artifacts.len());
        self.counts
            .insert("audio_analyses".to_string(), self.audio_analyses.len());
        self.counts
            .insert("event_traces".to_string(), self.event_traces.len());
        self.counts
            .insert("visual_audits".to_string(), self.visual_audits.len());
        self.counts
            .insert("profiles".to_string(), self.profiles.len());
        self.counts.insert("logs".to_string(), self.logs.len());
        self.counts.insert("errors".to_string(), self.errors.len());
        self.counts
            .insert("warnings".to_string(), self.warnings.len());
        self.counts.insert(
            "unknown_artifacts".to_string(),
            self.unknown_artifacts.len(),
        );
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct ArtifactEntry {
    pub relative_path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub absolute_path: Option<String>,
    #[serde(rename = "type")]
    pub artifact_type: String,
    pub size_bytes: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mtime: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub linked_manifest_or_result: Option<String>,
    pub important_fields: Value,
    pub warnings: Vec<ArtifactFinding>,
    pub failures: Vec<ArtifactFinding>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ArtifactFinding {
    pub source: String,
    pub rule_id: String,
    pub severity: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

pub fn summarize_artifact_root(
    run_root: &Path,
    options: &ArtifactSummaryOptions,
) -> io::Result<ArtifactSummary> {
    let summary_path = options.write_summary.then(|| {
        run_root
            .join(&options.summary_filename)
            .to_string_lossy()
            .into_owned()
    });
    let mut summary = ArtifactSummary::empty(run_root, summary_path);
    let filters = options.normalized_type_filters();
    let expected = options.normalized_expected_types();

    if !run_root.is_dir() {
        summary.important_findings.push(ArtifactFinding {
            source: ".".to_string(),
            rule_id: "artifact_summary.run_root_missing".to_string(),
            severity: "error".to_string(),
            message: Some("artifact run root is missing".to_string()),
        });
        summary.refresh_counts();
        return Ok(summary);
    }

    let root_canonical = fs::canonicalize(run_root).unwrap_or_else(|_| run_root.to_path_buf());
    let files = collect_files(run_root)?;
    let mut unfiltered_counts: BTreeMap<String, usize> = SUMMARY_TYPES
        .iter()
        .map(|key| ((*key).to_string(), 0))
        .collect();
    let mut all_findings = Vec::new();

    for path in files {
        if path.file_name().and_then(|name| name.to_str()) == Some(&options.summary_filename) {
            continue;
        }
        let Ok(metadata) = fs::metadata(&path) else {
            continue;
        };
        if !metadata.is_file() {
            continue;
        }
        let relative_path = relative_slash_path(run_root, &path);
        let category = classify_artifact(&relative_path);
        *unfiltered_counts.entry(category.to_string()).or_insert(0) += 1;
        let mut entry = artifact_entry(
            run_root,
            &root_canonical,
            &path,
            &relative_path,
            category,
            &metadata,
        );
        all_findings.extend(entry.warnings.iter().cloned());
        all_findings.extend(entry.failures.iter().cloned());
        if category == "unknown_artifacts" {
            let finding = ArtifactFinding {
                source: relative_path.clone(),
                rule_id: "artifact_summary.unrecognized_artifact".to_string(),
                severity: "warning".to_string(),
                message: Some("artifact was not recognized by generic path contracts".to_string()),
            };
            entry.warnings.push(finding.clone());
            all_findings.push(finding);
        }
        if !filters.is_empty() && !filters.contains(category) {
            continue;
        }
        if options.warnings_only && entry.warnings.is_empty() && entry.failures.is_empty() {
            continue;
        }
        summary.push(category, entry);
    }

    for expected_type in expected {
        if unfiltered_counts.get(&expected_type).copied().unwrap_or(0) == 0 {
            all_findings.push(ArtifactFinding {
                source: ".".to_string(),
                rule_id: "artifact_summary.missing_expected_artifact".to_string(),
                severity: "warning".to_string(),
                message: Some(format!(
                    "expected artifact type '{expected_type}' was not found"
                )),
            });
        }
    }

    summary.important_findings = dedupe_findings(all_findings);
    summary.refresh_counts();
    if options.write_summary {
        let bytes = serde_json::to_vec_pretty(&summary)
            .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?;
        fs::write(run_root.join(&options.summary_filename), bytes)?;
    }
    Ok(summary)
}

pub fn collect_important_findings(summary: &ArtifactSummary) -> Vec<ArtifactFinding> {
    summary.important_findings.clone()
}

pub fn render_markdown_summary(summary: &ArtifactSummary) -> String {
    let mut out = String::new();
    out.push_str("# Artifact Summary\n\n");
    out.push_str("## Run root\n\n");
    out.push_str(&format!("{}\n\n", summary.run_root));
    out.push_str("## High-priority findings\n\n");
    if summary.important_findings.is_empty() {
        out.push_str("- None\n\n");
    } else {
        for finding in &summary.important_findings {
            out.push_str(&format!(
                "- [{}] {} `{}`",
                finding.severity, finding.rule_id, finding.source
            ));
            if let Some(message) = &finding.message {
                out.push_str(&format!(" - {}", compact_line(message)));
            }
            out.push('\n');
        }
        out.push('\n');
    }
    render_section(
        &mut out,
        "Screenshots and visual artifacts",
        &[
            ("screenshots", &summary.screenshots),
            ("screenshot_sequences", &summary.screenshot_sequences),
            ("contact_sheets", &summary.contact_sheets),
            ("screenshot_diffs", &summary.screenshot_diffs),
            ("layout_audits", &summary.layout_audits),
            ("visual_audits", &summary.visual_audits),
            ("snapshots", &summary.snapshots),
        ],
    );
    render_section(
        &mut out,
        "Runtime traces and physics",
        &[
            ("traces", &summary.traces),
            ("physics_inspections", &summary.physics_inspections),
            ("physics_overlays", &summary.physics_overlays),
            ("collision_reports", &summary.collision_reports),
            ("surface_audits", &summary.surface_audits),
            ("profiles", &summary.profiles),
        ],
    );
    render_section(
        &mut out,
        "Audio",
        &[
            ("audio_artifacts", &summary.audio_artifacts),
            ("audio_analyses", &summary.audio_analyses),
        ],
    );
    render_section(
        &mut out,
        "Events",
        &[("event_traces", &summary.event_traces)],
    );
    render_section(
        &mut out,
        "Logs and errors",
        &[
            ("scenario_results", &summary.scenario_results),
            ("logs", &summary.logs),
            ("errors", &summary.errors),
            ("warnings", &summary.warnings),
        ],
    );
    render_section(
        &mut out,
        "Unknown artifacts",
        &[("unknown_artifacts", &summary.unknown_artifacts)],
    );
    out
}

fn collect_files(root: &Path) -> io::Result<Vec<PathBuf>> {
    let mut files = Vec::new();
    collect_files_into(root, &mut files)?;
    files.sort_by(|a, b| a.to_string_lossy().cmp(&b.to_string_lossy()));
    Ok(files)
}

fn collect_files_into(root: &Path, files: &mut Vec<PathBuf>) -> io::Result<()> {
    for entry in fs::read_dir(root)? {
        let entry = entry?;
        let path = entry.path();
        let file_type = entry.file_type()?;
        if file_type.is_dir() {
            collect_files_into(&path, files)?;
        } else if file_type.is_file() {
            files.push(path);
        }
    }
    Ok(())
}

fn artifact_entry(
    run_root: &Path,
    root_canonical: &Path,
    path: &Path,
    relative_path: &str,
    category: &str,
    metadata: &fs::Metadata,
) -> ArtifactEntry {
    let absolute_path = safe_absolute_path(root_canonical, path);
    let mtime = metadata
        .modified()
        .ok()
        .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
        .map(|duration| duration.as_secs());
    let linked_manifest_or_result = linked_manifest_or_result(run_root, relative_path);
    let mut important_fields = Map::new();
    let mut warnings = Vec::new();
    let mut failures = Vec::new();
    if is_text_artifact(relative_path) {
        if metadata.len() > LARGE_TEXT_LIMIT_BYTES {
            warnings.push(ArtifactFinding {
                source: relative_path.to_string(),
                rule_id: "artifact_summary.large_artifact_skipped".to_string(),
                severity: "warning".to_string(),
                message: Some(format!(
                    "content summary skipped for {} byte artifact",
                    metadata.len()
                )),
            });
        } else if let Some(value) = read_json_value(path) {
            important_fields = important_fields_from_json(&value);
            let mut findings = findings_from_json(relative_path, category, &value);
            failures.append(&mut findings.failures);
            warnings.append(&mut findings.warnings);
        } else if category == "logs" || category == "errors" || category == "warnings" {
            let mut findings = findings_from_log(relative_path, path);
            failures.append(&mut findings.failures);
            warnings.append(&mut findings.warnings);
        }
    }
    ArtifactEntry {
        relative_path: relative_path.to_string(),
        absolute_path,
        artifact_type: category.to_string(),
        size_bytes: metadata.len(),
        mtime,
        linked_manifest_or_result,
        important_fields: Value::Object(important_fields),
        warnings,
        failures,
    }
}

fn safe_absolute_path(root_canonical: &Path, path: &Path) -> Option<String> {
    let canonical = fs::canonicalize(path).ok()?;
    canonical
        .starts_with(root_canonical)
        .then(|| canonical.to_string_lossy().into_owned())
}

fn linked_manifest_or_result(run_root: &Path, relative_path: &str) -> Option<String> {
    if relative_path == "result.json" || relative_path.ends_with("/result.json") {
        return Some(relative_path.to_string());
    }
    let result = run_root.join("result.json");
    result.is_file().then(|| "result.json".to_string())
}

fn relative_slash_path(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .components()
        .map(|component| component.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/")
}

fn normalize_artifact_type(value: &str) -> Option<String> {
    let normalized = value.trim().to_ascii_lowercase().replace('-', "_");
    let canonical = match normalized.as_str() {
        "" => return None,
        "scenario_result" | "scenario_results" | "result" | "results" => "scenario_results",
        "snapshot" | "snapshots" => "snapshots",
        "screenshot" | "screenshots" => "screenshots",
        "screenshot_sequence" | "screenshot_sequences" | "sequence" | "sequences" => {
            "screenshot_sequences"
        }
        "contact_sheet" | "contact_sheets" => "contact_sheets",
        "screenshot_diff" | "screenshot_diffs" | "diff" | "diffs" => "screenshot_diffs",
        "layout_audit" | "layout_audits" => "layout_audits",
        "trace" | "traces" | "runtime_trace" | "runtime_traces" => "traces",
        "physics" | "physics_inspection" | "physics_inspections" => "physics_inspections",
        "physics_overlay" | "physics_overlays" => "physics_overlays",
        "collision" | "collision_report" | "collision_reports" => "collision_reports",
        "surface" | "surface_audit" | "surface_audits" => "surface_audits",
        "audio" | "audio_artifact" | "audio_artifacts" => "audio_artifacts",
        "audio_analysis" | "audio_analyses" => "audio_analyses",
        "event" | "events" | "event_trace" | "event_traces" => "event_traces",
        "visual" | "visual_audit" | "visual_audits" => "visual_audits",
        "profile" | "profiles" => "profiles",
        "log" | "logs" => "logs",
        "error" | "errors" => "errors",
        "warning" | "warnings" => "warnings",
        "unknown" | "unknown_artifact" | "unknown_artifacts" => "unknown_artifacts",
        _ => return None,
    };
    Some(canonical.to_string())
}

fn classify_artifact(relative_path: &str) -> &'static str {
    let lower = relative_path.to_ascii_lowercase();
    let file_name = lower.rsplit('/').next().unwrap_or(&lower);
    let segments: Vec<&str> = lower.split('/').collect();
    if file_name == "result.json" || file_name == "summary.json" {
        return "scenario_results";
    }
    if lower.contains("/snapshots/") || lower.starts_with("snapshots/") {
        if file_name == "errors.json" {
            return "errors";
        }
        if is_image_name(file_name) && file_name.contains("screenshot") {
            return "screenshots";
        }
        return "snapshots";
    }
    if lower.contains("screenshot-sequence")
        || lower.contains("screenshot_sequence")
        || lower.contains("/sequences/")
        || file_name == "manifest.json" && segments.contains(&"screenshots")
    {
        return "screenshot_sequences";
    }
    if file_name.contains("contact") && file_name.contains("sheet") {
        return "contact_sheets";
    }
    if file_name.contains("diff") && (is_image_name(file_name) || file_name.ends_with(".json")) {
        return "screenshot_diffs";
    }
    if file_name.contains("layout") && file_name.contains("audit") && file_name.ends_with(".json") {
        return "layout_audits";
    }
    if file_name.contains("visual") && file_name.contains("audit") && file_name.ends_with(".json") {
        return "visual_audits";
    }
    if segments.contains(&"physics") && file_name.ends_with(".json") {
        if file_name.contains("overlay") || lower.contains("/overlays/") {
            return "physics_overlays";
        }
        return "physics_inspections";
    }
    if file_name.contains("physics") && file_name.contains("overlay") {
        return "physics_overlays";
    }
    if file_name.contains("collision") && file_name.ends_with(".json") {
        return "collision_reports";
    }
    if file_name.contains("surface") && file_name.contains("audit") && file_name.ends_with(".json")
    {
        return "surface_audits";
    }
    if segments.contains(&"audio") {
        if file_name.ends_with(".analysis.json") {
            return "audio_analyses";
        }
        if is_audio_name(file_name) {
            return "audio_artifacts";
        }
    }
    if file_name.ends_with(".analysis.json") && file_name.contains("audio") {
        return "audio_analyses";
    }
    if is_audio_name(file_name) {
        return "audio_artifacts";
    }
    if (file_name.contains("event") || lower.contains("/events/"))
        && (file_name.ends_with(".json") || file_name.ends_with(".jsonl"))
    {
        return "event_traces";
    }
    if (file_name.contains("trace") || lower.contains("/traces/"))
        && (file_name.ends_with(".json") || file_name.ends_with(".jsonl"))
    {
        return "traces";
    }
    if file_name == "profile.json" || lower.contains("/profiles/") {
        return "profiles";
    }
    if file_name.ends_with(".log") || file_name.ends_with(".txt") || file_name == "logs.txt" {
        if file_name.contains("error") {
            return "errors";
        }
        if file_name.contains("warning") {
            return "warnings";
        }
        return "logs";
    }
    if file_name.contains("error") && file_name.ends_with(".json") {
        return "errors";
    }
    if file_name.contains("warning") && file_name.ends_with(".json") {
        return "warnings";
    }
    if is_image_name(file_name)
        && (lower.contains("/screenshots/") || file_name.contains("screenshot"))
    {
        return "screenshots";
    }
    "unknown_artifacts"
}

fn is_image_name(file_name: &str) -> bool {
    file_name.ends_with(".png")
        || file_name.ends_with(".jpg")
        || file_name.ends_with(".jpeg")
        || file_name.ends_with(".webp")
}

fn is_audio_name(file_name: &str) -> bool {
    file_name.ends_with(".wav")
        || file_name.ends_with(".ogg")
        || file_name.ends_with(".mp3")
        || file_name.ends_with(".flac")
}

fn is_text_artifact(relative_path: &str) -> bool {
    let lower = relative_path.to_ascii_lowercase();
    lower.ends_with(".json")
        || lower.ends_with(".jsonl")
        || lower.ends_with(".txt")
        || lower.ends_with(".log")
}

fn read_json_value(path: &Path) -> Option<Value> {
    let raw = fs::read_to_string(path).ok()?;
    serde_json::from_str(&raw).ok()
}

fn important_fields_from_json(value: &Value) -> Map<String, Value> {
    let mut out = Map::new();
    collect_named_fields(value, "", &mut out);
    out
}

fn collect_named_fields(value: &Value, prefix: &str, out: &mut Map<String, Value>) {
    if out.len() >= MAX_IMPORTANT_FIELDS {
        return;
    }
    let Some(object) = value.as_object() else {
        return;
    };
    for key in [
        "ok",
        "status",
        "type",
        "kind",
        "rule_id",
        "severity",
        "message",
        "duration_seconds",
        "duration_ms",
        "sample_rate",
        "channels",
        "peak",
        "rms",
        "silence_ratio",
        "near_silence",
        "clipping_sample_count",
        "clipping_region_count",
        "low_information_frame_count",
        "failed_capture_count",
        "warning_count",
        "error_count",
    ] {
        if let Some(field) = object.get(key) {
            let name = if prefix.is_empty() {
                key.to_string()
            } else {
                format!("{prefix}.{key}")
            };
            out.entry(name).or_insert_with(|| bounded_value(field));
            if out.len() >= MAX_IMPORTANT_FIELDS {
                return;
            }
        }
    }
    for nested in [
        "summary",
        "metrics",
        "counts",
        "audio_analysis",
        "visual_audit",
    ] {
        if let Some(field) = object.get(nested) {
            let next = if prefix.is_empty() {
                nested.to_string()
            } else {
                format!("{prefix}.{nested}")
            };
            collect_named_fields(field, &next, out);
            if out.len() >= MAX_IMPORTANT_FIELDS {
                return;
            }
        }
    }
}

fn bounded_value(value: &Value) -> Value {
    match value {
        Value::String(text) if text.len() > 240 => {
            Value::String(format!("{}...", text.chars().take(240).collect::<String>()))
        }
        Value::Array(values) => Value::Array(values.iter().take(8).map(bounded_value).collect()),
        Value::Object(object) => Value::Object(
            object
                .iter()
                .take(8)
                .map(|(key, value)| (key.clone(), bounded_value(value)))
                .collect(),
        ),
        _ => value.clone(),
    }
}

#[derive(Default)]
struct SplitFindings {
    warnings: Vec<ArtifactFinding>,
    failures: Vec<ArtifactFinding>,
}

fn findings_from_json(source: &str, category: &str, value: &Value) -> SplitFindings {
    let mut out = SplitFindings::default();
    if value.get("ok").and_then(Value::as_bool) == Some(false) {
        out.failures.push(ArtifactFinding {
            source: source.to_string(),
            rule_id: "artifact_summary.result_failure_detected".to_string(),
            severity: "error".to_string(),
            message: message_from_value(value),
        });
    }
    collect_array_findings(source, category, value, &mut out);
    if category == "errors" && json_array_non_empty(value, "errors") {
        out.failures.push(ArtifactFinding {
            source: source.to_string(),
            rule_id: "artifact_summary.runtime_errors_detected".to_string(),
            severity: "error".to_string(),
            message: Some("runtime errors were recorded".to_string()),
        });
    }
    if category == "audio_analyses" {
        add_audio_findings(source, value, &mut out);
    }
    truncate_findings(&mut out);
    out
}

fn collect_array_findings(source: &str, category: &str, value: &Value, out: &mut SplitFindings) {
    match value {
        Value::Object(object) => {
            for (key, child) in object {
                if matches!(key.as_str(), "failures" | "errors") {
                    collect_finding_array(source, child, "error", category, out);
                } else if matches!(key.as_str(), "warnings" | "findings") {
                    collect_finding_array(source, child, "warning", category, out);
                } else {
                    collect_array_findings(source, category, child, out);
                }
            }
        }
        Value::Array(values) => {
            for child in values.iter().take(32) {
                collect_array_findings(source, category, child, out);
            }
        }
        _ => {}
    }
}

fn collect_finding_array(
    source: &str,
    value: &Value,
    default_severity: &str,
    category: &str,
    out: &mut SplitFindings,
) {
    let Some(values) = value.as_array() else {
        return;
    };
    for item in values.iter().take(MAX_FINDINGS_PER_ARTIFACT) {
        let severity = item
            .get("severity")
            .and_then(Value::as_str)
            .unwrap_or(default_severity);
        let rule_id = mapped_rule_id(category, severity);
        let finding = ArtifactFinding {
            source: source.to_string(),
            rule_id,
            severity: if severity == "error" || severity == "failure" {
                "error".to_string()
            } else {
                "warning".to_string()
            },
            message: message_from_value(item),
        };
        if finding.severity == "error" {
            out.failures.push(finding);
        } else {
            out.warnings.push(finding);
        }
    }
}

fn mapped_rule_id(category: &str, severity: &str) -> String {
    if severity == "error" || severity == "failure" {
        if category == "errors" {
            return "artifact_summary.runtime_errors_detected".to_string();
        }
        return "artifact_summary.result_failure_detected".to_string();
    }
    match category {
        "audio_analyses" | "audio_artifacts" => "artifact_summary.audio_warning_detected",
        "visual_audits"
        | "screenshots"
        | "screenshot_sequences"
        | "contact_sheets"
        | "screenshot_diffs"
        | "layout_audits" => "artifact_summary.visual_warning_detected",
        "physics_inspections" | "physics_overlays" | "collision_reports" | "surface_audits" => {
            "artifact_summary.physics_warning_detected"
        }
        "event_traces" => "artifact_summary.event_warning_detected",
        _ => "artifact_summary.result_failure_detected",
    }
    .to_string()
}

fn add_audio_findings(source: &str, value: &Value, out: &mut SplitFindings) {
    let clipping = value
        .pointer("/clipping_sample_count")
        .or_else(|| value.pointer("/analysis/clipping_sample_count"))
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let near_silence = value
        .pointer("/near_silence")
        .or_else(|| value.pointer("/analysis/near_silence"))
        .and_then(Value::as_bool)
        .unwrap_or(false);
    if clipping > 0 || near_silence {
        out.warnings.push(ArtifactFinding {
            source: source.to_string(),
            rule_id: "artifact_summary.audio_warning_detected".to_string(),
            severity: "warning".to_string(),
            message: Some("audio analysis reported clipping or near-silence risk".to_string()),
        });
    }
}

fn json_array_non_empty(value: &Value, key: &str) -> bool {
    value
        .get(key)
        .and_then(Value::as_array)
        .is_some_and(|values| !values.is_empty())
}

fn message_from_value(value: &Value) -> Option<String> {
    value
        .get("message")
        .and_then(Value::as_str)
        .or_else(|| value.get("summary").and_then(Value::as_str))
        .or_else(|| value.get("type").and_then(Value::as_str))
        .map(compact_line)
}

fn findings_from_log(source: &str, path: &Path) -> SplitFindings {
    let mut out = SplitFindings::default();
    let Ok(raw) = fs::read_to_string(path) else {
        return out;
    };
    for line in raw.lines().take(MAX_LOG_LINES) {
        let lower = line.to_ascii_lowercase();
        if lower.contains("error") || lower.contains("panic") || lower.contains("exception") {
            out.failures.push(ArtifactFinding {
                source: source.to_string(),
                rule_id: "artifact_summary.runtime_errors_detected".to_string(),
                severity: "error".to_string(),
                message: Some(compact_line(line)),
            });
        } else if lower.contains("warning") || lower.contains("warn:") {
            out.warnings.push(ArtifactFinding {
                source: source.to_string(),
                rule_id: "artifact_summary.result_failure_detected".to_string(),
                severity: "warning".to_string(),
                message: Some(compact_line(line)),
            });
        }
        if out.failures.len() + out.warnings.len() >= MAX_FINDINGS_PER_ARTIFACT {
            break;
        }
    }
    out
}

fn truncate_findings(findings: &mut SplitFindings) {
    findings.warnings.truncate(MAX_FINDINGS_PER_ARTIFACT);
    findings.failures.truncate(MAX_FINDINGS_PER_ARTIFACT);
}

fn dedupe_findings(findings: Vec<ArtifactFinding>) -> Vec<ArtifactFinding> {
    let mut seen = BTreeSet::new();
    let mut out = Vec::new();
    for finding in findings {
        let key = (
            finding.source.clone(),
            finding.rule_id.clone(),
            finding.severity.clone(),
            finding.message.clone(),
        );
        if seen.insert(key) {
            out.push(finding);
        }
    }
    out
}

fn compact_line(line: &str) -> String {
    let mut compact = line.split_whitespace().collect::<Vec<_>>().join(" ");
    if compact.len() > 240 {
        compact = format!("{}...", compact.chars().take(240).collect::<String>());
    }
    compact
}

fn render_section(out: &mut String, title: &str, groups: &[(&str, &Vec<ArtifactEntry>)]) {
    out.push_str(&format!("## {title}\n\n"));
    let mut wrote = false;
    for (name, entries) in groups {
        if entries.is_empty() {
            continue;
        }
        wrote = true;
        out.push_str(&format!("### {name}\n\n"));
        for entry in entries.iter().take(12) {
            out.push_str(&format!(
                "- `{}` ({} bytes)",
                entry.relative_path, entry.size_bytes
            ));
            let finding_count = entry.warnings.len() + entry.failures.len();
            if finding_count > 0 {
                out.push_str(&format!(" - {finding_count} finding(s)"));
            }
            out.push('\n');
        }
        if entries.len() > 12 {
            out.push_str(&format!("- ... {} more\n", entries.len() - 12));
        }
        out.push('\n');
    }
    if !wrote {
        out.push_str("- None\n\n");
    }
}
