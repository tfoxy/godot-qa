use super::{
    ScenarioIndex, find_scenarios_by_name, list_scenarios, normalize_scenario_document,
    validate_scenario_document,
};
use serde_json::json;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

fn temp_scenarios_dir(name: &str) -> PathBuf {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let path = std::env::temp_dir().join(format!("godot-qa-{name}-{unique}/qa/scenarios"));
    fs::create_dir_all(&path).unwrap();
    path
}

fn write_scenario(path: &Path, contents: &str) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(path, contents).unwrap();
}

#[test]
fn scenario_index_returns_every_duplicate_path_in_listing_order() {
    let dir = temp_scenarios_dir("index-duplicates");
    let second = dir.join("nested/b_second.yaml");
    let first = dir.join("a_first.yaml");
    let third = dir.join("z_third.json");
    write_scenario(&first, "name: shared\nsteps: []\n");
    write_scenario(&second, "name: shared\nsteps: []\n");
    write_scenario(&third, "{\"name\": \"shared\"}\n");

    let index = ScenarioIndex::load(&dir);
    assert_eq!(
        index.paths_for("shared"),
        find_scenarios_by_name(&dir, "shared").as_slice()
    );
    assert_eq!(
        index.paths_for("shared"),
        [first, second, third].as_slice(),
        "duplicate names resolve to every path, sorted by path"
    );
}

#[test]
fn scenario_index_excludes_abstract_templates() {
    let dir = temp_scenarios_dir("index-abstract");
    let concrete = dir.join("concrete.yaml");
    write_scenario(&concrete, "name: shared\nsteps: []\n");
    write_scenario(
        &dir.join("template.yaml"),
        "abstract: true\nname: shared\nsteps: []\n",
    );
    write_scenario(
        &dir.join("template.json"),
        "{\"abstract\": true, \"name\": \"shared\"}\n",
    );

    let index = ScenarioIndex::load(&dir);
    assert_eq!(index.paths_for("shared"), [concrete].as_slice());
}

#[test]
fn scenario_index_falls_back_to_file_stem_and_mixes_formats() {
    let dir = temp_scenarios_dir("index-fallback");
    let yaml_stem = dir.join("yaml_stem.yaml");
    let yml_stem = dir.join("yml_stem.yml");
    let json_stem = dir.join("json_stem.json");
    let json_named = dir.join("json_named.json");
    let broken = dir.join("broken.json");
    // No `name`, an empty `name`, and an unparsable document all fall back to
    // the file stem.
    write_scenario(&yaml_stem, "steps: []\n");
    write_scenario(&yml_stem, "name:\nsteps: []\n");
    write_scenario(&json_stem, "{}\n");
    write_scenario(&json_named, "{\"name\": \"Declared Name\"}\n");
    write_scenario(&broken, "{not json\n");

    let index = ScenarioIndex::load(&dir);
    assert_eq!(index.paths_for("yaml_stem"), [yaml_stem].as_slice());
    assert_eq!(index.paths_for("yml_stem"), [yml_stem].as_slice());
    assert_eq!(index.paths_for("json_stem"), [json_stem].as_slice());
    assert_eq!(index.paths_for("Declared Name"), [json_named].as_slice());
    assert_eq!(index.paths_for("broken"), [broken].as_slice());
    // Names match exactly; the stem of a declared-name file is not an alias.
    assert!(index.paths_for("json_named").is_empty());
    assert!(index.paths_for("missing").is_empty());
}

#[test]
fn scenario_index_agrees_with_listing_for_every_name() {
    let dir = temp_scenarios_dir("index-parity");
    write_scenario(&dir.join("a.yaml"), "name: Alpha\nsteps: []\n");
    write_scenario(&dir.join("nested/a.yaml"), "name: Alpha\nsteps: []\n");
    write_scenario(&dir.join("b.json"), "{\"name\": \"Beta\"}\n");
    write_scenario(&dir.join("stem.yml"), "steps: []\n");
    write_scenario(&dir.join("tpl.yaml"), "abstract: true\nname: Gamma\n");

    let listed = list_scenarios(&dir);
    assert_eq!(
        listed.len(),
        4,
        "abstract template stays out of the listing"
    );
    for entry in listed {
        let name = entry.get("name").and_then(|v| v.as_str()).unwrap();
        assert_eq!(
            ScenarioIndex::load(&dir).paths_for(name),
            find_scenarios_by_name(&dir, name).as_slice(),
            "index disagrees with find_scenarios_by_name for {name}"
        );
    }
    assert!(ScenarioIndex::load(&dir).paths_for("Gamma").is_empty());
}

