use serde_json::{Value, json};
use std::env;
use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::thread;
use std::time::{SystemTime, UNIX_EPOCH};

fn unique_suffix() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or(0);
    format!("{nanos}-{}", std::process::id())
}

fn temp_root(name: &str) -> PathBuf {
    env::temp_dir().join(format!("godot-qa-{name}-{}", unique_suffix()))
}

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .to_path_buf()
}

fn write_json(path: &Path, value: &Value) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(path, serde_json::to_string_pretty(value).unwrap()).unwrap();
}

fn write_live_session(root: &Path, port: u16) {
    let qa_dir = root.join(".qa");
    let token_path = qa_dir.join("session.token");
    fs::create_dir_all(&qa_dir).unwrap();
    fs::write(&token_path, "token-test").unwrap();
    write_json(
        &qa_dir.join("session.json"),
        &json!({
            "protocol_version": "0.1",
            "session_id": "qa-state-live-test",
            "pid": std::process::id(),
            "host": "127.0.0.1",
            "port": port,
            "scene": "res://main.tscn",
            "viewport": [800, 600],
            "artifact_dir": qa_dir.join("artifacts/live/qa-state-live-test"),
            "request_path": qa_dir.join("requests/qa-state-live-test.json"),
            "token_file": token_path,
            "started_at": "2026-05-17T00:00:00Z",
            "headless": true
        }),
    );
}

fn run_validate(args: &[&Path]) -> Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_godot-qa"));
    command.arg("--json").arg("qa-state").arg("validate");
    for arg in args {
        command.arg(arg);
    }
    command.output().unwrap()
}

