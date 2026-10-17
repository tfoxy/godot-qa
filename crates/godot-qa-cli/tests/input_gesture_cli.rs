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
fn input_mouse_wheel_sends_protocol_command_and_reports_payload() {
    let root = unique_temp_root("input-mouse-wheel");
    let qa_dir = root.join(".qa");
    let artifact_root = root.join(".qa/artifacts/live/session-input-wheel-test");
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
        assert_eq!(payload["command"], "input.mouse_wheel");
        assert_eq!(payload["token"], "token-test");
        assert_eq!(payload["params"]["x"], 500);
        assert_eq!(payload["params"]["y"], 300);
        assert_eq!(payload["params"]["dy"], 3.0);
        assert_eq!(payload["params"]["dx"], 0.0);
        stream
            .write_all(
                serde_json::to_string(&json!({
                    "protocol_version": "0.1",
                    "ok": true,
                    "data": {
                        "device": "mouse",
                        "position": {"x": 500, "y": 300},
                        "dx": 0.0,
                        "dy": 3.0,
                        "buttons": ["wheel_down", "wheel_down", "wheel_down"]
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
            "session_id": "session-input-wheel-test",
            "pid": std::process::id(),
            "host": "127.0.0.1",
            "port": port,
            "scene": "res://ui/main_menu.tscn",
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
        .arg("input")
        .arg("mouse")
        .arg("wheel")
        .arg("--x")
        .arg("500")
        .arg("--y")
        .arg("300")
        .arg("--dy")
        .arg("3")
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
        "input mouse wheel should succeed: {}",
        serde_json::to_string_pretty(&body).unwrap()
    );
    assert_eq!(body["ok"], true);
    assert_eq!(body["command"], "input mouse wheel");
    assert_eq!(body["data"]["device"], "mouse");
    assert_eq!(body["data"]["position"]["x"], 500);
    assert_eq!(body["data"]["position"]["y"], 300);
    assert_eq!(body["data"]["dy"], 3.0);
    assert_eq!(body["data"]["dx"], 0.0);
    assert_eq!(
        body["data"]["buttons"],
        json!(["wheel_down", "wheel_down", "wheel_down"])
    );

    let _ = fs::remove_dir_all(root);
}

/// Writes the `.qa/session.json` + token scaffolding pointing at `port` and
/// returns nothing; mirrors the wheel test's session setup.
fn write_session_files(root: &Path, session_id: &str, port: u16) {
    let qa_dir = root.join(".qa");
    let artifact_root = qa_dir.join("artifacts/live").join(session_id);
    fs::create_dir_all(&artifact_root).unwrap();
    let token_path = qa_dir.join("session.token");
    fs::write(&token_path, "token-test").unwrap();
    fs::write(
        root.join(".qa/session.json"),
        serde_json::to_string_pretty(&json!({
            "protocol_version": "0.1",
            "session_id": session_id,
            "pid": std::process::id(),
            "host": "127.0.0.1",
            "port": port,
            "scene": "res://ui/main_menu.tscn",
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
}

#[test]
fn input_touch_down_sends_protocol_command_and_reports_payload() {
    let root = unique_temp_root("input-touch-down");

    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let server = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut request = String::new();
        BufReader::new(stream.try_clone().unwrap())
            .read_line(&mut request)
            .unwrap();
        let payload: Value = serde_json::from_str(&request).unwrap();
        assert_eq!(payload["command"], "input.touch_down");
        assert_eq!(payload["token"], "token-test");
        assert_eq!(payload["params"]["x"], 640);
        assert_eq!(payload["params"]["y"], 360);
        assert_eq!(payload["params"]["index"], 0);
        stream
            .write_all(
                serde_json::to_string(&json!({
                    "protocol_version": "0.1",
                    "ok": true,
                    "data": {
                        "device": "touch",
                        "index": 0,
                        "position": {"x": 640, "y": 360},
                        "pressed": true
                    }
                }))
                .unwrap()
                .as_bytes(),
            )
            .unwrap();
        stream.write_all(b"\n").unwrap();
    });

    write_session_files(&root, "session-input-touch-down-test", port);

    let output = Command::new(env!("CARGO_BIN_EXE_godot-qa"))
        .arg("--project")
        .arg(&root)
        .arg("input")
        .arg("touch")
        .arg("down")
        .arg("--x")
        .arg("640")
        .arg("--y")
        .arg("360")
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
        "input touch down should succeed: {}",
        serde_json::to_string_pretty(&body).unwrap()
    );
    assert_eq!(body["ok"], true);
    assert_eq!(body["command"], "input touch down");
    assert_eq!(body["data"]["device"], "touch");
    assert_eq!(body["data"]["index"], 0);
    assert_eq!(body["data"]["position"]["x"], 640);
    assert_eq!(body["data"]["position"]["y"], 360);
    assert_eq!(body["data"]["pressed"], true);

    let _ = fs::remove_dir_all(root);
}

#[test]
fn input_touch_up_omitted_coords_send_null_params() {
    let root = unique_temp_root("input-touch-up");

    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let server = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut request = String::new();
        BufReader::new(stream.try_clone().unwrap())
            .read_line(&mut request)
            .unwrap();
        let payload: Value = serde_json::from_str(&request).unwrap();
        assert_eq!(payload["command"], "input.touch_up");
        assert_eq!(payload["token"], "token-test");
        // Omitted --x/--y must be sent as explicit JSON null (Option -> null).
        assert!(payload["params"].get("x").is_some());
        assert!(payload["params"]["x"].is_null());
        assert!(payload["params"].get("y").is_some());
        assert!(payload["params"]["y"].is_null());
        assert_eq!(payload["params"]["index"], 1);
        stream
            .write_all(
                serde_json::to_string(&json!({
                    "protocol_version": "0.1",
                    "ok": true,
                    "data": {
                        "device": "touch",
                        "index": 1,
                        "position": {"x": 400, "y": 360},
                        "pressed": false
                    }
                }))
                .unwrap()
                .as_bytes(),
            )
            .unwrap();
        stream.write_all(b"\n").unwrap();
    });

    write_session_files(&root, "session-input-touch-up-test", port);

    let output = Command::new(env!("CARGO_BIN_EXE_godot-qa"))
        .arg("--project")
        .arg(&root)
        .arg("input")
        .arg("touch")
        .arg("up")
        .arg("--index")
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
        "input touch up should succeed: {}",
        serde_json::to_string_pretty(&body).unwrap()
    );
    assert_eq!(body["ok"], true);
    assert_eq!(body["command"], "input touch up");
    assert_eq!(body["data"]["device"], "touch");
    assert_eq!(body["data"]["index"], 1);
    assert_eq!(body["data"]["pressed"], false);

    let _ = fs::remove_dir_all(root);
}

#[test]
fn input_drag_sends_touch_protocol_command_and_reports_payload() {
    let root = unique_temp_root("input-drag-touch");

    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let server = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut request = String::new();
        BufReader::new(stream.try_clone().unwrap())
            .read_line(&mut request)
            .unwrap();
        let payload: Value = serde_json::from_str(&request).unwrap();
        assert_eq!(payload["command"], "input.drag");
        assert_eq!(payload["token"], "token-test");
        assert_eq!(payload["params"]["from_x"], 600);
        assert_eq!(payload["params"]["from_y"], 300);
        assert_eq!(payload["params"]["to_x"], 200);
        assert_eq!(payload["params"]["to_y"], 300);
        assert_eq!(payload["params"]["frames"], 8);
        assert_eq!(payload["params"]["pointer"], "touch");
        assert_eq!(payload["params"]["index"], 0);
        stream
            .write_all(
                serde_json::to_string(&json!({
                    "protocol_version": "0.1",
                    "ok": true,
                    "data": {
                        "device": "touch",
                        "pointer": "touch",
                        "index": 0,
                        "from": {"x": 600, "y": 300},
                        "to": {"x": 200, "y": 300},
                        "frames": 8,
                        "steps": 8
                    }
                }))
                .unwrap()
                .as_bytes(),
            )
            .unwrap();
        stream.write_all(b"\n").unwrap();
    });

    write_session_files(&root, "session-input-drag-touch-test", port);

    let output = Command::new(env!("CARGO_BIN_EXE_godot-qa"))
        .arg("--project")
        .arg(&root)
        .arg("input")
        .arg("drag")
        .arg("--from-x")
        .arg("600")
        .arg("--from-y")
        .arg("300")
        .arg("--to-x")
        .arg("200")
        .arg("--to-y")
        .arg("300")
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
        "input drag should succeed: {}",
        serde_json::to_string_pretty(&body).unwrap()
    );
    assert_eq!(body["ok"], true);
    assert_eq!(body["command"], "input drag");
    assert_eq!(body["data"]["device"], "touch");
    assert_eq!(body["data"]["pointer"], "touch");
    assert_eq!(body["data"]["index"], 0);
    assert_eq!(body["data"]["from"]["x"], 600);
    assert_eq!(body["data"]["to"]["x"], 200);
    assert_eq!(body["data"]["frames"], 8);
    assert_eq!(body["data"]["steps"], 8);

    let _ = fs::remove_dir_all(root);
}

