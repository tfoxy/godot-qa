use serde_json::{Value, json};
use std::env;
use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus};
use std::thread;
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

fn save_png(path: &Path) {
    save_png_with_rect(path, [16, 16, 20, 20]);
}

fn save_png_with_rect(path: &Path, rect: [u32; 4]) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    let image = image::ImageBuffer::from_fn(64, 64, |x, y| {
        if (rect[0]..rect[0] + rect[2]).contains(&x) && (rect[1]..rect[1] + rect[3]).contains(&y) {
            image::Rgba([240u8, 240, 240, 255])
        } else {
            image::Rgba([20u8, 20, 24, 255])
        }
    });
    image.save(path).unwrap();
}

fn write_targets(path: &Path) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(
        path,
        serde_json::to_string_pretty(&json!({
            "targets": [
                {
                    "target": "qa:button",
                    "target_rect": [16, 16, 20, 20],
                    "visible_pixel_count": 400
                }
            ]
        }))
        .unwrap(),
    )
    .unwrap();
}

fn write_live_session(project_root: &Path, artifact_root: &Path, port: u16, headless: bool) {
    let qa_dir = project_root.join(".qa");
    let token_path = qa_dir.join("session.token");
    fs::create_dir_all(&qa_dir).unwrap();
    fs::create_dir_all(artifact_root).unwrap();
    fs::write(&token_path, "token-test").unwrap();
    fs::write(
        qa_dir.join("session.json"),
        serde_json::to_string_pretty(&json!({
            "protocol_version": "0.1",
            "session_id": "session-visual-audit-test",
            "pid": std::process::id(),
            "host": "127.0.0.1",
            "port": port,
            "scene": "res://ui/fixtures/visual_audit_demo.tscn",
            "viewport": [64, 64],
            "artifact_dir": artifact_root,
            "request_path": artifact_root.join("request.json"),
            "token_file": token_path,
            "started_at": "2026-05-17T00:00:00Z",
            "headless": headless
        }))
        .unwrap(),
    )
    .unwrap();
}

