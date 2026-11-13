extends RefCounted

const QaSnapshot = preload("res://addons/godot_qa/qa_snapshot.gd")
const AUDIO_EXPORT_MAX_BYTES := 10485760
const MAX_PHYSICS_OVERLAY_EVIDENCE := 128

var _executor: WeakRef


func _init(executor: RefCounted) -> void:
	_executor = weakref(executor)


func _result_payload() -> Dictionary:
	if _executor.get_ref()._mode == "check":
		if str(_executor.get_ref()._check.get("execution_mode", "")) == "batch":
			return {
				"ok": _executor.get_ref()._failures.is_empty(),
				"execution_mode": "batch",
				"duration_ms": Time.get_ticks_msec() - _executor.get_ref()._started_at_msec,
				"display_server": DisplayServer.get_name(),
				"failures": _executor.get_ref()._failures.duplicate(true),
				"artifacts": _executor.get_ref()._artifacts.duplicate(true),
			}
		if _executor.get_ref()._check.has("id") and _executor.get_ref()._check.has("kind") and _executor.get_ref()._check.has("target"):
			var payload := {
				"ok": _executor.get_ref()._failures.is_empty(),
				"id": str(_executor.get_ref()._check.get("id", "")),
				"kind": str(_executor.get_ref()._check.get("kind", "")),
				"target": str(_executor.get_ref()._check.get("target", "")),
				"duration_ms": Time.get_ticks_msec() - _executor.get_ref()._started_at_msec,
				"display_server": DisplayServer.get_name(),
				"runtime_errors": _executor.get_ref()._runtime_errors.duplicate(true),
				"failures": _executor.get_ref()._failures.duplicate(true),
				"artifacts": _executor.get_ref()._artifacts.duplicate(true),
			}
			if not _executor.get_ref()._check_data.is_empty():
				payload["data"] = _executor.get_ref()._check_data.duplicate(true)
			if _executor.get_ref()._has_time_control_evidence():
				payload["time_control"] = _executor.get_ref()._time_control_evidence.duplicate(true)
			if _executor.get_ref()._profile_enabled:
				payload["profile"] = _executor.get_ref()._profile_payload()
			return payload
		var scene_payload := {
			"ok": _executor.get_ref()._failures.is_empty(),
			"scene": str(_executor.get_ref()._check.get("scene", "")),
			"duration_ms": Time.get_ticks_msec() - _executor.get_ref()._started_at_msec,
			"startup_frames": max(int(_executor.get_ref()._check.get("startup_frames", 0)), 0),
			"display_server": DisplayServer.get_name(),
			"runtime_errors": _executor.get_ref()._runtime_errors.duplicate(true),
			"failures": _executor.get_ref()._failures.duplicate(true),
			"artifacts": _executor.get_ref()._artifacts.duplicate(true),
		}
		if _executor.get_ref()._has_time_control_evidence():
			scene_payload["time_control"] = _executor.get_ref()._time_control_evidence.duplicate(true)
		return scene_payload
	var payload := {
		"ok": _executor.get_ref()._failures.is_empty(),
		"run_identity": {
			"run_id": str(_executor.get_ref()._request_run.get("id", "")),
			"nonce": str(_executor.get_ref()._request_run.get("nonce", "")),
			"started_at": str(_executor.get_ref()._request_run.get("started_at", "")),
		},
		"scenario": str(_executor.get_ref()._scenario.get("name", "")),
		"scene": str(_executor.get_ref()._scenario.get("scene", "")),
		"duration_ms": Time.get_ticks_msec() - _executor.get_ref()._started_at_msec,
		"display_server": DisplayServer.get_name(),
		"snapshots": _executor.get_ref()._snapshots.duplicate(),
		"snapshot_artifacts": _executor.get_ref()._snapshot_artifacts.duplicate(true),
		"failures": _executor.get_ref()._failures.duplicate(),
		"artifacts": _executor.get_ref()._artifacts.duplicate(),
	}
	if _executor.get_ref()._asset_trial != null:
		payload["asset_trial"] = _executor.get_ref()._asset_trial.payload()
	if _executor.get_ref()._assertion_proof != null and _executor.get_ref()._assertion_proof.enabled:
		payload["assertion_coverage"] = _executor.get_ref()._assertion_proof.payload()
	if not _executor.get_ref()._node_inspections.is_empty():
		payload["node_inspections"] = _executor.get_ref()._node_inspections.duplicate(true)
	if not _executor.get_ref()._state_records.is_empty():
		payload["state_records"] = _executor.get_ref()._state_records.duplicate(true)
	if not _executor.get_ref()._physics_overlays.is_empty():
		payload["physics_overlays"] = _executor.get_ref()._physics_overlays.duplicate(true)
	if not _executor.get_ref()._policy_drives.is_empty():
		payload["policy_drives"] = _executor.get_ref()._policy_drives.duplicate(true)
	if not _executor.get_ref()._audio_artifacts.is_empty():
		payload["audio_artifacts"] = _executor.get_ref()._audio_artifacts.duplicate(true)
	payload["godot_args"] = _executor.get_ref()._scenario.get("godot_args", _executor.get_ref()._request_run.get("godot_args", [])).duplicate(true)
	payload["project_args"] = _executor.get_ref()._scenario.get("project_args", _executor.get_ref()._request_run.get("project_args", [])).duplicate(true)
	var timing: Dictionary = _executor.get_ref()._timing.duplicate()
	timing["engine_total_ms"] = float(Time.get_ticks_msec() - _executor.get_ref()._started_at_msec)
	payload["timing"] = timing
	if _executor.get_ref()._profile_enabled:
		payload["profile"] = _executor.get_ref()._profile_payload()
	if _executor.get_ref()._soak != null:
		payload["soak"] = _executor.get_ref()._soak.summary()
	if _executor.get_ref()._has_time_control_evidence():
		payload["time_control"] = _executor.get_ref()._time_control_evidence.duplicate(true)
	return payload

