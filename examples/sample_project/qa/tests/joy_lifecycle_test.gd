extends SceneTree
## Headless QA fake-pad ownership and cleanup; no physical controller required.

const QaRuntime = preload("res://addons/godot_qa/qa_runtime.gd")
var _events: Array[Dictionary] = []
var _failures: Array[String] = []


func _initialize() -> void:
	call_deferred("_run")


func _check(ok: bool, label: String) -> void:
	if not ok:
		_failures.append(label)
		push_error(label)


func _on_connection(device: int, connected: bool) -> void:
	_events.append({"device": device, "connected": connected,
		"roster": QaRuntime.effective_connected_joypads()})


func _run() -> void:
	Input.joy_connection_changed.connect(_on_connection)
	var runtime = QaRuntime.new(root, root)
	var connected: Dictionary = await runtime.execute("setup.joy_connect", {"devices": [40, 41]})
	_check(bool(connected.get("ok", false)), "two synthetic devices connect")
	_check(_events.size() == 2 and _events[0]["roster"] == [40]
		and _events[1]["roster"] == [40, 41], "signal listeners poll the updated roster")
	var held: Dictionary = await runtime.execute("input.joy_state", {"inputs": [
		{"device": 40, "button": {"name": "b", "pressed": true}},
		{"device": 40, "motion": {"axis": "left_x", "value": 1.0}},
		{"device": 41, "button": {"name": "b", "pressed": true}},
		{"device": 41, "motion": {"axis": "left_x", "value": -1.0}},
	], "frames": 1})
	_check(bool(held.get("ok", false)), "held button and axis inputs are injected")
	var rejected: Dictionary = await runtime.execute("setup.joy_disconnect", {"devices": [42]})
	_check(not bool(rejected.get("ok", false)) and _events.size() == 2,
		"unowned device disconnect is rejected without signal")
	var disconnected: Dictionary = await runtime.execute("setup.joy_disconnect", {"devices": [40]})
	_check(bool(disconnected.get("ok", false)), "owned device disconnect succeeds")
	_check(_events.back()["roster"] == [41] and not Input.is_joy_button_pressed(40, JOY_BUTTON_B)
		and is_zero_approx(Input.get_joy_axis(40, JOY_AXIS_LEFT_X)),
		"disconnect releases only the detached device and updates roster before signal")
	_check(Input.is_joy_button_pressed(41, JOY_BUTTON_B)
		and Input.get_joy_axis(41, JOY_AXIS_LEFT_X) < -0.9,
		"disconnect leaves the other synthetic device's input held")
	# This is the same reset path used by live scene change and session stop.
	var reset_failures: Array = await runtime.reset_input()
	_check(reset_failures.is_empty() and QaRuntime.effective_connected_joypads().is_empty(),
		"reset removes the remaining synthetic connection")
	_check(_events.back()["roster"].is_empty() and not Input.is_joy_button_pressed(41, JOY_BUTTON_B)
		and is_zero_approx(Input.get_joy_axis(41, JOY_AXIS_LEFT_X)),
		"reset releases held button and axis before final disconnect")
	await runtime.execute("setup.joy_connect", {"devices": [40]})
	await runtime.execute("input.joy_state", {"inputs": [
		{"device": 40, "button": {"name": "b", "pressed": true}},
		{"device": 40, "motion": {"axis": "left_x", "value": 1.0}},
	], "frames": 1})
	await runtime.release_faked_joypads() # Scenario failure/finalize path.
	_check(_events.back()["roster"].is_empty() and QaRuntime.effective_connected_joypads().is_empty()
		and not Input.is_joy_button_pressed(40, JOY_BUTTON_B)
		and is_zero_approx(Input.get_joy_axis(40, JOY_AXIS_LEFT_X)),
		"failed-scenario finalizer cannot leak virtual roster or held input to the next run")
	Input.joy_connection_changed.disconnect(_on_connection)
	runtime.teardown()
	runtime = null
	await process_frame
	quit(0 if _failures.is_empty() else 1)
