#![allow(unused_imports)]

use crate::args::*;
use crate::*;
use godot_qa_core::config::{ProjectConfig, load_project_config};
use godot_qa_core::results::{CommandResult, FailureDetail};
use godot_qa_runtime as runtime;
use godot_qa_scenario::{
    artifact_query, artifact_summary,
    audio::{AudioAnalysisOptions, analyze_wav_file_with_options},
    bundle as scenario_bundle, check as scenario_check, contact_sheet as scenario_contact_sheet,
    edit as scenario_edit, init as scenario_init, lint as scenario_lint, live_gate,
    record as scenario_record, review as scenario_review, run as scenario_run, scenario_doc,
    screenshots as scenario_screenshots, time_control, visual_audit, visual_isolate,
};
use notify::{Config as NotifyConfig, PollWatcher, RecursiveMode, Watcher};
use serde_json::{Map, Value, json};
use std::collections::BTreeSet;
use std::fmt::Write as _;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
    mpsc,
};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

pub(crate) fn visual_preflight_headless(args: &VisualPreflightArgs) -> bool {
    args.headless
}

pub(crate) fn visual_preflight_result(cli: &Cli, args: &VisualPreflightArgs) -> CommandResult {
    let config = load_project_config(
        cli.project.as_deref().map(Path::new),
        cli.godot_bin.as_deref(),
        cli.artifact_dir.as_deref(),
        cli.verbose,
        godot_qa_process::resolve_godot_binary(cli.godot_bin.as_deref()),
    );
    scenario_run::run_visual_preflight(
        &config,
        &visual_preflight_command_string(cli, args),
        visual_preflight_headless(args),
    )
}