#[test]
fn input_drag_pointer_mouse_sends_mouse_pointer_param() {
    let root = unique_temp_root("input-drag-mouse");

    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let server = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut request = String::new();
        BufReader::new(stream.try_clone().unwrap())
            .read_line(&mut request)
            .unwrap();
        let payload: Value = serde_json::from_str(&request).unwrap();
        assert_eq!(payload["command"], "input.drag");
        assert_eq!(payload["token"], "token-test");
        assert_eq!(payload["params"]["from_x"], 600);
        assert_eq!(payload["params"]["to_x"], 200);
        assert_eq!(payload["params"]["frames"], 8);
        assert_eq!(payload["params"]["pointer"], "mouse");
        assert_eq!(payload["params"]["index"], 0);
        stream
            .write_all(
                serde_json::to_string(&json!({
                    "protocol_version": "0.1",
                    "ok": true,
                    "data": {
                        "device": "mouse",
                        "pointer": "mouse",
                        "from": {"x": 600, "y": 300},
                        "to": {"x": 200, "y": 300},
                        "frames": 8,
                        "steps": 8
                    }
                }))
                .unwrap()
                .as_bytes(),
            )
            .unwrap();
        stream.write_all(b"\n").unwrap();
    });

    write_session_files(&root, "session-input-drag-mouse-test", port);

    let output = Command::new(env!("CARGO_BIN_EXE_godot-qa"))
        .arg("--project")
        .arg(&root)
        .arg("input")
        .arg("drag")
        .arg("--from-x")
        .arg("600")
        .arg("--from-y")
        .arg("300")
        .arg("--to-x")
        .arg("200")
        .arg("--to-y")
        .arg("300")
        .arg("--pointer")
        .arg("mouse")
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
        "input drag --pointer mouse should succeed: {}",
        serde_json::to_string_pretty(&body).unwrap()
    );
    assert_eq!(body["ok"], true);
    assert_eq!(body["command"], "input drag");
    assert_eq!(body["data"]["device"], "mouse");
    assert_eq!(body["data"]["pointer"], "mouse");

    let _ = fs::remove_dir_all(root);
}