fn run_visual_audit(project_root: &Path, extra_args: &[&str]) -> (ExitStatus, Value) {
    let output = Command::new(env!("CARGO_BIN_EXE_godot-qa"))
        .arg("visual")
        .arg("audit")
        .args(extra_args)
        .arg("--project")
        .arg(project_root)
        .arg("--json")
        .output()
        .unwrap();
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
fn visual_audit_cli_saved_screenshot_contract() {
    let project_root = make_project_root("visual-audit-saved");
    let screenshot_path = project_root.join("artifacts/current.png");
    let targets_path = project_root.join("artifacts/current.targets.json");
    let annotate_path = project_root.join("artifacts/current.annotated.png");
    save_png(&screenshot_path);
    write_targets(&targets_path);

    let (status, body) = run_visual_audit(
        &project_root,
        &[
            "--screenshot",
            "artifacts/current.png",
            "--targets",
            "artifacts/current.targets.json",
            "--target",
            "qa:button",
            "--min-pixel-area",
            "500",
            "--annotate-out",
            "artifacts/current.annotated.png",
        ],
    );

    assert!(
        status.success(),
        "{}",
        serde_json::to_string_pretty(&body).unwrap()
    );
    assert_eq!(body["ok"], true);
    assert_eq!(body["command"], "visual audit");
    assert_eq!(body["failures"], json!([]));
    assert_eq!(body["data"]["visual_audit"]["width"], json!(64));
    assert_eq!(body["data"]["visual_audit"]["height"], json!(64));
    assert_eq!(
        body["data"]["visual_audit"]["thresholds"]["min_pixel_area"],
        json!(500)
    );
    assert_eq!(
        body["data"]["visual_audit"]["targets"][0]["target_id"],
        json!("qa:button")
    );
    assert_eq!(
        body["data"]["visual_audit"]["targets"][0]["pixel_area"],
        json!(400)
    );
    assert_eq!(
        body["data"]["visual_audit"]["findings"][0]["rule_id"],
        json!("visual_audit.target_too_small")
    );
    assert_eq!(
        body["data"]["warnings"][0]["rule_id"],
        json!("visual_audit.target_too_small")
    );
    assert_eq!(
        body["data"]["visual_audit"]["artifacts"]["screenshot"],
        json!(screenshot_path.to_string_lossy().to_string())
    );
    assert_eq!(
        body["data"]["visual_audit"]["artifacts"]["targets"],
        json!(targets_path.to_string_lossy().to_string())
    );
    assert_eq!(
        body["data"]["visual_audit"]["artifacts"]["annotated"],
        json!(annotate_path.to_string_lossy().to_string())
    );
    assert!(annotate_path.is_file());

    let (strict_status, strict_body) = run_visual_audit(
        &project_root,
        &[
            "--screenshot",
            "artifacts/current.png",
            "--targets",
            "artifacts/current.targets.json",
            "--min-pixel-area",
            "500",
            "--strict-warnings",
        ],
    );
    assert_eq!(strict_status.code(), Some(1));
    assert_eq!(strict_body["ok"], false);
    assert_eq!(
        strict_body["failures"][0]["rule_id"],
        json!("visual_audit.target_too_small")
    );
    assert_eq!(strict_body["failures"][0]["command"], json!("visual audit"));
    assert_eq!(strict_body["failures"][0]["severity"], json!("error"));

    let (invalid_status, invalid_body) = run_visual_audit(&project_root, &[]);
    assert_eq!(invalid_status.code(), Some(2));
    assert_eq!(invalid_body["failures"][0]["type"], json!("invalid_params"));
    assert_eq!(
        invalid_body["failures"][0]["command"],
        json!("visual audit")
    );

    let (missing_status, missing_body) = run_visual_audit(
        &project_root,
        &[
            "--screenshot",
            "artifacts/current.png",
            "--targets",
            "artifacts/missing.targets.json",
        ],
    );
    assert_eq!(missing_status.code(), Some(1));
    assert_eq!(
        missing_body["failures"][0]["type"],
        json!("target_bounds_not_found")
    );
    assert_eq!(
        missing_body["failures"][0]["command"],
        json!("visual audit")
    );
    assert_eq!(
        missing_body["failures"][0]["expected"],
        json!("existing target bounds JSON file")
    );
    assert_eq!(missing_body["failures"][0]["actual"], json!("missing"));
    assert_eq!(
        missing_body["failures"][0]["path"],
        json!(
            project_root
                .join("artifacts/missing.targets.json")
                .to_string_lossy()
                .to_string()
        )
    );

    fs::remove_dir_all(project_root).unwrap();
}

#[test]
fn visual_audit_sequence_and_scenario_contracts() {
    let project_root = make_project_root("visual-audit-sequence");
    let baseline_path = project_root.join("qa/baselines/current.png");
    fs::create_dir_all(baseline_path.parent().unwrap()).unwrap();
    fs::write(&baseline_path, b"baseline-bytes").unwrap();

    let sequence_dir = project_root.join(".qa/artifacts/manual/race_sequence");
    save_png_with_rect(
        &sequence_dir.join("frames/frame_0000.png"),
        [16, 16, 20, 20],
    );
    save_png_with_rect(&sequence_dir.join("frames/frame_0010.png"), [16, 16, 8, 8]);
    save_png_with_rect(
        &sequence_dir.join("frames/frame_0020.png"),
        [16, 16, 20, 20],
    );
    fs::write(
        sequence_dir.join("manifest.json"),
        serde_json::to_string_pretty(&json!({
            "frames": [
                {
                    "path": "frames/frame_0000.png",
                    "targets": [{"target": "qa:button", "target_rect": [16, 16, 20, 20]}]
                },
                {
                    "path": "frames/frame_0010.png",
                    "targets": [{"target": "qa:button", "target_rect": [16, 16, 8, 8]}]
                },
                {
                    "path": "frames/frame_0020.png",
                    "targets": []
                }
            ]
        }))
        .unwrap(),
    )
    .unwrap();

    let (sequence_status, sequence_body) = run_visual_audit(
        &project_root,
        &[
            "--sequence",
            ".qa/artifacts/manual/race_sequence/manifest.json",
            "--target",
            "qa:button",
        ],
    );
    assert!(
        sequence_status.success(),
        "{}",
        serde_json::to_string_pretty(&sequence_body).unwrap()
    );
    let sequence_audit = &sequence_body["data"]["visual_audit"];
    assert_eq!(sequence_audit["frames_checked"], json!(3));
    assert_eq!(
        sequence_audit["frames_with_target_missing"]["qa:button"],
        json!(1)
    );
    assert_eq!(
        sequence_audit["minimum_pixel_area_across_frames"]["qa:button"],
        json!(64)
    );
    assert!(sequence_audit["minimum_contrast_across_frames"]["qa:button"].is_number());
    assert_eq!(
        Path::new(
            sequence_audit["artifacts"]["sequence_manifest"]
                .as_str()
                .unwrap()
        ),
        sequence_dir.join("manifest.json")
    );
    assert_eq!(
        sequence_body["data"]["warnings"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|warning| {
                warning["rule_id"] == json!("visual_audit.sequence_target_missing_frames")
            })
            .count(),
        1
    );

    let run_root = project_root.join(".qa/artifacts/menu/run-001");
    let scenario_screenshot = run_root.join("snapshots/current/screenshot.png");
    save_png_with_rect(&scenario_screenshot, [16, 16, 20, 20]);
    fs::write(
        run_root.join("result.json"),
        serde_json::to_string_pretty(&json!({
            "snapshot_artifacts": {
                "current": {
                    "screenshot": "snapshots/current/screenshot.png",
                    "targets": [
                        {"target": "qa:button", "target_rect": [16, 16, 20, 20]}
                    ]
                }
            }
        }))
        .unwrap(),
    )
    .unwrap();
    let (scenario_status, scenario_body) = run_visual_audit(
        &project_root,
        &[
            "--scenario",
            "menu",
            "--run-id",
            "run-001",
            "--target",
            "qa:button",
        ],
    );
    assert!(
        scenario_status.success(),
        "{}",
        serde_json::to_string_pretty(&scenario_body).unwrap()
    );
    let scenario_audit = &scenario_body["data"]["visual_audit"];
    assert_eq!(scenario_audit["frames_checked"], json!(1));
    assert_eq!(
        scenario_audit["targets"][0]["target_id"],
        json!("qa:button")
    );
    assert_eq!(scenario_body["data"]["warnings"], json!([]));
    assert_eq!(fs::read(&baseline_path).unwrap(), b"baseline-bytes");
    assert!(!sequence_dir.join("contact-sheet.png").exists());

    fs::remove_dir_all(project_root).unwrap();
}

#[test]
fn visual_audit_live_cli_contract() {
    let project_root = make_project_root("visual-audit-live");
    let (missing_target_status, missing_target_body) = run_visual_audit(&project_root, &["--live"]);
    assert_eq!(missing_target_status.code(), Some(2));
    assert_eq!(
        missing_target_body["failures"][0]["type"],
        json!("invalid_params")
    );
    assert_eq!(
        missing_target_body["failures"][0]["expected"],
        json!("one or more non-empty --target values")
    );

    let (missing_session_status, missing_session_body) =
        run_visual_audit(&project_root, &["--live", "--target", "qa:button"]);
    assert_eq!(missing_session_status.code(), Some(1));
    assert_eq!(
        missing_session_body["failures"][0]["type"],
        json!("session_not_found")
    );
    assert_eq!(
        missing_session_body["failures"][0]["command"],
        json!("visual audit")
    );

    let headless_artifacts = project_root.join(".qa/artifacts/live/session-headless");
    write_live_session(&project_root, &headless_artifacts, 9, true);
    let (headless_status, headless_body) =
        run_visual_audit(&project_root, &["--live", "--target", "qa:button"]);
    assert_eq!(headless_status.code(), Some(1));
    assert_eq!(
        headless_body["failures"][0]["type"],
        json!("headless_screenshot_unsupported")
    );
    assert_eq!(headless_body["failures"][0]["actual"]["headless"], true);

    fs::remove_file(project_root.join(".qa/session.json")).unwrap();
    let live_artifacts = project_root.join(".qa/artifacts/live/session-windowed");
    let screenshot_path = live_artifacts.join("screenshots/visual_audit.png");
    save_png(&screenshot_path);
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    write_live_session(&project_root, &live_artifacts, port, false);
    let server_screenshot = screenshot_path.clone();
    let server = thread::spawn(move || {
        for expected_command in ["screenshot.capture", "visual.audit_targets"] {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = String::new();
            BufReader::new(stream.try_clone().unwrap())
                .read_line(&mut request)
                .unwrap();
            let payload: Value = serde_json::from_str(&request).unwrap();
            assert_eq!(payload["command"], expected_command);
            assert_eq!(payload["token"], "token-test");
            let response = if expected_command == "screenshot.capture" {
                assert_eq!(payload["params"]["name"], "visual_audit");
                json!({
                    "protocol_version": "0.1",
                    "ok": true,
                    "data": {
                        "artifacts": {"screenshot": server_screenshot},
                        "engine_frame": 12,
                        "time_msec": 34,
                        "display_server": "x11"
                    }
                })
            } else {
                assert_eq!(payload["params"]["targets"], json!(["qa:button"]));
                json!({
                    "protocol_version": "0.1",
                    "ok": true,
                    "data": {
                        "target_count": 1,
                        "targets": [
                            {
                                "locator": "qa:button",
                                "resolved": "path:/root/Button",
                                "visible": true,
                                "visual_evidence": {
                                    "coordinate_space": "viewport_2d",
                                    "rect": [16.0, 16.0, 20.0, 20.0],
                                    "rect_source": "control_global_rect"
                                }
                            }
                        ]
                    }
                })
            };
            stream
                .write_all(serde_json::to_string(&response).unwrap().as_bytes())
                .unwrap();
            stream.write_all(b"\n").unwrap();
        }
    });

    let annotate_path = project_root.join("artifacts/live.annotated.png");
    let (status, body) = run_visual_audit(
        &project_root,
        &[
            "--live",
            "--target",
            "qa:button",
            "--annotate-out",
            "artifacts/live.annotated.png",
        ],
    );
    server.join().unwrap();
    assert!(
        status.success(),
        "{}",
        serde_json::to_string_pretty(&body).unwrap()
    );
    assert_eq!(
        body["data"]["visual_audit"]["targets"][0]["target_id"],
        json!("qa:button")
    );
    assert_eq!(
        body["data"]["visual_audit"]["artifacts"]["screenshot"],
        json!(screenshot_path.to_string_lossy().to_string())
    );
    assert_eq!(
        body["data"]["visual_audit"]["artifacts"]["annotated"],
        json!(annotate_path.to_string_lossy().to_string())
    );
    assert_eq!(
        body["data"]["visual_audit"]["live_screenshot_metadata"]["display_server"],
        json!("x11")
    );
    assert_eq!(
        body["data"]["visual_audit"]["live_target_evidence"]["targets"][0]["target_rect"],
        json!([16.0, 16.0, 20.0, 20.0])
    );
    assert!(annotate_path.is_file());

    fs::remove_dir_all(project_root).unwrap();
}