#[test]
fn scenario_index_over_missing_directory_is_empty() {
    let dir = temp_scenarios_dir("index-missing").join("does_not_exist");
    let index = ScenarioIndex::load(&dir);
    assert!(index.paths_for("anything").is_empty());
    assert!(list_scenarios(&dir).is_empty());
}

#[test]
fn screenshot_match_with_multi_viewport_matrix_is_rejected() {
    let doc = json!({
        "name": "shot_matrix",
        "scene": "res://ui/main_menu.tscn",
        "viewports": [[1280, 720], [1920, 1080]],
        "steps": [{"snapshot": "after"}],
        "assert": [
            {"screenshot_match": {"snapshot": "after", "baseline": "qa/baselines/a.png"}}
        ],
    });
    let failure = validate_scenario_document(&doc, Path::new("scenario.json"))
        .expect_err("screenshot_match + multi-viewport must fail statically");
    assert_eq!(failure.failure.kind, "scenario_validation_failed");
    assert!(
        failure
            .failure
            .message
            .as_deref()
            .unwrap_or_default()
            .contains("screenshot_match does not support multi-viewport")
    );
}

#[test]
fn screenshot_match_with_single_viewport_is_accepted() {
    let doc = json!({
        "name": "shot_single",
        "scene": "res://ui/main_menu.tscn",
        "viewports": [[1280, 720]],
        "steps": [{"snapshot": "after"}],
        "assert": [
            {"screenshot_match": {"snapshot": "after", "baseline": "qa/baselines/a.png"}}
        ],
    });
    validate_scenario_document(&doc, Path::new("scenario.json"))
        .expect("screenshot_match with a single viewport should validate");
}

#[test]
fn layout_audit_with_single_viewport_is_accepted() {
    let doc = json!({
        "name": "layout_single",
        "scene": "res://ui/main_menu.tscn",
        "viewports": [[1280, 720]],
        "steps": [{"snapshot": "after"}],
        "assert": [{"layout_audit": "strict"}],
    });
    validate_scenario_document(&doc, Path::new("scenario.json"))
        .expect("single-viewport layout_audit should validate");
}

#[test]
fn validate_scenario_document_accepts_visual_assertions() {
    let doc = json!({
        "name": "visual_asserts_ok",
        "scene": "res://ui/main_menu.tscn",
        "steps": [],
        "assert": [
            { "visible_pixels": { "target": " qa:important_actor ", "snapshot": " occluded ", "min_ratio": 0.25, "min_count": 4 } },
            { "not_occluded": { "target": " qa:important_actor ", "snapshot": " occluded ", "occluder": " qa:hud_overlay ", "min_ratio": 0.1 } },
            { "renders_above": { "target": " qa:important_actor ", "other": " qa:background ", "snapshot": " order " } },
            { "renders_below": { "target": " qa:cursor ", "other": " qa:tooltip ", "snapshot": " order " } }
        ]
    });
    validate_scenario_document(&doc, Path::new("scenario.json"))
        .expect("visual assertions should validate");

    let normalized = normalize_scenario_document(&doc);
    let assertions = normalized
        .get("assert")
        .and_then(|value| value.as_array())
        .expect("normalized assertions array");
    assert_eq!(
        assertions[0]["visible_pixels"]["target"].as_str(),
        Some("qa:important_actor")
    );
    assert_eq!(
        assertions[0]["visible_pixels"]["snapshot"].as_str(),
        Some("occluded")
    );
    assert_eq!(
        assertions[1]["not_occluded"]["occluder"].as_str(),
        Some("qa:hud_overlay")
    );
    assert_eq!(
        assertions[2]["renders_above"]["other"].as_str(),
        Some("qa:background")
    );
    assert_eq!(
        assertions[3]["renders_below"]["target"].as_str(),
        Some("qa:cursor")
    );
}

