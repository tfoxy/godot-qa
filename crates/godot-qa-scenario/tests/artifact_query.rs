use godot_qa_core::config::ProjectConfig;
use godot_qa_scenario::artifact_query::{QueryRequest, query_run_snapshot};
use godot_qa_scenario::assertions::AssertOptions;
use godot_qa_scenario::assertions::run_assert;
use image::{ImageBuffer, Rgba};
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

fn write_json(path: &Path, value: &Value) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(path, serde_json::to_string_pretty(value).unwrap()).unwrap();
}

fn save_png(path: &Path, width: u32, height: u32, fill: [u8; 4], rect: Option<[u32; 4]>) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    let mut image = ImageBuffer::from_pixel(width, height, Rgba(fill));
    if let Some([x, y, w, h]) = rect {
        for yy in y..(y + h).min(height) {
            for xx in x..(x + w).min(width) {
                image.put_pixel(xx, yy, Rgba([220, 40, 30, 255]));
            }
        }
    }
    image.save(path).unwrap();
}

fn write_run(root: &Path) -> PathBuf {
    let run_root = root.join(".qa/artifacts/combat_flow/100Z");
    write_json(
        &run_root.join("result.json"),
        &json!({
            "ok": true,
            "snapshots": ["impact"],
            "snapshot_artifacts": {
                "impact": {
                    "tree": run_root.join("snapshots/impact/tree.json"),
                    "ui": run_root.join("snapshots/impact/ui.json"),
                    "focus": run_root.join("snapshots/impact/focus.json"),
                    "errors": run_root.join("snapshots/impact/errors.json"),
                    "screenshot": run_root.join("snapshots/impact/screenshot.png")
                }
            }
        }),
    );
    write_json(
        &run_root.join("snapshots/impact/tree.json"),
        &json!({
            "root": {
                "path": "/root/Main",
                "name": "Main",
                "type": "Node2D",
                "qa_id": "",
                "visible": true,
                "inside_tree": true,
                "groups": [],
                "children": [
                    {
                        "path": "/root/Main/Projectile",
                        "name": "Projectile",
                        "type": "Area2D",
                        "qa_id": "weapon_projectile",
                        "visible": true,
                        "inside_tree": true,
                        "groups": ["qa_visible"],
                        "children": []
                    },
                    {
                        "path": "/root/Main/HiddenSpark",
                        "name": "HiddenSpark",
                        "type": "GPUParticles2D",
                        "qa_id": "weapon_hit_spark",
                        "visible": false,
                        "inside_tree": true,
                        "groups": [],
                        "children": []
                    }
                ]
            }
        }),
    );
    write_json(
        &run_root.join("snapshots/impact/ui.json"),
        &json!({"controls": [], "viewport": [1280, 720]}),
    );
    write_json(
        &run_root.join("snapshots/impact/focus.json"),
        &json!({"focused": null}),
    );
    write_json(
        &run_root.join("snapshots/impact/errors.json"),
        &json!({"errors": []}),
    );
    run_root
}

fn write_visual_run(root: &Path, include_target: bool, write_screenshot: bool) -> PathBuf {
    let run_root = root.join(".qa/artifacts/visual_case/100Z");
    write_json(
        &run_root.join("result.json"),
        &json!({
            "ok": true,
            "snapshots": ["occluded"],
            "snapshot_artifacts": {
                "occluded": {
                    "tree": run_root.join("snapshots/occluded/tree.json"),
                    "ui": run_root.join("snapshots/occluded/ui.json"),
                    "focus": run_root.join("snapshots/occluded/focus.json"),
                    "errors": run_root.join("snapshots/occluded/errors.json"),
                    "screenshot": run_root.join("snapshots/occluded/screenshot.png")
                }
            }
        }),
    );
    let children = if include_target {
        json!([{
            "path": "/root/Main/ImportantActor",
            "name": "ImportantActor",
            "type": "ColorRect",
            "qa_id": "important_actor",
            "visible": true,
            "inside_tree": true,
            "groups": [],
            "visual_evidence": {
                "coordinate_space": "viewport_2d",
                "rect": [10.0, 10.0, 40.0, 20.0],
                "rect_source": "control_global_rect",
                "target_hidden_supported": true
            },
            "children": []
        }])
    } else {
        json!([])
    };
    write_json(
        &run_root.join("snapshots/occluded/tree.json"),
        &json!({
            "root": {
                "path": "/root/Main",
                "name": "Main",
                "type": "Node",
                "qa_id": "",
                "visible": true,
                "inside_tree": true,
                "groups": [],
                "children": children
            }
        }),
    );
    write_json(
        &run_root.join("snapshots/occluded/ui.json"),
        &json!({"controls": [], "viewport": [1280, 720]}),
    );
    write_json(
        &run_root.join("snapshots/occluded/focus.json"),
        &json!({"focused": null}),
    );
    write_json(
        &run_root.join("snapshots/occluded/errors.json"),
        &json!({"errors": []}),
    );
    if write_screenshot {
        let screenshot = run_root.join("snapshots/occluded/screenshot.png");
        save_png(
            &screenshot,
            128,
            128,
            [12, 18, 24, 255],
            Some([10, 10, 40, 20]),
        );
    }
    run_root
}

