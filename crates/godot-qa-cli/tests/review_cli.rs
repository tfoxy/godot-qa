use serde_json::{Value, json};
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

fn unique_suffix() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or(0);
    format!("{nanos}-{}", std::process::id())
}

fn make_project_root(prefix: &str) -> PathBuf {
    let root = env::temp_dir().join(format!("godot-qa-{prefix}-{}", unique_suffix()));
    fs::create_dir_all(&root).unwrap();
    fs::write(root.join("project.godot"), "[application]\n").unwrap();
    root
}

fn run_review(
    project_root: Option<&Path>,
    extra_args: &[&str],
) -> (std::process::ExitStatus, Value) {
    let mut command = Command::new(env!("CARGO_BIN_EXE_godot-qa"));
    command.arg("--json");
    if let Some(project_root) = project_root {
        command.arg("--project").arg(project_root);
    }
    let output = command.arg("review").args(extra_args).output().unwrap();
    let body = serde_json::from_slice(&output.stdout).unwrap_or_else(|error| {
        panic!(
            "invalid stdout JSON: {error}\nstdout: {}\nstderr: {}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        )
    });
    (output.status, body)
}

#[test]
fn review_list_templates_contract() {
    let (status, body) = run_review(None, &["list-templates"]);

    assert!(status.success());
    assert_eq!(body["ok"], true);
    assert_eq!(body["command"], "review list-templates");

    let templates = body["data"]["templates"].as_array().unwrap();
    let ids = templates
        .iter()
        .map(|template| template["id"].as_str().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(
        ids,
        vec![
            "scenario-smoke",
            "live-visual-review",
            "live-physics-review",
            "live-audio-review",
            "live-event-review",
            "visual-physics-audio",
            "deterministic-runtime-review",
        ]
    );

    for template in templates {
        assert!(template["description"].as_str().is_some());
        assert!(template["output_type"].as_str().is_some());
        assert!(template["generated_command_type"].as_str().is_some());
        assert!(template["required_parameters"].as_array().is_some());
        assert!(template["required_tools"].as_array().is_some());
    }
}

#[test]
fn review_init_cli_contracts() {
    let project_root = make_project_root("review-init");
    let out = ".qa/review-scenarios/smoke_review.json";
    let (status, body) = run_review(
        Some(&project_root),
        &[
            "init",
            "scenario-smoke",
            "--name",
            "smoke_review",
            "--scene",
            "res://ui/main_menu.tscn",
            "--target",
            "qa:menu",
            "--project-arg",
            "--seed",
            "--project-arg",
            "123456",
            "--frames",
            "90",
            "--screenshot-every",
            "30",
            "--audio-seconds",
            "12",
            "--trace-target",
            "qa:events",
            "--event-provider",
            "qa:event_provider",
            "--out",
            out,
        ],
    );

    assert!(status.success());
    assert_eq!(body["ok"], true);
    assert_eq!(body["command"], "review init");
    assert_eq!(body["data"]["template"], "scenario-smoke");
    assert_eq!(
        body["data"]["required_tools"],
        json!(["scenario run --file"])
    );
    assert_eq!(body["data"]["warnings"], json!([]));
    assert_eq!(body["data"]["project_args"], json!(["--seed", "123456"]));

    let output_file = project_root.join(out);
    let reported_output_file = body["data"]["output_file"].as_str().unwrap();
    assert_eq!(Path::new(reported_output_file), output_file);
    assert_eq!(
        body["data"]["run_command"],
        format!(
            "godot-qa scenario run --file {} --project <project> --json",
            reported_output_file
        )
    );
    assert!(output_file.is_file());

    let document: Value = serde_json::from_slice(&fs::read(&output_file).unwrap()).unwrap();
    assert_eq!(document["name"], "smoke_review");
    assert_eq!(document["scene"], "res://ui/main_menu.tscn");
    assert_eq!(document["project_args"], json!(["--seed", "123456"]));
    assert_eq!(document["steps"][0]["wait_frames"], 90);
    assert_eq!(document["review"]["template"], "scenario-smoke");
    assert_eq!(
        document["review"]["run_command"],
        body["data"]["run_command"]
    );

    fs::remove_dir_all(project_root).unwrap();
}

#[test]
fn review_init_deterministic_live_script_contract() {
    let project_root = make_project_root("review-init-deterministic");
    let out = ".qa/review-scripts/deterministic_review.json";
    let (status, body) = run_review(
        Some(&project_root),
        &[
            "init",
            "deterministic-runtime-review",
            "--name",
            "deterministic_review",
            "--scene",
            "res://ui/main_menu.tscn",
            "--target",
            "qa:main",
            "--project-arg",
            "--seed",
            "--project-arg",
            "123456",
            "--project-arg",
            "--save-slot=qa profile",
            "--frames",
            "60",
            "--screenshot-every",
            "15",
            "--out",
            out,
        ],
    );

    assert!(status.success(), "{body:#}");
    assert_eq!(body["ok"], true);
    assert_eq!(body["data"]["template"], "deterministic-runtime-review");
    assert_eq!(
        body["data"]["project_args"],
        json!(["--seed", "123456", "--save-slot=qa profile"])
    );
    assert_eq!(body["data"]["generated_command_type"], "session run-script");

    let output_file = project_root.join(out);
    let reported_output_file = body["data"]["output_file"].as_str().unwrap();
    assert_eq!(Path::new(reported_output_file), output_file);
    assert_eq!(
        body["data"]["run_command"],
        format!(
            "godot-qa session run-script {} --project <project> --json",
            reported_output_file
        )
    );

    let document: Value = serde_json::from_slice(&fs::read(&output_file).unwrap()).unwrap();
    assert_eq!(document["name"], "deterministic_review");
    assert_eq!(
        document["steps"][0]["session_start"]["project_args"],
        json!(["--seed", "123456", "--save-slot=qa profile"])
    );
    assert_eq!(
        document["steps"][0]["session_start"]["scene"],
        "res://ui/main_menu.tscn"
    );

    let ops = document["steps"]
        .as_array()
        .unwrap()
        .iter()
        .map(|step| {
            step.as_object()
                .and_then(|object| object.keys().next())
                .unwrap()
                .as_str()
        })
        .collect::<Vec<_>>();
    assert_eq!(
        ops,
        vec![
            "session_start",
            "wait",
            "screenshot_sequence",
            "trace_record",
            "errors",
            "logs",
            "no_errors",
            "session_stop",
        ]
    );
    assert_eq!(document["steps"][2]["screenshot_sequence"]["frames"], 60);
    assert_eq!(document["steps"][2]["screenshot_sequence"]["every"], 15);
    assert_eq!(
        document["steps"][3]["trace_record"]["targets"],
        json!(["qa:main"])
    );
    assert_eq!(document["steps"][3]["trace_record"]["physics"], false);

    for forbidden in [
        "racing",
        "race_review",
        "raft",
        "combat",
        "island",
        "RRR",
        "FrankenRaft",
    ] {
        assert!(
            !document.to_string().contains(forbidden),
            "generated deterministic template contained {forbidden}"
        );
    }

    fs::remove_dir_all(project_root).unwrap();
}

#[test]
fn review_init_visual_physics_audio_required_tools() {
    let project_root = make_project_root("review-init-vpa");
    let out = ".qa/review-scripts/current_review.json";
    let (status, body) = run_review(
        Some(&project_root),
        &[
            "init",
            "visual-physics-audio",
            "--name",
            "current_review",
            "--scene",
            "res://main.tscn",
            "--target",
            "qa:player",
            "--frames",
            "120",
            "--screenshot-every",
            "30",
            "--audio-seconds",
            "12",
            "--out",
            out,
        ],
    );

    assert!(status.success(), "{body:#}");
    assert_eq!(
        body["data"]["required_tools"],
        json!([
            "session run-script",
            "screenshot sequence",
            "physics collision-report",
            "audio capture"
        ])
    );
    assert_eq!(body["data"]["warnings"], json!([]));

    let output_file = project_root.join(out);
    let document: Value = serde_json::from_slice(&fs::read(&output_file).unwrap()).unwrap();
    let ops = document["steps"]
        .as_array()
        .unwrap()
        .iter()
        .map(|step| {
            step.as_object()
                .and_then(|object| object.keys().next())
                .unwrap()
                .as_str()
        })
        .collect::<Vec<_>>();
    assert_eq!(
        ops,
        vec![
            "session_start",
            "wait",
            "screenshot_sequence",
            "trace_record",
            "physics_collision_report",
            "audio_capture",
            "errors",
            "logs",
            "no_errors",
            "session_stop",
        ]
    );
    assert_eq!(document["steps"][2]["screenshot_sequence"]["frames"], 120);
    assert_eq!(document["steps"][2]["screenshot_sequence"]["every"], 30);
    assert_eq!(
        document["steps"][3]["trace_record"]["targets"],
        json!(["qa:player"])
    );
    assert_eq!(
        document["steps"][4]["physics_collision_report"]["target"],
        "qa:player"
    );
    assert_eq!(document["steps"][5]["audio_capture"]["seconds"], 12);
    assert_eq!(document["steps"][5]["audio_capture"]["analyze"], true);

    fs::remove_dir_all(project_root).unwrap();
}
