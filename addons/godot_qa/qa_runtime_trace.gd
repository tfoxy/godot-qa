extends RefCounted

var _runtime: WeakRef


func _init(runtime: RefCounted) -> void:
	_runtime = weakref(runtime)


func _trace_record(params: Dictionary) -> Dictionary:
	var raw_targets: Array = params.get("targets", []) if params.get("targets", []) is Array else []
	if raw_targets.is_empty():
		return _runtime.get_ref()._error(
			"invalid_params",
			"trace.record requires at least one target",
			{
				"expected": "non-empty targets array",
				"actual": params.get("targets", null),
			},
		)
	var resolved_targets: Array = []
	for raw_target in raw_targets:
		var target := str(raw_target).strip_edges()
		if target == "":
			return _runtime.get_ref()._error(
				"invalid_params",
				"trace.record target must be non-empty",
				{
					"expected": "non-empty target locator",
					"actual": raw_target,
				},
			)
		var resolution: Dictionary = _runtime.get_ref().resolve_target(target)
		if not bool(resolution.get("ok", false)):
			return {"ok": false, "error": (resolution.get("error", {}) as Dictionary).duplicate(true)}
		resolved_targets.append({
			"target": target,
			"resolution": resolution,
			"node": resolution.get("node") as Node,
		})

	var project_state_preflight := _trace_project_state_preflight(params)
	if not bool(project_state_preflight.get("ok", false)):
		return project_state_preflight

	var frames := clampi(int(params.get("frames", 180)), 1, 3600)
	var sample_every_frames := clampi(int(params.get("sample_every_frames", 1)), 1, 240)
	var include_physics := bool(params.get("include_physics", false))
	var radius := clampf(float(params.get("radius", 256.0)), 1.0, 4096.0)
	var trace_id := "trace-%d-%d" % [Time.get_unix_time_from_system(), Time.get_ticks_msec()]
	var stop_after_state: Dictionary = _runtime.get_ref()._stop_after_state_params(params)
	var samples: Array = []
	var events: Array = []
	var elapsed_frames := 0
	var sample_index := 0
	var previous_sample: Dictionary = {}
	var stop_condition_matched := false
	var stop_condition_sample_count := 0
	var stop_condition_frame := -1
	var stop_condition_last_error: Dictionary = {}

	while elapsed_frames <= frames:
		var sample := _trace_capture_sample(
			resolved_targets,
			params,
			sample_index,
			elapsed_frames,
			include_physics,
			radius,
		)
		samples.append(sample)
		events.append_array(_trace_derive_events(previous_sample, sample))
		previous_sample = sample
		if not stop_after_state.is_empty():
			stop_condition_sample_count += 1
			var stop_result: Dictionary = await _runtime.get_ref()._assertions.execute("assert.state_matches", stop_after_state)
			if bool(stop_result.get("ok", false)):
				stop_condition_matched = true
				stop_condition_frame = elapsed_frames
				break
			var stop_error: Dictionary = stop_result.get("error", {}) if stop_result.get("error", {}) is Dictionary else {}
			stop_condition_last_error = stop_error.duplicate(true)
			if str(stop_error.get("type", "")) != "state_matches_mismatch":
				return {"ok": false, "error": stop_error}
		if elapsed_frames == frames:
			break
		var wait_frames := mini(sample_every_frames, frames - elapsed_frames)
		await _runtime.get_ref()._process_frames(wait_frames)
		elapsed_frames += wait_frames
		sample_index += 1

	var payload := {
		"trace_id": trace_id,
		"session": {
			"scene": str(_runtime.get_ref()._scene_root.scene_file_path) if _runtime.get_ref()._scene_root != null else "",
			"started_frame": samples[0].get("frame", 0) if not samples.is_empty() else Engine.get_process_frames(),
			"ended_frame": samples[samples.size() - 1].get("frame", 0) if not samples.is_empty() else Engine.get_process_frames(),
			"started_time_msec": samples[0].get("time_msec", 0) if not samples.is_empty() else Time.get_ticks_msec(),
			"ended_time_msec": samples[samples.size() - 1].get("time_msec", 0) if not samples.is_empty() else Time.get_ticks_msec(),
		},
		"config": {
			"frames": frames,
			"sample_every_frames": sample_every_frames,
			"include_physics": include_physics,
			"physics_radius": radius,
			"include_project_state": bool(params.get("include_project_state", false)),
			"state_provider": str(params.get("state_provider", "scene-root")),
			"state_methods": params.get("state_methods", []) if params.get("state_methods", []) is Array else [],
		},
		"targets": _trace_target_summaries(resolved_targets),
		"samples": samples,
		"events": events,
		"summary": {
			"target_count": resolved_targets.size(),
			"sample_count": samples.size(),
			"event_count": events.size(),
			"event_counts": _trace_event_counts(events),
			"physics_enabled": include_physics,
			"project_state_enabled": bool(params.get("include_project_state", false)),
		},
	}
	if not stop_after_state.is_empty():
		payload["stop_after_state"] = {
			"requested": stop_after_state.duplicate(true),
			"matched": stop_condition_matched,
			"sample_count": stop_condition_sample_count,
			"stopped_at_frame": stop_condition_frame,
			"last_error": stop_condition_last_error,
		}
		payload["stop_condition_matched"] = stop_condition_matched
		payload["stop_condition_timed_out"] = not stop_condition_matched
	return _runtime.get_ref()._ok(payload)