func _record_node_inspection(step_index: int, target: String, result: Dictionary) -> void:
	var data = result.get("data", {})
	if not data is Dictionary:
		return
	var inspection: Dictionary = (data as Dictionary).duplicate(true)
	inspection["step_index"] = step_index
	inspection["target"] = target
	_executor.get_ref()._node_inspections["%d:%s" % [step_index, target]] = inspection

# record_state evidence is keyed by the step's `name` (validated unique per
# scenario doc-side), mirroring how node_inspect keys `_node_inspections`. The
# bounded value plus its source descriptor come straight from the state.record
# runtime response; we only stamp the step index and name so the evidence stays
# addressable in result.json and the sweep pass-through.
func _record_state_record(step_index: int, name: String, result: Dictionary) -> void:
	var data = result.get("data", {})
	if not data is Dictionary:
		return
	var record: Dictionary = (data as Dictionary).duplicate(true)
	record["step_index"] = step_index
	record["name"] = name
	_executor.get_ref()._state_records[name] = record

func _physics_overlay_step_params(raw_params: Variant) -> Dictionary:
	var params: Dictionary = {}
	if raw_params is Dictionary:
		params = (raw_params as Dictionary).duplicate(true)
	else:
		params["target"] = str(raw_params)
	if not params.has("target"):
		params["target"] = ""
	params["target"] = str(params.get("target", "")).strip_edges()
	params["viewport"] = bool(params.get("viewport", false))
	if params.has("radius"):
		params["radius"] = clampf(float(params.get("radius", 256.0)), 1.0, 4096.0)
	if params.has("frames"):
		params["frames"] = clampi(int(params.get("frames", 1)), 1, 240)
	if params.has("include_aabb"):
		params["include_aabb"] = bool(params.get("include_aabb", true))
	if params.has("include_contacts"):
		params["include_contacts"] = bool(params.get("include_contacts", false))
	if params.has("max_shapes"):
		params["max_shapes"] = clampi(int(params.get("max_shapes", 512)), 1, 4096)
	if params.has("state_method") and not params.has("state_methods"):
		params["state_methods"] = params.get("state_method")
	return params