#[test]
fn validate_scenario_document_accepts_saved_snapshot_pixels() {
    let doc = json!({
        "name": "saved_snapshot_pixels_ok",
        "scene": "res://ui/main_menu.tscn",
        "steps": [{"snapshot": " after "}],
        "assert": [
            { "saved_snapshot_pixels": { "target": " qa:important_actor ", "snapshot": " after ", "min_ratio": 0.25, "min_count": 4 } }
        ]
    });
    validate_scenario_document(&doc, Path::new("scenario.json"))
        .expect("saved snapshot pixel assertions should validate");

    let normalized = normalize_scenario_document(&doc);
    let assertion = &normalized["assert"][0]["saved_snapshot_pixels"];
    assert_eq!(assertion["target"].as_str(), Some("qa:important_actor"));
    assert_eq!(assertion["snapshot"].as_str(), Some("after"));
    assert_eq!(assertion["min_ratio"].as_f64(), Some(0.25));
    assert_eq!(assertion["min_count"].as_i64(), Some(4));

    let missing_threshold = json!({
        "name": "saved_snapshot_pixels_missing_threshold",
        "scene": "res://ui/main_menu.tscn",
        "steps": [{"snapshot": "after"}],
        "assert": [{ "saved_snapshot_pixels": { "target": "qa:actor", "snapshot": "after" } }]
    });
    assert!(validate_scenario_document(&missing_threshold, Path::new("scenario.json")).is_err());

    let bad_ratio = json!({
        "name": "saved_snapshot_pixels_bad_ratio",
        "scene": "res://ui/main_menu.tscn",
        "steps": [{"snapshot": "after"}],
        "assert": [{ "saved_snapshot_pixels": { "target": "qa:actor", "snapshot": "after", "min_ratio": 1.5 } }]
    });
    assert!(validate_scenario_document(&bad_ratio, Path::new("scenario.json")).is_err());
}

#[test]
fn validate_and_normalize_step_local_assertions() {
    let doc = json!({
        "name": "step_assertions_ok",
        "scene": "res://ui/main_menu.tscn",
        "steps": [
            {"wait_frames": 1},
            {
                "assert": [
                    {"visible": " qa:primary_start "},
                    {"text_contains": {"target": " qa:status ", "expected": "ready"}},
                    {"distance_between": {"a": " qa:a ", "b": " qa:b ", "min_distance": 12.0}},
                    {"max_delta_position": {"target": " qa:player ", "max_delta": 3.0}},
                    {"surface_matches": {"target": " qa:player ", "expected": "water"}},
                    {"surface_probe": {"provider": " qa:match_controller ", "probe_id": " sample ", "expected_surface": "water", "expected_walkable": true}},
                    {"renders_above": {"a": " qa:player ", "b": " qa:boat ", "snapshot": " water "}}
                ]
            }
        ]
    });
    validate_scenario_document(&doc, Path::new("scenario.json"))
        .expect("step-local assertions should validate");

    let normalized = normalize_scenario_document(&doc);
    let assertions = normalized["steps"][1]["assert"].as_array().unwrap();
    assert_eq!(
        assertions[0]["visible"].as_str(),
        Some(" qa:primary_start ")
    );
    assert_eq!(
        assertions[1]["text_contains"]["target"].as_str(),
        Some("qa:status")
    );
    assert_eq!(assertions[2]["distance"]["min"].as_f64(), Some(12.0));
    assert_eq!(
        assertions[3]["max_delta_position"]["max"].as_f64(),
        Some(3.0)
    );
    assert_eq!(
        assertions[3]["max_delta_position"]["window_frames"].as_i64(),
        Some(1)
    );
    assert_eq!(
        assertions[4]["surface_matches"]["surface"].as_str(),
        Some("water")
    );
    assert_eq!(
        assertions[5]["surface_probe"]["probe_id"].as_str(),
        Some("sample")
    );
    assert_eq!(
        assertions[6]["renders_above"]["target"].as_str(),
        Some("qa:player")
    );
    assert_eq!(
        assertions[6]["renders_above"]["other"].as_str(),
        Some("qa:boat")
    );
}

#[test]
fn validate_and_normalize_node_inspect_step() {
    let doc = json!({
        "name": "node_inspect_step_ok",
        "scene": "res://ui/main_menu.tscn",
        "steps": [{"node_inspect": " qa:start_button "}],
    });
    validate_scenario_document(&doc, Path::new("scenario.json"))
        .expect("node_inspect step should validate");

    let normalized = normalize_scenario_document(&doc);
    assert_eq!(
        normalized["steps"][0]["node_inspect"].as_str(),
        Some("qa:start_button")
    );

    let invalid = json!({
        "name": "node_inspect_step_invalid",
        "scene": "res://ui/main_menu.tscn",
        "steps": [{"node_inspect": "  "}],
    });
    let failure = validate_scenario_document(&invalid, Path::new("scenario.json"))
        .expect_err("empty node_inspect target should fail");
    assert_eq!(failure.failure.kind, "scenario_validation_failed");
    assert!(
        failure
            .failure
            .message
            .as_deref()
            .unwrap_or_default()
            .contains("node_inspect step value")
    );
}