func _event_trace(params: Dictionary) -> Dictionary:
	var target := str(params.get("target", "")).strip_edges()
	if target == "":
		return _runtime.get_ref()._error(
			"invalid_params",
			"event.trace requires a non-empty target",
			{
				"command": "event.trace",
				"field": "target",
				"expected": "non-empty target locator",
				"actual": params.get("target", null),
			},
		)
	var provider_method := str(params.get("provider", "")).strip_edges()
	if provider_method == "":
		return _runtime.get_ref()._error(
			"event_trace.provider_missing",
			"event.trace requires an explicit approved event provider method",
			{
				"command": "event.trace",
				"field": "provider",
				"expected": "get_qa_event_stream_state, get_event_stream_qa_state, get_qa_events, or qa_event_stream",
				"actual": params.get("provider", null),
			},
		)
	if not _is_allowed_event_provider_method(provider_method):
		return _runtime.get_ref()._error(
			"event_trace.provider_invalid",
			"event.trace provider is not an approved read-only event provider method",
			{
				"command": "event.trace",
				"field": "provider",
				"expected": "get_qa_event_stream_state, get_event_stream_qa_state, get_qa_events, or qa_event_stream",
				"actual": provider_method,
			},
		)
	var resolution: Dictionary = _runtime.get_ref().resolve_target(target)
	if not bool(resolution.get("ok", false)):
		return {"ok": false, "error": (resolution.get("error", {}) as Dictionary).duplicate(true)}
	var provider := resolution.get("node") as Node
	if provider == null or not provider.has_method(provider_method):
		return _runtime.get_ref()._error(
			"event_trace.provider_missing",
			"event.trace target does not expose the requested event provider",
			{
				"command": "event.trace",
				"target": target,
				"provider": provider_method,
				"expected": "provider method present on resolved target",
				"actual": "missing",
			},
		)
	if not _runtime.get_ref()._method_has_zero_required_arguments(provider, provider_method):
		return _runtime.get_ref()._error(
			"event_trace.provider_invalid",
			"event.trace providers must take zero required arguments",
			{
				"command": "event.trace",
				"target": target,
				"provider": provider_method,
				"expected": "zero required arguments",
				"actual": "requires arguments",
			},
		)

	var frames := clampi(int(params.get("frames", 180)), 1, 3600)
	var sample_every_frames := clampi(int(params.get("sample_every_frames", 1)), 1, 240)
	var max_events_per_sample := clampi(int(params.get("max_events_per_sample", 512)), 1, 4096)
	var trace_id := "event-trace-%d-%d" % [Time.get_unix_time_from_system(), Time.get_ticks_msec()]
	var samples: Array = []
	var events: Array = []
	var warnings: Array = []
	var provider_error_count := 0
	var elapsed_frames := 0
	var sample_index := 0

	while elapsed_frames <= frames:
		var sample_frame := Engine.get_process_frames()
		var sample_time_msec := Time.get_ticks_msec()
		var provider_result := _call_event_provider(provider, provider_method)
		if not bool(provider_result.get("ok", false)):
			provider_error_count += 1
			warnings.append(_event_trace_warning(
				_event_provider_warning_id(str(provider_result.get("status", ""))),
				"event.trace provider sample failed",
				{
					"target": target,
					"provider": provider_method,
					"sample_index": sample_index,
					"elapsed_frames": elapsed_frames,
					"frame": sample_frame,
					"status": provider_result.get("status", "provider_error"),
					"provider_result": provider_result,
				},
			))
			samples.append({
				"index": sample_index,
				"elapsed_frames": elapsed_frames,
				"frame": sample_frame,
				"time_msec": sample_time_msec,
				"event_count": 0,
				"events": [],
				"provider_status": str(provider_result.get("status", "provider_error")),
			})
		else:
			var normalized := _normalize_event_stream_value(
				provider_result.get("value", null),
				sample_index,
				elapsed_frames,
				sample_frame,
				sample_time_msec,
				max_events_per_sample,
			)
			var sample_events: Array = normalized.get("events", []) if normalized.get("events", []) is Array else []
			events.append_array(sample_events)
			for warning in (normalized.get("warnings", []) if normalized.get("warnings", []) is Array else []):
				warnings.append(warning)
			samples.append({
				"index": sample_index,
				"elapsed_frames": elapsed_frames,
				"frame": sample_frame,
				"time_msec": sample_time_msec,
				"event_count": int(normalized.get("event_count", sample_events.size())),
				"returned_event_count": sample_events.size(),
				"events": sample_events,
				"provider_status": "ok",
				"truncated": bool(normalized.get("truncated", false)),
			})

		if elapsed_frames == frames:
			break
		var wait_frames := mini(sample_every_frames, frames - elapsed_frames)
		await _runtime.get_ref()._process_frames(wait_frames)
		elapsed_frames += wait_frames
		sample_index += 1

	if events.is_empty():
		warnings.append(_event_trace_warning(
			"event_trace.no_events_recorded",
			"event.trace recorded no events",
			{
				"target": target,
				"provider": provider_method,
				"frames": frames,
				"sample_every_frames": sample_every_frames,
			},
		))

	return _runtime.get_ref()._ok({
		"trace_id": trace_id,
		"session": {
			"scene": str(_runtime.get_ref()._scene_root.scene_file_path) if _runtime.get_ref()._scene_root != null else "",
			"started_frame": samples[0].get("frame", 0) if not samples.is_empty() else Engine.get_process_frames(),
			"ended_frame": samples[samples.size() - 1].get("frame", 0) if not samples.is_empty() else Engine.get_process_frames(),
			"started_time_msec": samples[0].get("time_msec", 0) if not samples.is_empty() else Time.get_ticks_msec(),
			"ended_time_msec": samples[samples.size() - 1].get("time_msec", 0) if not samples.is_empty() else Time.get_ticks_msec(),
		},
		"config": {
			"frames": frames,
			"sample_every_frames": sample_every_frames,
			"max_events_per_sample": max_events_per_sample,
		},
		"target": {
			"requested": target,
			"resolved": str(resolution.get("resolved", "path:%s" % str(provider.get_path()))),
			"path": str(provider.get_path()),
			"type": provider.get_class(),
		},
		"provider": {
			"method": provider_method,
			"allowed": true,
			"zero_required_arguments": _runtime.get_ref()._method_has_zero_required_arguments(provider, provider_method),
		},
		"viewport": _event_trace_viewport_metadata(),
		"samples": samples,
		"events": events,
		"warnings": warnings,
		"provider_error_count": provider_error_count,
		"summary": {
			"sample_count": samples.size(),
			"total_event_count": events.size(),
			"provider_error_count": provider_error_count,
			"warning_count": warnings.size(),
		},
	})


