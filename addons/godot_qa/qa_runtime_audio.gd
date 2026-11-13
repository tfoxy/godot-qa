extends RefCounted

var _runtime: WeakRef


func _init(runtime: RefCounted) -> void:
	_runtime = weakref(runtime)


func _audio_capture(params: Dictionary) -> Dictionary:
	var name := _safe_audio_artifact_name(str(params.get("name", "capture")))
	var bus := str(params.get("bus", "Master"))
	var audio_dir := str(params.get("audio_dir", ""))
	var wav_path := str(params.get("wav_path", params.get("output_path", "")))
	var requested_duration := float(params.get("duration_seconds", params.get("seconds", 0.0)))
	var max_seconds := clampf(float(params.get("max_seconds", _runtime.get_ref().AUDIO_CAPTURE_MAX_SECONDS)), 0.001, _runtime.get_ref().AUDIO_CAPTURE_MAX_SECONDS)
	var effective_duration := clampf(requested_duration, 0.0, max_seconds)
	var max_bytes := maxi(1, int(params.get("max_bytes", _runtime.get_ref().AUDIO_EXPORT_MAX_BYTES)))
	var stop_after_state: Dictionary = _runtime.get_ref()._stop_after_state_params(params)
	var stop_sample_every_frames := maxi(1, int(stop_after_state.get("sample_every_frames", 1))) if not stop_after_state.is_empty() else 1
	var warnings: Array = []
	if requested_duration > max_seconds:
		warnings.append(_audio_warning("audio_capture.duration_too_long", "requested live audio capture duration was clamped", {
			"requested_duration_seconds": requested_duration,
			"effective_duration_seconds": effective_duration,
			"max_seconds": max_seconds,
		}))
	var validation := _validate_audio_output("audio.capture", name, audio_dir, wav_path, effective_duration)
	if not bool(validation.get("ok", false)):
		return validation
	var bus_index := AudioServer.get_bus_index(bus)
	if bus_index < 0:
		return _audio_error(
			"audio_capture.bus_not_found",
			"audio.capture could not find the requested AudioServer bus",
			"audio.capture",
			{"bus": bus, "expected": "existing AudioServer bus", "actual": "missing"},
			warnings,
		)
	var record_effect := AudioEffectRecord.new()
	if record_effect == null or not record_effect.has_method("set_recording_active") or not record_effect.has_method("get_recording"):
		return _audio_error(
			"audio_capture.unavailable",
			"Godot safe bus recording APIs are unavailable",
			"audio.capture",
			{"bus": bus, "expected": "AudioEffectRecord recording API", "actual": "unavailable"},
			warnings,
		)
	var dir_error := DirAccess.make_dir_recursive_absolute(audio_dir)
	if dir_error != OK:
		return _audio_error(
			"audio_capture.failed",
			"audio.capture could not create the controlled audio artifact directory",
			"audio.capture",
			{"path": audio_dir, "expected": "writable controlled audio_dir", "actual": dir_error},
			warnings,
		)
	var effect_index := AudioServer.get_bus_effect_count(bus_index)
	AudioServer.add_bus_effect(bus_index, record_effect, effect_index)
	record_effect.call("set_recording_active", true)
	var started_msec := Time.get_ticks_msec()
	var stop_condition_matched := false
	var stop_condition_sample_count := 0
	var stop_condition_last_error: Dictionary = {}
	while float(Time.get_ticks_msec() - started_msec) / 1000.0 < effective_duration:
		await _runtime.get_ref()._process_frames(stop_sample_every_frames)
		if not stop_after_state.is_empty():
			stop_condition_sample_count += 1
			var stop_result: Dictionary = await _runtime.get_ref()._assertions.execute("assert.state_matches", stop_after_state)
			if bool(stop_result.get("ok", false)):
				stop_condition_matched = true
				break
			var stop_error: Dictionary = stop_result.get("error", {}) if stop_result.get("error", {}) is Dictionary else {}
			stop_condition_last_error = stop_error.duplicate(true)
			if str(stop_error.get("type", "")) != "state_matches_mismatch":
				record_effect.call("set_recording_active", false)
				AudioServer.remove_bus_effect(bus_index, effect_index)
				return {"ok": false, "error": stop_error}
	record_effect.call("set_recording_active", false)
	var recording = record_effect.call("get_recording")
	AudioServer.remove_bus_effect(bus_index, effect_index)
	if recording == null or not recording is AudioStreamWAV:
		return _audio_error(
			"audio_capture.failed",
			"audio.capture did not produce WAV recording data",
			"audio.capture",
			{"bus": bus, "expected": "AudioStreamWAV recording", "actual": _runtime.get_ref()._json_compatible_value(recording)},
			warnings,
		)
	var save_error: int = (recording as AudioStreamWAV).save_to_wav(wav_path)
	if save_error != OK or not FileAccess.file_exists(wav_path):
		return _audio_error(
			"audio_capture.failed",
			"audio.capture could not write the WAV artifact",
			"audio.capture",
			{"path": wav_path, "expected": "writable WAV path under audio_dir", "actual": save_error},
			warnings,
		)
	var file := FileAccess.open(wav_path, FileAccess.READ)
	var byte_count := file.get_length() if file != null else 0
	if byte_count <= 0:
		return _audio_error(
			"audio_capture.failed",
			"audio.capture captured no audio frames; the runtime may be running without an active audio output device",
			"audio.capture",
			{
				"path": wav_path,
				"bus": bus,
				"expected": "non-empty WAV recording",
				"actual": byte_count,
				"remediation": "Confirm the live session has an audio output device (or dummy-driver mixing) and that the selected bus produces sound during the capture window.",
			},
			warnings,
		)
	if byte_count > max_bytes:
		return _audio_error(
			"audio_capture.failed",
			"audio.capture wrote an oversized WAV artifact",
			"audio.capture",
			{"path": wav_path, "expected": "1..%d bytes" % max_bytes, "actual": byte_count},
			warnings,
		)
	var artifacts := _audio_artifacts(name, wav_path, audio_dir, {"source": "audio.capture", "bus": bus, "bytes": byte_count})
	var payload := {
		"name": name,
		"bus": bus,
		"duration_seconds": requested_duration,
		"effective_duration_seconds": effective_duration,
		"capabilities": {"bus_capture": true, "host_microphone_capture": false},
		"artifacts": artifacts.get(name, {}),
		"audio_artifacts": artifacts,
		"warnings": warnings,
		"wav_path": wav_path,
	}
	if not stop_after_state.is_empty():
		payload["stop_after_state"] = {
			"requested": stop_after_state.duplicate(true),
			"matched": stop_condition_matched,
			"sample_count": stop_condition_sample_count,
			"sample_every_frames": stop_sample_every_frames,
			"last_error": stop_condition_last_error,
		}
		payload["stop_condition_matched"] = stop_condition_matched
		payload["stop_condition_timed_out"] = not stop_condition_matched
	return _runtime.get_ref()._ok(payload)


