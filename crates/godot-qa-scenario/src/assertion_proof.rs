//! Nonce-bound evidence that every requested runtime assertion was executed.
use godot_qa_core::results::FailureDetail;
use serde_json::{Value, json};
use std::fs::File;
use std::io::Read;
use std::path::Path;

pub fn is_cli_assertion(assertion: &Value) -> bool {
    ["layout_audit", "screenshot_match", "saved_snapshot_pixels"]
        .iter()
        .any(|operation| assertion.get(operation).is_some())
}

/// Capture original source locations before runtime-only filtering renumbers arrays.
pub fn manifest(scenario: &Value) -> Value {
    let mut entries = Vec::new();
    let mut excluded = 0usize;
    let mut append = |assertions: &[Value], prefix: &str| {
        let mut runtime_index = 0usize;
        for (source_index, assertion) in assertions.iter().enumerate() {
            if is_cli_assertion(assertion) {
                excluded += 1;
                continue;
            }
            let operation = assertion
                .as_object()
                .and_then(|object| object.keys().find(|key| key.as_str() != "temporal"))
                .cloned()
                .unwrap_or_default();
            entries.push(json!({
                "source_id": format!("{prefix}/{source_index}"),
                "runtime_path": format!("{prefix}/{runtime_index}"),
                "operation": operation,
                "assertion": assertion,
                "assertion_identity": definition_identity(assertion),
            }));
            runtime_index += 1;
        }
    };
    // Execution order is step assertions followed by final assertions.
    if let Some(steps) = scenario.get("steps").and_then(Value::as_array) {
        for (index, step) in steps.iter().enumerate() {
            if let Some(assertions) = step.get("assert").and_then(Value::as_array) {
                append(assertions, &format!("/steps/{index}/assert"));
            }
        }
    }
    if let Some(assertions) = scenario.get("assert").and_then(Value::as_array) {
        append(assertions, "/assert");
    }
    json!({"schema_version": 1, "entries": entries, "cli_assertions_excluded": excluded})
}

pub fn validate(
    manifest: &Value,
    identity: &Value,
    result: &Value,
) -> Result<Value, FailureDetail> {
    let failure = |kind: &str, message: &str, actual: Value| FailureDetail {
        kind: kind.to_string(),
        message: Some(message.to_string()),
        expected: Some(manifest.clone()),
        actual: Some(actual),
        ..FailureDetail::default()
    };
    let Some(coverage) = result.get("assertion_coverage") else {
        return Err(failure(
            "assertion_proof_missing",
            "runtime did not return required assertion execution proof; update the project's Godot QA addon to the revision matching this CLI and rerun",
            Value::Null,
        ));
    };
    let invalid = || {
        failure(
            "assertion_proof_invalid",
            "runtime assertion execution proof does not match this run's original assertion manifest",
            coverage.clone(),
        )
    };
    if coverage.get("schema_version").and_then(Value::as_u64) != Some(1)
        || coverage.get("run_id") != identity.get("run_id")
        || coverage.get("nonce") != identity.get("nonce")
        || identity
            .get("run_id")
            .and_then(Value::as_str)
            .is_none_or(str::is_empty)
        || identity
            .get("nonce")
            .and_then(Value::as_str)
            .is_none_or(str::is_empty)
        || coverage.get("cli_assertions_excluded") != manifest.get("cli_assertions_excluded")
    {
        return Err(invalid());
    }
    let expected = manifest
        .get("entries")
        .and_then(Value::as_array)
        .ok_or_else(invalid)?;
    let entries = coverage
        .get("entries")
        .and_then(Value::as_array)
        .ok_or_else(invalid)?;
    if entries.len() != expected.len() {
        return Err(invalid());
    }
    let mut started = 0usize;
    let mut passed = 0usize;
    let mut failed = 0usize;
    for (entry, expected) in entries.iter().zip(expected) {
        if has_unrepresentable_integer(&expected["assertion_identity"]) {
            return Err(failure(
                "assertion_proof_invalid",
                "assertion contains an integer that cannot be represented exactly by Godot JSON binary64",
                coverage.clone(),
            ));
        }
        for key in ["source_id", "runtime_path", "operation"] {
            if entry.get(key) != expected.get(key) {
                return Err(invalid());
            }
        }
        if entry.get("assertion_identity") != expected.get("assertion_identity") {
            return Err(invalid());
        }
        match entry.get("status").and_then(Value::as_str) {
            Some("not_started") => {}
            Some("running") => started += 1,
            Some("passed") => {
                started += 1;
                passed += 1;
            }
            Some("failed") => {
                started += 1;
                failed += 1;
            }
            _ => return Err(invalid()),
        }
    }
    let finished = passed + failed;
    let complete = finished == expected.len();
    for (key, count) in [
        ("expected", expected.len()),
        ("started", started),
        ("finished", finished),
        ("passed", passed),
        ("failed", failed),
    ] {
        if coverage.get(key).and_then(Value::as_u64) != Some(count as u64) {
            return Err(invalid());
        }
    }
    if coverage.get("complete").and_then(Value::as_bool) != Some(complete)
        || (result.get("ok").and_then(Value::as_bool) == Some(true) && failed != 0)
    {
        return Err(invalid());
    }
    if !complete {
        return Err(failure(
            "assertion_proof_incomplete",
            "runtime did not finish every requested assertion; inspect assertion coverage and progress artifacts",
            coverage.clone(),
        ));
    }
    Ok(coverage.clone())
}