fn parse_stdout(output: &Output) -> Value {
    serde_json::from_slice(&output.stdout).unwrap_or_else(|error| {
        panic!(
            "failed to parse stdout as JSON: {error}\nstdout:\n{}\nstderr:\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        )
    })
}

#[test]
fn qa_state_validate_input_passes() {
    let root = temp_root("qa-state-validate-pass");
    let input = root.join("state.json");
    let schema = root.join("schema.json");
    write_json(
        &input,
        &json!({
            "phase": "racing",
            "laps": 3,
            "rafts": [{"id": "raft_0", "speed": 12.5}]
        }),
    );
    write_json(
        &schema,
        &json!({
            "type": "object",
            "required": ["phase", "laps", "rafts"],
            "properties": {
                "phase": {"type": "string", "enum": ["staging", "racing", "finished"]},
                "laps": {"type": "integer", "minimum": 1},
                "rafts": {
                    "type": "array",
                    "minItems": 1,
                    "items": {
                        "type": "object",
                        "required": ["id", "speed"],
                        "properties": {
                            "id": {"type": "string"},
                            "speed": {"type": "number", "minimum": 0.0}
                        },
                        "additionalProperties": false
                    }
                }
            },
            "additionalProperties": false
        }),
    );

    let output = run_validate(&[Path::new("--input"), &input, Path::new("--schema"), &schema]);
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let body = parse_stdout(&output);
    assert_eq!(body["ok"], true);
    assert_eq!(body["command"], "qa_state.validate");
    assert_eq!(body["data"]["summary"]["selected_values"], 1);
    assert!(body["data"]["summary"]["checked_paths"].as_u64().unwrap() > 0);
}

#[test]
fn qa_state_validate_input_reports_schema_errors() {
    let root = temp_root("qa-state-validate-fail");
    let input = root.join("state.json");
    let schema = root.join("schema.json");
    write_json(
        &input,
        &json!({
            "rafts": [{"velocity": {"x": "fast"}}]
        }),
    );
    write_json(
        &schema,
        &json!({
            "type": "object",
            "properties": {
                "rafts": {
                    "type": "array",
                    "items": {
                        "type": "object",
                        "properties": {
                            "velocity": {
                                "type": "object",
                                "properties": {
                                    "x": {"type": "number"}
                                }
                            }
                        }
                    }
                }
            }
        }),
    );

    let output = run_validate(&[Path::new("--input"), &input, Path::new("--schema"), &schema]);
    assert!(!output.status.success());
    let body = parse_stdout(&output);
    assert_eq!(body["ok"], false);
    assert_eq!(
        body["failures"][0]["type"],
        "qa_state_schema_validation_failed"
    );
    let error = &body["failures"][0]["errors"][0];
    assert_eq!(error["json_path"], "$.rafts[0].velocity.x");
    assert_eq!(
        error["schema_path"],
        "$.properties.rafts.items.properties.velocity.properties.x.type"
    );
    assert_eq!(error["expected"], "number");
    assert_eq!(error["actual"], "string");
    assert_eq!(error["actual_value_preview"], "fast");
}

#[test]
fn qa_state_validate_trace_state_path_selects_repeated_values() {
    let root = temp_root("qa-state-validate-trace");
    let trace = root.join("trace.json");
    let schema = root.join("schema.json");
    write_json(
        &trace,
        &json!({
            "samples": [
                {"frame": 1, "state": {"position": {"x": 1.0}}},
                {"frame": 2, "state": {"position": {"x": 2.0}}},
                {"frame": 3, "state": {"position": {"x": "bad"}}}
            ]
        }),
    );
    write_json(
        &schema,
        &json!({
            "type": "object",
            "properties": {
                "position": {
                    "type": "object",
                    "properties": {
                        "x": {"type": "number"}
                    },
                    "required": ["x"]
                }
            },
            "required": ["position"]
        }),
    );

    let output = run_validate(&[
        Path::new("--trace"),
        &trace,
        Path::new("--state-path"),
        Path::new("samples[*].state"),
        Path::new("--schema"),
        &schema,
    ]);
    assert!(!output.status.success());
    let body = parse_stdout(&output);
    assert_eq!(body["data"]["summary"]["selected_values"], 3);
    assert_eq!(
        body["failures"][0]["errors"][0]["json_path"],
        "$.samples[2].state.position.x"
    );
}

#[test]
fn qa_state_validate_sample_input_fixture() {
    let root = repo_root();
    let input = root.join("examples/sample_project/qa/fixtures/schema_demo_state_valid.json");
    let schema = root.join("examples/sample_project/qa/schemas/schema_demo_state.schema.json");

    let output = run_validate(&[Path::new("--input"), &input, Path::new("--schema"), &schema]);
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let body = parse_stdout(&output);
    assert_eq!(body["ok"], true);
    assert_eq!(body["command"], "qa_state.validate");
    assert_eq!(body["data"]["input"], json!(input));
    assert_eq!(body["data"]["schema"], json!(schema));
    assert_eq!(body["data"]["summary"]["selected_values"], 1);
    assert!(body["data"]["summary"]["checked_paths"].as_u64().unwrap() > 0);
}

#[test]
fn qa_state_validate_sample_trace_fixture() {
    let root = repo_root();
    let trace = root.join("examples/sample_project/qa/fixtures/schema_demo_trace.json");
    let schema = root.join("examples/sample_project/qa/schemas/schema_demo_state.schema.json");

    let output = run_validate(&[
        Path::new("--trace"),
        &trace,
        Path::new("--state-path"),
        Path::new("samples[*].state"),
        Path::new("--schema"),
        &schema,
    ]);
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let body = parse_stdout(&output);
    assert_eq!(body["ok"], true);
    assert_eq!(body["command"], "qa_state.validate");
    assert_eq!(body["data"]["trace"], json!(trace));
    assert_eq!(body["data"]["schema"], json!(schema));
    assert_eq!(body["data"]["state_path"], "samples[*].state");
    assert_eq!(body["data"]["summary"]["selected_values"], 2);
    assert!(body["data"]["summary"]["checked_paths"].as_u64().unwrap() > 0);
}

#[test]
fn qa_state_validate_rejects_unsupported_schema_keyword() {
    let root = temp_root("qa-state-validate-unsupported");
    let input = root.join("state.json");
    let schema = root.join("schema.json");
    write_json(&input, &json!({"name": "demo"}));
    write_json(
        &schema,
        &json!({
            "type": "object",
            "properties": {
                "name": {"type": "string", "pattern": "^[a-z]+$"}
            }
        }),
    );

    let output = run_validate(&[Path::new("--input"), &input, Path::new("--schema"), &schema]);
    assert!(!output.status.success());
    let body = parse_stdout(&output);
    assert_eq!(body["ok"], false);
    assert_eq!(
        body["failures"][0]["type"],
        "qa_state_schema_unsupported_keyword"
    );
    assert_eq!(body["failures"][0]["actual"], "pattern");
    assert_eq!(body["failures"][0]["path"], "$.properties.name.pattern");
}

#[test]
fn qa_state_validate_live_sends_read_request() {
    let root = temp_root("qa-state-validate-live-read");
    let schema = root.join("schema.json");
    write_json(
        &schema,
        &json!({
            "type": "object",
            "required": ["phase", "laps"],
            "properties": {
                "phase": {"type": "string"},
                "laps": {"type": "integer", "minimum": 1}
            },
            "additionalProperties": false
        }),
    );

    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    write_live_session(&root, port);
    let server = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut request = String::new();
        BufReader::new(stream.try_clone().unwrap())
            .read_line(&mut request)
            .unwrap();
        let payload: Value = serde_json::from_str(&request).unwrap();
        assert_eq!(payload["command"], "qa_state.read");
        assert_eq!(payload["token"], "token-test");
        assert_eq!(payload["params"]["target"], "qa:race");
        assert_eq!(payload["params"]["method"], "get_qa_race_state");
        stream
            .write_all(
                br#"{"protocol_version":"0.1","ok":true,"data":{"target":"qa:race","resolved":"/root/Race","method":"get_qa_race_state","state":{"phase":"racing","laps":3}}}"#,
            )
            .unwrap();
        stream.write_all(b"\n").unwrap();
    });

    let output = Command::new(env!("CARGO_BIN_EXE_godot-qa"))
        .arg("--project")
        .arg(&root)
        .arg("--json")
        .arg("qa-state")
        .arg("validate")
        .arg("--target")
        .arg("qa:race")
        .arg("--method")
        .arg("get_qa_race_state")
        .arg("--schema")
        .arg(&schema)
        .output()
        .unwrap();
    server.join().unwrap();

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let body = parse_stdout(&output);
    assert_eq!(body["ok"], true);
    assert_eq!(body["command"], "qa_state.validate");
    assert_eq!(body["data"]["target"], "qa:race");
    assert_eq!(body["data"]["method"], "get_qa_race_state");
    assert_eq!(body["data"]["schema"], json!(schema));
    assert!(body["data"]["summary"]["checked_paths"].as_u64().unwrap() > 0);
}