func _record_physics_overlay(step_index: int, step: Dictionary, params: Dictionary, result: Dictionary) -> void:
	if _executor.get_ref()._physics_overlays.size() >= MAX_PHYSICS_OVERLAY_EVIDENCE:
		return
	var data = result.get("data", {})
	if not data is Dictionary:
		return
	var raw_step = step.get("physics_overlay", {})
	var step_params: Dictionary = raw_step if raw_step is Dictionary else {}
	var name := str(step_params.get("name", "")).strip_edges()
	var target := str(params.get("target", data.get("target", ""))).strip_edges()
	var key_target := "viewport" if bool(params.get("viewport", false)) else target
	var key := "%d:%s" % [step_index, name if name != "" else key_target]
	var evidence := {
		"step_index": step_index,
		"name": name,
		"target": target,
		"viewport": bool(params.get("viewport", false)),
		"metadata": (data as Dictionary).duplicate(true),
	}
	var snapshot := str(step_params.get("snapshot", "")).strip_edges()
	if snapshot == "":
		snapshot = _executor.get_ref()._last_snapshot_name
	if snapshot != "":
		evidence["snapshot"] = snapshot
	_executor.get_ref()._physics_overlays[key] = evidence

func _capture_snapshot(snapshot_name: String) -> void:
	var started_usec: int = _executor.get_ref()._profile_now_usec()
	var failure_count: int = _executor.get_ref()._failures.size()
	if not _executor.get_ref()._snapshots.has(snapshot_name):
		_executor.get_ref()._snapshots.append(snapshot_name)
	_executor.get_ref()._last_snapshot_name = snapshot_name

	var snapshot_dir := _snapshot_dir(snapshot_name)
	var snapshot_paths := QaSnapshot.snapshot_paths(snapshot_dir)
	_executor.get_ref()._snapshot_artifacts[snapshot_name] = snapshot_paths.duplicate(true)
	_executor.get_ref()._last_snapshot_artifacts = snapshot_paths.duplicate(true)
	var filter_params := {}
	if _executor.get_ref()._scenario.get("snapshots", {}) is Dictionary:
		filter_params = (_executor.get_ref()._scenario.get("snapshots", {}) as Dictionary).duplicate(true)
	var snapshot_scope: Dictionary = _executor.get_ref()._runtime.resolve_snapshot_filter(filter_params)
	if not bool(snapshot_scope.get("ok", false)):
		var scope_error: Dictionary = snapshot_scope.get("error", {})
		_executor.get_ref()._failures.append(_executor.get_ref()._failure(
			str(scope_error.get("type", "invalid_snapshot_filter")),
			str(scope_error.get("message", "snapshot filter could not be resolved")),
			scope_error,
		))
		_executor.get_ref()._record_profile_timing("snapshot_capture", started_usec, failure_count, {
			"kind": "snapshot",
			"snapshot": snapshot_name,
			"artifacts": ["scope"],
		})
		return
	if _executor.get_ref()._asset_trial != null:
		_executor.get_ref()._asset_trial.snapshot_begin(_executor.get_ref()._runtime, snapshot_name)
	var capture_result: Dictionary = await QaSnapshot.capture_snapshot_bundle(
		snapshot_scope.get("root"),
		_executor.get_ref()._tree.get_root(),
		_executor.get_ref()._runtime_errors,
		snapshot_dir,
		snapshot_scope.get("options", {}),
	)
	if _executor.get_ref()._asset_trial != null:
		await _executor.get_ref()._asset_trial.snapshot(_executor.get_ref()._runtime, snapshot_name, snapshot_dir.path_join("asset-trial-visual"))
	if bool(capture_result.get("ok", false)):
		_executor.get_ref()._record_profile_timing("snapshot_capture", started_usec, failure_count, {
			"kind": "snapshot",
			"snapshot": snapshot_name,
			"artifacts": ["screenshot", "tree", "ui", "focus", "errors"],
		})
		return
	for failure in capture_result.get("failures", []):
		if failure is Dictionary:
			var failure_dict: Dictionary = (failure as Dictionary).duplicate(true)
			_executor.get_ref()._record_runtime_error("Unable to capture snapshot artifact %s" % str(failure_dict.get("target", snapshot_name)))
			_executor.get_ref()._failures.append(_executor.get_ref()._failure(
				str(failure_dict.get("type", "snapshot_write_failed")),
				str(failure_dict.get("message", "snapshot artifact failed")),
				{
					"target": failure_dict.get("target"),
					"expected": failure_dict.get("expected"),
					"actual": failure_dict.get("actual"),
				},
			))
	_executor.get_ref()._record_profile_timing("snapshot_capture", started_usec, failure_count, {
		"kind": "snapshot",
		"snapshot": snapshot_name,
		"artifacts": ["screenshot", "tree", "ui", "focus", "errors"],
	})