func _audio_export(params: Dictionary) -> Dictionary:
	var name := _safe_audio_artifact_name(str(params.get("name", "export")))
	var target := str(params.get("target", ""))
	var method_name := str(params.get("method", ""))
	var audio_dir := str(params.get("audio_dir", ""))
	var wav_path := str(params.get("wav_path", params.get("output_path", "")))
	var max_bytes := maxi(1, int(params.get("max_bytes", _runtime.get_ref().AUDIO_EXPORT_MAX_BYTES)))
	var validation := _validate_audio_output("audio.export", name, audio_dir, wav_path, 1.0)
	if not bool(validation.get("ok", false)):
		return validation
	if target == "":
		return _audio_error("invalid_params", "audio.export requires a target", "audio.export", {"expected": "non-empty target", "actual": target})
	if not method_name.begins_with("qa_"):
		return _audio_error(
			"audio_export.rejected",
			"audio.export only allows project-owned qa_ methods",
			"audio.export",
			{"target": target, "method": method_name, "expected": "method name beginning with qa_", "actual": method_name},
		)
	var resolution: Dictionary = _runtime.get_ref().resolve_target(target)
	if not bool(resolution.get("ok", false)):
		return {"ok": false, "error": (resolution.get("error", {}) as Dictionary).duplicate(true)}
	var node := resolution.get("node") as Node
	if node == null or not node.has_method(method_name):
		return _audio_error(
			"audio_export.rejected",
			"audio.export could not find the requested qa_ method",
			"audio.export",
			{"target": target, "method": method_name, "expected": "method on resolved node", "actual": node.get_class() if node != null else "missing"},
		)
	var dir_error := DirAccess.make_dir_recursive_absolute(audio_dir)
	if dir_error != OK:
		return _audio_error("audio_export.failed", "audio.export could not create the controlled audio artifact directory", "audio.export", {"path": audio_dir, "expected": "writable controlled audio_dir", "actual": dir_error})
	var args_value = params.get("args", [])
	var args: Array = args_value if args_value is Array else []
	var call_args := args.duplicate(true)
	call_args.append({"audio_dir": audio_dir, "name": name, "wav_path": wav_path, "max_bytes": max_bytes})
	var result = node.callv(method_name, call_args)
	await _runtime.get_ref()._process_frames(1)
	if result is PackedByteArray:
		return _accept_audio_export_bytes(name, wav_path, audio_dir, result, target, method_name, max_bytes)
	if result is String:
		return _accept_audio_export_bytes(name, wav_path, audio_dir, Marshalls.base64_to_raw(str(result)), target, method_name, max_bytes)
	if result is Dictionary:
		return _accept_audio_export_dictionary(name, audio_dir, result, target, method_name, max_bytes)
	return _audio_error(
		"audio_export.invalid_return",
		"audio.export method returned an unsupported value",
		"audio.export",
		{"target": target, "method": method_name, "expected": "base64 WAV bytes, PackedByteArray, or dictionary describing a WAV under audio_dir", "actual": _runtime.get_ref()._json_compatible_value(result)},
	)


