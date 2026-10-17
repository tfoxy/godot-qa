@tool
extends RefCounted

const SOURCE := "editor_script_functions"
const CAPTURE_METHOD := "editor_debugger_raw_servers_profile"
const MAX_RETAINED_FRAMES := 3000

var _editor_interface = null
var _debugger_plugin = null
var _capture_active := false
var _run_id := ""
var _signature_by_id := {}
var _aggregate_by_signature := {}
var _frames: Array[Dictionary] = []
var _warnings: Array[String] = []
var _connected_debuggers := {}
var _last_artifact_path := ""
var _last_frame_count := 0
var _last_function_count := 0
var _last_export_reason := ""


func configure(editor_interface, debugger_plugin = null) -> void:
	_editor_interface = editor_interface
	_debugger_plugin = debugger_plugin


func begin_capture() -> void:
	_capture_active = true
	_run_id = _make_run_id()
	_signature_by_id.clear()
	_aggregate_by_signature.clear()
	_frames.clear()
	_warnings.clear()
	_last_export_reason = "editor_profiler_started"
	_connect_debug_data_sources()
	if _debugger_plugin != null and _debugger_plugin.has_method("start_servers_profiler"):
		var result = _debugger_plugin.call("start_servers_profiler")
		_record_toggle_result(result, "start")
	else:
		_add_warning("editor_profiler.debugger_plugin_unavailable")
	export_latest()


func stop_and_export() -> void:
	if _debugger_plugin != null and _debugger_plugin.has_method("stop_servers_profiler"):
		var result = _debugger_plugin.call("stop_servers_profiler")
		_record_toggle_result(result, "stop")
	_capture_active = false
	_last_export_reason = "editor_profiler_stopped"
	export_latest()


func export_latest() -> void:
	if _run_id == "":
		_run_id = _make_run_id()
	_connect_debug_data_sources()
	if _connected_debuggers.is_empty():
		_add_warning("editor_profiler.debug_data_unavailable")
	var payload := _profile_payload(_last_export_reason if _last_export_reason != "" else "editor_profiler_export_latest")
	var root_dir := _bridge_root_abs()
	var run_root := "%s/%s" % [root_dir, _run_id]
	var profile_path := "%s/profile.json" % run_root
	var latest_path := "%s/latest.json" % root_dir
	DirAccess.make_dir_recursive_absolute(run_root)
	_write_json(profile_path, payload)
	_write_json(latest_path, payload)
	_last_artifact_path = profile_path
	_last_frame_count = int(payload.get("frame_count", 0))
	_last_function_count = int(payload.get("script_function_count", 0))


func poll(_delta: float) -> void:
	if _capture_active:
		_connect_debug_data_sources()


func status_text() -> String:
	var state := "Capturing" if _capture_active else "Idle"
	var suffix := ""
	if _last_artifact_path != "":
		suffix = " - frames: %d, functions: %d, artifact: %s" % [_last_frame_count, _last_function_count, _last_artifact_path]
	return "%s%s" % [state, suffix]


func _connect_debug_data_sources() -> void:
	var base_control = _editor_base_control()
	if base_control == null:
		return
	_scan_for_debug_data(base_control)


func _scan_for_debug_data(node: Node) -> void:
	if _has_signal_named(node, "debug_data"):
		var instance_id := int(node.get_instance_id())
		if not _connected_debuggers.has(instance_id):
			var label := _session_label(node)
			var callable := Callable(self, "_on_debug_data").bind(label)
			node.connect("debug_data", callable)
			_connected_debuggers[instance_id] = label
	for child in node.get_children():
		if child is Node:
			_scan_for_debug_data(child)


func _has_signal_named(node: Object, signal_name: String) -> bool:
	for signal_info in node.get_signal_list():
		if str(signal_info.get("name", "")) == signal_name:
			return true
	return false


func _on_debug_data(message: String, data: Array, session_id: String) -> void:
	if not _capture_active:
		return
	if message == "servers:function_signature":
		_capture_function_signature(data)
	elif message == "servers:profile_frame" or message == "servers:profile_total":
		_capture_profile_frame(message, data, session_id)


func _capture_function_signature(data: Array) -> void:
	if data.size() < 2:
		_add_warning("editor_profiler.invalid_function_signature")
		return
	var signature := str(data[0])
	var signature_id := int(data[1])
	_signature_by_id[signature_id] = signature