pub(crate) fn visual_audit_result(cli: &Cli, args: &VisualAuditArgs) -> CommandResult {
    let command = "visual audit";
    let config = load_project_config(
        cli.project.as_deref().map(Path::new),
        cli.godot_bin.as_deref(),
        cli.artifact_dir.as_deref(),
        cli.verbose,
        godot_qa_process::resolve_godot_binary(cli.godot_bin.as_deref()),
    );
    let options = match visual_audit_options(args) {
        Ok(options) => options,
        Err(failure) => return visual_audit_failure_result(failure, 2),
    };
    let source_count = usize::from(args.live)
        + usize::from(args.screenshot.is_some())
        + usize::from(args.sequence.is_some())
        + usize::from(args.scenario.is_some());
    if source_count != 1 {
        return visual_audit_failure_result(
            FailureDetail {
                kind: "invalid_params".to_string(),
                command: Some(command.to_string()),
                message: Some(
                    "visual audit requires exactly one source mode: --live, --screenshot, --sequence, or --scenario".to_string(),
                ),
                expected: Some(json!("exactly one visual audit source mode")),
                actual: Some(json!({
                    "live": args.live,
                    "screenshot": args.screenshot,
                    "sequence": args.sequence,
                    "scenario": args.scenario,
                })),
                ..FailureDetail::default()
            },
            2,
        );
    }
    if args.live {
        let requested_targets = visual_audit_requested_targets(&args.target);
        if requested_targets.is_empty() {
            return visual_audit_failure_result(
                FailureDetail {
                    kind: "invalid_params".to_string(),
                    command: Some(command.to_string()),
                    message: Some("visual audit --live requires at least one --target".to_string()),
                    expected: Some(json!("one or more non-empty --target values")),
                    actual: Some(json!(args.target)),
                    ..FailureDetail::default()
                },
                2,
            );
        }
        let live_report = match visual_audit::audit_live_session(
            &config,
            command,
            &requested_targets,
            &options,
        ) {
            Ok(report) => report,
            Err(result) => return result,
        };
        let mut report = live_report.report;
        normalize_visual_audit_findings(command, &mut report.findings);
        let screenshot_path = PathBuf::from(&report.screenshot_path);
        let mut artifacts = Map::from_iter([(
            "screenshot".to_string(),
            json!(
                live_report
                    .capture
                    .screenshot_path
                    .to_string_lossy()
                    .into_owned()
            ),
        )]);
        if let Some(annotate_out) = args.annotate_out.as_deref() {
            write_visual_audit_annotation_artifact(
                &config,
                command,
                &options,
                &screenshot_path,
                annotate_out,
                &mut report,
                &mut artifacts,
            );
        }
        return visual_audit_success_result(
            command,
            report,
            &options,
            artifacts,
            Some(live_report.target_evidence),
            Some(live_report.capture.metadata),
        );
    }
    if args.sequence.is_some() {
        let requested_targets = visual_audit_requested_targets(&args.target);
        if requested_targets.is_empty() {
            return visual_audit_failure_result(
                FailureDetail {
                    kind: "invalid_params".to_string(),
                    command: Some(command.to_string()),
                    message: Some(
                        "visual audit --sequence requires at least one --target".to_string(),
                    ),
                    expected: Some(json!("one or more non-empty --target values")),
                    actual: Some(json!(args.target)),
                    ..FailureDetail::default()
                },
                2,
            );
        }
        let sequence_path = match required_project_path(
            &config.project_root,
            command,
            "--sequence",
            args.sequence.as_deref(),
        ) {
            Ok(path) => path,
            Err(failure) => return visual_audit_failure_result(failure, 2),
        };
        if !sequence_path.is_file() {
            return visual_audit_failure_result(
                FailureDetail {
                    kind: "sequence_manifest_not_found".to_string(),
                    command: Some(command.to_string()),
                    path: Some(sequence_path.to_string_lossy().into_owned()),
                    message: Some("visual audit sequence manifest was not found".to_string()),
                    expected: Some(json!("existing screenshot sequence manifest.json")),
                    actual: Some(json!("missing")),
                    ..FailureDetail::default()
                },
                1,
            );
        }
        let mut report = match visual_audit::audit_sequence_manifest(
            &sequence_path,
            &requested_targets,
            &options,
        ) {
            Ok(report) => report,
            Err(error) => {
                return visual_audit_failure_result(
                    FailureDetail {
                        kind: "visual_audit_failed".to_string(),
                        command: Some(command.to_string()),
                        path: Some(sequence_path.to_string_lossy().into_owned()),
                        message: Some(
                            "visual audit could not analyze the screenshot sequence".to_string(),
                        ),
                        expected: Some(json!(
                            "loadable sequence manifest, frame screenshots, and target evidence"
                        )),
                        actual: Some(json!(error)),
                        ..FailureDetail::default()
                    },
                    1,
                );
            }
        };
        normalize_visual_audit_findings(command, &mut report.findings);
        let artifacts = Map::from_iter([(
            "sequence_manifest".to_string(),
            json!(sequence_path.to_string_lossy().into_owned()),
        )]);
        return visual_audit_success_result(command, report, &options, artifacts, None, None);
    }
    if args.scenario.is_some() {
        let requested_targets = visual_audit_requested_targets(&args.target);
        let scenario_source =
            visual_audit_scenario_source(&config.project_root, args.scenario.as_deref());
        let mut report = match visual_audit::audit_scenario_artifacts(
            &config.artifacts_dir,
            scenario_source.as_deref(),
            args.run_id.as_deref(),
            &requested_targets,
            &options,
        ) {
            Ok(report) => report,
            Err(error) => {
                return visual_audit_failure_result(
                    FailureDetail {
                        kind: "visual_audit_failed".to_string(),
                        command: Some(command.to_string()),
                        path: Some(config.artifacts_dir.to_string_lossy().into_owned()),
                        message: Some(
                            "visual audit could not analyze saved scenario artifacts".to_string(),
                        ),
                        expected: Some(json!(
                            "scenario result.json with snapshot screenshot artifacts and target evidence"
                        )),
                        actual: Some(json!(error)),
                        ..FailureDetail::default()
                    },
                    1,
                );
            }
        };
        normalize_visual_audit_findings(command, &mut report.findings);
        let artifacts = Map::from_iter([
            (
                "scenario_result".to_string(),
                json!(report.screenshot_path.clone()),
            ),
            (
                "artifacts_dir".to_string(),
                json!(config.artifacts_dir.to_string_lossy().into_owned()),
            ),
        ]);
        return visual_audit_success_result(command, report, &options, artifacts, None, None);
    }
    let screenshot_path = match required_project_path(
        &config.project_root,
        command,
        "--screenshot",
        args.screenshot.as_deref(),
    ) {
        Ok(path) => path,
        Err(failure) => return visual_audit_failure_result(failure, 2),
    };
    if !screenshot_path.is_file() {
        return visual_audit_failure_result(
            FailureDetail {
                kind: "screenshot_not_found".to_string(),
                command: Some(command.to_string()),
                path: Some(screenshot_path.to_string_lossy().into_owned()),
                message: Some("visual audit screenshot file was not found".to_string()),
                expected: Some(json!("existing screenshot PNG file")),
                actual: Some(json!("missing")),
                ..FailureDetail::default()
            },
            1,
        );
    }

    let (target_bounds, targets_artifact) = if let Some(targets) = args.targets.as_deref() {
        let targets_path = match required_project_path(
            &config.project_root,
            command,
            "--targets",
            Some(targets),
        ) {
            Ok(path) => path,
            Err(failure) => return visual_audit_failure_result(failure, 2),
        };
        if !targets_path.is_file() {
            return visual_audit_failure_result(
                FailureDetail {
                    kind: "target_bounds_not_found".to_string(),
                    command: Some(command.to_string()),
                    path: Some(targets_path.to_string_lossy().into_owned()),
                    message: Some("visual audit target bounds file was not found".to_string()),
                    expected: Some(json!("existing target bounds JSON file")),
                    actual: Some(json!("missing")),
                    ..FailureDetail::default()
                },
                1,
            );
        }
        let target_bounds = match std::fs::read_to_string(&targets_path)
            .map_err(|error| error.to_string())
            .and_then(|contents| {
                serde_json::from_str::<Value>(&contents).map_err(|error| error.to_string())
            }) {
            Ok(value) => filter_visual_audit_targets(value, &args.target),
            Err(error) => {
                return visual_audit_failure_result(
                    FailureDetail {
                        kind: "target_bounds_invalid".to_string(),
                        command: Some(command.to_string()),
                        path: Some(targets_path.to_string_lossy().into_owned()),
                        message: Some(
                            "visual audit target bounds file could not be parsed".to_string(),
                        ),
                        expected: Some(json!("valid target bounds JSON")),
                        actual: Some(json!(error)),
                        ..FailureDetail::default()
                    },
                    1,
                );
            }
        };
        (
            target_bounds,
            json!(targets_path.to_string_lossy().into_owned()),
        )
    } else {
        let image = match image::open(&screenshot_path) {
            Ok(image) => image,
            Err(error) => {
                return visual_audit_failure_result(
                    FailureDetail {
                        kind: "screenshot_invalid".to_string(),
                        command: Some(command.to_string()),
                        path: Some(screenshot_path.to_string_lossy().into_owned()),
                        message: Some(
                            "visual audit screenshot file could not be opened".to_string(),
                        ),
                        expected: Some(json!("loadable screenshot PNG")),
                        actual: Some(json!(error.to_string())),
                        ..FailureDetail::default()
                    },
                    1,
                );
            }
        };
        let width = image.width();
        let height = image.height();
        (
            json!([{
                "target": "qa:full_screenshot",
                "target_rect": [0, 0, width, height],
                "visible_pixel_count": u64::from(width) * u64::from(height)
            }]),
            json!("implicit full screenshot target"),
        )
    };

    let mut report =
        match visual_audit::audit_screenshot(&screenshot_path, &target_bounds, &options) {
            Ok(report) => report,
            Err(error) => {
                return visual_audit_failure_result(
                    FailureDetail {
                        kind: "visual_audit_failed".to_string(),
                        command: Some(command.to_string()),
                        path: Some(screenshot_path.to_string_lossy().into_owned()),
                        message: Some(
                            "visual audit could not analyze the saved screenshot".to_string(),
                        ),
                        expected: Some(json!("loadable screenshot PNG and valid target bounds")),
                        actual: Some(json!(error)),
                        ..FailureDetail::default()
                    },
                    1,
                );
            }
        };
    normalize_visual_audit_findings(command, &mut report.findings);

    let mut artifacts = Map::from_iter([
        (
            "screenshot".to_string(),
            json!(screenshot_path.to_string_lossy().into_owned()),
        ),
        ("targets".to_string(), targets_artifact),
    ]);
    if let Some(annotate_out) = args.annotate_out.as_deref() {
        write_visual_audit_annotation_artifact(
            &config,
            command,
            &options,
            &screenshot_path,
            annotate_out,
            &mut report,
            &mut artifacts,
        );
    }

    visual_audit_success_result(command, report, &options, artifacts, None, None)
}

