@tool
extends EditorDebuggerPlugin

const PROFILER_NAME := "servers"
const DEFAULT_MAX_FUNCTIONS := 512
const DEFAULT_INCLUDE_NATIVE := false

var _sessions := {}


func _setup_session(session_id: int) -> void:
	_sessions[session_id] = true


func _goto_script_line(_script: RefCounted, _line: int) -> void:
	pass


func start_servers_profiler(max_functions: int = DEFAULT_MAX_FUNCTIONS, include_native: bool = DEFAULT_INCLUDE_NATIVE) -> Dictionary:
	return _toggle_servers_profiler(true, max_functions, include_native)


func stop_servers_profiler() -> Dictionary:
	return _toggle_servers_profiler(false, DEFAULT_MAX_FUNCTIONS, DEFAULT_INCLUDE_NATIVE)


func active_session_ids() -> Array:
	return _sessions.keys()


func _toggle_servers_profiler(enabled: bool, max_functions: int, include_native: bool) -> Dictionary:
	var toggled := 0
	var failed: Array[String] = []
	for session_id in _sessions.keys():
		var session = get_session(int(session_id))
		if session == null:
			failed.append(str(session_id))
			continue
		session.toggle_profiler(PROFILER_NAME, enabled, [max_functions, include_native])
		toggled += 1
	return {
		"session_count": _sessions.size(),
		"toggled_count": toggled,
		"failed_sessions": failed,
	}
