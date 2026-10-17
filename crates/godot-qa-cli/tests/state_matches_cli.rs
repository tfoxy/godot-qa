use serde_json::{Value, json};
use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::thread;
use std::time::{SystemTime, UNIX_EPOCH};

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("repo root")
        .to_path_buf()
}

fn sample_project_root() -> PathBuf {
    repo_root().join("examples/sample_project")
}

fn run_sample_scenario(name: &str) -> (std::process::ExitStatus, Value) {
    let output = Command::new(env!("CARGO_BIN_EXE_godot-qa"))
        .arg("scenario")
        .arg("run")
        .arg(name)
        .arg("--project")
        .arg(sample_project_root())
        .arg("--json")
        .output()
        .unwrap();
    let body = serde_json::from_slice(&output.stdout).unwrap_or_else(|error| {
        panic!(
            "invalid stdout JSON for {name}: {error}\nstdout: {}\nstderr: {}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        )
    });
    (output.status, body)
}

#[test]
fn session_list_compatibility() {
    let root = unique_temp_root("session-list-compatibility");
    let qa_dir = root.join(".qa");
    fs::create_dir_all(&qa_dir).unwrap();

    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let server = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut request = String::new();
        BufReader::new(stream.try_clone().unwrap())
            .read_line(&mut request)
            .unwrap();
        let payload: Value = serde_json::from_str(&request).unwrap();
        assert_eq!(payload["command"], "session.status");
        assert_eq!(payload["token"], "token-test");
        stream
            .write_all(
                br#"{"protocol_version":"0.1","ok":true,"data":{"display_server":"headless","paused":false}}"#,
            )
            .unwrap();
        stream.write_all(b"\n").unwrap();
    });

    let session_path = qa_dir.join("session.json");
    let token_path = qa_dir.join("session.token");
    fs::write(&token_path, "token-test").unwrap();
    fs::write(
        &session_path,
        serde_json::to_string_pretty(&json!({
            "protocol_version": "0.1",
            "session_id": "session-list-test",
            "pid": std::process::id(),
            "host": "127.0.0.1",
            "port": port,
            "scene": "res://ui/main_menu.tscn",
            "viewport": [800, 600],
            "artifact_dir": qa_dir.join("artifacts/live/session-list-test"),
            "request_path": qa_dir.join("requests/session-list-test.json"),
            "token_file": token_path,
            "started_at": "2026-05-06T00:00:00Z",
            "headless": true
        }))
        .unwrap(),
    )
    .unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_godot-qa"))
        .arg("--project")
        .arg(&root)
        .arg("session")
        .arg("list")
        .arg("--json")
        .output()
        .unwrap();
    server.join().unwrap();

    let body: Value = serde_json::from_slice(&output.stdout).unwrap_or_else(|error| {
        panic!(
            "invalid stdout JSON: {error}\nstdout: {}\nstderr: {}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        )
    });
    assert!(
        output.status.success(),
        "session list should succeed: {}",
        serde_json::to_string_pretty(&body).unwrap()
    );
    assert_eq!(body["ok"], true);
    assert_eq!(body["command"], "session list");
    assert_eq!(body["data"]["session_id"], "session-list-test");
    assert_eq!(body["data"]["host"], "127.0.0.1");
    assert_eq!(body["data"]["port"], port);
    assert_eq!(body["data"]["display_server"], "headless");
    assert_eq!(body["data"]["runtime"]["paused"], false);
}

#[test]
fn session_reload_uses_explicit_live_artifact_root() {
    let root = unique_temp_root("session-reload-artifact-root");
    let qa_dir = root.join(".qa");
    let artifact_root = root.join(".qa/artifacts/live/session-reload-test");
    fs::create_dir_all(&artifact_root).unwrap();

    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let server = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut request = String::new();
        BufReader::new(stream.try_clone().unwrap())
            .read_line(&mut request)
            .unwrap();
        let payload: Value = serde_json::from_str(&request).unwrap();
        assert_eq!(payload["command"], "session.reload_scene");
        assert_eq!(payload["token"], "token-test");
        stream
            .write_all(
                br#"{"protocol_version":"0.1","ok":true,"data":{"scene":"res://main.tscn","old_root":"/root/Main","new_root":"/root/Main2","paused":false}}"#,
            )
            .unwrap();
        stream.write_all(b"\n").unwrap();
    });

    let token_path = qa_dir.join("session.token");
    fs::create_dir_all(&qa_dir).unwrap();
    fs::write(&token_path, "token-test").unwrap();
    fs::write(
        artifact_root.join("session.json"),
        serde_json::to_string_pretty(&json!({
            "protocol_version": "0.1",
            "session_id": "session-reload-test",
            "pid": std::process::id(),
            "host": "127.0.0.1",
            "port": port,
            "scene": "res://main.tscn",
            "viewport": [800, 600],
            "artifact_dir": artifact_root.clone(),
            "request_path": artifact_root.join("request.json"),
            "token_file": token_path.clone(),
            "started_at": "2026-05-06T00:00:00Z",
            "headless": false
        }))
        .unwrap(),
    )
    .unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_godot-qa"))
        .arg("--project")
        .arg(&root)
        .arg("--artifact-dir")
        .arg(&artifact_root)
        .arg("session")
        .arg("reload")
        .arg("--json")
        .output()
        .unwrap();
    server.join().unwrap();

    let body: Value = serde_json::from_slice(&output.stdout).unwrap_or_else(|error| {
        panic!(
            "invalid stdout JSON: {error}\nstdout: {}\nstderr: {}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        )
    });
    assert!(
        output.status.success(),
        "session reload should succeed: {}",
        serde_json::to_string_pretty(&body).unwrap()
    );
    assert_eq!(body["ok"], true);
    assert_eq!(body["command"], "session reload");
    assert_eq!(body["data"]["scene"], "res://main.tscn");
}