fn config(root: &Path) -> ProjectConfig {
    ProjectConfig {
        project_root: root.to_path_buf(),
        repo_root: root.to_path_buf(),
        verbose: false,
        godot_binary: None,
        qa_dir: root.join("qa"),
        artifacts_dir: root.join(".qa/artifacts"),
        scenarios_dir: root.join("qa/scenarios"),
        addon_dir: root.join("addons/godot_qa"),
        repo_addon_dir: root.join("addons/godot_qa"),
        addon_identity_cache: Default::default(),
        godot_launch_prefix: Vec::new(),
        godot_windowed_launch_prefix: Vec::new(),
        godot_args: Vec::new(),
        godot_project_args: Vec::new(),
        godot_launch_mode: godot_qa_core::config::GodotLaunchMode::Auto,
        godot_audio_mode: godot_qa_core::config::GodotAudioMode::Auto,
        scenario_warm_default: false,
        scenario_warm_strict_default: false,
        scenario_jobs_default: None,
        project_config_failure: None,
        artifacts_keep_runs: None,
        scenario_schedule_default: None,
        log_ignore_patterns: Vec::new(),
        config_warnings: Vec::new(),
    }
}

#[test]
fn query_run_snapshot_matches_visible_qa_id() {
    let root = temp_dir("artifact-query-qa");
    let run_root = write_run(&root);

    let result = query_run_snapshot(
        &run_root,
        "impact",
        QueryRequest {
            target: Some("qa:weapon_projectile".to_string()),
            visible: true,
            ..QueryRequest::default()
        },
        "artifact query",
    )
    .unwrap();

    assert_eq!(result.matches.len(), 1);
    assert_eq!(result.matches[0].path, "/root/Main/Projectile");
    assert_eq!(result.matches[0].node_type, "Area2D");
    assert_eq!(
        result.matches[0].qa_id.as_deref(),
        Some("weapon_projectile")
    );
    assert!(result.matches[0].visible);
}

#[test]
fn query_run_snapshot_filters_by_type() {
    let root = temp_dir("artifact-query-type");
    let run_root = write_run(&root);

    let result = query_run_snapshot(
        &run_root,
        "impact",
        QueryRequest {
            node_type: Some("GPUParticles2D".to_string()),
            ..QueryRequest::default()
        },
        "artifact query",
    )
    .unwrap();

    assert_eq!(result.matches.len(), 1);
    assert_eq!(result.matches[0].name, "HiddenSpark");
    assert!(!result.matches[0].visible);
}

#[test]
fn filtered_query_reports_omitted_target_evidence() {
    let root = temp_dir("artifact-query-filtered-omitted");
    let run_root = write_run(&root);
    write_json(
        &run_root.join("snapshots/impact/tree.json"),
        &json!({
            "filter": {"target": "qa:main_menu_buttons", "max_depth": 0},
            "root": {
                "path": "/root/Main/MenuButtons",
                "name": "MenuButtons",
                "type": "VBoxContainer",
                "qa_id": "main_menu_buttons",
                "visible": true,
                "inside_tree": true,
                "groups": [],
                "children": []
            }
        }),
    );

    let failure = query_run_snapshot(
        &run_root,
        "impact",
        QueryRequest {
            target: Some("qa:start_button".to_string()),
            ..QueryRequest::default()
        },
        "artifact query",
    )
    .unwrap_err();

    assert_eq!(failure.kind, "snapshot_evidence_omitted");
    assert_eq!(failure.target.as_deref(), Some("qa:start_button"));
    assert_eq!(
        failure.actual.unwrap()["filter"]["target"],
        "qa:main_menu_buttons"
    );
}

