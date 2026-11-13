extends SceneTree
## godot --headless --path examples/sample_project --script res://qa/tests/key_lifecycle_test.gd

const QaRuntime = preload("res://addons/godot_qa/qa_runtime.gd")
const QaScenarioExecutor = preload("res://addons/godot_qa/qa_scenario_executor.gd")

var _failures: Array[String] = []
var _checks := 0


func _initialize() -> void:
	call_deferred("_run")


func _check(ok: bool, label: String) -> void:
	_checks += 1
	if not ok:
		_failures.append(label)
		push_error(label)


func _run() -> void:
	await process_frame
	var runtime := QaRuntime.new(root, root)
	for frames in [0, 1, 5]:
		var started := Engine.get_process_frames()
		var tapped: Dictionary = await runtime.execute("input.key", {"key": "Backspace", "frames": frames})
		_check(bool(tapped.get("ok", false)) and not Input.is_key_pressed(KEY_BACKSPACE),
			"tap releases Backspace after %d frames" % frames)
		_check(Engine.get_process_frames() - started == frames + 1,
			"tap waits %d press frames and one delivery frame" % frames)
	var default_started := Engine.get_process_frames()
	var default_tap: Dictionary = await runtime.execute("input.key", {"key": "return"})
	_check(bool(default_tap.get("ok", false)) and default_tap.get("data", {}).get("key") == "Enter"
		and Engine.get_process_frames() - default_started == 2, "default tap retains one-frame alias behavior")
	for command in ["input.key", "input.key_hold"]:
		for frames in [-1, 1.5, "2", true, null]:
			var invalid: Dictionary = await runtime.execute(command, {"key": "Backspace", "frames": frames})
			_check(not bool(invalid.get("ok", false)) and invalid.get("error", {}).get("type") == "invalid_params"
				and not Input.is_key_pressed(KEY_BACKSPACE), "%s rejects invalid frames %s before pressing" % [command, frames])
	for command in ["input.key", "input.key_hold", "input.key_release"]:
		var invalid: Dictionary = await runtime.execute(command, {"key": "unsupported-key"})
		_check(not bool(invalid.get("ok", false)) and invalid.get("error", {}).get("type") == "invalid_params",
			"%s rejects unknown keys" % command)
	runtime.teardown()

	var held_keys := {}
	var first := QaRuntime.new(root, root, [], {}, held_keys)
	var second := QaRuntime.new(root, root, [], {}, held_keys)
	var held_started := Engine.get_process_frames()
	var held: Dictionary = await first.execute("input.key_hold", {"key": "Backspace", "frames": 5})
	_check(bool(held.get("ok", false)) and held.get("data", {}).get("held", false)
		and held.get("data", {}).get("held_keys") == ["Backspace"] and Input.is_key_pressed(KEY_BACKSPACE)
		and Engine.get_process_frames() - held_started == 5, "hold waits requested frames and leaves key pressed")
	first.teardown()
	var repeated: Dictionary = await second.execute("input.key_hold", {"key": "backspace", "frames": 0})
	_check(bool(repeated.get("ok", false)) and repeated.get("data", {}).get("held_keys") == ["Backspace"],
		"repeat hold is idempotent across runtime instances and aliases")
	var other: Dictionary = await second.execute("input.key_hold", {"key": "a"})
	_check(bool(other.get("ok", false)) and other.get("data", {}).get("frames") == 1
		and other.get("data", {}).get("held_keys") == ["A", "Backspace"], "held keys are sorted and default frames is one")
	var released: Dictionary = await second.execute("input.key_release", {"key": "BACKSPACE"})
	_check(bool(released.get("ok", false)) and released.get("data", {}).get("held_keys") == ["A"]
		and not Input.is_key_pressed(KEY_BACKSPACE) and Input.is_key_pressed(KEY_A), "release affects only the named key")
	var missing: Dictionary = await second.execute("input.key_release", {"key": "Backspace"})
	_check(not bool(missing.get("ok", false)) and missing.get("error", {}).get("type") == "key_not_held",
		"second release fails key_not_held")
	var reset: Dictionary = await second.execute("input.reset")
	_check(bool(reset.get("ok", false)) and held_keys.is_empty() and not Input.is_key_pressed(KEY_A),
		"reset clears shared tracking and Godot key state")
	_check((await second.reset_input()).is_empty(), "reset without held keys is a no-op")
	second.teardown()
	await _check_tapping_held_keys()
	await _check_scenario_keys()
	print("key lifecycle: %d checks, %d failures" % [_checks, _failures.size()])
	quit(0 if _failures.is_empty() else 1)