func _run_audio_capture_step(params: Dictionary) -> void:
	var name := str(params.get("name", ""))
	var safe_name := _safe_audio_artifact_name(name)
	var bus := str(params.get("bus", "Master"))
	var duration_seconds := float(params.get("duration_seconds", 0.0))
	var audio_dir := _audio_artifact_dir()
	var wav_path := _audio_wav_path(safe_name)
	if safe_name == "" or audio_dir == "" or duration_seconds <= 0.0 or not is_finite(duration_seconds):
		_executor.get_ref()._failures.append(_audio_failure(
			"audio_capture_failed",
			"audio_capture requires a non-empty name, controlled audio_dir, and positive duration_seconds",
			"audio_capture",
			{
				"target": bus,
				"expected": "valid name, artifacts.audio_dir, duration_seconds > 0",
				"actual": params.duplicate(true),
			},
		))
		return
	var bus_index := AudioServer.get_bus_index(bus)
	if bus_index < 0:
		_executor.get_ref()._failures.append(_audio_failure(
			"audio_capture_unavailable",
			"audio_capture could not find the requested bus",
			"audio_capture",
			{
				"target": bus,
				"expected": "existing AudioServer bus",
				"actual": "missing",
			},
		))
		return
	var record_effect := AudioEffectRecord.new()
	if record_effect == null or not record_effect.has_method("set_recording_active") or not record_effect.has_method("get_recording"):
		_executor.get_ref()._failures.append(_audio_failure(
			"audio_capture_unavailable",
			"Godot audio recording APIs are unavailable",
			"audio_capture",
			{
				"target": bus,
				"expected": "AudioEffectRecord with recording API",
				"actual": "unavailable",
			},
		))
		return
	var effect_index := AudioServer.get_bus_effect_count(bus_index)
	var added_effect := false
	# Globalize so a user:// audio_dir (Android scenario sessions) resolves to a real
	# OS path; a desktop absolute path passes through globalize_path unchanged.
	var dir_error := DirAccess.make_dir_recursive_absolute(ProjectSettings.globalize_path(audio_dir))
	if dir_error != OK:
		_executor.get_ref()._failures.append(_audio_failure(
			"audio_capture_failed",
			"audio_capture could not create the controlled audio artifact directory",
			"audio_capture",
			{
				"target": bus,
				"expected": audio_dir,
				"actual": dir_error,
			},
		))
		return
	AudioServer.add_bus_effect(bus_index, record_effect, effect_index)
	added_effect = true
	record_effect.call("set_recording_active", true)
	if _executor.get_ref()._asset_trial != null:
		_executor.get_ref()._asset_trial.audio_start(_executor.get_ref()._runtime, name, bus)
	var capture_started_msec := Time.get_ticks_msec()
	while float(Time.get_ticks_msec() - capture_started_msec) / 1000.0 < duration_seconds:
		await _executor.get_ref()._tree.process_frame
		_executor.get_ref()._sample_profile_frame()
		if _executor.get_ref()._asset_trial != null:
			_executor.get_ref()._asset_trial.audio_frame(_executor.get_ref()._runtime)
	record_effect.call("set_recording_active", false)
	if _executor.get_ref()._asset_trial != null:
		_executor.get_ref()._asset_trial.audio_end(_executor.get_ref()._runtime)
	var recording = record_effect.call("get_recording")
	if added_effect:
		AudioServer.remove_bus_effect(bus_index, effect_index)
	record_effect = null
	if recording == null or not recording is AudioStreamWAV:
		_executor.get_ref()._failures.append(_audio_failure(
			"audio_capture_failed",
			"audio_capture did not produce a WAV recording",
			"audio_capture",
			{
				"target": bus,
				"expected": "AudioStreamWAV recording",
				"actual": recording,
			},
		))
		return
	var save_error: int = (recording as AudioStreamWAV).save_to_wav(wav_path)
	if save_error != OK or not FileAccess.file_exists(wav_path):
		_executor.get_ref()._failures.append(_audio_failure(
			"audio_capture_failed",
			"audio_capture could not write the WAV artifact",
			"audio_capture",
			{
				"target": bus,
				"expected": wav_path,
				"actual": save_error,
			},
		))
		return
	_record_audio_artifact(name, safe_name, wav_path, {"source": "audio_capture", "bus": bus, "duration_seconds": duration_seconds})