func _accept_audio_export_bytes(name: String, wav_path: String, audio_dir: String, bytes: PackedByteArray, target: String, method_name: String, max_bytes: int) -> Dictionary:
	if bytes.size() <= 0 or bytes.size() > max_bytes:
		return _audio_error("audio_export.oversized", "audio.export returned an empty or oversized WAV payload", "audio.export", {"target": target, "method": method_name, "expected": "1..%d bytes" % max_bytes, "actual": bytes.size()})
	if not _looks_like_wav(bytes):
		return _audio_error("audio_export.invalid_return", "audio.export returned bytes that are not RIFF/WAVE data", "audio.export", {"target": target, "method": method_name, "expected": "RIFF/WAVE bytes", "actual": "invalid_header"})
	var file := FileAccess.open(wav_path, FileAccess.WRITE)
	if file == null:
		return _audio_error("audio_export.failed", "audio.export could not write the WAV artifact", "audio.export", {"target": target, "method": method_name, "expected": wav_path, "actual": "open_failed"})
	file.store_buffer(bytes)
	var artifacts := _audio_artifacts(name, wav_path, audio_dir, {"source": "audio.export", "target": target, "method": method_name, "bytes": bytes.size()})
	return _runtime.get_ref()._ok({"name": name, "target": target, "method": method_name, "artifacts": artifacts.get(name, {}), "audio_artifacts": artifacts, "warnings": [], "wav_path": wav_path})