#[test]
fn filtered_query_still_returns_available_subset() {
    let root = temp_dir("artifact-query-filtered-subset");
    let run_root = write_run(&root);
    write_json(
        &run_root.join("snapshots/impact/tree.json"),
        &json!({
            "filter": {"target": "qa:weapon_projectile", "max_depth": 0},
            "root": {
                "path": "/root/Main/Projectile",
                "name": "Projectile",
                "type": "Area2D",
                "qa_id": "weapon_projectile",
                "visible": true,
                "inside_tree": true,
                "groups": ["qa_visible"],
                "children": []
            }
        }),
    );

    let result = query_run_snapshot(
        &run_root,
        "impact",
        QueryRequest {
            target: Some("qa:weapon_projectile".to_string()),
            visible: true,
            ..QueryRequest::default()
        },
        "artifact query",
    )
    .unwrap();

    assert_eq!(result.matches.len(), 1);
    assert_eq!(result.matches[0].path, "/root/Main/Projectile");
}

#[test]
fn visual_snapshot_geometry_contract() {
    let root = temp_dir("artifact-query-visual-evidence");
    let run_root = write_run(&root);
    write_json(
        &run_root.join("snapshots/impact/tree.json"),
        &json!({
            "root": {
                "path": "/root/Main",
                "name": "Main",
                "type": "Node2D",
                "qa_id": "",
                "visible": true,
                "inside_tree": true,
                "groups": [],
                "visual_evidence": {
                    "coordinate_space": "viewport_2d",
                    "rect": [98.0, 118.0, 84.0, 24.0],
                    "rect_source": "line2d_points",
                    "target_hidden_supported": true
                },
                "children": [{
                    "path": "/root/Main/Indicator",
                    "name": "Indicator",
                    "type": "Line2D",
                    "qa_id": "line_indicator",
                    "visible": true,
                    "inside_tree": true,
                    "groups": ["qa_visible"],
                    "visual_evidence": {
                        "coordinate_space": "viewport_2d",
                        "rect": [98.0, 118.0, 84.0, 24.0],
                        "rect_source": "line2d_points",
                        "target_hidden_supported": true
                    },
                    "children": []
                }, {
                    "path": "/root/Main/Unsupported",
                    "name": "Unsupported",
                    "type": "Node",
                    "qa_id": "unsupported_visual_target",
                    "visible": true,
                    "inside_tree": true,
                    "groups": [],
                    "visual_evidence": {
                        "coordinate_space": "",
                        "rect": null,
                        "rect_source": "",
                        "target_hidden_supported": false,
                        "unsupported_reason": "target_is_not_canvas_item_and_has_no_qa_visual_rect"
                    },
                    "children": []
                }]
            }
        }),
    );

    let tree: Value = serde_json::from_str(
        &fs::read_to_string(run_root.join("snapshots/impact/tree.json")).unwrap(),
    )
    .unwrap();
    let indicator = &tree["root"]["children"][0]["visual_evidence"];
    assert_eq!(indicator["coordinate_space"], "viewport_2d");
    assert_eq!(indicator["rect"], json!([98.0, 118.0, 84.0, 24.0]));
    assert_eq!(indicator["rect_source"], "line2d_points");
    assert_eq!(indicator["target_hidden_supported"], true);
    assert!(indicator.get("unsupported_reason").is_none());
    assert_eq!(
        tree["root"]["children"][1]["visual_evidence"]["unsupported_reason"],
        "target_is_not_canvas_item_and_has_no_qa_visual_rect"
    );

    let result = query_run_snapshot(
        &run_root,
        "impact",
        QueryRequest {
            target: Some("qa:line_indicator".to_string()),
            visible: true,
            ..QueryRequest::default()
        },
        "artifact query",
    )
    .unwrap();

    assert_eq!(result.matches.len(), 1);
    assert_eq!(result.matches[0].node_type, "Line2D");

    write_json(
        &run_root.join("snapshots/impact/tree.json"),
        &json!({
            "filter": {"target": "qa:line_indicator", "max_depth": 0},
            "root": {
                "path": "/root/Main/Indicator",
                "name": "Indicator",
                "type": "Line2D",
                "qa_id": "line_indicator",
                "visible": true,
                "inside_tree": true,
                "groups": ["qa_visible"],
                "visual_evidence": {
                    "coordinate_space": "viewport_2d",
                    "rect": [98.0, 118.0, 84.0, 24.0],
                    "rect_source": "line2d_points",
                    "target_hidden_supported": true
                },
                "children": []
            }
        }),
    );

    let filtered = query_run_snapshot(
        &run_root,
        "impact",
        QueryRequest {
            target: Some("qa:line_indicator".to_string()),
            visible: true,
            ..QueryRequest::default()
        },
        "artifact query",
    )
    .unwrap();
    assert_eq!(filtered.matches.len(), 1);
    assert_eq!(filtered.matches[0].path, "/root/Main/Indicator");
}