# Sample one read-only qa_ state source every N frames into a bounded row
# buffer for telemetry evidence (pacing/difficulty curves). Rows are capped at
# MAX_TELEMETRY_ROWS so a long window can never grow unbounded in memory; the
# recording reports `truncated: true` when the cap stops collection early.
func _telemetry_record(params: Dictionary) -> Dictionary:
	var provider_id := str(params.get("provider", "")).strip_edges()
	var target := str(params.get("target", "")).strip_edges()
	var method := str(params.get("method", "")).strip_edges()
	if provider_id == "" and (target == "" or method == ""):
		return _runtime.get_ref()._error(
			"invalid_params",
			"telemetry.record requires a provider id or a target with a method",
			{
				"command": "telemetry.record",
				"expected": "provider, or target + method",
				"actual": {"provider": params.get("provider", null), "target": params.get("target", null), "method": params.get("method", null)},
			},
		)
	var source := {}
	var provider_node: Node = null
	if provider_id != "":
		var provider_probe: Dictionary = _runtime.get_ref()._providers.read_state_provider(provider_id)
		if not bool(provider_probe.get("ok", false)):
			return provider_probe
		source = {"provider": provider_id, "method": str(provider_probe.get("method", ""))}
	else:
		if not _is_allowed_telemetry_method(method):
			return _runtime.get_ref()._error(
				"telemetry.method_invalid",
				"telemetry.record method must be a read-only qa_ state method",
				{
					"command": "telemetry.record",
					"field": "method",
					"expected": "qa_* (non-mutating), get_qa_*_state, or get_*_qa_state",
					"actual": method,
				},
			)
		var resolution: Dictionary = _runtime.get_ref().resolve_target(target)
		if not bool(resolution.get("ok", false)):
			return {"ok": false, "error": (resolution.get("error", {}) as Dictionary).duplicate(true)}
		provider_node = resolution.get("node") as Node
		if provider_node == null or not provider_node.has_method(method):
			return _runtime.get_ref()._error(
				"telemetry.provider_missing",
				"telemetry.record target does not expose the requested method",
				{
					"command": "telemetry.record",
					"target": target,
					"method": method,
					"expected": "method present on resolved target",
					"actual": "missing",
				},
			)
		if not _runtime.get_ref()._method_has_zero_required_arguments(provider_node, method):
			return _runtime.get_ref()._error(
				"telemetry.method_invalid",
				"telemetry.record methods must take zero required arguments",
				{
					"command": "telemetry.record",
					"target": target,
					"method": method,
					"expected": "zero required arguments",
					"actual": "requires arguments",
				},
			)
		source = {"target": target, "method": method}

	var frames := clampi(int(params.get("frames", 600)), 1, 3600)
	var every := clampi(int(params.get("every", 1)), 1, 240)
	var telemetry_id := "telemetry-%d-%d" % [Time.get_unix_time_from_system(), Time.get_ticks_msec()]
	var rows: Array = []
	var warnings: Array = []
	var provider_error_count := 0
	var truncated := false
	var elapsed_frames := 0

	while elapsed_frames <= frames:
		if rows.size() >= _runtime.get_ref().MAX_TELEMETRY_ROWS:
			truncated = true
			break
		var row := {
			"frame": Engine.get_process_frames(),
			"t_ms": Time.get_ticks_msec(),
			"value": null,
		}
		if provider_id != "":
			var provider_result: Dictionary = _runtime.get_ref()._providers.read_state_provider(provider_id)
			if bool(provider_result.get("ok", false)):
				row["value"] = provider_result.get("state", null)
			else:
				provider_error_count += 1
				row["error"] = str(((provider_result.get("error", {}) as Dictionary)).get("type", "provider_error"))
		else:
			var sample := _call_telemetry_method(provider_node, method)
			if bool(sample.get("ok", false)):
				row["value"] = sample.get("value", null)
			else:
				provider_error_count += 1
				row["error"] = str(sample.get("status", "provider_error"))
		if row.has("error") and warnings.size() < 32:
			warnings.append({
				"id": "telemetry.sample_failed",
				"message": "telemetry.record sample failed",
				"data": {"elapsed_frames": elapsed_frames, "error": row.get("error", "")},
			})
		rows.append(row)
		if elapsed_frames == frames:
			break
		var wait_frames := mini(every, frames - elapsed_frames)
		await _runtime.get_ref()._process_frames(wait_frames)
		elapsed_frames += wait_frames

	var config := {
		"frames": frames,
		"every": every,
		"max_rows": _runtime.get_ref().MAX_TELEMETRY_ROWS,
	}
	for key in source.keys():
		config[key] = source[key]
	return _runtime.get_ref()._ok({
		"telemetry_id": telemetry_id,
		"session": {
			"scene": str(_runtime.get_ref()._scene_root.scene_file_path) if _runtime.get_ref()._scene_root != null else "",
			"started_frame": rows[0].get("frame", 0) if not rows.is_empty() else Engine.get_process_frames(),
			"ended_frame": rows[rows.size() - 1].get("frame", 0) if not rows.is_empty() else Engine.get_process_frames(),
			"started_time_msec": rows[0].get("t_ms", 0) if not rows.is_empty() else Time.get_ticks_msec(),
			"ended_time_msec": rows[rows.size() - 1].get("t_ms", 0) if not rows.is_empty() else Time.get_ticks_msec(),
		},
		"config": config,
		"rows": rows,
		"row_count": rows.size(),
		"truncated": truncated,
		"provider_error_count": provider_error_count,
		"warnings": warnings,
	})