func _accept_audio_export_dictionary(name: String, audio_dir: String, result: Dictionary, target: String, method_name: String, max_bytes: int) -> Dictionary:
	if not _is_json_compatible_audio_value(result):
		return _audio_error("audio_export.invalid_metadata", "audio.export returned non-JSON-compatible metadata", "audio.export", {"target": target, "method": method_name, "expected": "JSON-compatible dictionary", "actual": _runtime.get_ref()._json_compatible_value(result)})
	var wav_path := str(result.get("wav", result.get("path", "")))
	if not _is_path_under_audio_dir(audio_dir, wav_path):
		return _audio_error("audio_export.path_rejected", "audio.export returned a WAV path outside the controlled audio directory", "audio.export", {"target": target, "method": method_name, "expected": audio_dir, "actual": wav_path})
	var file := FileAccess.open(wav_path, FileAccess.READ)
	var byte_count := file.get_length() if file != null else 0
	if byte_count <= 0 or byte_count > max_bytes:
		return _audio_error("audio_export.oversized", "audio.export wrote an empty or oversized WAV artifact", "audio.export", {"target": target, "method": method_name, "expected": "1..%d bytes" % max_bytes, "actual": byte_count})
	if file == null or not _looks_like_wav(file.get_buffer(mini(byte_count, 12))):
		return _audio_error("audio_export.invalid_return", "audio.export wrote a file that is not RIFF/WAVE data", "audio.export", {"target": target, "method": method_name, "expected": "RIFF/WAVE file", "actual": wav_path})
	var metadata := result.duplicate(true)
	metadata.erase("wav")
	metadata.erase("path")
	metadata["source"] = "audio.export"
	metadata["target"] = target
	metadata["method"] = method_name
	metadata["bytes"] = byte_count
	var artifacts := _audio_artifacts(name, wav_path, audio_dir, metadata)
	return _runtime.get_ref()._ok({"name": name, "target": target, "method": method_name, "artifacts": artifacts.get(name, {}), "audio_artifacts": artifacts, "warnings": [], "wav_path": wav_path})