func _run_audio_export_step(params: Dictionary) -> void:
	var name := str(params.get("name", ""))
	var safe_name := _safe_audio_artifact_name(name)
	var target := str(params.get("target", ""))
	var method_name := str(params.get("method", ""))
	var audio_dir := _audio_artifact_dir()
	var wav_path := _audio_wav_path(safe_name)
	if safe_name == "" or target == "" or audio_dir == "":
		_executor.get_ref()._failures.append(_audio_failure(
			"audio_export_invalid",
			"audio_export requires a non-empty name, target, and controlled audio_dir",
			"audio_export",
			{
				"target": target,
				"expected": "valid name, target, and artifacts.audio_dir",
				"actual": params.duplicate(true),
			},
		))
		return
	if not method_name.begins_with("qa_"):
		_executor.get_ref()._failures.append(_audio_failure(
			"audio_export_rejected",
			"audio_export only allows qa_ methods",
			"audio_export",
			{
				"target": target,
				"method": method_name,
				"expected": "method name beginning with qa_",
				"actual": method_name,
			},
		))
		return
	var resolution: Dictionary = _executor.get_ref()._runtime.resolve_target(target)
	if not bool(resolution.get("ok", false)):
		_executor.get_ref()._failures.append(_executor.get_ref()._runtime_error_to_failure(resolution.get("error", {})))
		return
	var node := resolution.get("node") as Node
	if node == null or not node.has_method(method_name):
		_executor.get_ref()._failures.append(_audio_failure(
			"audio_export_rejected",
			"audio_export could not find the requested qa_ method",
			"audio_export",
			{
				"target": target,
				"method": method_name,
				"expected": "method on resolved node",
				"actual": node.get_class() if node != null else "missing",
			},
		))
		return
	# Globalize so a user:// audio_dir (Android scenario sessions) resolves to a real
	# OS path; a desktop absolute path passes through globalize_path unchanged.
	var dir_error := DirAccess.make_dir_recursive_absolute(ProjectSettings.globalize_path(audio_dir))
	if dir_error != OK:
		_executor.get_ref()._failures.append(_audio_failure(
			"audio_export_failed",
			"audio_export could not create the controlled audio artifact directory",
			"audio_export",
			{
				"target": target,
				"expected": audio_dir,
				"actual": dir_error,
			},
		))
		return
	var args_value = params.get("args", [])
	var args: Array = args_value if args_value is Array else []
	var call_args := args.duplicate(true)
	call_args.append({
		"audio_dir": audio_dir,
		"name": safe_name,
		"wav_path": wav_path,
		"max_bytes": AUDIO_EXPORT_MAX_BYTES,
	})
	var result = node.callv(method_name, call_args)
	await _executor.get_ref()._tree.process_frame
	if result is PackedByteArray:
		_accept_audio_export_bytes(name, safe_name, wav_path, result, target, method_name)
		return
	if result is String:
		var decoded := Marshalls.base64_to_raw(str(result))
		_accept_audio_export_bytes(name, safe_name, wav_path, decoded, target, method_name)
		return
	if result is Dictionary:
		_accept_audio_export_dictionary(name, safe_name, result, target, method_name)
		return
	_executor.get_ref()._failures.append(_audio_failure(
		"audio_export_invalid_return",
		"audio_export method returned an unsupported value",
		"audio_export",
		{
			"target": target,
			"method": method_name,
			"expected": "base64 WAV bytes, PackedByteArray, or dictionary describing a WAV under audio_dir",
			"actual": result,
		},
	))