func _check_tapping_held_keys() -> void:
	for case in [
		{"key": "Backspace", "keycode": KEY_BACKSPACE, "command": "input.key", "params": {"key": "BACKSPACE"}},
		{"key": "Enter", "keycode": KEY_ENTER, "command": "input.key", "params": {"key": "return"}},
		{"key": "A", "keycode": KEY_A, "command": "input.text", "params": {"text": "a"}},
	]:
		var held_keys := {}
		var runtime := QaRuntime.new(root, root, [], {}, held_keys)
		var held: Dictionary = await runtime.execute("input.key_hold", {"key": case.key, "frames": 1})
		_check(bool(held.get("ok", false)) and Input.is_key_pressed(case.keycode),
			"hold %s before tapping through %s" % [case.key, case.command])
		var tapped: Dictionary = await runtime.execute(case.command, case.params)
		_check(bool(tapped.get("ok", false)) and not Input.is_key_pressed(case.keycode) and held_keys.is_empty(),
			"%s clears held %s when its tap releases" % [case.command, case.key])
		var other_key := "B" if case.key == "A" else "A"
		var other: Dictionary = await runtime.execute("input.key_hold", {"key": other_key, "frames": 1})
		_check(bool(other.get("ok", false)) and other.get("data", {}).get("held_keys") == [other_key],
			"holding %s after tapping %s reports only the second key" % [other_key, case.key])
		var released: Dictionary = await runtime.execute("input.key_release", {"key": case.key})
		_check(not bool(released.get("ok", false)) and released.get("error", {}).get("type") == "key_not_held",
			"release of tapped %s fails key_not_held after %s" % [case.key, case.command])
		await runtime.reset_input()
		runtime.teardown()


func _check_scenario_keys() -> void:
	var executor := QaScenarioExecutor.new()
	for kind in ["input_key", "input_key_hold", "input_key_release"]:
		_check(kind in executor.SUPPORTED_STEP_TYPES, "%s is a supported saved step" % kind)
		for payload in ["Backspace", {"key": "Backspace"}]:
			var mapped: Dictionary = executor._runtime_command_for_step({kind: payload})
			var expected := {"key": "Backspace"}
			if kind != "input_key_release":
				expected["frames"] = 1
			_check(mapped == {"command": kind.replace("input_", "input."), "params": expected},
				"%s maps string/object payload with default frames" % kind)
	for kind in ["input_key", "input_key_hold"]:
		for frames in [0, 75]:
			var mapped: Dictionary = executor._runtime_command_for_step({kind: {"key": "Backspace", "frames": frames}})
			_check(mapped.get("params") == {"key": "Backspace", "frames": frames},
				"%s preserves explicit frames %d" % [kind, frames])
	var held_keys := {}
	var runtime := QaRuntime.new(root, root, [], {}, held_keys)
	executor.set("_held_keys", held_keys)
	executor.set("_runtime", runtime)
	executor.set("_scenario", {"name": "keyboard_cleanup"})
	await runtime.execute("input.key_hold", {"key": "Backspace", "frames": 0})
	await runtime.execute("input.key_hold", {"key": "a", "frames": 0})
	var finalized: Dictionary = await executor._finalize_result_payload(false)
	var failures: Array = finalized.get("payload", {}).get("failures", [])
	_check(failures.size() == 1 and failures[0].get("type") == "runtime_contamination_detected"
		and failures[0].get("actual") == ["A", "Backspace"]
		and "A, Backspace" in str(failures[0].get("message")), "finalize fails with sorted unreleased key names")
	_check(held_keys.is_empty() and not Input.is_key_pressed(KEY_A) and not Input.is_key_pressed(KEY_BACKSPACE),
		"failed finalization releases key state for the next warm scenario")
	# Explicit release leaves a clean finalizer without a contamination failure.
	var clean := QaScenarioExecutor.new()
	held_keys = {}
	runtime = QaRuntime.new(root, root, [], {}, held_keys)
	clean.set("_held_keys", held_keys)
	clean.set("_runtime", runtime)
	await runtime.execute("input.key_hold", {"key": "Backspace", "frames": 0})
	await runtime.execute("input.key_release", {"key": "Backspace"})
	var clean_finalized: Dictionary = await clean._finalize_result_payload(false)
	_check(clean_finalized.get("payload", {}).get("failures", []).is_empty(),
		"explicitly released keys finalize without failures")