fn has_unrepresentable_integer(identity: &Value) -> bool {
    match identity {
        Value::Array(items) => {
            items.first().and_then(Value::as_str) == Some("unrepresentable_integer")
                || items.iter().any(has_unrepresentable_integer)
        }
        Value::Object(items) => items.values().any(has_unrepresentable_integer),
        _ => false,
    }
}

/// Lossless `json_f64_le_v1` binding. Numeric JSON rendering in Godot can lose
/// precision, so numbers travel as little-endian binary64 hex, with typed nodes
/// preventing collisions with user objects/arrays. Signed zero is normalized.
pub fn definition_identity(value: &Value) -> Value {
    match value {
        Value::Null => json!(["null"]),
        Value::Bool(value) => json!(["bool", value]),
        Value::String(value) => json!(["string", value]),
        Value::Number(number) => {
            let float = number.as_f64().unwrap();
            let integer = number
                .as_i64()
                .map(i128::from)
                .or_else(|| number.as_u64().map(i128::from));
            if integer.is_some_and(|integer| float as i128 != integer) {
                return json!(["unrepresentable_integer", number.to_string()]);
            }
            let float = if float == 0.0 { 0.0 } else { float };
            let hex: String = float
                .to_le_bytes()
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect();
            json!(["number", hex])
        }
        Value::Array(values) => json!([
            "array",
            values.iter().map(definition_identity).collect::<Vec<_>>()
        ]),
        Value::Object(values) => json!([
            "object",
            values
                .iter()
                .map(|(key, value)| (key.clone(), definition_identity(value)))
                .collect::<serde_json::Map<_, _>>()
        ]),
    }
}

/// Diagnostic-only progress after a crash. A progress ledger cannot prove completion.
pub fn partial_progress(manifest: &Value, identity: &Value, progress_path: &Path) -> Value {
    let mut entries = manifest["entries"].as_array().cloned().unwrap_or_default();
    for entry in &mut entries {
        entry["status"] = json!("not_started");
    }
    let mut text = String::new();
    if let Ok(file) = File::open(progress_path) {
        let _ = file.take(8 * 1024 * 1024).read_to_string(&mut text);
    }
    let identity_present = identity
        .get("run_id")
        .and_then(Value::as_str)
        .is_some_and(|id| !id.is_empty())
        && identity
            .get("nonce")
            .and_then(Value::as_str)
            .is_some_and(|nonce| !nonce.is_empty());
    for line in text.lines() {
        let Ok(event) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        if !identity_present
            || event.get("run_id") != identity.get("run_id")
            || event.get("nonce") != identity.get("nonce")
            || event["operation"] != "assertion"
        {
            continue;
        }
        let Some(entry) = entries
            .iter_mut()
            .find(|entry| entry.get("source_id") == event.get("source_id"))
        else {
            continue;
        };
        match event["event"].as_str() {
            Some("start") if entry["status"] == "not_started" => entry["status"] = json!("running"),
            Some("finish") if entry["status"] == "running" => {
                if let Some(passed) = event["ok"].as_bool() {
                    entry["status"] = json!(if passed { "passed" } else { "failed" });
                }
            }
            _ => {}
        }
    }
    json!({"schema_version":1,"source":"partial_progress","verified":false,"complete":false,"run_id":identity.get("run_id"),"nonce":identity.get("nonce"),"expected":entries.len(),"cli_assertions_excluded":manifest["cli_assertions_excluded"],"entries":entries,"progress_path":progress_path})
}

