extends RefCounted

const QaProfiler = preload("res://addons/godot_qa/qa_profiler.gd")
const PROFILE_MONITORS := {
	"TIME_FPS": Performance.TIME_FPS,
	"TIME_PROCESS": Performance.TIME_PROCESS,
	"TIME_PHYSICS_PROCESS": Performance.TIME_PHYSICS_PROCESS,
	"OBJECT_COUNT": Performance.OBJECT_COUNT,
	"OBJECT_NODE_COUNT": Performance.OBJECT_NODE_COUNT,
	"RENDER_TOTAL_OBJECTS_IN_FRAME": Performance.RENDER_TOTAL_OBJECTS_IN_FRAME,
	"RENDER_TOTAL_PRIMITIVES_IN_FRAME": Performance.RENDER_TOTAL_PRIMITIVES_IN_FRAME,
	"RENDER_TOTAL_DRAW_CALLS_IN_FRAME": Performance.RENDER_TOTAL_DRAW_CALLS_IN_FRAME,
	"MEMORY_STATIC": Performance.MEMORY_STATIC,
	"MEMORY_STATIC_MAX": Performance.MEMORY_STATIC_MAX,
}

var _executor: WeakRef


func _init(executor: RefCounted) -> void:
	_executor = weakref(executor)


func _initialize_profile(request: Dictionary) -> void:
	_executor.get_ref()._profile_enabled = false
	_executor.get_ref()._profile_max_frame_samples = 0
	_executor.get_ref()._profile_last_frame_usec = 0
	_executor.get_ref()._profile_frame_times_ms = []
	_executor.get_ref()._profile_frame_samples = [] as Array[Dictionary]
	_executor.get_ref()._profile_monitor_samples = [] as Array[Dictionary]
	_executor.get_ref()._profile_unsupported_monitors = [] as Array[String]
	_executor.get_ref()._profile_step_timings = [] as Array[Dictionary]
	_executor.get_ref()._profile_top_script_methods = 10
	_executor.get_ref()._profile_next_step_index = 0
	_executor.get_ref()._profile_next_assertion_index = 0
	_executor.get_ref()._profile_summary = {}
	var profile_request = request.get("profiling", request.get("profile", {}))
	if not profile_request is Dictionary:
		return
	_executor.get_ref()._profile_enabled = bool(profile_request.get("enabled", false))
	if not _executor.get_ref()._profile_enabled:
		return
	QaProfiler.reset()
	_executor.get_ref()._profile_max_frame_samples = max(int(profile_request.get("max_frame_samples", 120)), 0)
	_executor.get_ref()._profile_top_script_methods = max(int(profile_request.get("top", 10)), 0)
	_executor.get_ref()._profile_last_frame_usec = Time.get_ticks_usec()
	_executor.get_ref()._profile_summary = {
		"started_usec": _executor.get_ref()._profile_last_frame_usec,
		"max_frame_samples": _executor.get_ref()._profile_max_frame_samples,
		"top_script_methods": _executor.get_ref()._profile_top_script_methods,
	}

func _profile_now_usec() -> int:
	return Time.get_ticks_usec() if _executor.get_ref()._profile_enabled else 0

# Record an always-on coarse timing phase (independent of the profile gate).
# started_usec must come from Time.get_ticks_usec(), not _profile_now_usec().
func _mark_timing(key: String, started_usec: int) -> void:
	_executor.get_ref()._timing[key] = float(max(Time.get_ticks_usec() - started_usec, 0)) / 1000.0

func _record_profile_timing(label: String, started_usec: int, failure_count: int, extra: Dictionary) -> void:
	if not _executor.get_ref()._profile_enabled:
		return
	var entry := extra.duplicate(true)
	entry["label"] = label
	entry["duration_ms"] = float(max(Time.get_ticks_usec() - started_usec, 0)) / 1000.0
	entry["failure_count_delta"] = max(_executor.get_ref()._failures.size() - failure_count, 0)
	_executor.get_ref()._profile_step_timings.append(entry)

func _profile_step_kind(step: Variant) -> String:
	if not step is Dictionary:
		return "unsupported"
	if step.has("wait_frames"):
		return "wait_frames"
	if step.has("wait_until"):
		return "wait_until"
	if step.has("drive_policy"):
		return "drive_policy"
	if step.has("snapshot"):
		return "snapshot"
	if step.has("assert"):
		return "assert"
	if step.has("audio_capture"):
		return "audio_capture"
	if step.has("audio_export"):
		return "audio_export"
	var runtime_command: Dictionary = _executor.get_ref()._runtime_command_for_step(step)
	if runtime_command.is_empty():
		return "unsupported"
	var command := str(runtime_command.get("command", ""))
	if command.begins_with("input."):
		return "input"
	if command.begins_with("setup.") or command.begins_with("fixture."):
		return "setup"
	return "runtime_request"

func _sample_profile_frame() -> void:
	if not _executor.get_ref()._profile_enabled:
		return
	var now_usec := Time.get_ticks_usec()
	var elapsed_ms := float(max(now_usec - _executor.get_ref()._profile_last_frame_usec, 0)) / 1000.0
	_executor.get_ref()._profile_last_frame_usec = now_usec
	if _executor.get_ref()._profile_frame_times_ms.size() < _executor.get_ref()._profile_max_frame_samples:
		_executor.get_ref()._profile_frame_times_ms.append(elapsed_ms)
		_executor.get_ref()._profile_frame_samples.append({
			"index": _executor.get_ref()._profile_frame_samples.size(),
			"timestamp_usec": now_usec,
			"duration_ms": elapsed_ms,
		})
	for monitor_name in PROFILE_MONITORS.keys():
		var monitor_id: int = PROFILE_MONITORS[monitor_name]
		var value = Performance.get_monitor(monitor_id)
		if value == null:
			if not _executor.get_ref()._profile_unsupported_monitors.has(monitor_name):
				_executor.get_ref()._profile_unsupported_monitors.append(monitor_name)
			continue
		if _executor.get_ref()._profile_monitor_samples.size() < _executor.get_ref()._profile_max_frame_samples * PROFILE_MONITORS.size():
			_executor.get_ref()._profile_monitor_samples.append({
				"name": monitor_name,
				"value": float(value),
				"frame_index": max(_executor.get_ref()._profile_frame_samples.size() - 1, 0),
				"timestamp_usec": now_usec,
			})