#[test]
fn snapshot_visible_assertion_fails_when_target_is_hidden() {
    let root = temp_dir("artifact-query-assert-hidden");
    write_run(&root);

    let result = run_assert(
        &config(&root),
        "snapshot_visible",
        Some("qa:weapon_hit_spark"),
        None,
        AssertOptions::default(),
        false,
        Some("combat_flow"),
        Some("100Z"),
        Some("impact"),
    );

    assert!(!result.ok);
    assert_eq!(result.failures[0].kind, "snapshot_visible_mismatch");
    assert_eq!(
        result.failures[0].target.as_deref(),
        Some("qa:weapon_hit_spark")
    );
}

#[test]
fn snapshot_assertion_fails_when_filtered_evidence_omits_target() {
    let root = temp_dir("artifact-query-assert-filtered-omitted");
    let run_root = write_run(&root);
    write_json(
        &run_root.join("snapshots/impact/tree.json"),
        &json!({
            "filter": {"target": "qa:main_menu_buttons", "max_depth": 0},
            "root": {
                "path": "/root/Main/MenuButtons",
                "name": "MenuButtons",
                "type": "VBoxContainer",
                "qa_id": "main_menu_buttons",
                "visible": true,
                "inside_tree": true,
                "groups": [],
                "children": []
            }
        }),
    );

    let result = run_assert(
        &config(&root),
        "snapshot_visible",
        Some("qa:start_button"),
        None,
        AssertOptions::default(),
        false,
        Some("combat_flow"),
        Some("100Z"),
        Some("impact"),
    );

    assert!(!result.ok);
    assert_eq!(result.failures[0].kind, "snapshot_evidence_omitted");
    assert_eq!(
        result.failures[0].target.as_deref(),
        Some("qa:start_button")
    );
}

#[test]
fn snapshot_contains_type_assertion_passes_for_visible_projectile_type() {
    let root = temp_dir("artifact-query-assert-type");
    write_run(&root);

    let result = run_assert(
        &config(&root),
        "snapshot_contains_type",
        Some("Area2D"),
        None,
        AssertOptions::default(),
        false,
        Some("combat_flow"),
        Some("100Z"),
        Some("impact"),
    );

    assert!(result.ok, "{:?}", result.failures);
    assert_eq!(result.data["assertion"]["name"], "snapshot_contains_type");
    assert_eq!(result.data["matches"][0]["path"], "/root/Main/Projectile");
}

#[test]
fn visual_assertions_generate_windowed_scenario() {
    let root = temp_dir("visual-assert-generated");
    let result = run_assert(
        &config(&root),
        "visible_pixels",
        Some("qa:important_actor"),
        None,
        AssertOptions {
            min_ratio: Some(0.25),
            ..AssertOptions::default()
        },
        false,
        None,
        None,
        Some("occluded"),
    );

    assert!(!result.ok);
    assert_eq!(result.failures[0].kind, "main_scene_missing");
}