pub(crate) fn visual_isolate_result(cli: &Cli, args: &VisualIsolateArgs) -> CommandResult {
    let config = load_project_config(
        cli.project.as_deref().map(Path::new),
        cli.godot_bin.as_deref(),
        cli.artifact_dir.as_deref(),
        cli.verbose,
        godot_qa_process::resolve_godot_binary(cli.godot_bin.as_deref()),
    );
    if let Some(result) =
        run_setup_script_before(&config, "visual isolate", args.setup_script.as_deref())
    {
        return result;
    }
    let result = visual_isolate::isolate_visual_layers(
        &config,
        visual_isolate::VisualIsolateOptions {
            out: args.out.clone(),
            provider: args.provider.clone(),
            driver: args.driver.clone(),
            action: args.action.clone(),
            layers: args.layers.clone(),
            columns: args.columns,
            thumb_width: args.thumb_width,
            contact_sheet_out: args.contact_sheet_out.clone(),
        },
    );
    attach_setup_script_metadata(result, args.setup_script.as_deref())
}

pub(crate) fn visual_audit_success_result(
    command: &str,
    report: visual_audit::VisualAuditReport,
    options: &visual_audit::VisualAuditOptions,
    artifacts: Map<String, Value>,
    live_target_evidence: Option<Value>,
    live_screenshot_metadata: Option<Map<String, Value>>,
) -> CommandResult {
    let strict_failures = if options.strict_warnings {
        report.findings.clone()
    } else {
        Vec::new()
    };
    let ok = strict_failures.is_empty();
    let warnings = if options.strict_warnings {
        Vec::new()
    } else {
        report.findings.clone()
    };
    let mut visual_audit_data = Map::from_iter([
        ("screenshot_path".to_string(), json!(report.screenshot_path)),
        ("width".to_string(), json!(report.width)),
        ("height".to_string(), json!(report.height)),
        ("targets".to_string(), json!(report.targets)),
        ("findings".to_string(), json!(report.findings)),
        (
            "thresholds".to_string(),
            json!({
                "min_pixel_area": options.min_pixel_area,
                "min_width": options.min_width,
                "min_height": options.min_height,
                "min_contrast_ratio": options.min_contrast_ratio,
                "min_color_distance": options.min_color_distance,
                "min_edge_contrast": options.min_edge_contrast,
            }),
        ),
        ("strict".to_string(), json!(options.strict_warnings)),
        ("artifacts".to_string(), Value::Object(artifacts)),
    ]);
    if let Some(frames_checked) = report.frames_checked {
        visual_audit_data.insert("frames_checked".to_string(), json!(frames_checked));
    }
    if let Some(frames_with_target_missing) = report.frames_with_target_missing {
        visual_audit_data.insert(
            "frames_with_target_missing".to_string(),
            json!(frames_with_target_missing),
        );
    }
    if let Some(minimum_pixel_area_across_frames) = report.minimum_pixel_area_across_frames {
        visual_audit_data.insert(
            "minimum_pixel_area_across_frames".to_string(),
            json!(minimum_pixel_area_across_frames),
        );
    }
    if let Some(minimum_contrast_across_frames) = report.minimum_contrast_across_frames {
        visual_audit_data.insert(
            "minimum_contrast_across_frames".to_string(),
            json!(minimum_contrast_across_frames),
        );
    }
    if let Some(worst_frame_path_by_rule) = report.worst_frame_path_by_rule {
        visual_audit_data.insert(
            "worst_frame_path_by_rule".to_string(),
            json!(worst_frame_path_by_rule),
        );
    }
    if let Some(evidence) = live_target_evidence {
        visual_audit_data.insert("live_target_evidence".to_string(), evidence);
    }
    if let Some(metadata) = live_screenshot_metadata.filter(|metadata| !metadata.is_empty()) {
        visual_audit_data.insert(
            "live_screenshot_metadata".to_string(),
            Value::Object(metadata),
        );
    }
    CommandResult {
        ok,
        command: command.to_string(),
        message: if ok {
            "visual audit completed".to_string()
        } else {
            "visual audit completed with strict warning failures".to_string()
        },
        data: Map::from_iter([
            ("visual_audit".to_string(), Value::Object(visual_audit_data)),
            ("warnings".to_string(), json!(warnings)),
        ]),
        failures: strict_failures,
        exit_code: if ok { 0 } else { 1 },
    }
}

