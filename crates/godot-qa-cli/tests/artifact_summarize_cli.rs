use serde_json::{Value, json};
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

static NEXT_ID: AtomicU64 = AtomicU64::new(0);

fn unique_suffix() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or(0);
    format!(
        "{nanos}-{}-{}",
        std::process::id(),
        NEXT_ID.fetch_add(1, Ordering::Relaxed)
    )
}

fn write_json(path: &Path, value: &Value) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(path, serde_json::to_string_pretty(value).unwrap()).unwrap();
}

fn write_file(path: &Path, contents: &[u8]) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(path, contents).unwrap();
}

fn artifact_run() -> PathBuf {
    let root = env::temp_dir().join(format!(
        "godot-qa-artifact-summarize-cli-{}",
        unique_suffix()
    ));
    let run_root = root.join(".qa/artifacts/live/session-1");
    write_json(
        &run_root.join("result.json"),
        &json!({"ok": true, "command": "scenario.run"}),
    );
    write_file(&run_root.join("screenshots/current_screenshot.png"), b"png");
    write_json(
        &run_root.join("audio/current.analysis.json"),
        &json!({"analysis": {"clipping_sample_count": 2}}),
    );
    write_json(
        &run_root.join("events/event_trace.jsonl"),
        &json!({"events": []}),
    );
    run_root
}

fn summarize(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_godot-qa"))
        .arg("--json")
        .arg("artifact")
        .arg("summarize")
        .args(args)
        .output()
        .unwrap()
}

fn body(output: &Output) -> Value {
    serde_json::from_slice(&output.stdout).unwrap()
}

#[test]
fn artifact_summarize_cli_writes_default_json_and_command_contract() {
    let run_root = artifact_run();
    let output = summarize(&[run_root.to_str().unwrap()]);

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let body = body(&output);
    assert_eq!(body["ok"], true);
    assert_eq!(body["command"], "artifact.summarize");
    assert_eq!(body["data"]["run_root"], json!(run_root));
    assert_eq!(body["data"]["counts"]["screenshots"], 1);
    assert_eq!(body["data"]["counts"]["audio_analyses"], 1);

    let summary_path = run_root.join("artifact_summary.json");
    assert_eq!(body["data"]["summary_path"], json!(summary_path));
    assert!(summary_path.is_file(), "missing {}", summary_path.display());
    let summary: Value = serde_json::from_slice(&fs::read(summary_path).unwrap()).unwrap();
    assert_eq!(summary["screenshots"].as_array().unwrap().len(), 1);
    assert_eq!(summary["audio_analyses"].as_array().unwrap().len(), 1);
}

#[test]
fn artifact_summarize_cli_markdown_out_and_no_write_controls_artifacts() {
    let run_root = artifact_run();
    let markdown = run_root.join("review_summary.md");
    let output = summarize(&[
        run_root.to_str().unwrap(),
        "--markdown-out",
        markdown.to_str().unwrap(),
        "--no-write",
    ]);

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let body = body(&output);
    assert_eq!(body["data"]["summary_path"], Value::Null);
    assert_eq!(body["data"]["markdown_path"], json!(markdown));
    assert!(
        !run_root.join("artifact_summary.json").exists(),
        "summary JSON should be suppressed by --no-write"
    );
    let markdown_text = fs::read_to_string(markdown).unwrap();
    assert!(markdown_text.contains("# Artifact Summary"));
    assert!(markdown_text.contains("## High-priority findings"));
}

#[test]
fn artifact_summarize_cli_filters_warnings_and_expected_artifacts() {
    let run_root = artifact_run();
    let output = summarize(&[
        run_root.to_str().unwrap(),
        "--type",
        "audio_analysis",
        "--type",
        "screenshots",
        "--warnings-only",
        "--expect",
        "trace",
        "--expect",
        "contact_sheet",
    ]);

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let body = body(&output);
    assert_eq!(body["data"]["counts"]["audio_analyses"], 1);
    assert_eq!(body["data"]["counts"]["screenshots"], 0);
    assert_eq!(body["data"]["counts"]["event_traces"], 0);
    let findings = body["data"]["important_findings"].as_array().unwrap();
    assert!(findings.iter().any(|finding| {
        finding["rule_id"] == "artifact_summary.audio_warning_detected"
            && finding["source"] == "audio/current.analysis.json"
    }));
    assert!(findings.iter().any(|finding| {
        finding["rule_id"] == "artifact_summary.missing_expected_artifact"
            && finding["message"]
                .as_str()
                .unwrap()
                .contains("contact_sheets")
    }));
}

#[test]
fn artifact_summarize_cli_missing_root_reports_actionable_failure() {
    let missing = env::temp_dir()
        .join(format!(
            "godot-qa-artifact-summarize-cli-missing-{}",
            unique_suffix()
        ))
        .join("missing-run");
    let output = summarize(&[missing.to_str().unwrap()]);

    assert!(!output.status.success(), "missing root unexpectedly passed");
    let body = body(&output);
    assert_eq!(body["ok"], false);
    assert_eq!(body["command"], "artifact.summarize");
    assert_eq!(
        body["failures"][0]["type"],
        "artifact_summary.run_root_missing"
    );
    assert_eq!(body["failures"][0]["command"], "artifact.summarize");
    assert_eq!(body["failures"][0]["path"], json!(missing));
}

/// `.qa/artifacts/bundles/<bundle>/latest` is a relative symlink to the newest bundle run,
/// and the documented review command points straight at it. Summarizing through the link
/// must therefore behave exactly like summarizing the run directory it names.
#[cfg(unix)]
#[test]
fn artifact_summarize_accepts_the_bundle_latest_link() {
    let root = env::temp_dir().join(format!(
        "godot-qa-artifact-summarize-bundle-latest-{}",
        unique_suffix()
    ));
    let bundle_dir = root.join(".qa/artifacts/bundles/sample_regression_gate");
    let run_root = bundle_dir.join("1700000000");
    write_json(
        &run_root.join("result.json"),
        &json!({"ok": true, "summary": "summary.json"}),
    );
    write_json(
        &run_root.join("summary.json"),
        &json!({"bundle": "sample_regression_gate", "runs": [], "failures": []}),
    );
    write_json(
        &run_root.join("artifact-index.json"),
        &json!({"children": []}),
    );
    std::os::unix::fs::symlink("1700000000", bundle_dir.join("latest")).unwrap();

    let link = bundle_dir.join("latest");
    let output = summarize(&[
        link.to_str().unwrap(),
        "--expect",
        "scenario_results",
        "--no-write",
    ]);

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let body = body(&output);
    assert_eq!(body["ok"], true);
    assert_eq!(body["data"]["run_root"], json!(link));
    assert!(
        body["data"]["counts"]["scenario_results"]
            .as_u64()
            .is_some_and(|count| count >= 1)
    );
    let findings = body["data"]["important_findings"].as_array().unwrap();
    assert!(
        !findings
            .iter()
            .any(|finding| finding["rule_id"] == "artifact_summary.missing_expected_artifact"),
        "{findings:#?}"
    );

    fs::remove_dir_all(&root).unwrap();
}