# Telemetry samples must stay read-only: accept the read-only QA state naming
# contract plus plain qa_-prefixed provider methods, while rejecting the
# mutating qa_prepare_/qa_apply_/qa_mutate_/qa_set_ fixture prefixes. Mirrors
# is_allowed_telemetry_method on the CLI side.
func _is_allowed_telemetry_method(method: String) -> bool:
	if method == "" \
			or method.begins_with("qa_prepare_") \
			or method.begins_with("qa_apply_") \
			or method.begins_with("qa_mutate_") \
			or method.begins_with("qa_set_"):
		return false
	for character in method:
		var codepoint := character.unicode_at(0)
		if not ((codepoint >= 48 and codepoint <= 57) or (codepoint >= 65 and codepoint <= 90) or (codepoint >= 97 and codepoint <= 122) or character == "_"):
			return false
	return method.begins_with("qa_") \
		or (method.begins_with("get_qa_") and method.ends_with("_state")) \
		or (method.begins_with("get_") and method.ends_with("_qa_state"))


func _call_telemetry_method(provider: Node, method: String) -> Dictionary:
	if provider == null or not is_instance_valid(provider):
		return {"ok": false, "status": "provider_freed"}
	if not provider.has_method(method):
		return {"ok": false, "status": "missing_method"}
	var runtime_error_count_before: int = _runtime.get_ref()._runtime_errors.size()
	var value = provider.call(method)
	if _runtime.get_ref()._runtime_errors.size() > runtime_error_count_before:
		return {"ok": false, "status": "provider_runtime_error"}
	if not _runtime.get_ref()._is_json_compatible_project_state_value(value):
		return {"ok": false, "status": "non_json_compatible_value"}
	return {"ok": true, "value": value}


func _is_allowed_event_provider_method(method: String) -> bool:
	return method in [
		"get_qa_event_stream_state",
		"get_event_stream_qa_state",
		"get_qa_events",
		"qa_event_stream",
	]


func _call_event_provider(provider: Node, method: String) -> Dictionary:
	if not _is_allowed_event_provider_method(method):
		return {"method": method, "ok": false, "status": "rejected_method"}
	if provider == null or not provider.has_method(method):
		return {"method": method, "ok": false, "status": "missing_method"}
	if not _runtime.get_ref()._method_has_zero_required_arguments(provider, method):
		return {"method": method, "ok": false, "status": "requires_arguments"}
	var runtime_error_count_before: int = _runtime.get_ref()._runtime_errors.size()
	var value = provider.call(method)
	if _runtime.get_ref()._runtime_errors.size() > runtime_error_count_before:
		return {
			"method": method,
			"ok": false,
			"status": "runtime_error",
			"runtime_errors": _runtime.get_ref()._runtime_errors.slice(runtime_error_count_before).duplicate(true),
		}
	return {
		"method": method,
		"ok": true,
		"value": value,
	}


func _normalize_event_stream_value(
	value: Variant,
	sample_index: int,
	elapsed_frames: int,
	sample_frame: int,
	sample_time_msec: int,
	max_events_per_sample: int
) -> Dictionary:
	var raw_events: Array = []
	if value is Array:
		raw_events = value
	elif value is Dictionary:
		var dictionary: Dictionary = value
		if dictionary.has("events"):
			if not dictionary.get("events") is Array:
				return {
					"events": [],
					"event_count": 0,
					"truncated": false,
					"warnings": [_event_trace_warning(
						"event_trace.event_not_json_compatible",
						"event.trace provider returned an events field that is not an array",
						{
							"sample_index": sample_index,
							"elapsed_frames": elapsed_frames,
							"frame": sample_frame,
							"actual_type": type_string(typeof(dictionary.get("events"))),
						},
					)],
				}
			raw_events = dictionary.get("events")
		else:
			raw_events = [dictionary]
	elif value == null:
		raw_events = []
	else:
		return {
			"events": [],
			"event_count": 0,
			"truncated": false,
			"warnings": [_event_trace_warning(
				"event_trace.event_not_json_compatible",
				"event.trace provider returned a value that is not a dictionary or array",
				{
					"sample_index": sample_index,
					"elapsed_frames": elapsed_frames,
					"frame": sample_frame,
					"actual_type": type_string(typeof(value)),
				},
			)],
		}

	var returned_count := mini(raw_events.size(), max_events_per_sample)
	var events: Array = []
	var warnings: Array = []
	for index in range(returned_count):
		var normalized := _normalize_event_record(raw_events[index], sample_index, elapsed_frames, sample_frame, sample_time_msec, index)
		if bool(normalized.get("ok", false)):
			events.append(normalized.get("event", {}))
		else:
			warnings.append(normalized.get("warning", {}))
	if raw_events.size() > returned_count:
		warnings.append(_event_trace_warning(
			"event_trace.event_stream_truncated",
			"event.trace provider returned more events than max_events_per_sample",
			{
				"sample_index": sample_index,
				"elapsed_frames": elapsed_frames,
				"frame": sample_frame,
				"event_count": raw_events.size(),
				"returned_event_count": returned_count,
				"max_events_per_sample": max_events_per_sample,
			},
		))
	return {
		"events": events,
		"event_count": raw_events.size(),
		"truncated": raw_events.size() > returned_count,
		"warnings": warnings,
	}