pub(crate) fn write_visual_audit_annotation_artifact(
    config: &ProjectConfig,
    command: &str,
    options: &visual_audit::VisualAuditOptions,
    screenshot_path: &Path,
    annotate_out: &str,
    report: &mut visual_audit::VisualAuditReport,
    artifacts: &mut Map<String, Value>,
) {
    let annotate_path = resolve_project_path(&config.project_root, annotate_out);
    match write_visual_audit_annotation(screenshot_path, &annotate_path, report) {
        Ok(()) => {
            artifacts.insert(
                "annotated".to_string(),
                json!(annotate_path.to_string_lossy().into_owned()),
            );
        }
        Err(error) => {
            let mut failure = FailureDetail {
                kind: visual_audit::RULE_ANNOTATED_ARTIFACT_FAILED.to_string(),
                command: Some(command.to_string()),
                path: Some(annotate_path.to_string_lossy().into_owned()),
                message: Some("visual audit annotated artifact could not be written".to_string()),
                expected: Some(json!("writable annotated PNG artifact path")),
                actual: Some(json!(error)),
                severity: Some(
                    if options.strict_warnings {
                        "error"
                    } else {
                        "warning"
                    }
                    .to_string(),
                ),
                artifacts: Some(Map::from_iter([(
                    "screenshot".to_string(),
                    json!(screenshot_path.to_string_lossy().into_owned()),
                )])),
                ..FailureDetail::default()
            };
            failure.details.insert(
                "rule_id".to_string(),
                json!(visual_audit::RULE_ANNOTATED_ARTIFACT_FAILED),
            );
            report.findings.push(failure);
        }
    }
}