#[test]
fn visual_assertions_reject_invalid_snapshot_artifacts() {
    let root = temp_dir("visual-assert-artifacts");
    let run_root = root.join(".qa/artifacts/visual_case/100Z");
    write_json(
        &run_root.join("result.json"),
        &json!({
            "ok": true,
            "snapshots": ["occluded"]
        }),
    );
    write_json(
        &run_root.join("snapshots/occluded/tree.json"),
        &json!({
            "root": {
                "path": "/root/Main",
                "name": "Main",
                "type": "Node",
                "qa_id": "",
                "visible": true,
                "inside_tree": true,
                "groups": [],
                "children": []
            }
        }),
    );
    write_json(
        &run_root.join("snapshots/occluded/ui.json"),
        &json!({"controls": [], "viewport": [1280, 720]}),
    );
    write_json(
        &run_root.join("snapshots/occluded/focus.json"),
        &json!({"focused": null}),
    );
    write_json(
        &run_root.join("snapshots/occluded/errors.json"),
        &json!({"errors": []}),
    );

    let result = run_assert(
        &config(&root),
        "visible_pixels",
        Some("qa:important_actor"),
        None,
        AssertOptions {
            min_ratio: Some(0.25),
            ..AssertOptions::default()
        },
        false,
        Some("visual_case"),
        Some("100Z"),
        Some("missing"),
    );
    assert!(!result.ok);
    assert_eq!(result.failures[0].kind, "artifact_read_failed");
}

#[test]
fn saved_visual_assertion_rejects_missing_screenshot() {
    let root = temp_dir("visual-assert-missing-screenshot");
    write_visual_run(&root, true, false);

    let result = run_assert(
        &config(&root),
        "visible_pixels",
        Some("qa:important_actor"),
        None,
        AssertOptions {
            min_ratio: Some(0.25),
            ..AssertOptions::default()
        },
        false,
        Some("visual_case"),
        Some("100Z"),
        Some("occluded"),
    );

    assert!(!result.ok);
    assert_eq!(result.failures[0].kind, "missing_screenshot_image");
    assert_eq!(
        result.failures[0].target.as_deref(),
        Some("qa:important_actor")
    );
}

#[test]
fn saved_visual_assertion_rejects_missing_target() {
    let root = temp_dir("visual-assert-missing-target");
    write_visual_run(&root, false, true);

    let result = run_assert(
        &config(&root),
        "visible_pixels",
        Some("qa:important_actor"),
        None,
        AssertOptions {
            min_ratio: Some(0.25),
            ..AssertOptions::default()
        },
        false,
        Some("visual_case"),
        Some("100Z"),
        Some("occluded"),
    );

    assert!(!result.ok);
    assert_eq!(result.failures[0].kind, "target_not_found");
}

#[test]
fn saved_snapshot_pixels_passes_with_real_screenshot_and_rect_evidence() {
    let root = temp_dir("saved-snapshot-pixels-pass");
    write_visual_run(&root, true, true);

    let result = run_assert(
        &config(&root),
        "visible_pixels",
        Some("qa:important_actor"),
        None,
        AssertOptions {
            min_ratio: Some(0.25),
            ..AssertOptions::default()
        },
        false,
        Some("visual_case"),
        Some("100Z"),
        Some("occluded"),
    );

    assert!(result.ok, "{:?}", result.failures);
    assert!(
        result.data["visual_evidence"]["source_screenshot"]
            .as_str()
            .is_some()
    );
    assert_eq!(
        result.data["visual_evidence"]["target"],
        "qa:important_actor"
    );
    assert_eq!(
        result.data["visual_evidence"]["target_rect"],
        json!([10.0, 10.0, 40.0, 20.0])
    );
    assert_eq!(result.data["visual_evidence"]["useful_pixel_count"], 800);
    assert_eq!(result.data["visual_evidence"]["useful_pixel_ratio"], 1.0);
    assert_eq!(result.data["visual_evidence"]["min_ratio"], 0.25);
}

#[test]
fn saved_snapshot_pixels_fails_when_thresholds_are_not_met() {
    let root = temp_dir("saved-snapshot-pixels-threshold");
    write_visual_run(&root, true, true);

    let result = run_assert(
        &config(&root),
        "visible_pixels",
        Some("qa:important_actor"),
        None,
        AssertOptions {
            min_count: Some(801),
            min_ratio: Some(1.0),
            ..AssertOptions::default()
        },
        false,
        Some("visual_case"),
        Some("100Z"),
        Some("occluded"),
    );

    assert!(!result.ok);
    assert_eq!(result.failures[0].kind, "visible_pixels_threshold_mismatch");
    let actual = result.failures[0].actual.as_ref().unwrap();
    assert_eq!(actual["useful_pixel_count"], 800);
    assert_eq!(actual["min_count"], 801);
    assert!(actual["source_screenshot"].as_str().is_some());
}