func _normalize_event_record(value: Variant, sample_index: int, elapsed_frames: int, sample_frame: int, sample_time_msec: int, event_index: int) -> Dictionary:
	if not _runtime.get_ref()._is_json_compatible_project_state_value(value):
		return {
			"ok": false,
			"warning": _event_trace_warning(
				"event_trace.event_not_json_compatible",
				"event.trace provider returned a non JSON-compatible event record",
				{
					"sample_index": sample_index,
					"elapsed_frames": elapsed_frames,
					"frame": sample_frame,
					"event_index": event_index,
					"actual_type": type_string(typeof(value)),
				},
			),
		}
	var event: Variant = _runtime.get_ref()._json_compatible_value(value)
	if event is Dictionary:
		var event_payload: Dictionary = event
		event_payload["_qa_sample"] = {
			"sample_index": sample_index,
			"elapsed_frames": elapsed_frames,
			"frame": sample_frame,
			"time_msec": sample_time_msec,
			"event_index": event_index,
		}
		return {"ok": true, "event": event_payload}
	return {
		"ok": false,
		"warning": _event_trace_warning(
			"event_trace.event_not_json_compatible",
			"event.trace event records must be dictionaries",
			{
				"sample_index": sample_index,
				"elapsed_frames": elapsed_frames,
				"frame": sample_frame,
				"event_index": event_index,
				"actual_type": type_string(typeof(value)),
			},
		),
	}


func _event_provider_warning_id(status: String) -> String:
	match status:
		"missing_method":
			return "event_trace.provider_missing"
		"rejected_method", "requires_arguments":
			return "event_trace.provider_invalid"
		"runtime_error":
			return "event_trace.provider_error"
		_:
			return "event_trace.provider_error"


func _event_trace_warning(warning_id: String, message: String, data := {}) -> Dictionary:
	var warning := {"id": warning_id, "message": message}
	if data is Dictionary and not (data as Dictionary).is_empty():
		warning["data"] = (data as Dictionary).duplicate(true)
	return warning


func _event_trace_viewport_metadata() -> Dictionary:
	if _runtime.get_ref()._scene_root == null or _runtime.get_ref()._scene_root.get_viewport() == null:
		return {"available": false}
	var rect: Rect2 = _runtime.get_ref()._scene_root.get_viewport().get_visible_rect()
	return {
		"available": true,
		"rect": _runtime.get_ref()._shapes._rect2_payload(rect),
		"size": _runtime.get_ref()._shapes._vector2_payload(rect.size),
	}


func _trace_capture_sample(
	resolved_targets: Array,
	params: Dictionary,
	sample_index: int,
	elapsed_frames: int,
	include_physics: bool,
	radius: float
) -> Dictionary:
	var targets: Array = []
	for entry in resolved_targets:
		if not entry is Dictionary:
			continue
		targets.append(_trace_target_sample(entry as Dictionary, params, include_physics, radius))
	return {
		"index": sample_index,
		"elapsed_frames": elapsed_frames,
		"frame": Engine.get_process_frames(),
		"time_msec": Time.get_ticks_msec(),
		"targets": targets,
	}


func _trace_target_sample(entry: Dictionary, params: Dictionary, include_physics: bool, radius: float) -> Dictionary:
	var target := str(entry.get("target", ""))
	var resolution: Dictionary = entry.get("resolution", {})
	var node := entry.get("node") as Node
	var position: Vector2 = _runtime.get_ref()._shapes._global_position_vector2_for_node(node)
	var visual_rect: Dictionary = _runtime.get_ref()._shapes._visual_rect_for_node(node)
	var target_info: Dictionary = _runtime.get_ref()._physics._physics_target_info(target, resolution, node, position, visual_rect)
	var payload := target_info.duplicate(true)
	if bool(params.get("include_project_state", false)):
		payload["project_state"] = _runtime.get_ref()._physics._collect_physics_project_state(node, params, true)
	if include_physics:
		payload["physics"] = _trace_physics_summary(node, position, visual_rect, params, radius)
	return payload


func _trace_physics_summary(
	target_node: Node,
	target_position: Vector2,
	target_visual_rect: Dictionary,
	params: Dictionary,
	radius: float
) -> Dictionary:
	var collision_objects: Array[Dictionary] = []
	var target_shapes: Array[Dictionary] = []
	_runtime.get_ref()._physics._collect_collision_object_reports(_runtime.get_ref()._scene_root, target_node, target_position, radius, target_visual_rect, collision_objects, target_shapes)
	var space_queries: Dictionary = _runtime.get_ref()._physics._physics_space_queries(target_node, target_position, radius, target_shapes)
	var contacts: Array[Dictionary] = _runtime.get_ref()._physics._collect_physics_contacts(collision_objects, true, target_node, space_queries)
	var target_info: Dictionary = _runtime.get_ref()._physics._physics_target_info("", {"resolved": "path:%s" % str(target_node.get_path())}, target_node, target_position, target_visual_rect)
	var project_state: Dictionary = _runtime.get_ref()._physics._collect_physics_project_state(
		target_node,
		params,
		bool(params.get("include_project_state", false)),
	)
	var diagnostics: Array[Dictionary] = _runtime.get_ref()._physics._physics_diagnostics(target_info, collision_objects, target_shapes, project_state)
	return {
		"radius": radius,
		"nearby_collision_object_count": collision_objects.size(),
		"target_shape_count": target_shapes.size(),
		"contact_count": contacts.size(),
		"radius_query_hit_count": int((space_queries.get("radius_query", {}) as Dictionary).get("hit_count", 0)) if space_queries.get("radius_query", {}) is Dictionary else 0,
		"target_shape_overlap_count": _trace_overlap_count(space_queries),
		"contacts": contacts,
		"hidden_collision_objects": _trace_diagnostic_targets(diagnostics, "hidden_collision_object"),
		"layer_mask_mismatches": _trace_diagnostic_targets(diagnostics, "layer_mask_cannot_collide"),
		"diagnostics": diagnostics,
		"project_state": project_state,
	}