fn unique_temp_root(label: &str) -> PathBuf {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis();
    std::env::temp_dir().join(format!("godot-qa-{label}-{}-{millis}", std::process::id()))
}

#[test]
fn state_matches_rejected_method_contract() {
    let cases = [
        (
            "state_matches_rejected_get_runtime_state_demo",
            "get_runtime_state",
            "name",
        ),
        ("state_matches_rejected_get_state_demo", "get_state", "name"),
        (
            "state_matches_rejected_private_demo",
            "_get_secret_qa_state",
            "name",
        ),
        (
            "state_matches_rejected_setter_demo",
            "set_raft_motion_qa_state",
            "name",
        ),
        (
            "state_matches_rejected_qa_mutate_demo",
            "qa_mutate_motion_state",
            "name",
        ),
        (
            "state_matches_rejected_arguments_demo",
            "get_argument_qa_state",
            "arguments",
        ),
        (
            "state_matches_rejected_missing_demo",
            "get_missing_qa_state",
            "missing_method",
        ),
        (
            "state_matches_rejected_non_json_demo",
            "get_object_qa_state",
            "non_json_return",
        ),
        (
            "state_matches_rejected_runtime_error_demo",
            "get_error_qa_state",
            "runtime_error",
        ),
    ];

    for (scenario, method, rejection_reason) in cases {
        let (status, body) = run_sample_scenario(scenario);
        assert!(
            !status.success(),
            "{scenario} should fail, body: {}",
            serde_json::to_string_pretty(&body).unwrap()
        );
        assert_eq!(body["ok"], false, "{scenario}");
        assert_eq!(body["command"], "scenario run", "{scenario}");

        let failure = body["failures"]
            .as_array()
            .and_then(|failures| failures.iter().find(|failure| failure["method"] == method))
            .unwrap_or_else(|| {
                panic!(
                    "missing state_matches failure for {scenario}/{method}: {}",
                    serde_json::to_string_pretty(&body["failures"]).unwrap()
                )
            });

        assert_eq!(failure["method"], method, "{scenario}");
        assert_eq!(failure["target"], "qa:demo_actor", "{scenario}");
        assert_eq!(failure["source"], "method", "{scenario}");
        assert_eq!(failure["rejection_reason"], rejection_reason, "{scenario}");
        assert_eq!(
            failure["allowed_patterns"],
            json!(["qa_*", "get_qa_*", "get_*_qa_state"]),
            "{scenario}"
        );
        assert!(
            failure["resolved"]
                .as_str()
                .is_some_and(|value| !value.is_empty()),
            "{scenario} should name the resolved target"
        );
        assert_ne!(failure["expected"], Value::Null, "{scenario}");
        assert_ne!(failure["actual"], Value::Null, "{scenario}");

        if rejection_reason == "non_json_return" {
            assert_eq!(failure["actual_type"], "Object", "{scenario}");
        }
        if rejection_reason == "runtime_error" {
            let actual_errors = failure["actual"].as_array().unwrap_or_else(|| {
                panic!(
                    "{scenario} should preserve runtime error evidence: {}",
                    serde_json::to_string_pretty(failure).unwrap()
                )
            });
            assert!(
                actual_errors
                    .iter()
                    .any(|entry| entry["method"] == "get_error_qa_state"),
                "{scenario} should include the recorded method in runtime error evidence"
            );
        }
    }
}

#[test]
fn state_matches_accepts_array_methods_and_numeric_operators() {
    let (status, body) = run_sample_scenario("state_matches_array_method_demo");
    assert!(
        status.success(),
        "array/numeric state_matches scenario should pass: {}",
        serde_json::to_string_pretty(&body).unwrap()
    );
    assert_eq!(body["ok"], true);
    assert_eq!(body["command"], "scenario run");
    assert!(
        body["failures"]
            .as_array()
            .is_some_and(|failures| failures.is_empty()),
        "scenario should not report failures: {}",
        serde_json::to_string_pretty(&body["failures"]).unwrap()
    );
}

#[test]
fn state_matches_numeric_equals_mismatch_reports_precision() {
    let (status, body) = run_sample_scenario("state_matches_numeric_equals_precision_failure_demo");
    assert!(
        !status.success(),
        "precision mismatch scenario should fail: {}",
        serde_json::to_string_pretty(&body).unwrap()
    );
    assert_eq!(body["ok"], false);

    let mismatch = body["failures"]
        .as_array()
        .and_then(|failures| failures.first())
        .and_then(|failure| failure["actual"].as_array())
        .and_then(|actual| actual.first())
        .unwrap_or_else(|| {
            panic!(
                "missing state_matches mismatch payload: {}",
                serde_json::to_string_pretty(&body["failures"]).unwrap()
            )
        });
    assert_eq!(mismatch["mismatch_type"], "value_mismatch");
    assert_eq!(mismatch["operator"], "equals");
    assert!(
        mismatch["actual_full_precision"]
            .as_str()
            .is_some_and(|value| value.contains("1.400000")),
        "actual should include full precision: {}",
        serde_json::to_string_pretty(mismatch).unwrap()
    );
    assert!(
        mismatch["expected_full_precision"]
            .as_str()
            .is_some_and(|value| value.starts_with("1.4") || value.starts_with("1.399999")),
        "expected should include full precision: {}",
        serde_json::to_string_pretty(mismatch).unwrap()
    );
    assert!(
        mismatch["numeric_delta"]
            .as_f64()
            .is_some_and(|value| value > 0.0),
        "numeric delta should be positive: {}",
        serde_json::to_string_pretty(mismatch).unwrap()
    );
}
