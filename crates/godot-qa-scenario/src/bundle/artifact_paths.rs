// Bundle artifact accounting: harvesting the artifact paths a child run
// produced (screenshots, diagnostics, failure evidence), the per-child entries
// and totals published as `data.artifacts`, and the small child-result readers
// the summary needs.
//
// Included into `bundle.rs`, so every item keeps its `crate::bundle::*` path.

fn artifact_string(artifacts: &Option<Value>, key: &str) -> Option<String> {
    artifacts
        .as_ref()
        .and_then(|value| value.get(key))
        .and_then(Value::as_str)
        .map(str::to_string)
}

fn child_project_args(child: &CommandResult) -> Value {
    child
        .data
        .get("project_args")
        .cloned()
        .unwrap_or_else(|| json!([]))
}

fn child_screenshot_paths(child: &CommandResult) -> Vec<String> {
    let mut paths = Vec::new();
    if let Some(snapshot_artifacts) = child.data.get("snapshot_artifacts") {
        collect_paths_by_key(
            snapshot_artifacts,
            &["screenshot", "screenshot_path"],
            &mut paths,
        );
    }
    for failure in &child.failures {
        if let Some(artifacts) = &failure.artifacts {
            collect_paths_by_key(
                &Value::Object(artifacts.clone()),
                &["screenshot", "screenshot_path"],
                &mut paths,
            );
        }
        collect_paths_by_key(
            &Value::Object(failure.details.clone()),
            &["screenshot", "screenshot_path"],
            &mut paths,
        );
    }
    dedupe_paths(paths)
}

fn child_diagnostic_paths(child: &CommandResult) -> Vec<String> {
    let mut paths = Vec::new();
    let keys = [
        "diagnostic",
        "diagnostic_path",
        "diagnostics",
        "diagnostics_path",
        "diff",
        "diff_path",
        "report",
        "report_path",
        "temporal_trace",
        "temporal_trace_path",
        "temporal_evidence",
        "temporal_evidence_path",
    ];
    if let Some(artifacts) = child.data.get("artifacts") {
        collect_paths_by_key(artifacts, &keys, &mut paths);
    }
    for failure in &child.failures {
        if let Some(artifacts) = &failure.artifacts {
            collect_paths_by_key(&Value::Object(artifacts.clone()), &keys, &mut paths);
        }
        collect_paths_by_key(&Value::Object(failure.details.clone()), &keys, &mut paths);
    }
    dedupe_paths(paths)
}

fn child_failure_artifact_paths_from_failures(failures: &[FailureDetail]) -> Vec<String> {
    let mut paths = Vec::new();
    for failure in failures {
        collect_failure_artifact_paths(failure, &mut paths);
    }
    dedupe_paths(paths)
}

fn collect_failure_artifact_paths(failure: &FailureDetail, out: &mut Vec<String>) {
    if let Some(path) = &failure.path {
        out.push(path.clone());
    }
    if let Some(artifacts) = &failure.artifacts {
        collect_path_like_values(&Value::Object(artifacts.clone()), out);
    }
    collect_path_like_values(&Value::Object(failure.details.clone()), out);
}

fn collect_path_like_values(value: &Value, out: &mut Vec<String>) {
    match value {
        Value::Object(map) => {
            for (key, value) in map {
                if is_path_like_key(key) {
                    collect_string_values(value, out);
                }
                collect_path_like_values(value, out);
            }
        }
        Value::Array(values) => {
            for value in values {
                collect_path_like_values(value, out);
            }
        }
        _ => {}
    }
}

fn is_path_like_key(key: &str) -> bool {
    matches!(key, "path" | "paths" | "artifact" | "artifacts")
        || key.ends_with("_path")
        || key.ends_with("_paths")
}

fn collect_paths_by_key(value: &Value, keys: &[&str], out: &mut Vec<String>) {
    match value {
        Value::Object(map) => {
            for (key, value) in map {
                if keys.contains(&key.as_str()) {
                    collect_string_values(value, out);
                }
                collect_paths_by_key(value, keys, out);
            }
        }
        Value::Array(values) => {
            for value in values {
                collect_paths_by_key(value, keys, out);
            }
        }
        _ => {}
    }
}

fn collect_string_values(value: &Value, out: &mut Vec<String>) {
    match value {
        Value::String(path) => out.push(path.clone()),
        Value::Array(values) => {
            for value in values {
                collect_string_values(value, out);
            }
        }
        Value::Object(map) => {
            for value in map.values() {
                collect_string_values(value, out);
            }
        }
        _ => {}
    }
}