func _trace_target_summaries(resolved_targets: Array) -> Array:
	var summaries: Array = []
	for entry in resolved_targets:
		if not entry is Dictionary:
			continue
		var dictionary: Dictionary = entry
		var node := dictionary.get("node") as Node
		summaries.append({
			"target": str(dictionary.get("target", "")),
			"resolved": str((dictionary.get("resolution", {}) as Dictionary).get("resolved", "")),
			"path": str(node.get_path()) if node != null else "",
			"type": node.get_class() if node != null else "",
			"qa_id": str(node.get_meta("qa_id", "")) if node != null else "",
		})
	return summaries


func _trace_derive_events(previous_sample: Dictionary, sample: Dictionary) -> Array:
	if previous_sample.is_empty():
		return []
	var events: Array = []
	for current_target in sample.get("targets", []):
		if not current_target is Dictionary:
			continue
		var current: Dictionary = current_target
		var previous := _trace_find_target_sample(previous_sample, str(current.get("target", "")))
		if previous.is_empty():
			continue
		events.append_array(_trace_motion_events(previous, current, sample))
		events.append_array(_trace_physics_events(previous, current, sample))
		if _trace_project_state_changed(previous, current):
			events.append(_trace_event("project_state_changed", current, sample, {
				"previous_elapsed_frames": previous_sample.get("elapsed_frames", null),
			}))
	return events


func _trace_motion_events(previous: Dictionary, current: Dictionary, sample: Dictionary) -> Array:
	var events: Array = []
	var previous_position: Vector2 = _runtime.get_ref()._shapes._vector2_from_payload(previous.get("position", {}), Vector2.ZERO)
	var current_position: Vector2 = _runtime.get_ref()._shapes._vector2_from_payload(current.get("position", {}), Vector2.ZERO)
	var previous_velocity := _trace_velocity_vector(previous)
	var current_velocity := _trace_velocity_vector(current)
	var displacement := current_position - previous_position
	if previous_velocity.length() > 1.0 and displacement.dot(previous_velocity.normalized()) < -1.0:
		events.append(_trace_event("large_backward_displacement", current, sample, {
			"displacement": _runtime.get_ref()._shapes._vector2_payload(displacement),
			"previous_velocity": _runtime.get_ref()._shapes._vector2_payload(previous_velocity),
		}))
	if previous_velocity.length() > 1.0 and current_velocity.length() > 1.0 and previous_velocity.dot(current_velocity) < 0.0:
		events.append(_trace_event("velocity_sign_reversal", current, sample, {
			"previous_velocity": _runtime.get_ref()._shapes._vector2_payload(previous_velocity),
			"current_velocity": _runtime.get_ref()._shapes._vector2_payload(current_velocity),
		}))
	var previous_speed := previous_velocity.length()
	var current_speed := current_velocity.length()
	if current_speed > previous_speed * 2.0 and current_speed - previous_speed > 50.0:
		events.append(_trace_event("sudden_speed_spike", current, sample, {
			"previous_speed": previous_speed,
			"current_speed": current_speed,
		}))
	return events


func _trace_physics_events(previous: Dictionary, current: Dictionary, sample: Dictionary) -> Array:
	var events: Array = []
	var previous_physics: Dictionary = previous.get("physics", {}) if previous.get("physics", {}) is Dictionary else {}
	var current_physics: Dictionary = current.get("physics", {}) if current.get("physics", {}) is Dictionary else {}
	if current_physics.is_empty():
		return events
	var previous_overlap_count := int(previous_physics.get("target_shape_overlap_count", 0))
	var current_overlap_count := int(current_physics.get("target_shape_overlap_count", 0))
	if previous_overlap_count == 0 and current_overlap_count > 0:
		events.append(_trace_event("new_overlap", current, sample, {
			"previous_overlap_count": previous_overlap_count,
			"current_overlap_count": current_overlap_count,
		}))
	var hidden_colliders: Array = current_physics.get("hidden_collision_objects", []) if current_physics.get("hidden_collision_objects", []) is Array else []
	if not hidden_colliders.is_empty():
		events.append(_trace_event("hidden_collider_nearby", current, sample, {
			"colliders": hidden_colliders,
		}))
	var layer_mismatches: Array = current_physics.get("layer_mask_mismatches", []) if current_physics.get("layer_mask_mismatches", []) is Array else []
	if not layer_mismatches.is_empty():
		events.append(_trace_event("layer_mask_mismatch", current, sample, {
			"objects": layer_mismatches,
		}))
	return events


func _trace_event(event_type: String, target_sample: Dictionary, sample: Dictionary, details: Dictionary) -> Dictionary:
	return {
		"type": event_type,
		"target": target_sample.get("target", ""),
		"resolved": target_sample.get("resolved", ""),
		"frame": sample.get("frame", 0),
		"elapsed_frames": sample.get("elapsed_frames", 0),
		"time_msec": sample.get("time_msec", 0),
		"details": details,
	}


