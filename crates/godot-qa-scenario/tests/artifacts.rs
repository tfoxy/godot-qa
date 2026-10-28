use godot_qa_scenario::artifacts::prepare_run_artifacts;
use serde_json::json;
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

#[test]
fn prepare_run_artifacts_creates_expected_layout_and_metadata() {
    let root = temp_dir("run-artifacts");
    let scenario_path = root.join("qa/scenarios/examples/example.yaml");
    write(
        &scenario_path,
        "name: example\nscene: res://ui/main_menu.tscn\nsteps: []\n",
    );
    let scenario = json!({
        "name": "example",
        "scene": "res://ui/main_menu.tscn",
        "viewport": {"width": 1920, "height": 1080},
        "determinism": {},
        "snapshots": {},
        "godot_args": ["--rendering-driver", "opengl3"],
        "steps": [],
        "assert": [],
    });

    let artifacts = prepare_run_artifacts(
        &root.join(".qa/artifacts"),
        &scenario,
        &root,
        &scenario_path,
        "godot-qa scenario run example --json",
        None,
        &[],
        &[],
        Some("20260421T120000Z"),
        Some("2026-04-21T12:00:00Z"),
    )
    .unwrap();

    assert_eq!(
        artifacts.root_dir,
        root.join(".qa/artifacts/example/20260421T120000Z")
    );
    assert!(artifacts.snapshots_dir.is_dir());
    assert!(artifacts.temporal_dir.is_dir());
    assert!(artifacts.audio_dir.is_dir());
    assert!(artifacts.meta_path.is_file());
    assert!(artifacts.request_path.is_file());

    let meta: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&artifacts.meta_path).unwrap()).unwrap();
    let nonce = meta["run_nonce"].as_str().unwrap();
    assert_eq!(nonce.len(), 32);
    assert!(nonce.bytes().all(|byte| byte.is_ascii_hexdigit()));
    assert_eq!(
        meta,
        json!({
            "tool": "godot-qa",
            "tool_version": env!("CARGO_PKG_VERSION"),
            "run_id": "20260421T120000Z",
            "run_nonce": nonce,
            "project_root": root,
            "scenario": "example",
            "scene": "res://ui/main_menu.tscn",
            "viewport": [1920, 1080],
            "godot_binary": null,
            "godot_launch_prefix": [],
            "godot_windowed_launch_prefix": [],
            "godot_args": ["--rendering-driver", "opengl3"],
            "project_args": [],
            "started_at": "2026-04-21T12:00:00Z",
            "command": "godot-qa scenario run example --json",
            "scenario_path": scenario_path,
            "assertion_manifest": {"schema_version":1,"entries":[],"cli_assertions_excluded":0},
        })
    );

    let request: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&artifacts.request_path).unwrap()).unwrap();
    assert_eq!(
        request,
        json!({
            "scenario": scenario,
            "assertion_proof": {"schema_version":1,"entries":[],"cli_assertions_excluded":0},
            "artifacts": {
                "run_root": artifacts.root_dir,
                "meta_path": artifacts.meta_path,
                "request_path": artifacts.request_path,
                "result_path": artifacts.result_path,
                "logs_path": artifacts.logs_path,
                "progress": artifacts.progress_path,
                "progress_path": artifacts.progress_path,
                "slow_summary": artifacts.slow_summary_path,
                "snapshots_dir": artifacts.snapshots_dir,
                "temporal_dir": artifacts.temporal_dir,
                "audio_dir": artifacts.audio_dir,
            },
            "run": {
                "id": "20260421T120000Z",
                "nonce": nonce,
                "started_at": "2026-04-21T12:00:00Z",
                "command": "godot-qa scenario run example --json",
                "godot_binary": null,
                "godot_launch_prefix": [],
                "godot_windowed_launch_prefix": [],
                "godot_args": ["--rendering-driver", "opengl3"],
                "project_args": [],
            },
        })
    );
    assert_eq!(
        Path::new(request["artifacts"]["audio_dir"].as_str().unwrap()),
        root.join(".qa/artifacts/example/20260421T120000Z/audio")
    );
    let artifact_map = request["artifacts"].as_object().unwrap();
    for legacy_key in ["root", "meta", "request", "result", "logs"] {
        assert!(
            !artifact_map.contains_key(legacy_key),
            "legacy artifact key should be absent: {legacy_key}"
        );
    }
}