pub(crate) fn visual_audit_failure_result(failure: FailureDetail, exit_code: i32) -> CommandResult {
    CommandResult {
        ok: false,
        command: "visual audit".to_string(),
        message: "visual audit failed".to_string(),
        data: Map::new(),
        failures: vec![failure],
        exit_code,
    }
}

pub(crate) fn required_project_path(
    project_root: &Path,
    command: &str,
    flag: &str,
    raw_path: Option<&str>,
) -> Result<PathBuf, FailureDetail> {
    let Some(raw_path) = raw_path.map(str::trim).filter(|path| !path.is_empty()) else {
        return Err(FailureDetail {
            kind: "invalid_params".to_string(),
            command: Some(command.to_string()),
            message: Some(format!("{command} requires {flag}")),
            expected: Some(json!(format!("non-empty {flag} path"))),
            actual: Some(json!(raw_path.unwrap_or_default())),
            ..FailureDetail::default()
        });
    };
    Ok(resolve_project_path(project_root, raw_path))
}

pub(crate) fn visual_audit_requested_targets(targets: &[String]) -> Vec<String> {
    targets
        .iter()
        .map(|target| target.trim())
        .filter(|target| !target.is_empty())
        .map(ToOwned::to_owned)
        .collect()
}

pub(crate) fn resolve_project_path(project_root: &Path, raw_path: &str) -> PathBuf {
    let path = PathBuf::from(raw_path);
    if path.is_absolute() {
        path
    } else {
        project_root.join(path)
    }
}

pub(crate) fn visual_audit_scenario_source(
    project_root: &Path,
    raw_scenario: Option<&str>,
) -> Option<String> {
    let scenario = raw_scenario?.trim();
    if scenario.is_empty() {
        return None;
    }
    let project_path = resolve_project_path(project_root, scenario);
    if project_path.exists() {
        Some(project_path.to_string_lossy().into_owned())
    } else {
        Some(scenario.to_string())
    }
}

pub(crate) fn visual_audit_options(
    args: &VisualAuditArgs,
) -> Result<visual_audit::VisualAuditOptions, FailureDetail> {
    let mut options = visual_audit::VisualAuditOptions {
        strict_warnings: args.strict_warnings,
        ..visual_audit::VisualAuditOptions::default()
    };
    if let Some(value) = args.min_pixel_area {
        options.min_pixel_area = value;
    }
    if let Some(value) = args.min_width {
        options.min_width = value;
    }
    if let Some(value) = args.min_height {
        options.min_height = value;
    }
    if let Some(value) = args.min_contrast_ratio {
        if !value.is_finite() || value < 1.0 {
            return Err(visual_audit_invalid_threshold_failure(
                "--min-contrast-ratio",
                json!("finite number greater than or equal to 1.0"),
                json!(value),
            ));
        }
        options.min_contrast_ratio = value;
    }
    if let Some(value) = args.min_color_distance {
        if !value.is_finite() || value < 0.0 {
            return Err(visual_audit_invalid_threshold_failure(
                "--min-color-distance",
                json!("finite number greater than or equal to 0.0"),
                json!(value),
            ));
        }
        options.min_color_distance = value;
    }
    Ok(options)
}

