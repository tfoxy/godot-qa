use serde_json::{Value, json};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

fn root(label: &str) -> PathBuf {
    let n = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::env::temp_dir().join(format!(
        "godot-qa-verdict-{label}-{n}-{}",
        std::process::id()
    ))
}

fn cli(args: &[&str]) -> (bool, Value) {
    let output = Command::new(env!("CARGO_BIN_EXE_godot-qa"))
        .arg("--json")
        .arg("artifact")
        .args(args)
        .output()
        .unwrap();
    let body = serde_json::from_slice(&output.stdout)
        .unwrap_or_else(|error| panic!("{error}: {}", String::from_utf8_lossy(&output.stdout)));
    (output.status.success(), body)
}

fn begin(label: &str) -> (PathBuf, String, String) {
    let root = root(label);
    let (ok, body) = cli(&["begin-run", "--out", root.to_str().unwrap()]);
    assert!(ok, "{body}");
    let id = body["data"]["run"]["run_id"].as_str().unwrap().to_owned();
    let nonce = body["data"]["run"]["nonce"].as_str().unwrap().to_owned();
    assert_eq!(nonce.len(), 32);
    (root, id, nonce)
}

#[test]
fn begin_run_accepts_out_and_positional_compatibility_exclusively() {
    let flagged = root("flagged");
    let (ok, body) = cli(&["begin-run", "--out", flagged.to_str().unwrap()]);
    assert!(ok, "{body}");
    assert_eq!(
        Path::new(body["data"]["run_root"].as_str().unwrap())
            .canonicalize()
            .unwrap(),
        flagged.canonicalize().unwrap()
    );
    let positional = root("positional");
    let (ok, body) = cli(&["begin-run", positional.to_str().unwrap()]);
    assert!(ok, "{body}");
    assert_eq!(
        Path::new(body["data"]["run_root"].as_str().unwrap())
            .canonicalize()
            .unwrap(),
        positional.canonicalize().unwrap()
    );
    let rejected = root("both-rejected");
    let output = Command::new(env!("CARGO_BIN_EXE_godot-qa"))
        .args([
            "artifact",
            "begin-run",
            rejected.to_str().unwrap(),
            "--out",
            rejected.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
    assert!(!rejected.exists());
}

fn write(root: &Path, name: &str, mut value: Value) {
    let run: Value = serde_json::from_slice(&fs::read(root.join("run.json")).unwrap()).unwrap();
    value["schema_version"] = json!(1);
    value["started_at_unix_ms"] = run["started_at_unix_ms"].clone();
    value["completed_at_unix_ms"] = run["started_at_unix_ms"].clone();
    fs::write(root.join(name), serde_json::to_vec(&value).unwrap()).unwrap();
}

fn validate(root: &Path, extra: &[&str]) -> (bool, Value) {
    let mut args = vec![
        "validate-result",
        root.to_str().unwrap(),
        "--wrapper-exit",
        "0",
    ];
    args.extend_from_slice(extra);
    cli(&args)
}

fn kinds(body: &Value) -> Vec<&str> {
    body["failures"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|f| f["type"].as_str())
        .collect()
}

#[test]
fn wrapper_zero_child_failure_is_nonzero_and_preserves_child_failure() {
    let (root, id, nonce) = begin("child-failure");
    write(
        &root,
        "child-status.json",
        json!({"run_id":id,"nonce":nonce,"termination":"exited","exit_code":1}),
    );
    write(
        &root,
        "result.json",
        json!({"run_id":id,"nonce":nonce,"ok":false,"failures":[{"type":"parser_failed","message":"bad fixture","actual":{"line":4}}]}),
    );
    let (ok, body) = validate(&root, &[]);
    assert!(!ok);
    assert_eq!(body["data"]["first_failure"], "parser_failed");
    assert_eq!(body["failures"][0]["message"], "bad fixture");
    assert_eq!(body["failures"][0]["actual"]["line"], 4);
    assert!(kinds(&body).contains(&"child_exit_nonzero"));
    assert_eq!(body["data"]["wrapper"]["exit_code"], 0);
    assert!(root.join("verdict.json").is_file());
}

#[test]
fn success_requires_child_receipt_and_matching_result() {
    let (root, id, nonce) = begin("success");
    write(
        &root,
        "result.json",
        json!({"run_id":id,"nonce":nonce,"ok":true,"failures":[]}),
    );
    let (ok, body) = validate(&root, &[]);
    assert!(!ok);
    assert!(kinds(&body).contains(&"child_status_missing"));

    let (root, id, nonce) = begin("success-complete");
    write(
        &root,
        "result.json",
        json!({"run_id":id,"nonce":nonce,"ok":true,"failures":[]}),
    );
    write(
        &root,
        "child-status.json",
        json!({"run_id":id,"nonce":nonce,"termination":"exited","exit_code":0}),
    );
    let (ok, body) = validate(&root, &[]);
    assert!(ok, "{body}");
    assert_eq!(body["data"]["assertions_completed"], true);
    assert_eq!(body["data"]["assertions_passed"], true);
}

#[test]
fn stale_and_malformed_receipts_fail() {
    let (root, id, nonce) = begin("stale");
    write(
        &root,
        "result.json",
        json!({"run_id":id,"nonce":"old","ok":true,"failures":[]}),
    );
    write(
        &root,
        "child-status.json",
        json!({"run_id":id,"nonce":nonce,"termination":"exited","exit_code":0}),
    );
    let (ok, body) = validate(&root, &[]);
    assert!(!ok);
    assert!(kinds(&body).contains(&"child_result_stale"));

    let (root, id, nonce) = begin("malformed");
    fs::write(root.join("result.json"), b"{").unwrap();
    write(
        &root,
        "child-status.json",
        json!({"run_id":id,"nonce":nonce,"termination":"exited","exit_code":0}),
    );
    let (ok, body) = validate(&root, &[]);
    assert!(!ok);
    assert!(kinds(&body).contains(&"child_result_malformed"));
}

#[test]
fn crash_timeout_wrapper_failure_and_missing_result_are_distinct() {
    for (label, status, extra, expected) in [
        (
            "crash",
            json!({"termination":"signaled","signal":11}),
            vec![],
            "child_crashed",
        ),
        (
            "timeout",
            json!({"termination":"timed_out"}),
            vec![],
            "child_timeout",
        ),
        (
            "missing",
            json!({"termination":"exited","exit_code":0}),
            vec![],
            "child_result_missing",
        ),
        (
            "wrapper",
            json!({"termination":"exited","exit_code":0}),
            vec!["--wrapper-timed-out"],
            "wrapper_timeout",
        ),
    ] {
        let (root, id, nonce) = begin(label);
        let mut status = status;
        status["run_id"] = json!(id);
        status["nonce"] = json!(nonce);
        write(&root, "child-status.json", status);
        if label == "wrapper" {
            write(
                &root,
                "result.json",
                json!({"run_id":id,"nonce":nonce,"ok":true,"failures":[]}),
            );
        }
        let (ok, body) = validate(&root, &extra);
        assert!(!ok, "{label}");
        assert!(kinds(&body).contains(&expected), "{label}: {body}");
    }
}

#[test]
fn begin_run_never_reuses_a_directory() {
    let (root, _, _) = begin("exclusive");
    let (ok, body) = cli(&["begin-run", root.to_str().unwrap()]);
    assert!(!ok);
    assert_eq!(body["failures"][0]["type"], "run_root_exists_or_unwritable");
}

#[test]
fn independent_ids_for_same_basename_and_moved_directory() {
    let parent_a = root("same-name-a");
    let parent_b = root("same-name-b");
    let original = parent_a.join("run");
    let second = parent_b.join("run");
    let (ok_a, body_a) = cli(&["begin-run", original.to_str().unwrap()]);
    let (ok_b, body_b) = cli(&["begin-run", second.to_str().unwrap()]);
    assert!(ok_a && ok_b);
    let id_a = body_a["data"]["run"]["run_id"].as_str().unwrap();
    let id_b = body_b["data"]["run"]["run_id"].as_str().unwrap();
    assert_ne!(id_a, id_b);
    assert!(id_a.starts_with("run-"));
    let moved = parent_a.join("renamed-after-capture");
    fs::rename(&original, &moved).unwrap();
    let nonce = body_a["data"]["run"]["nonce"].as_str().unwrap();
    write(
        &moved,
        "child-status.json",
        json!({"run_id":id_a,"nonce":nonce,"termination":"exited","exit_code":0}),
    );
    write(
        &moved,
        "result.json",
        json!({"run_id":id_a,"nonce":nonce,"ok":true,"failures":[]}),
    );
    let (ok, verdict) = validate(&moved, &[]);
    assert!(ok, "{verdict}");
}

#[test]
fn receipt_version_and_start_timestamp_are_required() {
    let (root, id, nonce) = begin("schema");
    write(
        &root,
        "child-status.json",
        json!({"run_id":id,"nonce":nonce,"termination":"exited","exit_code":0}),
    );
    write(
        &root,
        "result.json",
        json!({"run_id":id,"nonce":nonce,"ok":true,"failures":[]}),
    );
    let mut result: Value =
        serde_json::from_slice(&fs::read(root.join("result.json")).unwrap()).unwrap();
    result["schema_version"] = json!(2);
    fs::write(
        root.join("result.json"),
        serde_json::to_vec(&result).unwrap(),
    )
    .unwrap();
    let (ok, body) = validate(&root, &[]);
    assert!(!ok);
    assert!(kinds(&body).contains(&"child_result_stale"));
    assert_eq!(body["data"]["assertions_completed"], false);
}

#[test]
fn verdict_write_failure_keeps_the_original_first_failure() {
    let (root, id, nonce) = begin("verdict-exists");
    write(
        &root,
        "child-status.json",
        json!({"run_id":id,"nonce":nonce,"termination":"exited","exit_code":1}),
    );
    write(
        &root,
        "result.json",
        json!({"run_id":id,"nonce":nonce,"ok":false,"failures":[{"type":"startup_failed","message":"bad startup"}]}),
    );
    fs::write(root.join("verdict.json"), b"original").unwrap();
    let (ok, body) = validate(&root, &[]);
    assert!(!ok);
    assert_eq!(body["data"]["first_failure"], "startup_failed");
    assert_eq!(body["failures"][0]["message"], "bad startup");
    assert!(kinds(&body).contains(&"verdict_write_failed"));
    assert_eq!(fs::read(root.join("verdict.json")).unwrap(), b"original");
}

#[test]
fn parser_failure_before_result_is_identified_from_scoped_godot_log() {
    let (root, id, nonce) = begin("parse-before-result");
    write(
        &root,
        "child-status.json",
        json!({"run_id":id,"nonce":nonce,"termination":"exited","exit_code":1}),
    );
    fs::write(
        root.join("logs.txt"),
        b"Godot Engine\nSCRIPT ERROR: Parse Error: Unexpected token at line 9\n",
    )
    .unwrap();
    let (ok, body) = validate(&root, &[]);
    assert!(!ok);
    assert_eq!(body["data"]["wrapper"]["exit_code"], 0);
    assert_eq!(body["data"]["first_failure"], "child_parser_failed");
    assert!(kinds(&body).contains(&"child_result_missing"));
    assert!(kinds(&body).contains(&"child_exit_nonzero"));

    let (root, id, nonce) = begin("unknown-before-result");
    write(
        &root,
        "child-status.json",
        json!({"run_id":id,"nonce":nonce,"termination":"exited","exit_code":1}),
    );
    fs::write(root.join("logs.txt"), b"ordinary custom child exited\n").unwrap();
    let (_, body) = validate(&root, &[]);
    assert_eq!(body["data"]["first_failure"], "child_exit_nonzero");
    assert!(!kinds(&body).contains(&"child_parser_failed"));
}

#[cfg(target_os = "linux")]
#[test]
fn example_inner_adapter_records_failing_child_even_if_outer_is_zero() {
    let (root, id, nonce) = begin("adapter");
    let run: Value = serde_json::from_slice(&fs::read(root.join("run.json")).unwrap()).unwrap();
    let script =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/verdict_child_status.sh");
    let child = Command::new("/bin/sh")
        .arg(script)
        .arg("/bin/sh")
        .arg("-c")
        .arg("exit 7")
        .env("GODOT_QA_RUN_ROOT", &root)
        .env("GODOT_QA_RUN_ID", id)
        .env("GODOT_QA_RUN_NONCE", nonce)
        .env(
            "GODOT_QA_RUN_STARTED_AT_UNIX_MS",
            run["started_at_unix_ms"].as_u64().unwrap().to_string(),
        )
        .status()
        .unwrap();
    assert_eq!(child.code(), Some(7));
    let (ok, body) = validate(&root, &[]);
    assert!(!ok);
    assert!(kinds(&body).contains(&"child_exit_nonzero"));
    assert_eq!(body["data"]["wrapper"]["exit_code"], 0);
}

#[cfg(target_os = "linux")]
#[test]
fn inner_adapter_escapes_awkward_directory_names_by_using_generated_id() {
    let run_root = root("quoted").join("odd\"name\nline");
    let (ok, body) = cli(&["begin-run", run_root.to_str().unwrap()]);
    assert!(ok, "{body}");
    let id = body["data"]["run"]["run_id"].as_str().unwrap();
    let nonce = body["data"]["run"]["nonce"].as_str().unwrap();
    let started = body["data"]["run"]["started_at_unix_ms"].as_u64().unwrap();
    let script =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/verdict_child_status.sh");
    let child = Command::new("/bin/sh")
        .arg(script)
        .arg("/bin/true")
        .env("GODOT_QA_RUN_ROOT", &run_root)
        .env("GODOT_QA_RUN_ID", id)
        .env("GODOT_QA_RUN_NONCE", nonce)
        .env("GODOT_QA_RUN_STARTED_AT_UNIX_MS", started.to_string())
        .status()
        .unwrap();
    assert!(child.success());
    let receipt: Value =
        serde_json::from_slice(&fs::read(run_root.join("child-status.json")).unwrap()).unwrap();
    assert_eq!(receipt["run_id"], id);
    assert_eq!(receipt["nonce"], nonce);
}

#[test]
fn declared_startup_failure_and_outer_nonzero_are_distinct() {
    let (root, id, nonce) = begin("startup-phase");
    write(
        &root,
        "child-status.json",
        json!({"run_id":id,"nonce":nonce,"termination":"exited","exit_code":2,"failure_phase":"startup"}),
    );
    let (ok, body) = cli(&[
        "validate-result",
        root.to_str().unwrap(),
        "--wrapper-exit",
        "5",
    ]);
    assert!(!ok);
    assert_eq!(body["data"]["first_failure"], "child_startup_failed");
    assert!(kinds(&body).contains(&"child_result_missing"));
    assert!(kinds(&body).contains(&"wrapper_failed"));
}