#[test]
fn saved_snapshot_pixels_rejects_low_information_screenshot() {
    let root = temp_dir("saved-snapshot-pixels-low-info");
    let run_root = write_visual_run(&root, true, false);
    save_png(
        &run_root.join("snapshots/occluded/screenshot.png"),
        128,
        128,
        [0, 0, 0, 255],
        None,
    );

    let result = run_assert(
        &config(&root),
        "visible_pixels",
        Some("qa:important_actor"),
        None,
        AssertOptions {
            min_count: Some(1),
            ..AssertOptions::default()
        },
        false,
        Some("visual_case"),
        Some("100Z"),
        Some("occluded"),
    );

    assert!(!result.ok);
    assert_eq!(result.failures[0].kind, "screenshot_low_information");
    let actual = result.failures[0].actual.as_ref().unwrap();
    assert_eq!(actual["low_information"], true);
    assert_eq!(actual["low_information_reasons"][0], "flat_or_blank");
}

#[test]
fn saved_snapshot_pixels_rejects_unresolved_target() {
    let root = temp_dir("saved-snapshot-pixels-unresolved");
    write_visual_run(&root, false, true);

    let result = run_assert(
        &config(&root),
        "visible_pixels",
        Some("qa:important_actor"),
        None,
        AssertOptions {
            min_ratio: Some(0.25),
            ..AssertOptions::default()
        },
        false,
        Some("visual_case"),
        Some("100Z"),
        Some("occluded"),
    );

    assert!(!result.ok);
    assert_eq!(result.failures[0].kind, "target_not_found");
}

