extends SceneTree

class InstrumentedProfiler extends "res://addons/godot_qa/qa_editor_profiler.gd":
	var scans := 0

	func _scan_for_debug_data(node: Node) -> void:
		if node == _editor_interface.get_base_control():
			scans += 1
		super._scan_for_debug_data(node)

class EditorInterfaceStub extends RefCounted:
	var base_control := Control.new()

	func get_base_control() -> Control:
		return base_control

class DebuggerNode extends Node:
	signal debug_data(message: String, data: Array)
	var session_id := 1

	func get_session_id() -> int:
		return session_id

class SessionStub extends RefCounted:
	var active := true

	func is_active() -> bool:
		return active

class DebuggerPluginStub extends RefCounted:
	var sessions := {1: SessionStub.new()}

	func active_session_ids() -> Array:
		return sessions.keys()

	func get_session(session_id: int):
		return sessions.get(session_id)

	func start_servers_profiler() -> Dictionary:
		return {"session_count": sessions.size(), "toggled_count": sessions.size()}

	func stop_servers_profiler() -> Dictionary:
		return start_servers_profiler()

var checks: Array = []
var failures: Array = []


func check(name: String, condition: bool, actual: Variant) -> void:
	checks.append(name)
	if not condition:
		failures.append({"name": name, "actual": str(actual)})


func _initialize() -> void:
	call_deferred("run")


func run() -> void:
	var editor := EditorInterfaceStub.new()
	var plugin := DebuggerPluginStub.new()
	var profiler := InstrumentedProfiler.new()
	var debugger := DebuggerNode.new()
	var case := str(OS.get_cmdline_user_args()[0])
	if case == "late_debuggers":
		profiler.configure(editor)
	else:
		editor.base_control.add_child(debugger)
		profiler.configure(editor, plugin)
	profiler.begin_capture()
	var initial_scans := profiler.scans
	check("capture start discovers once", initial_scans == 1, initial_scans)
	match case:
		"throttled_discovery":
			for index in range(100):
				editor.base_control.add_child(Control.new())
			for frame in range(360):
				profiler.poll(1.0 / 60.0)
			var repeated_scans := profiler.scans - initial_scans
			check("six seconds of capture uses bounded fallback scans", repeated_scans >= 5 and repeated_scans <= 7, repeated_scans)
			check("debugger connected only once", debugger.get_signal_connection_list("debug_data").size() == 1, debugger.get_signal_connection_list("debug_data"))
			debugger.debug_data.emit("servers:function_signature", ["res://player.gd::Player::_process", 7])
			debugger.debug_data.emit("servers:profile_frame", [1, 0.01, 0.002, 0.003, 0.004, 0.005, 0, 5, 7, 2, 0.001, 0.002, 0.0])
			var payload := profiler._profile_payload("test")
			check("cached connection still captures frames and functions", payload.frame_count == 1 and payload.script_function_count == 1, payload)
			profiler.stop_and_export()
			var idle_scans := profiler.scans
			for frame in range(360):
				profiler.poll(1.0 / 60.0)
			check("idle polling never scans", profiler.scans == idle_scans, profiler.scans)
			profiler.begin_capture()
			check("capture restart discovers once", profiler.scans == idle_scans + 1, profiler.scans)
		"session_changes":
			var second := DebuggerNode.new()
			second.session_id = 2
			editor.base_control.add_child(second)
			plugin.sessions[2] = SessionStub.new()
			profiler.poll(0.0)
			check("new session immediately triggers discovery", profiler.scans == initial_scans + 1 and second.get_signal_connection_list("debug_data").size() == 1, profiler.scans)
			debugger.free()
			plugin.sessions.erase(1)
			profiler.poll(0.0)
			check("removed session prunes freed debugger", profiler._connected_debuggers.size() == 1 and profiler._profile_payload("test").session_id == "2", profiler._connected_debuggers)
			var before_restart := profiler.scans
			plugin.sessions[2].active = false
			profiler.poll(0.0)
			check("session stop triggers discovery", profiler.scans == before_restart + 1, profiler.scans)
			plugin.sessions[2].active = true
			profiler.poll(0.0)
			check("session restart triggers discovery", profiler.scans == before_restart + 2, profiler.scans)
		"late_debuggers":
			profiler.poll(0.25)
			check("empty tree retries are throttled", profiler.scans == initial_scans, profiler.scans)
			editor.base_control.add_child(debugger)
			profiler.poll(0.76)
			check("fallback discovers late debugger without plugin", debugger.get_signal_connection_list("debug_data").size() == 1 and profiler.scans == initial_scans + 1, profiler.scans)
			debugger.free()
			profiler.poll(1.01)
			check("fallback prunes freed debugger", profiler._connected_debuggers.is_empty(), profiler._connected_debuggers)
	profiler.stop_and_export()
	editor.base_control.free()
	var report := FileAccess.open("res://report.json", FileAccess.WRITE)
	report.store_string(JSON.stringify({"checks": checks, "failures": failures}))
	report.close()
	quit(0 if failures.is_empty() else 1)