#[test]
fn audio_scenario_document_accepts_and_normalizes_valid_audio_steps() {
    let doc = json!({
        "name": "audio_steps_ok",
        "scene": "res://ui/main_menu.tscn",
        "steps": [
            {
                "audio_capture": {
                    "name": " dodgeball_music ",
                    "duration_seconds": 4.0,
                    "bus": " Master ",
                    "analyze": true
                }
            },
            {
                "audio_export": {
                    "name": " generated_loop ",
                    "target": " qa:audio_controller ",
                    "method": " qa_export_audio_track ",
                    "args": ["dodgeball_rush_loop"],
                    "analyze": true
                }
            }
        ]
    });
    validate_scenario_document(&doc, Path::new("scenario.json"))
        .expect("well-formed audio steps should validate");

    let normalized = normalize_scenario_document(&doc);
    assert_eq!(
        normalized["steps"][0]["audio_capture"]["name"].as_str(),
        Some("dodgeball_music")
    );
    assert_eq!(
        normalized["steps"][0]["audio_capture"]["bus"].as_str(),
        Some("Master")
    );
    assert_eq!(
        normalized["steps"][1]["audio_export"]["name"].as_str(),
        Some("generated_loop")
    );
    assert_eq!(
        normalized["steps"][1]["audio_export"]["target"].as_str(),
        Some("qa:audio_controller")
    );
    assert_eq!(
        normalized["steps"][1]["audio_export"]["method"].as_str(),
        Some("qa_export_audio_track")
    );
}

#[test]
fn audio_scenario_document_rejects_unsafe_names() {
    let doc = json!({
        "name": "audio_bad_name",
        "scene": "res://ui/main_menu.tscn",
        "steps": [
            {"audio_capture": {"name": "../music", "duration_seconds": 1.0}}
        ]
    });
    assert!(validate_scenario_document(&doc, Path::new("scenario.json")).is_err());
}

#[test]
fn audio_scenario_document_rejects_non_qa_export_methods() {
    let doc = json!({
        "name": "audio_bad_method",
        "scene": "res://ui/main_menu.tscn",
        "steps": [
            {
                "audio_export": {
                    "name": "generated_loop",
                    "target": "qa:audio_controller",
                    "method": "export_audio_track"
                }
            }
        ]
    });
    assert!(validate_scenario_document(&doc, Path::new("scenario.json")).is_err());
}

#[test]
fn audio_scenario_document_rejects_invalid_durations() {
    for duration in [json!(0.0), json!(-1.0), json!(61.0), json!("1.0")] {
        let doc = json!({
            "name": "audio_bad_duration",
            "scene": "res://ui/main_menu.tscn",
            "steps": [
                {"audio_capture": {"name": "music", "duration_seconds": duration}}
            ]
        });
        assert!(validate_scenario_document(&doc, Path::new("scenario.json")).is_err());
    }
}

#[test]
fn audio_scenario_document_rejects_non_array_export_args() {
    let doc = json!({
        "name": "audio_bad_args",
        "scene": "res://ui/main_menu.tscn",
        "steps": [
            {
                "audio_export": {
                    "name": "generated_loop",
                    "target": "qa:audio_controller",
                    "method": "qa_export_audio_track",
                    "args": "dodgeball_rush_loop"
                }
            }
        ]
    });
    assert!(validate_scenario_document(&doc, Path::new("scenario.json")).is_err());
}