func _accept_audio_export_bytes(name: String, safe_name: String, wav_path: String, bytes: PackedByteArray, target: String, method_name: String) -> void:
	if bytes.size() <= 0 or bytes.size() > AUDIO_EXPORT_MAX_BYTES:
		_executor.get_ref()._failures.append(_audio_failure(
			"audio_export_oversized",
			"audio_export returned an empty or oversized WAV payload",
			"audio_export",
			{
				"target": target,
				"method": method_name,
				"expected": "1..%d bytes" % AUDIO_EXPORT_MAX_BYTES,
				"actual": bytes.size(),
			},
		))
		return
	if not _looks_like_wav(bytes):
		_executor.get_ref()._failures.append(_audio_failure(
			"audio_export_invalid_return",
			"audio_export returned bytes that are not RIFF/WAVE data",
			"audio_export",
			{
				"target": target,
				"method": method_name,
				"expected": "RIFF/WAVE bytes",
				"actual": "invalid_header",
			},
		))
		return
	var file := FileAccess.open(wav_path, FileAccess.WRITE)
	if file == null:
		_executor.get_ref()._failures.append(_audio_failure(
			"audio_export_failed",
			"audio_export could not write the WAV artifact",
			"audio_export",
			{
				"target": target,
				"method": method_name,
				"expected": wav_path,
				"actual": "open_failed",
			},
		))
		return
	file.store_buffer(bytes)
	_record_audio_artifact(name, safe_name, wav_path, {"source": "audio_export", "target": target, "method": method_name})