func _trace_find_target_sample(sample: Dictionary, target: String) -> Dictionary:
	for entry in sample.get("targets", []):
		if entry is Dictionary and str((entry as Dictionary).get("target", "")) == target:
			return entry as Dictionary
	return {}


func _trace_velocity_vector(sample: Dictionary) -> Vector2:
	return _runtime.get_ref()._shapes._vector2_from_payload(sample.get("velocity", {}), Vector2.ZERO)


func _trace_overlap_count(space_queries: Dictionary) -> int:
	var count := 0
	for overlap in space_queries.get("target_shape_overlaps", []):
		if overlap is Dictionary:
			var hits: Array = (overlap as Dictionary).get("hits", []) if (overlap as Dictionary).get("hits", []) is Array else []
			count += hits.size()
	return count


func _trace_diagnostic_targets(diagnostics: Array, diagnostic_type: String) -> Array:
	var targets: Array = []
	for diagnostic in diagnostics:
		if diagnostic is Dictionary and str((diagnostic as Dictionary).get("type", "")) == diagnostic_type:
			targets.append((diagnostic as Dictionary).get("target", ""))
	return targets


func _trace_project_state_changed(previous: Dictionary, current: Dictionary) -> bool:
	var previous_state := _trace_project_state_payload(previous)
	var current_state := _trace_project_state_payload(current)
	if not bool(previous_state.get("available", false)) or not bool(current_state.get("available", false)):
		return false
	return JSON.stringify(previous_state) != JSON.stringify(current_state)


func _trace_project_state_payload(sample: Dictionary) -> Dictionary:
	if sample.get("project_state", {}) is Dictionary:
		return sample.get("project_state", {}) as Dictionary
	var physics: Dictionary = sample.get("physics", {}) if sample.get("physics", {}) is Dictionary else {}
	if physics.get("project_state", {}) is Dictionary:
		return physics.get("project_state", {}) as Dictionary
	return {}


func _trace_event_counts(events: Array) -> Dictionary:
	var counts := {}
	for event in events:
		if not event is Dictionary:
			continue
		var event_type := str((event as Dictionary).get("type", "unknown"))
		counts[event_type] = int(counts.get(event_type, 0)) + 1
	return counts


func _trace_project_state_preflight(params: Dictionary) -> Dictionary:
	if not bool(params.get("include_project_state", false)):
		return {"ok": true}
	var raw_methods: Array = params.get("state_methods", []) if params.get("state_methods", []) is Array else []
	if raw_methods.is_empty():
		return {"ok": true}
	var methods: Array[String] = []
	for raw_method in raw_methods:
		var method := str(raw_method)
		if not _runtime.get_ref()._shapes._is_allowed_physics_state_method(method):
			return _runtime.get_ref()._error(
				"invalid_params",
				"trace.record state method is not an approved read-only QA state method",
				{
					"command": "trace.record",
					"field": "state_methods",
					"method": method,
					"expected": "get_qa_*_state, get_*_qa_state, or qa_*diagnostic*",
					"actual": raw_method,
				},
			)
		methods.append(method)
	var provider_result := _trace_project_state_provider(params)
	if not bool(provider_result.get("ok", false)):
		return provider_result
	var provider := provider_result.get("node") as Node
	for method in methods:
		var method_payload: Dictionary = _runtime.get_ref()._physics._call_physics_state_method(provider, method)
		if not bool(method_payload.get("ok", false)):
			var status := str(method_payload.get("status", "invalid_state_method"))
			return _runtime.get_ref()._error(
				_trace_state_method_failure_type(status),
				_trace_state_method_failure_message(status),
				{
					"command": "trace.record",
					"state_provider": str(params.get("state_provider", "scene-root")),
					"provider": provider_result.get("resolved", ""),
					"method": method,
					"status": status,
					"method_result": method_payload,
				},
			)
	return {"ok": true}


func _trace_project_state_provider(params: Dictionary) -> Dictionary:
	var provider_locator := str(params.get("state_provider", "scene-root")).strip_edges()
	if provider_locator == "":
		return _runtime.get_ref()._error(
			"invalid_params",
			"trace.record requires state_provider to be non-empty",
			{
				"command": "trace.record",
				"field": "state_provider",
				"expected": "scene-root or target locator",
				"actual": params.get("state_provider", null),
			},
		)
	if provider_locator == "scene-root":
		if _runtime.get_ref()._scene_root == null:
			return _runtime.get_ref()._error(
				"scene_unavailable",
				"trace.record state provider scene-root is not available",
				{
					"command": "trace.record",
					"state_provider": provider_locator,
					"expected": "active scene root",
					"actual": null,
				},
			)
		return {
			"ok": true,
			"node": _runtime.get_ref()._scene_root,
			"resolved": "scene-root",
			"path": str(_runtime.get_ref()._scene_root.get_path()),
		}
	var provider_resolution: Dictionary = _runtime.get_ref().resolve_target(provider_locator)
	if not bool(provider_resolution.get("ok", false)):
		var error_payload: Dictionary = provider_resolution.get("error", {}) if provider_resolution.get("error", {}) is Dictionary else {}
		error_payload = error_payload.duplicate(true)
		var error_data: Dictionary = error_payload.get("data", {}) if error_payload.get("data", {}) is Dictionary else {}
		error_data = error_data.duplicate(true)
		error_data["command"] = "trace.record"
		error_data["state_provider"] = provider_locator
		error_payload["data"] = error_data
		return {"ok": false, "error": error_payload}
	var provider := provider_resolution.get("node") as Node
	if provider == null:
		return _runtime.get_ref()._error(
			"target_not_found",
			"trace.record state provider did not resolve to a node",
			{
				"command": "trace.record",
				"state_provider": provider_locator,
				"expected": "resolved provider node",
				"actual": provider_resolution.get("resolved", ""),
			},
		)
	return {
		"ok": true,
		"node": provider,
		"resolved": str(provider_resolution.get("resolved", "path:%s" % str(provider.get_path()))),
		"path": str(provider.get_path()),
	}