/// Explicit current-addon response for process/transport tests, never runtime code.
#[cfg(test)]
pub(crate) fn passed_fixture(request: &Value, mut result: Value) -> Value {
    let entries: Vec<Value> = request["assertion_proof"]["entries"]
        .as_array()
        .unwrap()
        .iter()
        .map(|entry| {
            let mut entry = entry.clone();
            entry["status"] = json!("passed");
            entry
        })
        .collect();
    let count = entries.len();
    result["run_identity"] =
        json!({"run_id":request["run"]["id"], "nonce":request["run"]["nonce"]});
    result["assertion_coverage"] = json!({"schema_version":1,"run_id":request["run"]["id"],"nonce":request["run"]["nonce"],"expected":count,"started":count,"finished":count,"passed":count,"failed":0,"complete":true,"cli_assertions_excluded":request["assertion_proof"]["cli_assertions_excluded"],"entries":entries});
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn original_locations_survive_cli_filtering_in_both_scopes() {
        let scenario = json!({"steps":[{"assert":[{"layout_audit":"default"},{"exists":"qa:x"},{"exists":"qa:x"}]}],"assert":[{"screenshot_match":{}},{"no_errors":true},{"saved_snapshot_pixels":{}}]});
        let manifest = manifest(&scenario);
        assert_eq!(manifest["cli_assertions_excluded"], 3);
        assert_eq!(manifest["entries"][0]["source_id"], "/steps/0/assert/1");
        assert_eq!(manifest["entries"][0]["runtime_path"], "/steps/0/assert/0");
        assert_eq!(manifest["entries"][1]["source_id"], "/steps/0/assert/2");
        assert_eq!(manifest["entries"][2]["source_id"], "/assert/1");
        assert_eq!(manifest["entries"][2]["runtime_path"], "/assert/0");
    }

    fn proof_fixture() -> (Value, Value, Value) {
        let manifest = manifest(&json!({"assert":[{"no_errors":true}]}));
        let identity = json!({"run_id":"r", "nonce":"n"});
        let mut entry = manifest["entries"][0].clone();
        entry["status"] = json!("passed");
        let result = json!({"ok":true,"assertion_coverage":{"schema_version":1,"run_id":"r","nonce":"n","entries":[entry],"expected":1,"started":1,"finished":1,"passed":1,"failed":0,"complete":true,"cli_assertions_excluded":0}});
        (manifest, identity, result)
    }

    #[test]
    fn rejects_missing_stale_duplicate_and_inconsistent_proof() {
        let (manifest, identity, result) = proof_fixture();
        assert!(validate(&manifest, &identity, &result).is_ok());
        assert_eq!(
            validate(&manifest, &identity, &json!({"ok":true}))
                .unwrap_err()
                .kind,
            "assertion_proof_missing"
        );
        for (pointer, value) in [
            ("/nonce", json!("old")),
            ("/finished", json!(0)),
            ("/complete", json!(false)),
            ("/entries/0/source_id", json!("/assert/8")),
            ("/entries/0/status", json!("unknown")),
        ] {
            let mut changed = result.clone();
            *changed["assertion_coverage"].pointer_mut(pointer).unwrap() = value;
            assert_eq!(
                validate(&manifest, &identity, &changed).unwrap_err().kind,
                "assertion_proof_invalid"
            );
        }
        let mut duplicate = result.clone();
        let entry = duplicate["assertion_coverage"]["entries"][0].clone();
        duplicate["assertion_coverage"]["entries"]
            .as_array_mut()
            .unwrap()
            .push(entry);
        assert!(validate(&manifest, &identity, &duplicate).is_err());
    }

    #[test]
    fn failed_assertions_finish_but_early_aborts_do_not() {
        let (manifest, identity, mut result) = proof_fixture();
        result["ok"] = json!(false);
        result["assertion_coverage"]["passed"] = json!(0);
        result["assertion_coverage"]["failed"] = json!(1);
        result["assertion_coverage"]["entries"][0]["status"] = json!("failed");
        assert!(validate(&manifest, &identity, &result).is_ok());
        result["assertion_coverage"]["failed"] = json!(0);
        result["assertion_coverage"]["finished"] = json!(0);
        result["assertion_coverage"]["complete"] = json!(false);
        result["assertion_coverage"]["entries"][0]["status"] = json!("running");
        assert_eq!(
            validate(&manifest, &identity, &result).unwrap_err().kind,
            "assertion_proof_incomplete"
        );
    }

    #[test]
    fn changed_runtime_and_manifest_cannot_claim_original_assertion_execution() {
        let original = manifest(&json!({"assert":[{"exists":"qa:A"}]}));
        let request = json!({"run":{"id":"r","nonce":"n"},"assertion_proof":manifest(&json!({"assert":[{"exists":"qa:B"}]}))});
        let result = passed_fixture(&request, json!({"ok":true}));
        assert_eq!(
            validate(&original, &json!({"run_id":"r","nonce":"n"}), &result)
                .unwrap_err()
                .kind,
            "assertion_proof_invalid"
        );
    }

    #[test]
    fn definition_equality_accepts_exact_json_number_spelling_changes_only() {
        assert_eq!(
            definition_identity(&json!({"a":[1,2,{"x":3}]})),
            definition_identity(&json!({"a":[1.0,2.0,{"x":3.0}]}))
        );
        assert_ne!(
            definition_identity(&json!(9007199254740993u64)),
            definition_identity(&json!(9007199254740992.0))
        );
        assert_ne!(
            definition_identity(&json!(u64::MAX)),
            definition_identity(&json!(18446744073709551616.0))
        );
        assert_ne!(
            definition_identity(&json!([true, null])),
            definition_identity(&json!([1, 0]))
        );
        assert_eq!(
            definition_identity(&json!(-0.0)),
            definition_identity(&json!(0))
        );
        for float in [
            1.0000000000000002,
            0.12345678901234567,
            1.2345678901234567e-25,
            f64::MIN_POSITIVE,
            f64::from_bits(1),
        ] {
            assert_eq!(
                definition_identity(&json!(float))[1],
                float
                    .to_le_bytes()
                    .iter()
                    .map(|byte| format!("{byte:02x}"))
                    .collect::<String>()
            );
        }
    }

    #[test]
    fn integer_precision_loss_cannot_be_approved_by_a_matching_echo() {
        let manifest = manifest(
            &json!({"assert":[{"state_matches":{"target":"qa:state","path":"counter","equals":9007199254740993u64}}]}),
        );
        let request = json!({"run":{"id":"r","nonce":"n"},"assertion_proof":manifest});
        let result = passed_fixture(&request, json!({"ok":true}));
        assert_eq!(
            validate(&manifest, &json!({"run_id":"r","nonce":"n"}), &result)
                .unwrap_err()
                .kind,
            "assertion_proof_invalid"
        );
    }

    #[test]
    fn partial_progress_is_nonce_bound_and_never_proves_completion() {
        let (manifest, identity, _) = proof_fixture();
        let path = std::env::temp_dir().join(format!(
            "godot-qa-proof-progress-{}-{}.jsonl",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::write(&path, format!("{}\n{}\n{{\"event\":", json!({"event":"finish","operation":"assertion","source_id":"/assert/0","run_id":"r","nonce":"old","ok":true}), json!({"event":"start","operation":"assertion","source_id":"/assert/0","run_id":"r","nonce":"n"}))).unwrap();
        let partial = partial_progress(&manifest, &identity, &path);
        assert_eq!(partial["entries"][0]["status"], "running");
        assert_eq!(partial["complete"], false);
        std::fs::write(&path, format!("{}\n{}\n", json!({"event":"start","operation":"assertion","source_id":"/assert/0","run_id":"r","nonce":"n"}), json!({"event":"finish","operation":"assertion","source_id":"/assert/0","run_id":"r","nonce":"n","ok":false}))).unwrap();
        let partial = partial_progress(&manifest, &identity, &path);
        assert_eq!(partial["entries"][0]["status"], "failed");
        assert_eq!(partial["complete"], false);
        assert_eq!(partial["verified"], false);
        assert_eq!(
            partial_progress(&manifest, &Value::Null, &path)["entries"][0]["status"],
            "not_started"
        );
        std::fs::remove_file(path).unwrap();
    }
}