#[test]
fn saved_snapshot_pixels_failure_contract() {
    let invalid_threshold = run_assert(
        &config(&temp_dir("saved-snapshot-pixels-invalid-threshold")),
        "visible_pixels",
        Some("qa:important_actor"),
        None,
        AssertOptions {
            min_ratio: Some(1.5),
            ..AssertOptions::default()
        },
        false,
        Some("visual_case"),
        Some("100Z"),
        Some("occluded"),
    );
    assert!(!invalid_threshold.ok);
    assert_eq!(invalid_threshold.failures[0].kind, "invalid_assertion_args");
    assert!(invalid_threshold.failures[0].expected.is_some());
    assert!(invalid_threshold.failures[0].actual.is_some());

    let missing_screenshot_root = temp_dir("saved-snapshot-pixels-contract-missing-screenshot");
    write_visual_run(&missing_screenshot_root, true, false);
    let missing_screenshot = run_assert(
        &config(&missing_screenshot_root),
        "visible_pixels",
        Some("qa:important_actor"),
        None,
        AssertOptions {
            min_ratio: Some(0.25),
            ..AssertOptions::default()
        },
        false,
        Some("visual_case"),
        Some("100Z"),
        Some("occluded"),
    );
    assert!(!missing_screenshot.ok);
    assert_eq!(
        missing_screenshot.failures[0].kind,
        "missing_screenshot_image"
    );
    assert_eq!(
        missing_screenshot.failures[0].target.as_deref(),
        Some("qa:important_actor")
    );
    assert!(missing_screenshot.failures[0].artifacts.is_some());

    let stale_snapshot_root = temp_dir("saved-snapshot-pixels-contract-stale-snapshot");
    write_visual_run(&stale_snapshot_root, true, true);
    let stale_snapshot = run_assert(
        &config(&stale_snapshot_root),
        "visible_pixels",
        Some("qa:important_actor"),
        None,
        AssertOptions {
            min_ratio: Some(0.25),
            ..AssertOptions::default()
        },
        false,
        Some("visual_case"),
        Some("100Z"),
        Some("stale"),
    );
    assert!(!stale_snapshot.ok);
    assert_eq!(stale_snapshot.failures[0].kind, "artifact_read_failed");
    assert!(stale_snapshot.failures[0].expected.is_some());
    assert!(stale_snapshot.failures[0].actual.is_some());

    let low_information_root = temp_dir("saved-snapshot-pixels-contract-low-info");
    let low_information_run = write_visual_run(&low_information_root, true, false);
    save_png(
        &low_information_run.join("snapshots/occluded/screenshot.png"),
        128,
        128,
        [0, 0, 0, 255],
        None,
    );
    let low_information = run_assert(
        &config(&low_information_root),
        "visible_pixels",
        Some("qa:important_actor"),
        None,
        AssertOptions {
            min_count: Some(1),
            ..AssertOptions::default()
        },
        false,
        Some("visual_case"),
        Some("100Z"),
        Some("occluded"),
    );
    assert!(!low_information.ok);
    assert_eq!(
        low_information.failures[0].kind,
        "screenshot_low_information"
    );
    let low_information_actual = low_information.failures[0].actual.as_ref().unwrap();
    assert_eq!(low_information_actual["low_information"], true);
    assert_eq!(
        low_information_actual["low_information_reasons"][0],
        "flat_or_blank"
    );
    assert!(
        low_information.failures[0]
            .details
            .get("remediation")
            .and_then(Value::as_str)
            .is_some_and(|value| value.contains("real rendered viewport"))
    );

    let unsupported_root = temp_dir("saved-snapshot-pixels-contract-unsupported");
    let unsupported_run = write_visual_run(&unsupported_root, true, true);
    write_json(
        &unsupported_run.join("snapshots/occluded/tree.json"),
        &json!({
            "root": {
                "path": "/root/Main",
                "name": "Main",
                "type": "Node",
                "qa_id": "",
                "visible": true,
                "inside_tree": true,
                "groups": [],
                "children": [{
                    "path": "/root/Main/UnsupportedTarget",
                    "name": "UnsupportedTarget",
                    "type": "Node",
                    "qa_id": "unsupported_target",
                    "visible": true,
                    "inside_tree": true,
                    "groups": [],
                    "visual_evidence": {
                        "coordinate_space": "",
                        "rect": null,
                        "rect_source": "",
                        "target_hidden_supported": false,
                        "unsupported_reason": "target_is_not_canvas_item_and_has_no_qa_visual_rect",
                        "actual": {"class": "Node"}
                    },
                    "children": []
                }]
            }
        }),
    );
    let unsupported = run_assert(
        &config(&unsupported_root),
        "visible_pixels",
        Some("qa:unsupported_target"),
        None,
        AssertOptions {
            min_count: Some(1),
            ..AssertOptions::default()
        },
        false,
        Some("visual_case"),
        Some("100Z"),
        Some("occluded"),
    );
    assert!(!unsupported.ok);
    assert_eq!(unsupported.failures[0].kind, "unsupported_target_geometry");
    assert_eq!(
        unsupported.failures[0].target.as_deref(),
        Some("qa:unsupported_target")
    );
    assert!(
        unsupported.failures[0]
            .details
            .get("remediation")
            .and_then(Value::as_str)
            .is_some_and(|value| value.contains("viewport_2d"))
    );

    let unresolved_root = temp_dir("saved-snapshot-pixels-contract-unresolved");
    write_visual_run(&unresolved_root, false, true);
    let unresolved = run_assert(
        &config(&unresolved_root),
        "visible_pixels",
        Some("qa:important_actor"),
        None,
        AssertOptions {
            min_ratio: Some(0.25),
            ..AssertOptions::default()
        },
        false,
        Some("visual_case"),
        Some("100Z"),
        Some("occluded"),
    );
    assert!(!unresolved.ok);
    assert_eq!(unresolved.failures[0].kind, "target_not_found");
    assert_eq!(
        unresolved.failures[0].target.as_deref(),
        Some("qa:important_actor")
    );
}

#[test]
fn saved_visual_assertion_keeps_target_hidden_modes_unsupported() {
    let root = temp_dir("visual-assert-saved-unsupported");
    write_visual_run(&root, true, true);

    let result = run_assert(
        &config(&root),
        "not_occluded",
        Some("qa:important_actor"),
        None,
        AssertOptions {
            occluder: Some("qa:overlay".to_string()),
            ..AssertOptions::default()
        },
        false,
        Some("visual_case"),
        Some("100Z"),
        Some("occluded"),
    );

    assert!(!result.ok);
    assert_eq!(result.failures[0].kind, "unsupported_visual_assertion");
}