func _accept_audio_export_dictionary(name: String, safe_name: String, result: Dictionary, target: String, method_name: String) -> void:
	if not _is_json_compatible(result):
		_executor.get_ref()._failures.append(_audio_failure(
			"audio_export_invalid_metadata",
			"audio_export returned non-JSON-compatible metadata",
			"audio_export",
			{
				"target": target,
				"method": method_name,
				"expected": "JSON-compatible dictionary",
				"actual": result,
			},
		))
		return
	var wav_path := str(result.get("wav", result.get("path", "")))
	if not _is_path_under_audio_dir(wav_path):
		_executor.get_ref()._failures.append(_audio_failure(
			"audio_export_path_rejected",
			"audio_export returned a WAV path outside the controlled audio directory",
			"audio_export",
			{
				"target": target,
				"method": method_name,
				"expected": _audio_artifact_dir(),
				"actual": wav_path,
			},
		))
		return
	if not FileAccess.file_exists(wav_path):
		_executor.get_ref()._failures.append(_audio_failure(
			"audio_export_failed",
			"audio_export returned a WAV path that does not exist",
			"audio_export",
			{
				"target": target,
				"method": method_name,
				"expected": "existing WAV under audio_dir",
				"actual": wav_path,
			},
		))
		return
	var file := FileAccess.open(wav_path, FileAccess.READ)
	var size := file.get_length() if file != null else 0
	if size <= 0 or size > AUDIO_EXPORT_MAX_BYTES:
		_executor.get_ref()._failures.append(_audio_failure(
			"audio_export_oversized",
			"audio_export wrote an empty or oversized WAV artifact",
			"audio_export",
			{
				"target": target,
				"method": method_name,
				"expected": "1..%d bytes" % AUDIO_EXPORT_MAX_BYTES,
				"actual": size,
			},
		))
		return
	if file == null or not _looks_like_wav(file.get_buffer(min(size, 12))):
		_executor.get_ref()._failures.append(_audio_failure(
			"audio_export_invalid_return",
			"audio_export wrote a file that is not RIFF/WAVE data",
			"audio_export",
			{
				"target": target,
				"method": method_name,
				"expected": "RIFF/WAVE file",
				"actual": wav_path,
			},
		))
		return
	var metadata := result.duplicate(true)
	metadata.erase("wav")
	metadata.erase("path")
	metadata["source"] = "audio_export"
	metadata["target"] = target
	metadata["method"] = method_name
	_record_audio_artifact(name, safe_name, wav_path, metadata)

func _record_audio_artifact(name: String, safe_name: String, wav_path: String, metadata: Dictionary) -> void:
	var artifact := {
		"wav": wav_path,
		"name": name,
		"safe_name": safe_name,
	}
	if not metadata.is_empty():
		artifact["metadata"] = metadata.duplicate(true)
	_executor.get_ref()._audio_artifacts[name] = artifact

func _audio_failure(failure_type: String, message: String, command: String, extra: Dictionary) -> Dictionary:
	var data := extra.duplicate(true)
	data["command"] = command
	data["artifacts"] = {
		"run_root": _executor.get_ref()._artifact_run_root(),
		"audio_dir": _audio_artifact_dir(),
	}
	return _executor.get_ref()._failure(failure_type, message, data)

func _audio_artifact_dir() -> String:
	return str(_executor.get_ref()._artifacts.get("audio_dir", ""))

func _audio_wav_path(safe_name: String) -> String:
	if safe_name == "" or _audio_artifact_dir() == "":
		return ""
	return "%s/%s.wav" % [_audio_artifact_dir(), safe_name]

func _safe_audio_artifact_name(name: String) -> String:
	var safe := ""
	for index in range(name.length()):
		var character := name.substr(index, 1)
		if character.is_valid_identifier() or character.is_valid_int() or character in ["-", "_"]:
			safe += character
		else:
			safe += "_"
	return safe.strip_edges().trim_prefix(".")

func _is_path_under_audio_dir(path: String) -> bool:
	var audio_dir := _audio_artifact_dir().simplify_path()
	var simplified := path.simplify_path()
	return audio_dir != "" and (simplified == audio_dir or simplified.begins_with("%s/" % audio_dir))

func _looks_like_wav(bytes: PackedByteArray) -> bool:
	return bytes.size() >= 12 and bytes.slice(0, 4).get_string_from_ascii() == "RIFF" and bytes.slice(8, 12).get_string_from_ascii() == "WAVE"

func _is_json_compatible(value: Variant) -> bool:
	if value == null or value is bool or value is int or value is float or value is String:
		return true
	if value is Array:
		for item in value:
			if not _is_json_compatible(item):
				return false
		return true
	if value is Dictionary:
		for key in value.keys():
			if not (key is String or key is StringName):
				return false
			if not _is_json_compatible(value.get(key)):
				return false
		return true
	return false

func _snapshot_dir(snapshot_name: String) -> String:
	return "%s/%s" % [str(_executor.get_ref()._artifacts.get("snapshots_dir", "")), snapshot_name]