func _profile_payload() -> Dictionary:
	return {
		"enabled": true,
		"frame_times_ms": _executor.get_ref()._profile_frame_times_ms.duplicate(),
		"frame_samples": _executor.get_ref()._profile_frame_samples.duplicate(true),
		"monitor_samples": _executor.get_ref()._profile_monitor_samples.duplicate(true),
		"unsupported_monitors": _executor.get_ref()._profile_unsupported_monitors.duplicate(),
		"step_timings": _executor.get_ref()._profile_step_timings.duplicate(true),
		"top_script_methods": QaProfiler.snapshot(_executor.get_ref()._profile_top_script_methods, false),
		"summary": {
			"sample_count": _executor.get_ref()._profile_frame_times_ms.size(),
			"monitor_sample_count": _executor.get_ref()._profile_monitor_samples.size(),
			"step_timing_count": _executor.get_ref()._profile_step_timings.size(),
			"max_frame_samples": _executor.get_ref()._profile_max_frame_samples,
			"top_script_method_limit": _executor.get_ref()._profile_top_script_methods,
			"unsupported_monitor_count": _executor.get_ref()._profile_unsupported_monitors.size(),
		},
	}

func _write_check_progress_event(event: String, check: Dictionary, artifacts: Dictionary, batch_artifacts: Dictionary, started_at: String, elapsed_ms, ok, failure_count) -> void:
	var payload := {
		"event": event,
		"id": str(check.get("id", "")),
		"kind": str(check.get("kind", "")),
		"target": str(check.get("target", "")),
		"started_at": started_at,
		"elapsed_ms": elapsed_ms,
		"ok": ok,
		"failure_count": failure_count,
		"artifacts": artifacts.duplicate(true),
	}
	_append_jsonl(str(batch_artifacts.get("progress", "")), payload)
	_append_jsonl(str(artifacts.get("progress", "")), payload)

func _write_scenario_heartbeat(event: String, operation: String, index: int, kind: String, command: String, context: Dictionary, started_at: String, started_usec: int, failure_count: int) -> void:
	var elapsed_ms := float(max(Time.get_ticks_usec() - started_usec, 0)) / 1000.0
	var payload := {
		"run_id": str(_executor.get_ref()._request_run.get("id", "")),
		"nonce": str(_executor.get_ref()._request_run.get("nonce", "")),
		"event": event,
		"operation": operation,
		"index": index,
		"kind": kind,
		"started_at": started_at,
		"elapsed_ms": elapsed_ms,
		"ok": _executor.get_ref()._failures.size() == failure_count,
		"failure_count_delta": max(_executor.get_ref()._failures.size() - failure_count, 0),
		"artifacts": _scenario_progress_artifacts(),
	}
	if operation == "assertion" and _executor.get_ref()._active_assertion_source_id != "":
		payload["source_id"] = _executor.get_ref()._active_assertion_source_id
	if command != "":
		payload["command"] = command
	for key in context.keys():
		payload[str(key)] = context.get(key)
	_append_jsonl(str(_executor.get_ref()._artifacts.get("progress", _executor.get_ref()._artifacts.get("progress_path", ""))), payload)

func _scenario_progress_artifacts() -> Dictionary:
	var artifacts := {}
	for key in ["run_root", "result_path", "logs_path", "progress", "progress_path", "snapshots_dir", "temporal_dir", "audio_dir"]:
		if _executor.get_ref()._artifacts.has(key):
			artifacts[key] = str(_executor.get_ref()._artifacts.get(key, ""))
	return artifacts

func _progress_now_usec() -> int:
	return Time.get_ticks_usec()

func _progress_context_for_operation(entry: Variant, fallback: String) -> Dictionary:
	if not entry is Dictionary:
		return {"scenario_operation": fallback}
	var operation: String = _executor.get_ref()._operation_name(entry, fallback)
	var context := {"scenario_operation": operation}
	var value = entry.get(operation, null)
	if value is Dictionary:
		for key in _safe_progress_context(value).keys():
			context[key] = _safe_progress_context(value).get(key)
	elif value is String or value is StringName:
		if operation in ["snapshot"]:
			context["snapshot"] = str(value)
		elif operation in ["click", "node_inspect", "exists", "not_exists", "visible", "hidden", "enabled", "disabled", "focused", "inside_viewport"]:
			context["target"] = str(value)
	return context

func _safe_progress_context(params: Variant) -> Dictionary:
	var context := {}
	if not params is Dictionary:
		return context
	for key in ["target", "locator", "qa_id", "path", "node", "a", "b", "other", "snapshot"]:
		if params.has(key):
			context[key] = str(params.get(key, ""))
	return context

func _append_jsonl(path: String, payload: Dictionary) -> void:
	if path == "":
		return
	var file := FileAccess.open(path, FileAccess.READ_WRITE)
	if file == null:
		file = FileAccess.open(path, FileAccess.WRITE)
	if file == null:
		return
	file.seek_end()
	file.store_line(JSON.stringify(payload))

func _progress_started_at() -> String:
	return "%dZ" % int(Time.get_unix_time_from_system())