pub(crate) fn visual_audit_invalid_threshold_failure(
    flag: &str,
    expected: Value,
    actual: Value,
) -> FailureDetail {
    FailureDetail {
        kind: "invalid_threshold".to_string(),
        command: Some("visual audit".to_string()),
        message: Some(format!("{flag} is outside supported visual audit bounds")),
        expected: Some(expected),
        actual: Some(actual),
        ..FailureDetail::default()
    }
}

pub(crate) fn normalize_visual_audit_findings(command: &str, findings: &mut [FailureDetail]) {
    for finding in findings {
        finding.command = Some(command.to_string());
    }
}

pub(crate) fn filter_visual_audit_targets(value: Value, targets: &[String]) -> Value {
    let wanted = targets
        .iter()
        .map(|target| target.trim())
        .filter(|target| !target.is_empty())
        .collect::<BTreeSet<_>>();
    if wanted.is_empty() {
        return value;
    }
    match value {
        Value::Array(items) => Value::Array(
            items
                .into_iter()
                .filter(|item| visual_audit_target_matches(item, &wanted))
                .collect(),
        ),
        Value::Object(mut object) => {
            if let Some(Value::Array(items)) = object.remove("targets") {
                object.insert(
                    "targets".to_string(),
                    Value::Array(
                        items
                            .into_iter()
                            .filter(|item| visual_audit_target_matches(item, &wanted))
                            .collect(),
                    ),
                );
                return Value::Object(object);
            }
            Value::Object(
                object
                    .into_iter()
                    .filter(|(key, item)| {
                        wanted.contains(key.as_str()) || visual_audit_target_matches(item, &wanted)
                    })
                    .collect(),
            )
        }
        other => other,
    }
}

pub(crate) fn visual_audit_target_matches(item: &Value, wanted: &BTreeSet<&str>) -> bool {
    let Some(object) = item.as_object() else {
        return false;
    };
    ["target", "target_id", "qa_id", "locator"]
        .into_iter()
        .filter_map(|key| object.get(key).and_then(Value::as_str))
        .any(|value| wanted.contains(value))
        || object
            .get("visual_evidence")
            .and_then(Value::as_object)
            .and_then(|evidence| evidence.get("target").and_then(Value::as_str))
            .is_some_and(|value| wanted.contains(value))
}

pub(crate) fn write_visual_audit_annotation(
    screenshot_path: &Path,
    annotate_path: &Path,
    report: &visual_audit::VisualAuditReport,
) -> Result<(), String> {
    let mut image = image::open(screenshot_path)
        .map_err(|error| error.to_string())?
        .to_rgba8();
    for target in &report.targets {
        let Some(rect) = target.clamped_rect else {
            continue;
        };
        draw_visual_audit_rect(&mut image, rect);
    }
    if let Some(parent) = annotate_path.parent() {
        std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    image.save(annotate_path).map_err(|error| error.to_string())
}

pub(crate) fn draw_visual_audit_rect(image: &mut image::RgbaImage, rect: [u32; 4]) {
    let color = image::Rgba([255, 0, 0, 255]);
    let (image_width, image_height) = image.dimensions();
    if rect[2] == 0 || rect[3] == 0 || image_width == 0 || image_height == 0 {
        return;
    }
    let left = rect[0].min(image_width - 1);
    let top = rect[1].min(image_height - 1);
    let right = (rect[0] + rect[2]).saturating_sub(1).min(image_width - 1);
    let bottom = (rect[1] + rect[3]).saturating_sub(1).min(image_height - 1);
    for x in left..=right {
        image.put_pixel(x, top, color);
        image.put_pixel(x, bottom, color);
    }
    for y in top..=bottom {
        image.put_pixel(left, y, color);
        image.put_pixel(right, y, color);
    }
}
