use serde_json::{Value, json};
use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::thread;
use std::time::{SystemTime, UNIX_EPOCH};

fn unique_temp_root(label: &str) -> PathBuf {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis();
    std::env::temp_dir().join(format!("godot-qa-{label}-{}-{millis}", std::process::id()))
}

#[test]
fn event_trace_live_contract_shape() {
    let root = unique_temp_root("event-trace-live-contract");
    let qa_dir = root.join(".qa");
    let artifact_root = root.join(".qa/artifacts/live/session-event-trace-test");
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
        assert_eq!(payload["command"], "event.trace");
        assert_eq!(payload["token"], "token-test");
        assert_eq!(payload["params"]["target"], "qa:event_source");
        assert_eq!(payload["params"]["provider"], "get_qa_event_stream_state");
        assert_eq!(payload["params"]["frames"], 120);
        assert_eq!(payload["params"]["sample_every_frames"], 1);
        stream
            .write_all(
                serde_json::to_string(&json!({
                    "protocol_version": "0.1",
                    "ok": true,
                    "data": {
                        "event_trace_id": "trace-contract",
                        "events": [
                            {
                                "event_id": "gameplay.spawned:001",
                                "frame": 1,
                                "time_msec": 16.0,
                                "category": "gameplay",
                                "event_type": "gameplay.spawned",
                                "logical_id": "event_actor",
                                "position": {"x": 10.0, "y": 20.0},
                                "payload": {"phase": "spawn"}
                            },
                            {
                                "event_id": "sfx.spawned:002",
                                "frame": 2,
                                "time_msec": 32.0,
                                "category": "sfx",
                                "event_type": "sfx.spawned",
                                "logical_id": "event_actor",
                                "payload": {"cue": "spawn"}
                            }
                        ]
                    }
                }))
                .unwrap()
                .as_bytes(),
            )
            .unwrap();
        stream.write_all(b"\n").unwrap();
    });

    let token_path = qa_dir.join("session.token");
    fs::create_dir_all(&qa_dir).unwrap();
    fs::write(&token_path, "token-test").unwrap();
    fs::write(
        root.join(".qa/session.json"),
        serde_json::to_string_pretty(&json!({
            "protocol_version": "0.1",
            "session_id": "session-event-trace-test",
            "pid": std::process::id(),
            "host": "127.0.0.1",
            "port": port,
            "scene": "res://ui/fixtures/event_trace_demo.tscn",
            "viewport": [800, 600],
            "artifact_dir": artifact_root,
            "request_path": artifact_root.join("request.json"),
            "token_file": token_path,
            "started_at": "2026-05-17T00:00:00Z",
            "headless": true
        }))
        .unwrap(),
    )
    .unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_godot-qa"))
        .arg("--project")
        .arg(&root)
        .arg("event")
        .arg("trace")
        .arg("--target")
        .arg("qa:event_source")
        .arg("--provider")
        .arg("get_qa_event_stream_state")
        .arg("--frames")
        .arg("120")
        .arg("--sample-every-frames")
        .arg("1")
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
        "event trace should succeed: {}",
        serde_json::to_string_pretty(&body).unwrap()
    );
    assert_eq!(body["ok"], true);
    assert_eq!(body["command"], "event trace");
    assert_eq!(body["data"]["session_id"], "session-event-trace-test");
    assert_eq!(body["data"]["event_trace_id"], "trace-contract");
    assert_eq!(body["data"]["total_event_count"], 2);
    assert_eq!(body["data"]["summary"]["total_event_count"], 2);
    assert_eq!(
        body["data"]["event_count_by_category"]["gameplay"],
        json!(1)
    );
    assert_eq!(body["data"]["event_count_by_category"]["sfx"], json!(1));
    for field in [
        "event_count_by_type",
        "event_rate_per_second_by_type",
        "max_events_in_one_frame",
        "duplicate_event_count",
        "missing_event_id_count",
        "event_spam_warnings",
        "paired_event_warnings",
        "offscreen_event_warnings",
        "provider_error_count",
        "warning_ids",
    ] {
        assert!(
            body["data"].get(field).is_some(),
            "missing event trace field {field}: {}",
            serde_json::to_string_pretty(&body["data"]).unwrap()
        );
    }

    let events_path = Path::new(body["data"]["artifacts"]["events"].as_str().unwrap());
    let summary_path = Path::new(body["data"]["artifacts"]["summary"].as_str().unwrap());
    assert!(events_path.ends_with("events/trace-contract/events.json"));
    assert!(summary_path.ends_with("events/trace-contract/summary.json"));
    assert!(events_path.is_file());
    assert!(summary_path.is_file());
    assert!(
        root.join(".qa/artifacts/live/session-event-trace-test/events/latest/events.json")
            .exists()
    );

    let _ = fs::remove_dir_all(root);
}