## Offline-bake: the runtime owns the loop length (from seconds * mix_rate) and the WAV
## container; the project supplies only a qa_ voice-fill method returning mono samples in
## [-1, 1]. This generalizes per-project tools like coop-2's tools/bake_audio.gd and works
## headless because nothing is captured from an output device — it is synthesized to a buffer.
func _audio_render(params: Dictionary) -> Dictionary:
	var name := _safe_audio_artifact_name(str(params.get("name", "render")))
	var target := str(params.get("target", ""))
	var method_name := str(params.get("method", ""))
	var audio_dir := str(params.get("audio_dir", ""))
	var wav_path := str(params.get("wav_path", params.get("output_path", "")))
	var max_bytes := maxi(1, int(params.get("max_bytes", _runtime.get_ref().AUDIO_EXPORT_MAX_BYTES)))
	var mix_rate := maxi(1, int(params.get("mix_rate", 44100)))
	var duration_seconds := float(params.get("effective_duration_seconds", params.get("duration_seconds", 0.0)))
	var validation := _validate_audio_output("audio.render", name, audio_dir, wav_path, duration_seconds)
	if not bool(validation.get("ok", false)):
		return validation
	if target == "":
		return _audio_error("invalid_params", "audio.render requires a target", "audio.render", {"expected": "non-empty target", "actual": target})
	if not method_name.begins_with("qa_"):
		return _audio_error(
			"audio_render.rejected",
			"audio.render only allows project-owned qa_ methods",
			"audio.render",
			{"target": target, "method": method_name, "expected": "method name beginning with qa_", "actual": method_name},
		)
	var frame_count := int(ceil(duration_seconds * float(mix_rate)))
	if frame_count <= 0 or frame_count * 2 > max_bytes:
		return _audio_error(
			"audio_render.oversized",
			"audio.render requested duration is empty or would exceed the byte budget",
			"audio.render",
			{"target": target, "method": method_name, "expected": "1..%d frames" % (max_bytes / 2), "actual": frame_count},
		)
	var resolution: Dictionary = _runtime.get_ref().resolve_target(target)
	if not bool(resolution.get("ok", false)):
		return {"ok": false, "error": (resolution.get("error", {}) as Dictionary).duplicate(true)}
	var node := resolution.get("node") as Node
	if node == null or not node.has_method(method_name):
		return _audio_error(
			"audio_render.rejected",
			"audio.render could not find the requested qa_ method",
			"audio.render",
			{"target": target, "method": method_name, "expected": "method on resolved node", "actual": node.get_class() if node != null else "missing"},
		)
	var dir_error := DirAccess.make_dir_recursive_absolute(audio_dir)
	if dir_error != OK:
		return _audio_error("audio_render.failed", "audio.render could not create the controlled audio artifact directory", "audio.render", {"path": audio_dir, "expected": "writable controlled audio_dir", "actual": dir_error})
	var args_value = params.get("args", [])
	var args: Array = args_value if args_value is Array else []
	var call_args: Array = [frame_count, mix_rate]
	call_args.append_array(args)
	var result = node.callv(method_name, call_args)
	await _runtime.get_ref()._process_frames(1)
	var frames := _audio_render_frames(result)
	if frames == null:
		return _audio_error(
			"audio_render.invalid_return",
			"audio.render method returned an unsupported value",
			"audio.render",
			{"target": target, "method": method_name, "expected": "PackedFloat32Array, PackedVector2Array, or Array of mono samples in [-1, 1]", "actual": _runtime.get_ref()._json_compatible_value(result)},
		)
	if (frames as PackedFloat32Array).is_empty():
		return _audio_error("audio_render.failed", "audio.render produced no audio frames", "audio.render", {"target": target, "method": method_name, "expected": "non-empty sample buffer", "actual": 0})
	var wav := _audio_render_wav(frames as PackedFloat32Array, mix_rate)
	if wav.data.size() > max_bytes:
		return _audio_error("audio_render.oversized", "audio.render produced an oversized WAV payload", "audio.render", {"target": target, "method": method_name, "expected": "1..%d bytes" % max_bytes, "actual": wav.data.size()})
	if wav.save_to_wav(wav_path) != OK:
		return _audio_error("audio_render.failed", "audio.render could not write the WAV artifact", "audio.render", {"target": target, "method": method_name, "expected": wav_path, "actual": "save_failed"})
	var file := FileAccess.open(wav_path, FileAccess.READ)
	var byte_count := file.get_length() if file != null else 0
	if file == null or byte_count <= 0 or byte_count > max_bytes:
		return _audio_error("audio_render.oversized", "audio.render wrote an empty or oversized WAV artifact", "audio.render", {"target": target, "method": method_name, "expected": "1..%d bytes" % max_bytes, "actual": byte_count})
	if not _looks_like_wav(file.get_buffer(mini(byte_count, 12))):
		return _audio_error("audio_render.invalid_return", "audio.render wrote a file that is not RIFF/WAVE data", "audio.render", {"target": target, "method": method_name, "expected": "RIFF/WAVE file", "actual": wav_path})
	var metadata := {"source": "audio.render", "target": target, "method": method_name, "frames": (frames as PackedFloat32Array).size(), "mix_rate": mix_rate, "bytes": byte_count}
	var artifacts := _audio_artifacts(name, wav_path, audio_dir, metadata)
	return _runtime.get_ref()._ok({"name": name, "target": target, "method": method_name, "artifacts": artifacts.get(name, {}), "audio_artifacts": artifacts, "warnings": [], "wav_path": wav_path})


## Coerce a qa_ render return value into a mono float buffer, or null if unsupported.
func _audio_render_frames(result: Variant) -> Variant:
	if result is PackedFloat32Array:
		return result
	if result is PackedVector2Array:
		var stereo := result as PackedVector2Array
		var downmixed := PackedFloat32Array()
		downmixed.resize(stereo.size())
		for index in stereo.size():
			downmixed[index] = (stereo[index].x + stereo[index].y) * 0.5
		return downmixed
	if result is Array:
		var coerced := PackedFloat32Array()
		coerced.resize((result as Array).size())
		for index in (result as Array).size():
			var value = (result as Array)[index]
			if not (value is int or value is float):
				return null
			coerced[index] = float(value)
		return coerced
	return null