func _capture_profile_frame(message: String, data: Array, session_id: String) -> void:
	var parsed := _parse_servers_profile_frame(message, data, session_id)
	if parsed.is_empty():
		return
	var rows: Array = parsed.get("script_functions", [])
	for row in rows:
		_merge_script_function(row)
	parsed.erase("script_functions")
	_frames.append(parsed)
	while _frames.size() > MAX_RETAINED_FRAMES:
		_frames.pop_front()


func _parse_servers_profile_frame(message: String, data: Array, session_id: String) -> Dictionary:
	if data.size() < 7:
		_add_warning("editor_profiler.invalid_profile_frame")
		return {}
	var index := 0
	var frame_number := int(data[index])
	index += 1
	var frame_time_ms := _seconds_to_ms(float(data[index]))
	index += 1
	var process_time_ms := _seconds_to_ms(float(data[index]))
	index += 1
	var physics_time_ms := _seconds_to_ms(float(data[index]))
	index += 1
	var physics_frame_time_ms := _seconds_to_ms(float(data[index]))
	index += 1
	var script_time_ms := _seconds_to_ms(float(data[index]))
	index += 1

	var server_count := int(data[index])
	index += 1
	for _server_index in range(server_count):
		if index + 1 >= data.size():
			_add_warning("editor_profiler.invalid_server_profile_block")
			return {}
		index += 1
		var function_value_count := int(data[index])
		index += 1 + function_value_count
		if index > data.size():
			_add_warning("editor_profiler.invalid_server_profile_block")
			return {}

	if index >= data.size():
		_add_warning("editor_profiler.missing_script_function_block")
		return {}
	var script_value_count := int(data[index])
	index += 1
	if script_value_count % 5 != 0 or index + script_value_count > data.size():
		_add_warning("editor_profiler.invalid_script_function_block")
		return {}

	var script_functions: Array[Dictionary] = []
	var row_count := int(script_value_count / 5)
	for _row_index in range(row_count):
		var signature_id := int(data[index])
		index += 1
		var call_count := int(data[index])
		index += 1
		var self_ms := _seconds_to_ms(float(data[index]))
		index += 1
		var total_ms := _seconds_to_ms(float(data[index]))
		index += 1
		var internal_ms := _seconds_to_ms(float(data[index]))
		index += 1
		var signature := str(_signature_by_id.get(signature_id, "unknown:%d" % signature_id))
		var parsed_signature := _parse_signature(signature)
		script_functions.append({
			"signature": signature,
			"script": parsed_signature.get("script"),
			"function": parsed_signature.get("function"),
			"call_count": call_count,
			"total_ms": total_ms,
			"self_ms": self_ms,
			"internal_ms": internal_ms,
			"max_total_ms": total_ms,
		})

	return {
		"kind": message,
		"session_id": session_id,
		"frame_number": frame_number,
		"frame_time_ms": frame_time_ms,
		"process_time_ms": process_time_ms,
		"physics_time_ms": physics_time_ms,
		"physics_frame_time_ms": physics_frame_time_ms,
		"script_time_ms": script_time_ms,
		"script_functions": script_functions,
	}


func _merge_script_function(row: Dictionary) -> void:
	var signature := str(row.get("signature", "unknown"))
	var existing: Dictionary = _aggregate_by_signature.get(signature, {})
	if existing.is_empty():
		existing = row.duplicate(true)
	else:
		existing["call_count"] = int(existing.get("call_count", 0)) + int(row.get("call_count", 0))
		existing["total_ms"] = float(existing.get("total_ms", 0.0)) + float(row.get("total_ms", 0.0))
		existing["self_ms"] = float(existing.get("self_ms", 0.0)) + float(row.get("self_ms", 0.0))
		existing["internal_ms"] = float(existing.get("internal_ms", 0.0)) + float(row.get("internal_ms", 0.0))
		existing["max_total_ms"] = maxf(float(existing.get("max_total_ms", 0.0)), float(row.get("total_ms", 0.0)))
	_aggregate_by_signature[signature] = existing


func _script_functions() -> Array[Dictionary]:
	var rows: Array[Dictionary] = []
	for signature in _aggregate_by_signature.keys():
		var row: Dictionary = _aggregate_by_signature[signature].duplicate(true)
		rows.append(_rounded_script_function(row))
	rows.sort_custom(_sort_by_total_desc)
	return rows