#[test]
fn input_type_sends_protocol_command_and_reports_payload() {
    let root = unique_temp_root("input-type");

    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let server = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut request = String::new();
        BufReader::new(stream.try_clone().unwrap())
            .read_line(&mut request)
            .unwrap();
        let payload: Value = serde_json::from_str(&request).unwrap();
        assert_eq!(payload["command"], "input.text");
        assert_eq!(payload["token"], "token-test");
        assert_eq!(payload["params"]["text"], "Hello, 127.0.0.1!");
        stream
            .write_all(
                serde_json::to_string(&json!({
                    "protocol_version": "0.1",
                    "ok": true,
                    "data": {
                        "device": "keyboard",
                        "text": "Hello, 127.0.0.1!",
                        "characters": 17,
                        "focus_owner": "path:/root/Main/LineEdit"
                    }
                }))
                .unwrap()
                .as_bytes(),
            )
            .unwrap();
        stream.write_all(b"\n").unwrap();
    });

    write_session_files(&root, "session-input-type-test", port);

    let output = Command::new(env!("CARGO_BIN_EXE_godot-qa"))
        .arg("--project")
        .arg(&root)
        .arg("input")
        .arg("type")
        .arg("Hello, 127.0.0.1!")
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
        "input type should succeed: {}",
        serde_json::to_string_pretty(&body).unwrap()
    );
    assert_eq!(body["ok"], true);
    assert_eq!(body["command"], "input type");
    assert_eq!(body["data"]["device"], "keyboard");
    assert_eq!(body["data"]["text"], "Hello, 127.0.0.1!");
    assert_eq!(body["data"]["characters"], 17);
    assert_eq!(body["data"]["focus_owner"], "path:/root/Main/LineEdit");

    let _ = fs::remove_dir_all(root);
}

#[test]
fn input_type_round_trips_non_ascii_text_through_the_bridge() {
    let root = unique_temp_root("input-type-unicode");

    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let server = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut request = String::new();
        BufReader::new(stream.try_clone().unwrap())
            .read_line(&mut request)
            .unwrap();
        let payload: Value = serde_json::from_str(&request).unwrap();
        assert_eq!(payload["command"], "input.text");
        assert_eq!(payload["token"], "token-test");
        // Non-ASCII must survive UTF-8 through the JSON-line bridge unchanged.
        assert_eq!(payload["params"]["text"], "héllo");
        stream
            .write_all(
                serde_json::to_string(&json!({
                    "protocol_version": "0.1",
                    "ok": true,
                    "data": {
                        "device": "keyboard",
                        "text": "héllo",
                        "characters": 5,
                        "focus_owner": "path:/root/Main/LineEdit"
                    }
                }))
                .unwrap()
                .as_bytes(),
            )
            .unwrap();
        stream.write_all(b"\n").unwrap();
    });

    write_session_files(&root, "session-input-type-unicode-test", port);

    let output = Command::new(env!("CARGO_BIN_EXE_godot-qa"))
        .arg("--project")
        .arg(&root)
        .arg("input")
        .arg("type")
        .arg("héllo")
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
        "input type with non-ASCII text should succeed: {}",
        serde_json::to_string_pretty(&body).unwrap()
    );
    assert_eq!(body["ok"], true);
    assert_eq!(body["command"], "input type");
    assert_eq!(body["data"]["text"], "héllo");
    assert_eq!(body["data"]["characters"], 5);

    let _ = fs::remove_dir_all(root);
}