#[test]
fn validate_scenario_document_rejects_invalid_visual_assertions() {
    let bad_ratio = json!({
        "name": "bad_ratio",
        "scene": "res://ui/main_menu.tscn",
        "steps": [],
        "assert": [{ "visible_pixels": { "target": "qa:actor", "snapshot": "after", "min_ratio": 1.2 } }]
    });
    assert!(validate_scenario_document(&bad_ratio, Path::new("scenario.json")).is_err());

    let missing_target = json!({
        "name": "missing_target",
        "scene": "res://ui/main_menu.tscn",
        "steps": [],
        "assert": [{ "visible_pixels": { "min_count": 2 } }]
    });
    assert!(validate_scenario_document(&missing_target, Path::new("scenario.json")).is_err());

    let bad_count = json!({
        "name": "bad_count",
        "scene": "res://ui/main_menu.tscn",
        "steps": [],
        "assert": [{ "not_occluded": { "target": "qa:actor", "snapshot": "after", "occluder": "qa:overlay", "min_count": -1 } }]
    });
    assert!(validate_scenario_document(&bad_count, Path::new("scenario.json")).is_err());

    let missing_snapshot = json!({
        "name": "missing_snapshot",
        "scene": "res://ui/main_menu.tscn",
        "steps": [],
        "assert": [{ "visible_pixels": { "target": "qa:actor", "min_ratio": 0.25 } }]
    });
    assert!(validate_scenario_document(&missing_snapshot, Path::new("scenario.json")).is_err());

    let missing_threshold = json!({
        "name": "missing_threshold",
        "scene": "res://ui/main_menu.tscn",
        "steps": [],
        "assert": [{ "visible_pixels": { "target": "qa:actor", "snapshot": "after" } }]
    });
    assert!(validate_scenario_document(&missing_threshold, Path::new("scenario.json")).is_err());

    let missing_peer = json!({
        "name": "missing_peer",
        "scene": "res://ui/main_menu.tscn",
        "steps": [],
        "assert": [{ "renders_above": { "target": "qa:actor", "snapshot": "after" } }]
    });
    assert!(validate_scenario_document(&missing_peer, Path::new("scenario.json")).is_err());
}

#[test]
fn validate_and_normalize_snapshot_filters() {
    let doc = json!({
        "name": "snapshot_filters",
        "scene": "res://ui/main_menu.tscn",
        "steps": [{"snapshot": "after"}],
        "snapshots": {
            "save_tree": true,
            "save_ui": true,
            "save_screenshot": true,
            "save_logs": true,
            "fail_on_low_information": false,
            "target": " qa:start_button ",
            "max_depth": 1,
            "qa_only": true,
            "include_metadata": true,
            "metadata_keys": [" qa_id ", "state:current"]
        }
    });
    validate_scenario_document(&doc, Path::new("scenario.json"))
        .expect("snapshot filters should validate");
    let normalized = normalize_scenario_document(&doc);
    assert_eq!(
        normalized["snapshots"]["target"].as_str(),
        Some("qa:start_button")
    );
    assert_eq!(
        normalized["snapshots"]["metadata_keys"],
        json!(["qa_id", "state:current"])
    );
    assert_eq!(normalized["snapshots"]["save_tree"], true);
}

#[test]
fn validate_scenario_document_rejects_malformed_snapshot_filters() {
    let bad_depth = json!({
        "name": "bad_snapshot_depth",
        "scene": "res://ui/main_menu.tscn",
        "steps": [],
        "snapshots": {"max_depth": -1}
    });
    assert!(validate_scenario_document(&bad_depth, Path::new("scenario.json")).is_err());

    let bad_key = json!({
        "name": "bad_snapshot_key",
        "scene": "res://ui/main_menu.tscn",
        "steps": [],
        "snapshots": {"metadata_keys": ["qa_id", "../secret"]}
    });
    assert!(validate_scenario_document(&bad_key, Path::new("scenario.json")).is_err());
}

#[test]
fn validate_scenario_document_accepts_unfiltered_snapshot_compatibility() {
    let doc = json!({
        "name": "snapshot_compat",
        "scene": "res://ui/main_menu.tscn",
        "steps": [{"snapshot": "after"}],
        "snapshots": {
            "save_tree": true,
            "save_ui": true,
            "save_screenshot": false,
            "save_logs": true,
            "fail_on_low_information": true
        }
    });
    validate_scenario_document(&doc, Path::new("scenario.json"))
        .expect("legacy snapshot config should validate");
    let normalized = normalize_scenario_document(&doc);
    assert!(normalized["snapshots"].get("target").is_none());
    assert_eq!(normalized["snapshots"]["save_tree"], true);
}

#[test]
fn validate_scenario_document_accepts_and_preserves_execution_isolation() {
    let doc = json!({
        "name": "cold_only",
        "scene": "res://ui/main_menu.tscn",
        "steps": [],
        "execution": {"isolation": "cold"}
    });
    validate_scenario_document(&doc, Path::new("scenario.json"))
        .expect("execution.isolation: cold should validate");
    let normalized = normalize_scenario_document(&doc);
    assert_eq!(normalized["execution"]["isolation"], "cold");
}

#[test]
fn validate_scenario_document_rejects_invalid_execution_isolation() {
    let doc = json!({
        "name": "bad",
        "scene": "res://ui/main_menu.tscn",
        "steps": [],
        "execution": {"isolation": "hot"}
    });
    assert!(validate_scenario_document(&doc, Path::new("scenario.json")).is_err());
}
