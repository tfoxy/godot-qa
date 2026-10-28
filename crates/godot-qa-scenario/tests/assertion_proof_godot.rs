//! Explicit Godot acceptance check for the lossless assertion definition binding.
use godot_qa_scenario::assertion_proof;
use serde_json::{Value, json};
use std::{
    fs,
    process::Command,
    time::{SystemTime, UNIX_EPOCH},
};

#[test]
#[ignore = "requires Godot 4 on PATH; run with --ignored"]
fn godot_assertion_ledger_preserves_numeric_definitions_and_legacy_requests() {
    let root = std::env::temp_dir().join(format!(
        "godot-qa-proof-godot-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir_all(&root).unwrap();
    fs::write(root.join("project.godot"), "config_version=5\n").unwrap();
    fs::write(
        root.join("qa_assertion_proof.gd"),
        include_str!("../../../addons/godot_qa/qa_assertion_proof.gd"),
    )
    .unwrap();
    let scenario = json!({"steps":[{"assert":[{"state_matches":{"target":"qa:state","path":"value","equals":{"values":[1,1.0000000000000002,0.12345678901234567,1.2345678901234567e-25,-0.0,9007199254740992u64,true,null,["number","0000000000000000"]]}}}]}],"assert":[{"no_errors":true}]});
    let manifest = assertion_proof::manifest(&scenario);
    let request = json!({"scenario":scenario,"run":{"id":"proof-godot","nonce":"current"},"assertion_proof":manifest});
    fs::write(
        root.join("request.json"),
        serde_json::to_vec(&request).unwrap(),
    )
    .unwrap();
    let underflow_scenario = json!({"assert":[{"state_matches":{"target":"qa:state","path":"value","equals":f64::from_bits(1)}}]});
    fs::write(root.join("underflow.json"), serde_json::to_vec(&json!({"scenario":underflow_scenario,"run":{"id":"r","nonce":"n"},"assertion_proof":assertion_proof::manifest(&underflow_scenario)})).unwrap()).unwrap();
    fs::write(root.join("test.gd"), r#"extends SceneTree
const Proof = preload("res://qa_assertion_proof.gd")
func _initialize() -> void:
	var request: Dictionary = JSON.parse_string(FileAccess.get_file_as_string("res://request.json"))
	var proof = Proof.new()
	var error: Dictionary = proof.initialize(request)
	assert(error.is_empty(), JSON.stringify(error))
	assert(proof.begin("/steps/0/assert/0") == "/steps/0/assert/0")
	proof.finish("/steps/0/assert/0", false)
	proof.begin("/assert/0")
	proof.finish("/assert/0", true)
	var output := FileAccess.open("res://result.json", FileAccess.WRITE)
	output.store_string(JSON.stringify({"ok":false,"assertion_coverage":proof.payload()}))
	output.close()
	var legacy: Dictionary = request.duplicate(true)
	legacy.erase("assertion_proof")
	var old = Proof.new()
	assert(old.initialize(legacy).is_empty() and not old.enabled)
	var changed: Dictionary = request.duplicate(true)
	changed.scenario.assert[0] = {"exists":"qa:changed"}
	assert(not Proof.new().initialize(changed).is_empty())
	var imprecise: Dictionary = request.duplicate(true)
	imprecise.assertion_proof.entries[0].assertion_identity = ["unrepresentable_integer","9007199254740993"]
	assert(not Proof.new().initialize(imprecise).is_empty())
	var underflow: Dictionary = JSON.parse_string(FileAccess.get_file_as_string("res://underflow.json"))
	assert(not Proof.new().initialize(underflow).is_empty())
	quit(0)
"#).unwrap();
    let output = Command::new("godot")
        .arg("--headless")
        .arg("--quit-after")
        .arg("60")
        .arg("--path")
        .arg(&root)
        .arg("--script")
        .arg("res://test.gd")
        .output()
        .expect("Godot 4 must be installed for this acceptance check");
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        !String::from_utf8_lossy(&output.stderr).contains("SCRIPT ERROR"),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let result: Value =
        serde_json::from_slice(&fs::read(root.join("result.json")).unwrap()).unwrap();
    let coverage = assertion_proof::validate(
        &manifest,
        &json!({"run_id":"proof-godot","nonce":"current"}),
        &result,
    )
    .unwrap();
    assert_eq!(coverage["failed"], 1);
    assert_eq!(coverage["passed"], 1);
    fs::remove_dir_all(root).unwrap();
}