fn dedupe_paths(paths: Vec<String>) -> Vec<String> {
    let mut seen = HashSet::new();
    let mut deduped = Vec::new();
    for path in paths {
        if seen.insert(path.clone()) {
            deduped.push(path);
        }
    }
    deduped
}

fn bundle_artifact_summary(
    runs: &[BundleRunSummary],
    warnings: &[FailureDetail],
) -> BundleArtifactSummary {
    let mut children: Vec<BundleArtifactChildEntry> = runs
        .iter()
        .map(|run| BundleArtifactChildEntry {
            scenario: run.scenario.clone(),
            required: run.required,
            status: run.status.clone(),
            source_bundle: run.source_bundle.clone(),
            bundle_chain: run.bundle_chain.clone(),
            run_root: run.run_root.clone(),
            result_path: run.result_path.clone(),
            logs_path: run.logs_path.clone(),
            screenshot_paths: run.screenshot_paths.clone(),
            diagnostic_paths: run.diagnostic_paths.clone(),
            failure_artifact_paths: child_failure_artifact_paths_from_failures(&run.failures),
            warning: None,
        })
        .collect();

    for warning in warnings {
        if warning.kind != "missing_scenario" {
            continue;
        }
        let scenario = warning
            .actual
            .as_ref()
            .and_then(Value::as_str)
            .or(warning.target.as_deref())
            .unwrap_or("unknown")
            .to_string();
        children.push(BundleArtifactChildEntry {
            scenario,
            required: false,
            status: "missing_optional".to_string(),
            source_bundle: warning
                .details
                .get("source_bundle")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string(),
            bundle_chain: warning
                .details
                .get("bundle_chain")
                .and_then(Value::as_array)
                .map(|values| {
                    values
                        .iter()
                        .filter_map(Value::as_str)
                        .map(str::to_string)
                        .collect()
                })
                .unwrap_or_default(),
            run_root: None,
            result_path: None,
            logs_path: None,
            screenshot_paths: Vec::new(),
            diagnostic_paths: Vec::new(),
            failure_artifact_paths: {
                let mut paths = Vec::new();
                collect_failure_artifact_paths(warning, &mut paths);
                dedupe_paths(paths)
            },
            warning: Some(warning.clone()),
        });
    }

    BundleArtifactSummary { children }
}

fn child_cleanup_status(child: &CommandResult) -> Option<Value> {
    child
        .data
        .get("process")
        .and_then(|process| process.get("cleanup"))
        .cloned()
}

fn child_display_lifecycle(child: &CommandResult) -> Option<Value> {
    child.data.get("launch_display").map(|launch_display| {
        json!({
            "status": if child.ok { "completed" } else { "failed" },
            "launch_display": launch_display,
        })
    })
}

fn child_failure_category(child: &CommandResult) -> Option<String> {
    child.failures.iter().find_map(|failure| {
        failure
            .details
            .get("category")
            .or_else(|| failure.details.get("failure_category"))
            .and_then(Value::as_str)
            .map(str::to_string)
    })
}

fn child_retry_guidance(child: &CommandResult) -> Option<String> {
    child.failures.iter().find_map(|failure| {
        failure
            .details
            .get("retry_guidance")
            .and_then(Value::as_str)
            .map(str::to_string)
    })
}

fn is_retryable_startup_infrastructure_failure(child: &CommandResult) -> bool {
    // Once this run wrote runtime progress, a later display-looking log line
    // cannot justify replaying gameplay or teardown as a startup retry.
    if child
        .data
        .get("artifacts")
        .and_then(|artifacts| artifacts.get("progress").or_else(|| artifacts.get("progress_path")))
        .and_then(Value::as_str)
        .is_some_and(|path| std::fs::metadata(path).is_ok_and(|meta| meta.len() > 0))
    {
        return false;
    }
    child.failures.iter().any(|failure| {
        let category = failure
            .details
            .get("category")
            .or_else(|| failure.details.get("failure_category"))
            .and_then(Value::as_str);
        let retryable = failure
            .details
            .get("retryable")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let infrastructure_kind = failure.details.get("infrastructure_failure_kind");
        failure.kind == "scenario_result_missing"
            && category == Some("display_infrastructure")
            && retryable
            && infrastructure_kind.is_some()
    })
}