## Pack mono float samples in [-1, 1] into a 16-bit PCM AudioStreamWAV (same container the
## live AudioStreamGenerator feeds), matching coop-2's tools/bake_audio.gd `_bake_music`.
func _audio_render_wav(frames: PackedFloat32Array, mix_rate: int) -> AudioStreamWAV:
	var pcm := PackedByteArray()
	pcm.resize(frames.size() * 2)
	for index in frames.size():
		var sample := int(round(clampf(frames[index], -1.0, 1.0) * 32767.0))
		pcm.encode_s16(index * 2, sample)
	var wav := AudioStreamWAV.new()
	wav.format = AudioStreamWAV.FORMAT_16_BITS
	wav.mix_rate = mix_rate
	wav.stereo = false
	wav.data = pcm
	return wav


func _validate_audio_output(command: String, name: String, audio_dir: String, wav_path: String, duration_seconds: float) -> Dictionary:
	if name == "" or audio_dir == "" or wav_path == "" or duration_seconds <= 0.0 or not is_finite(duration_seconds):
		return _audio_error(
			"invalid_params",
			"%s requires a non-empty name, controlled audio_dir, WAV path, and positive bounded duration" % command,
			command,
			{"expected": "valid name, audio_dir, wav_path, duration_seconds > 0", "actual": {"name": name, "audio_dir": audio_dir, "wav_path": wav_path, "duration_seconds": duration_seconds}},
		)
	if not _is_path_under_audio_dir(audio_dir, wav_path):
		return _audio_error(
			"invalid_params",
			"%s WAV path must stay under the controlled audio_dir" % command,
			command,
			{"expected": audio_dir, "actual": wav_path},
		)
	return {"ok": true}


func _audio_artifacts(name: String, wav_path: String, audio_dir: String, metadata: Dictionary) -> Dictionary:
	return {
		name: {
			"wav": wav_path,
			"name": name,
			"metadata": metadata.duplicate(true),
		},
		"root": audio_dir,
		"wav": wav_path,
	}


func _audio_warning(warning_id: String, message: String, data := {}) -> Dictionary:
	var warning := {"id": warning_id, "message": message}
	if data is Dictionary and not data.is_empty():
		warning["data"] = data.duplicate(true)
	return warning


func _audio_error(error_type: String, message: String, command: String, data := {}, warnings := []) -> Dictionary:
	var payload: Dictionary = data.duplicate(true) if data is Dictionary else {}
	payload["command"] = command
	if warnings is Array and not warnings.is_empty():
		payload["warnings"] = warnings.duplicate(true)
	return _runtime.get_ref()._error(error_type, message, payload)


func _safe_audio_artifact_name(name: String) -> String:
	var safe := ""
	for index in range(name.length()):
		var character := name.substr(index, 1)
		if character.is_valid_identifier() or character.is_valid_int() or character in ["-", "_"]:
			safe += character
		else:
			safe += "_"
	return safe.strip_edges().trim_prefix(".")


func _is_path_under_audio_dir(audio_dir: String, path: String) -> bool:
	var simplified_audio_dir := audio_dir.simplify_path()
	var simplified_path := path.simplify_path()
	return simplified_audio_dir != "" and simplified_path.ends_with(".wav") and (simplified_path == simplified_audio_dir or simplified_path.begins_with("%s/" % simplified_audio_dir))


func _looks_like_wav(bytes: PackedByteArray) -> bool:
	return bytes.size() >= 12 and bytes.slice(0, 4).get_string_from_ascii() == "RIFF" and bytes.slice(8, 12).get_string_from_ascii() == "WAVE"


func _is_json_compatible_audio_value(value: Variant) -> bool:
	if value == null or value is bool or value is int or value is float or value is String:
		return true
	if value is Array:
		for item in value:
			if not _is_json_compatible_audio_value(item):
				return false
		return true
	if value is Dictionary:
		for key in value.keys():
			if not (key is String or key is StringName):
				return false
			if not _is_json_compatible_audio_value(value.get(key)):
				return false
		return true
	return false