#[test]
fn scenario_godot_args_persist_in_run_artifacts() {
    let root = temp_dir("run-artifacts-launch-args");
    let scenario_path = root.join("qa/scenarios/examples/example.yaml");
    write(
        &scenario_path,
        "name: example\nscene: res://ui/main_menu.tscn\nsteps: []\n",
    );
    let scenario = json!({
        "name": "example",
        "scene": "res://ui/main_menu.tscn",
        "godot_args": ["--single-window"],
        "steps": [],
    });

    let artifacts = prepare_run_artifacts(
        &root.join(".qa/artifacts"),
        &scenario,
        &root,
        &scenario_path,
        "godot-qa scenario run example --json",
        None,
        &[],
        &[],
        Some("20260421T120001Z"),
        Some("2026-04-21T12:00:01Z"),
    )
    .unwrap();

    let meta: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&artifacts.meta_path).unwrap()).unwrap();
    let request: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&artifacts.request_path).unwrap()).unwrap();

    assert_eq!(meta["godot_args"], json!(["--single-window"]));
    assert!(meta.get("launch_args").is_none());
    assert_eq!(request["run"]["godot_args"], json!(["--single-window"]));
    assert!(request["run"].get("launch_args").is_none());
    assert_eq!(
        request["scenario"]["godot_args"],
        json!(["--single-window"])
    );
    assert!(request["scenario"].get("launch_args").is_none());
}

#[test]
fn scenario_project_args_persist_in_run_artifacts() {
    let root = temp_dir("run-artifacts-project-args");
    let scenario_path = root.join("qa/scenarios/examples/example.yaml");
    write(
        &scenario_path,
        "name: example\nscene: res://ui/main_menu.tscn\nsteps: []\n",
    );
    let scenario = json!({
        "name": "example",
        "scene": "res://ui/main_menu.tscn",
        "godot_args": ["--single-window"],
        "project_args": ["--save-slot", "qa", "--token", "abcdef", "API_KEY=supersecret"],
        "steps": [],
    });

    let artifacts = prepare_run_artifacts(
        &root.join(".qa/artifacts"),
        &scenario,
        &root,
        &scenario_path,
        "godot-qa scenario run example --json",
        None,
        &[],
        &[],
        Some("20260421T120002Z"),
        Some("2026-04-21T12:00:02Z"),
    )
    .unwrap();

    let meta: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&artifacts.meta_path).unwrap()).unwrap();
    let request: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&artifacts.request_path).unwrap()).unwrap();

    assert_eq!(meta["godot_args"], json!(["--single-window"]));
    assert_eq!(
        meta["project_args"],
        json!([
            "--save-slot",
            "qa",
            "--token",
            "<redacted:6>",
            "API_KEY=<redacted:11>"
        ])
    );
    assert!(meta.get("launch_args").is_none());
    assert_eq!(request["run"]["godot_args"], json!(["--single-window"]));
    assert_eq!(
        request["run"]["project_args"],
        json!([
            "--save-slot",
            "qa",
            "--token",
            "<redacted:6>",
            "API_KEY=<redacted:11>"
        ])
    );
    assert!(request["run"].get("launch_args").is_none());
    assert_eq!(
        request["scenario"]["project_args"],
        json!([
            "--save-slot",
            "qa",
            "--token",
            "abcdef",
            "API_KEY=supersecret"
        ])
    );
    assert!(request["scenario"].get("launch_args").is_none());
}
