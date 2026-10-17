#![cfg(target_os = "linux")]

use serde_json::Value;
use std::fs;
use std::os::unix::fs::symlink;
use std::path::{Path, PathBuf};
use std::process::Command;

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .unwrap()
        .to_path_buf()
}

fn isolated_fixture_project() -> PathBuf {
    let root = std::env::temp_dir().join(format!(
        "godot-qa-presented-test-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir_all(root.join("addons")).unwrap();
    fs::create_dir_all(root.join("ui/fixtures")).unwrap();
    fs::write(root.join("project.godot"), "config_version=5\n\n[application]\nconfig/name=\"Presented fixture\"\nrun/main_scene=\"res://ui/fixtures/presented_startup_demo.tscn\"\n\n[autoload]\nGodotQaBridge=\"*res://addons/godot_qa/qa_bridge.gd\"\n\n[rendering]\nrenderer/rendering_method=\"gl_compatibility\"\n").unwrap();
    symlink(
        repo_root().join("addons/godot_qa"),
        root.join("addons/godot_qa"),
    )
    .unwrap();
    for name in [
        "presented_startup_demo.gd",
        "presented_startup_demo.tscn",
        "presented_startup_stale.tscn",
        "presented_startup_blank.tscn",
    ] {
        fs::copy(
            repo_root()
                .join("examples/sample_project/ui/fixtures")
                .join(name),
            root.join("ui/fixtures").join(name),
        )
        .unwrap();
    }
    root
}

fn run(
    root: &Path,
    godot: &str,
    backend: &str,
    scene: &str,
    viewport: &str,
    baseline: Option<&Path>,
) -> (std::process::ExitStatus, Value) {
    let mut command = Command::new(env!("CARGO_BIN_EXE_godot-qa"));
    command.args([
        "--project",
        root.to_str().unwrap(),
        "--godot-bin",
        godot,
        "--json",
        "screenshot",
        "assert-startup-presented",
        "--scene",
        scene,
        "--viewport",
        viewport,
        "--display-backend",
        backend,
    ]);
    if let Some(baseline) = baseline {
        command.arg("--baseline").arg(baseline);
    }
    let output = command.output().unwrap();
    let body = serde_json::from_slice(&output.stdout).unwrap_or_else(|error| {
        panic!(
            "{error}: stdout={} stderr={}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        )
    });
    (output.status, body)
}

#[test]
fn isolated_presented_startup_checks_both_backends_and_no_swap() {
    if std::env::var_os("GODOT_QA_VISUAL_TESTS").is_none() {
        return;
    }
    let godot = std::env::var("GODOT_BIN").expect("GODOT_BIN required for visual integration test");
    let root = isolated_fixture_project();
    for backend in ["wayland", "x11"] {
        let (status, body) = run(
            &root,
            &godot,
            backend,
            "res://ui/fixtures/presented_startup_demo.tscn",
            "1280x720",
            None,
        );
        assert!(status.success() && body["ok"] == true, "{backend}: {body}");
        assert_eq!(
            body["data"]["display_server"]
                .as_str()
                .unwrap()
                .to_ascii_lowercase(),
            backend
        );
        assert_eq!(
            body["data"]["comparison"]["metrics"]["different_ratio"],
            0.0
        );
        assert!(Path::new(body["data"]["artifacts"]["presented"].as_str().unwrap()).is_file());
        assert!(Path::new(body["data"]["artifacts"]["viewport"].as_str().unwrap()).is_file());
        let saved: Value = serde_json::from_slice(
            &fs::read(body["data"]["artifacts"]["result"].as_str().unwrap()).unwrap(),
        )
        .unwrap();
        assert_eq!(saved, body, "persist the parent's finalized verdict");
    }
    let (status, body) = run(
        &root,
        &godot,
        "wayland",
        "res://ui/fixtures/presented_startup_stale.tscn",
        "1280x720",
        None,
    );
    assert_eq!(status.code(), Some(1), "{body}");
    assert_eq!(body["failures"][0]["type"], "presented_startup_mismatch");
    assert!(
        body["data"]["comparison"]["metrics"]["different_ratio"]
            .as_f64()
            .unwrap()
            > 0.5
    );
    let (status, body) = run(
        &root,
        &godot,
        "wayland",
        "res://ui/fixtures/presented_startup_demo.tscn",
        "960x540",
        None,
    );
    assert!(status.success() && body["ok"] == true, "resized: {body}");
    assert_eq!(
        body["data"]["dimensions"]["presented"],
        serde_json::json!([960, 540])
    );
    let resized_viewport = body["data"]["artifacts"]["viewport"].as_str().unwrap();
    let (status, body) = run(
        &root,
        &godot,
        "wayland",
        "res://ui/fixtures/presented_startup_demo.tscn",
        "1280x720",
        Some(Path::new(resized_viewport)),
    );
    assert_eq!(status.code(), Some(1), "{body}");
    assert_eq!(body["failures"][0]["type"], "presented_size_mismatch");
    assert!(body["data"]["capture_provenance"]["baseline_sha256"].is_string());
    let (status, body) = run(
        &root,
        &godot,
        "wayland",
        "res://ui/fixtures/presented_startup_blank.tscn",
        "1280x720",
        None,
    );
    assert_eq!(status.code(), Some(1), "{body}");
    // Strict capture preserves the actual blank viewport pixels for review,
    // then rejects their lack of rendered information instead of substituting
    // the bridge's diagnostic fallback image.
    assert_eq!(
        body["failures"][0]["type"],
        "presented_capture_low_information"
    );
    assert!(Path::new(body["data"]["artifacts"]["presented"].as_str().unwrap()).is_file());
    assert!(Path::new(body["data"]["artifacts"]["viewport"].as_str().unwrap()).is_file());
    assert_eq!(
        body["data"]["capture_provenance"]["viewport"]["allow_fallback"],
        false
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn missing_gamescope_fails_without_desktop_capture() {
    let root = isolated_fixture_project();
    let output = Command::new(env!("CARGO_BIN_EXE_godot-qa"))
        .args([
            "--project",
            root.to_str().unwrap(),
            "--godot-bin",
            "/bin/true",
            "--json",
            "screenshot",
            "assert-startup-presented",
            "--scene",
            "res://ui/fixtures/presented_startup_demo.tscn",
        ])
        .env("PATH", "/does-not-exist")
        .output()
        .unwrap();
    let body: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(output.status.code(), Some(1));
    assert_eq!(body["failures"][0]["type"], "presented_capture_unsupported");
    assert!(
        !Path::new(body["data"]["artifacts"]["root"].as_str().unwrap())
            .join("presented.png")
            .exists()
    );
    fs::remove_dir_all(root).unwrap();
}