func _rounded_script_function(row: Dictionary) -> Dictionary:
	row["total_ms"] = _round_ms(float(row.get("total_ms", 0.0)))
	row["self_ms"] = _round_ms(float(row.get("self_ms", 0.0)))
	row["internal_ms"] = _round_ms(float(row.get("internal_ms", 0.0)))
	row["max_total_ms"] = _round_ms(float(row.get("max_total_ms", 0.0)))
	return row


func _sort_by_total_desc(left: Dictionary, right: Dictionary) -> bool:
	return float(left.get("total_ms", 0.0)) > float(right.get("total_ms", 0.0))


func _profile_payload(reason: String) -> Dictionary:
	var rows := _script_functions()
	var root_dir := _bridge_root_abs()
	var run_root := "%s/%s" % [root_dir, _run_id]
	var profile_path := "%s/profile.json" % run_root
	var latest_path := "%s/latest.json" % root_dir
	var session_id := "unknown"
	if _connected_debuggers.size() == 1:
		session_id = str(_connected_debuggers.values()[0])
	elif _connected_debuggers.size() > 1:
		_add_warning("editor_profiler.multiple_debugger_sessions")
	return {
		"source": SOURCE,
		"capture_method": CAPTURE_METHOD,
		"experimental": true,
		"schema_version": 1,
		"run_id": _run_id,
		"captured_at": _timestamp_string(),
		"reason": reason,
		"godot_version": str(Engine.get_version_info().get("string", "")),
		"project_root": ProjectSettings.globalize_path("res://"),
		"session_id": session_id,
		"frame_count": _frames.size(),
		"script_function_count": rows.size(),
		"script_functions": rows,
		"frames": _frames.duplicate(true),
		"warnings": _warnings.duplicate(),
		"artifacts": {
			"run_root": run_root,
			"profile": profile_path,
			"latest": latest_path,
		},
	}


func _parse_signature(signature: String) -> Dictionary:
	var parts := signature.split("::")
	if parts.size() >= 3:
		return {
			"script": parts[0],
			"function": parts[parts.size() - 1],
		}
	if parts.size() >= 1:
		return {
			"script": null,
			"function": parts[parts.size() - 1],
		}
	return {
		"script": null,
		"function": signature,
	}


func _record_toggle_result(result, action: String) -> void:
	if typeof(result) != TYPE_DICTIONARY:
		_add_warning("editor_profiler.%s_toggle_unavailable" % action)
		return
	if int(result.get("session_count", 0)) <= 0:
		_add_warning("editor_profiler.no_debugger_session")
	if int(result.get("toggled_count", 0)) <= 0:
		_add_warning("editor_profiler.%s_toggle_no_session" % action)


func _add_warning(warning: String) -> void:
	if not _warnings.has(warning):
		_warnings.append(warning)


func _editor_base_control():
	if _editor_interface != null and _editor_interface.has_method("get_base_control"):
		return _editor_interface.call("get_base_control")
	return null


func _session_label(node: Node) -> String:
	if node.has_method("get_session_id"):
		return str(node.call("get_session_id"))
	return str(node.get_instance_id())


func _seconds_to_ms(value: float) -> float:
	return value * 1000.0


func _round_ms(value: float) -> float:
	return snappedf(value, 0.001)


func _write_json(path: String, payload: Dictionary) -> void:
	var file := FileAccess.open(path, FileAccess.WRITE)
	if file == null:
		push_warning("[godot-qa] unable to write editor profiler artifact: %s" % path)
		return
	file.store_string(JSON.stringify(payload, "\t"))


func _bridge_root_abs() -> String:
	return ProjectSettings.globalize_path("res://.qa/artifacts/editor-profiler")


func _make_run_id() -> String:
	return "%d-%d" % [int(Time.get_unix_time_from_system() * 1000.0), OS.get_process_id()]


func _timestamp_string() -> String:
	var now := Time.get_datetime_dict_from_system(true)
	return "%04d-%02d-%02dT%02d:%02d:%02dZ" % [
		int(now.get("year", 0)),
		int(now.get("month", 0)),
		int(now.get("day", 0)),
		int(now.get("hour", 0)),
		int(now.get("minute", 0)),
		int(now.get("second", 0)),
	]
