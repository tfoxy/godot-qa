extends SceneTree
## godot --headless --path examples/sample_project --script res://qa/tests/json_equality_test.gd

const Equality = preload("res://addons/godot_qa/qa_json_equality.gd")
const QaRuntime = preload("res://addons/godot_qa/qa_runtime.gd")
const QaExecutor = preload("res://addons/godot_qa/qa_scenario_executor.gd")

class Probe extends Node:
	var payload: Array = [1, {"nested": [2, null, true]}]

	func qa_state() -> Dictionary:
		return {"payload": payload, "items": [{"id": {"parts": [7]}, "value": [3]}], "nullable": null}

var _failures: Array[String] = []
var _checks := 0


func _initialize() -> void:
	call_deferred("_run")


func _check(ok: bool, label: String) -> void:
	_checks += 1
	if not ok:
		_failures.append(label)
		push_error(label)


func _equal(actual: Variant, expected: Variant, verdict: bool, label: String) -> void:
	_check(Equality.equals(actual, expected) == verdict, label)
	_check(Equality.equals(expected, actual) == verdict, "%s (symmetric)" % label)


func _state(runtime: RefCounted, check: Dictionary, verdict: bool, label: String) -> void:
	var params := {"target": "qa:json_probe", "source": "method", "method": "qa_state"}
	params.merge(check)
	var result: Dictionary = await runtime.execute("assert.state_matches", params)
	_check(bool(result.get("ok", false)) == verdict, "%s: %s" % [label, result])


func _run() -> void:
	_equal([1, {"a": [2]}], JSON.parse_string('[1, {"a": [2]}]'), true, "nested JSON numbers equal runtime integers")
	_equal([1, 2], [2.0, 1.0], false, "array order matters")
	_equal([1], [1.0, 2.0], false, "array length matters")
	_equal({"a": null}, {}, false, "missing key differs from null")
	_equal({"a": 1}, {"b": 1.0}, false, "dictionary key presence matters")
	_equal({"a": 1, "b": 2}, {"b": 2.0, "a": 1.0}, true, "dictionary order does not matter")
	_equal(true, 1, false, "boolean differs from integer")
	_equal(false, 0.0, false, "boolean differs from float")
	_equal(null, 0, false, "null differs from integer")
	_equal(1, 1.0000000000000002, false, "fractional float has no epsilon")
	_equal(1.0, 1.0000000000000002, false, "float equality has no epsilon")
	_equal(9007199254740992, 9007199254740992.0, true, "2^53 is exact")
	_equal(9007199254740993, 9007199254740992.0, false, "integer above 2^53 does not round into equality")
	_equal(9007199254740994, 9007199254740994.0, true, "representable integer above 2^53 remains exact")
	_equal(9223372036854775807, 9223372036854775808.0, false, "rounded int64 upper boundary is excluded")
	_equal(-9223372036854775807 - 1, -9223372036854775808.0, true, "int64 lower boundary is exact")
	_equal(-9223372036854775807, -9223372036854775808.0, false, "int64 lower neighbor does not round into equality")
	_equal(0, -0.0, true, "negative floating zero equals integer zero")
	_equal(1, INF, false, "infinity is not an integer")
	_equal(1, NAN, false, "NaN is not an integer")

	var scene := Node.new()
	root.add_child(scene)
	var probe := Probe.new()
	probe.set_meta("qa_id", "json_probe")
	scene.add_child(probe)
	var runtime := QaRuntime.new(scene, root)
	var parsed_payload: Variant = JSON.parse_string('[1, {"nested": [2, null, true]}]')
	await _state(runtime, {"path": "payload", "expected": parsed_payload}, true, "live state equality")
	await _state(runtime, {"path": "items", "where": JSON.parse_string('{"id": {"parts": [7]}}'), "value_path": "value", "expected": [3.0]}, true, "selection uses recursive equality")
	await _state(runtime, {"path": "payload", "operator": "contains", "expected": JSON.parse_string('{"nested": [2, null, true]}')}, true, "array contains uses recursive equality")
	await _state(runtime, {"path": "", "operator": "contains", "expected": {"payload": parsed_payload}}, true, "dictionary subset contains uses recursive equality")
	await _state(runtime, {"path": "payload", "operator": "contains", "expected": true}, false, "contains does not confuse boolean and integer")
	await _state(runtime, {"path": "missing", "expected": null}, false, "missing state path differs from explicit null")
	await _state(runtime, {"path": "nullable", "expected": null}, true, "explicit null matches")

	var executor := QaExecutor.new(self)
	executor.set("_runtime", runtime)
	var state_params := {"target": "qa:json_probe", "source": "method", "method": "qa_state", "path": "payload", "expected": parsed_payload}
	var scenario_command: Dictionary = executor.call("_runtime_command_for_assertion", {"state_matches": state_params})
	var scenario_result: Dictionary = await runtime.execute(scenario_command.get("command", ""), scenario_command.get("params", {}))
	_check(scenario_command.get("command") == "assert.state_matches" and bool(scenario_result.get("ok", false)), "scenario assertion dispatch shares equality")
	var wait_params := state_params.duplicate(true)
	wait_params["timeout_frames"] = 1
	var wait_result: Dictionary = await executor.call("_run_wait_until", wait_params)
	_check(bool(wait_result.get("ok", false)), "wait_until shares recursive equality: %s" % wait_result)
	for property_operator in ["equals", "contains"]:
		var observe_result: Dictionary = await runtime.execute("assert.observe_until", {"target": "qa:json_probe", "property": "payload", "operator": property_operator, "expected": parsed_payload if property_operator == "equals" else {"nested": [2.0, null, true]}, "window_frames": 2})
		_check(bool(observe_result.get("ok", false)), "observe property %s shares equality: %s" % [property_operator, observe_result])
	var observe_state := state_params.duplicate(true)
	observe_state.erase("path")
	observe_state.erase("expected")
	observe_state["matches"] = [{"path": "payload", "expected": parsed_payload}]
	observe_state["window_frames"] = 2
	var observe_state_result: Dictionary = await runtime.execute("assert.observe_until", observe_state)
	_check(bool(observe_state_result.get("ok", false)), "observe state shares recursive equality: %s" % observe_state_result)
	executor.set("_runtime", null)
	executor = null
	runtime.teardown()
	runtime = null
	scene.queue_free()
	await process_frame
	print(JSON.stringify({"test": "json_equality", "ok": _failures.is_empty(), "checks": _checks, "failures": _failures}))
	quit(0 if _failures.is_empty() else 1)