func _trace_state_method_failure_type(status: String) -> String:
	match status:
		"missing_method":
			return "missing_state_method"
		"requires_arguments", "rejected_method":
			return "invalid_params"
		"runtime_error":
			return "runtime_errors_present"
		"non_json_return":
			return "invalid_state_source"
		_:
			return "invalid_state_method"


func _trace_state_method_failure_message(status: String) -> String:
	match status:
		"missing_method":
			return "trace.record state provider does not expose the requested QA state method"
		"requires_arguments":
			return "trace.record state methods must take zero arguments"
		"runtime_error":
			return "trace.record state method recorded runtime errors"
		"non_json_return":
			return "trace.record state method returned a non JSON-compatible value"
		"rejected_method":
			return "trace.record state method is not an approved read-only QA state method"
		_:
			return "trace.record state method failed preflight"


func capture_temporal_sample(target: String, field: String) -> Dictionary:
	var resolution: Dictionary = _runtime.get_ref().resolve_target(target)
	if not bool(resolution.get("ok", false)):
		return {"target": target, "field": field, "error": (resolution.get("error", {}) as Dictionary).duplicate(true), "frame": Engine.get_process_frames(), "time_msec": Time.get_ticks_msec()}
	var node := resolution.get("node") as Node
	return {"target": target, "resolved": str(resolution.get("resolved", "")), "field": field, "value": _runtime.get_ref().read_node_field(node, field), "frame": Engine.get_process_frames(), "time_msec": Time.get_ticks_msec()}


func sample_temporal_window(target: String, field: String, window: Dictionary, sample_every_frames: int) -> Array:
	var samples: Array = []
	var cadence := max(sample_every_frames, 1)
	var sampling: Dictionary = window.get("temporal_sampling", {}) if window.get("temporal_sampling", {}) is Dictionary else {}
	if str(window.get("mode", "frames")) == "seconds":
		var seconds := max(float(window.get("value", 0.0)), 0.0)
		var start := Time.get_ticks_msec()
		while (Time.get_ticks_msec() - start) / 1000.0 <= seconds:
			var sample := capture_temporal_sample(target, field)
			sample["cadence"] = {"sample_every_frames": cadence}
			if not sampling.is_empty():
				sample["temporal_sampling"] = sampling.duplicate(true)
				sample["sampling_mode"] = str(sampling.get("mode", ""))
			samples.append(sample)
			await _runtime.get_ref()._process_frames(cadence)
	else:
		var frame_count := max(int(window.get("value", 0)), 1)
		for _i in range(frame_count):
			var sample := capture_temporal_sample(target, field)
			sample["cadence"] = {"sample_every_frames": cadence}
			if not sampling.is_empty():
				sample["temporal_sampling"] = sampling.duplicate(true)
				sample["sampling_mode"] = str(sampling.get("mode", ""))
			samples.append(sample)
			await _runtime.get_ref()._process_frames(cadence)
	return samples


func write_temporal_evidence(kind: String, target: String, payload: Dictionary) -> Dictionary:
	if payload.has("temporal_sampling") and payload.get("temporal_sampling", {}) is Dictionary:
		payload["sampling_mode"] = str((payload.get("temporal_sampling", {}) as Dictionary).get("mode", ""))
	if not payload.has("cadence") and payload.has("window") and payload.get("window", {}) is Dictionary:
		var window: Dictionary = payload.get("window", {})
		if window.has("cadence"):
			payload["cadence"] = (window.get("cadence", {}) as Dictionary).duplicate(true)
	var temporal_dir := str(payload.get("temporal_dir", ""))
	if temporal_dir == "":
		temporal_dir = "user://qa_artifacts/temporal"
	var safe_target := target.replace(":", "_").replace("/", "_")
	var file_path := "%s/%s_%s.json" % [temporal_dir, kind, safe_target]
	var mkdir_error := DirAccess.make_dir_recursive_absolute(temporal_dir)
	if mkdir_error != OK:
		return _temporal_evidence_write_error(kind, target, temporal_dir, file_path, "mkdir", mkdir_error)
	var file := FileAccess.open(file_path, FileAccess.WRITE)
	if file == null:
		return _temporal_evidence_write_error(kind, target, temporal_dir, file_path, "open", FileAccess.get_open_error())
	file.store_string(JSON.stringify(payload, "\t"))
	file.flush()
	var write_error := file.get_error()
	file.close()
	if write_error != OK:
		return _temporal_evidence_write_error(kind, target, temporal_dir, file_path, "write", write_error)
	return {"temporal_evidence": file_path, "temporal_dir": temporal_dir}


func _temporal_evidence_write_error(kind: String, target: String, temporal_dir: String, path: String, operation: String, error_code: int) -> Dictionary:
	return _runtime.get_ref()._error("artifact_write_failed", "temporal evidence could not be persisted", {
		"command": "assert.observe_until" if kind == "observe_until" else "assert.temporal.%s" % kind,
		"target": target,
		"path": path,
		"temporal_dir": temporal_dir,
		"expected": "persisted temporal evidence JSON",
		"actual": {"operation": operation, "error": error_code, "message": error_string(error_code)},
	})