#[test]
fn qa_state_validate_live_preserves_rejected_method_payload() {
    let root = temp_root("qa-state-validate-live-rejected");
    let schema = root.join("schema.json");
    write_json(&schema, &json!({"type": "object"}));

    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    write_live_session(&root, port);
    let server = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut request = String::new();
        BufReader::new(stream.try_clone().unwrap())
            .read_line(&mut request)
            .unwrap();
        let payload: Value = serde_json::from_str(&request).unwrap();
        assert_eq!(payload["command"], "qa_state.read");
        assert_eq!(payload["token"], "token-test");
        assert_eq!(payload["params"]["target"], "qa:race");
        assert_eq!(payload["params"]["method"], "get_state");
        stream
            .write_all(
                br#"{"protocol_version":"0.1","ok":false,"error":{"type":"invalid_assertion_args","message":"state_matches method must match an approved read-only QA state pattern","data":{"command":"assert.state_matches","target":"qa:race","resolved":"/root/Race","source":"method","method":"get_state","allowed_patterns":["qa_*","get_qa_*","get_*_qa_state"],"rejection_reason":"name","name_rejection_reason":"unsupported_method_name","expected":"method matching allowed_patterns","actual":"get_state","severity":"error"}}}"#,
            )
            .unwrap();
        stream.write_all(b"\n").unwrap();
    });

    let output = Command::new(env!("CARGO_BIN_EXE_godot-qa"))
        .arg("--project")
        .arg(&root)
        .arg("--json")
        .arg("qa-state")
        .arg("validate")
        .arg("--target")
        .arg("qa:race")
        .arg("--method")
        .arg("get_state")
        .arg("--schema")
        .arg(&schema)
        .output()
        .unwrap();
    server.join().unwrap();

    assert!(!output.status.success());
    let body = parse_stdout(&output);
    assert_eq!(body["ok"], false);
    assert_eq!(body["command"], "qa_state.validate");
    let failure = &body["failures"][0];
    assert_eq!(failure["type"], "invalid_assertion_args");
    assert_eq!(failure["command"], "qa_state.validate");
    assert_eq!(failure["target"], "qa:race");
    assert_eq!(failure["method"], "get_state");
    assert_eq!(
        failure["allowed_patterns"],
        json!(["qa_*", "get_qa_*", "get_*_qa_state"])
    );
    assert_eq!(failure["rejection_reason"], "name");
    assert_eq!(failure["name_rejection_reason"], "unsupported_method_name");
}
