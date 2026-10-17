use godot_qa_scenario::artifact_summary::{
    ArtifactSummaryOptions, render_markdown_summary, summarize_artifact_root,
};
use serde_json::{Value, json};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

fn temp_dir(name: &str) -> PathBuf {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let path = std::env::temp_dir().join(format!("godot-qa-{name}-{unique}"));
    fs::create_dir_all(&path).unwrap();
    path
}

fn write(path: &Path, contents: &str) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(path, contents).unwrap();
}

fn write_json(path: &Path, value: &Value) {
    write(path, &serde_json::to_string_pretty(value).unwrap());
}

fn write_binary(path: &Path, contents: &[u8]) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(path, contents).unwrap();
}

fn mixed_artifact_root() -> PathBuf {
    let root = temp_dir("artifact-summary-mixed");
    write_json(
        &root.join("result.json"),
        &json!({
            "ok": false,
            "status": "failed",
            "failures": [{"type": "assertion_failed", "message": "button was hidden"}]
        }),
    );
    write_json(
        &root.join("snapshots/start/tree.json"),
        &json!({"ok": true}),
    );
    write_json(
        &root.join("snapshots/start/errors.json"),
        &json!({"errors": [{"message": "runtime script error"}]}),
    );
    write_binary(
        &root.join("snapshots/start/screenshot.png"),
        b"\x89PNG\r\nbinary",
    );
    write_json(
        &root.join("screenshots/menu/manifest.json"),
        &json!({"frame_count": 2, "warnings": [{"message": "low information frame"}]}),
    );
    write_binary(&root.join("contact-sheet.png"), b"\x89PNG\r\ncontact");
    write_binary(
        &root.join("screenshot-01-start.diff.png"),
        b"\x89PNG\r\ndiff",
    );
    write_json(
        &root.join("layout_audit.json"),
        &json!({"findings": [{"severity": "warning", "message": "control clipped"}]}),
    );
    write_json(
        &root.join("traces/runtime_trace.json"),
        &json!({"events": []}),
    );
    write_json(
        &root.join("physics/capture/physics.json"),
        &json!({"warnings": [{"message": "body asleep"}]}),
    );
    write_json(
        &root.join("physics/capture/overlay.json"),
        &json!({"overlay": true}),
    );
    write_json(
        &root.join("collision_report.json"),
        &json!({"warnings": [{"message": "pinball risk"}]}),
    );
    write_json(
        &root.join("surface_audit.json"),
        &json!({"findings": [{"severity": "warning", "message": "slippery floor"}]}),
    );
    write_binary(&root.join("audio/music.wav"), b"RIFF....WAVE");
    write_json(
        &root.join("audio/music.analysis.json"),
        &json!({"sample_rate": 44100, "channels": 2, "clipping_sample_count": 3}),
    );
    write_json(
        &root.join("events/runtime_events.jsonl"),
        &json!({"warnings": [{"message": "missing sfx correlation"}]}),
    );
    write_json(
        &root.join("visual_audit.json"),
        &json!({"findings": [{"severity": "warning", "message": "low contrast"}]}),
    );
    write_json(
        &root.join("profiles/profile.json"),
        &json!({"duration_ms": 12}),
    );
    write(
        &root.join("logs.txt"),
        "WARNING: slow frame\nERROR: runtime failure\n",
    );
    write_json(
        &root.join("warnings.json"),
        &json!({"warnings": [{"message": "runner warning"}]}),
    );
    write_binary(&root.join("manual/notes.bin"), b"\0\0\0not base64");
    root
}

#[test]
fn artifact_summary_populates_all_s76_categories_from_mixed_root() {
    let root = mixed_artifact_root();
    let summary = summarize_artifact_root(&root, &ArtifactSummaryOptions::default()).unwrap();
    let value = serde_json::to_value(&summary).unwrap();

    for key in [
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
    ] {
        assert!(value[key].is_array(), "missing array for {key}");
    }

    assert_eq!(summary.scenario_results.len(), 1);
    assert_eq!(summary.snapshots.len(), 1);
    assert_eq!(summary.screenshots.len(), 1);
    assert_eq!(summary.audio_artifacts.len(), 1);
    assert_eq!(summary.audio_analyses.len(), 1);
    assert_eq!(summary.unknown_artifacts.len(), 1);
    assert!(
        summary
            .important_findings
            .iter()
            .any(|finding| finding.rule_id == "artifact_summary.result_failure_detected")
    );
    assert!(
        summary
            .important_findings
            .iter()
            .any(|finding| finding.rule_id == "artifact_summary.runtime_errors_detected")
    );
    assert!(
        summary
            .important_findings
            .iter()
            .any(|finding| finding.rule_id == "artifact_summary.unrecognized_artifact")
    );
    assert!(root.join("artifact_summary.json").is_file());
}

#[test]
fn artifact_summary_filters_aliases_and_warnings_only() {
    let root = mixed_artifact_root();
    let options = ArtifactSummaryOptions {
        type_filters: vec!["screenshots".to_string(), "audio_analysis".to_string()],
        warnings_only: true,
        ..ArtifactSummaryOptions::default()
    };
    let summary = summarize_artifact_root(&root, &options).unwrap();

    assert!(summary.screenshots.is_empty());
    assert_eq!(summary.audio_analyses.len(), 1);
    assert!(summary.audio_artifacts.is_empty());
    assert!(
        summary.audio_analyses[0]
            .warnings
            .iter()
            .any(|finding| finding.rule_id == "artifact_summary.audio_warning_detected")
    );
}

#[test]
fn artifact_summary_reports_missing_expected_categories() {
    let root = temp_dir("artifact-summary-expect");
    write_json(&root.join("result.json"), &json!({"ok": true}));
    let options = ArtifactSummaryOptions {
        expected_types: vec!["screenshot_sequence".to_string(), "trace".to_string()],
        ..ArtifactSummaryOptions::default()
    };
    let summary = summarize_artifact_root(&root, &options).unwrap();

    let missing_count = summary
        .important_findings
        .iter()
        .filter(|finding| finding.rule_id == "artifact_summary.missing_expected_artifact")
        .count();
    assert_eq!(missing_count, 2);
}

#[test]
fn artifact_summary_markdown_has_required_sections_and_relative_paths() {
    let root = mixed_artifact_root();
    let summary = summarize_artifact_root(&root, &ArtifactSummaryOptions::default()).unwrap();
    let markdown = render_markdown_summary(&summary);

    for section in [
        "# Artifact Summary",
        "## Run root",
        "## High-priority findings",
        "## Screenshots and visual artifacts",
        "## Runtime traces and physics",
        "## Audio",
        "## Events",
        "## Logs and errors",
        "## Unknown artifacts",
    ] {
        assert!(markdown.contains(section), "missing section {section}");
    }
    assert!(markdown.contains("`snapshots/start/screenshot.png`"));
    assert!(!markdown.contains("\\x89PNG"));
    assert!(!markdown.contains("manual/notes.bin") || markdown.contains("`manual/notes.bin`"));
}

#[test]
fn artifact_summary_does_not_embed_binary_or_base64_payloads() {
    let root = mixed_artifact_root();
    let summary = summarize_artifact_root(&root, &ArtifactSummaryOptions::default()).unwrap();
    let value = serde_json::to_string(&summary).unwrap();

    assert!(value.contains("audio/music.wav"));
    assert!(value.contains("snapshots/start/screenshot.png"));
    assert!(!value.contains("RIFF....WAVE"));
    assert!(!value.contains("iVBOR"));
    assert!(!value.contains("not base64"));
}
