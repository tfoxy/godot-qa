extends RefCounted

# A bounded passive collector. The bridge remains serialized until encoding ends.
const QaAssertionProof = preload("res://addons/godot_qa/qa_assertion_proof.gd")
const QaAssertionsImpl = preload("res://addons/godot_qa/qa_assertions_impl.gd")
const MAX_FRAMES := 96
const MAX_BYTES := 1073741824
const ENCODE_SECONDS := 30.0

var _root: Window
var _assertions: RefCounted
var _params: Dictionary
var _pending: Array = []
var _failures: Array = []
var _done := false
var _draw_ordinal := -1
var _started_usec := 0
var _size := Vector2i.ZERO
var _window_size := Vector2i.ZERO
var _retained_bytes := 0
var _stop_reason := "schedule_completed"
var _stop_matched := false
var _stop_samples := 0
var _stop_last_error: Variant = null
var _stop_at: Variant = null
var _guard_identity: Array = []


func capture(root: Window, runtime: RefCounted, params: Dictionary, output_dir: String) -> Dictionary:
	_root = root
	_params = params
	_guard_identity = QaAssertionProof.new().definition_identity(params.get("state_guards", {}))
	_assertions = QaAssertionsImpl.new(runtime, root)
	_started_usec = Time.get_ticks_usec()
	var max_seconds := float(params.get("max_seconds", 3.0))
	var max_bytes := int(params.get("max_bytes", 536870912))
	var schedule = params.get("capture_frames", [])
	if DisplayServer.get_name() == "headless" or not is_finite(max_seconds) or max_seconds <= 0.0 or max_seconds > 10.0 or max_bytes <= 0 or max_bytes > MAX_BYTES or not schedule is Array or schedule.is_empty() or schedule.size() > 1000:
		_fail("screenshot_burst.args_invalid", "burst requires rendered output, a bounded schedule, and valid limits", {"max_seconds": max_seconds, "max_bytes": max_bytes})
	else:
		var previous := -1
		for frame in schedule:
			if not (frame is int or frame is float) or not is_finite(float(frame)) or float(frame) != float(int(frame)) or float(frame) < 0.0 or float(frame) > 100000.0 or int(frame) <= previous:
				_fail("screenshot_burst.args_invalid", "burst schedule must contain increasing non-negative integer ordinals", {"actual": schedule})
				break
			previous = int(frame)
		if str(params.get("timing_mode", "")) == "seconds":
			var fps := float(params.get("fps", 0.0))
			var seconds := float(params.get("seconds", 0.0))
			if not is_finite(fps) or fps <= 0.0 or not is_finite(seconds) or seconds <= 0.0:
				_fail("screenshot_burst.args_invalid", "burst seconds timing requires positive finite fps and seconds", {})
		elif str(params.get("timing_mode", "")) != "frames":
			_fail("screenshot_burst.args_invalid", "burst timing_mode must be frames or seconds", {})
	if not _done:
		RenderingServer.frame_post_draw.connect(_on_post_draw)
		while not _done:
			await root.get_tree().process_frame
			if float(Time.get_ticks_usec() - _started_usec) / 1000000.0 >= max_seconds and not _done:
				_fail("screenshot_burst.capture_deadline", "burst capture deadline exhausted", {"max_seconds": max_seconds})
		RenderingServer.frame_post_draw.disconnect(_on_post_draw)
	var capture_msec := float(Time.get_ticks_usec() - _started_usec) / 1000.0
	var encoded: Array = []
	var encoding_started := Time.get_ticks_usec()
	var mkdir_error := DirAccess.make_dir_recursive_absolute(output_dir)
	if mkdir_error != OK:
		_fail("artifact_write_failed", "could not create burst artifact directory", {"path": output_dir, "godot_error": mkdir_error})
	else:
		for record in _pending:
			if float(Time.get_ticks_usec() - encoding_started) / 1000000.0 >= ENCODE_SECONDS:
				_fail("screenshot_burst.encoding_deadline", "burst encoding deadline exhausted between images", {"encoding_seconds": ENCODE_SECONDS})
				break
			var image: Image = record.get("image")
			var path := "%s/frame_%04d.png" % [output_dir, int(record.get("index", 0))]
			var write_error := image.save_png(path)
			record.erase("image")
			if write_error != OK:
				_fail("artifact_write_failed", "could not encode burst frame", {"path": path, "godot_error": write_error})
				break
			record["artifacts"] = {"screenshot": path}
			encoded.append(record)
	var captured_in_memory := _pending.size()
	_pending.clear()
	_assertions.teardown()
	return {
		"capture_mode": "burst",
		"capture_source": "godot_viewport",
		"draw_forced": false,
		"allow_fallback": false,
		"complete": _failures.is_empty(),
		"stop_reason": _stop_reason,
		"effective_limits": {"max_bytes": max_bytes, "max_seconds": max_seconds, "max_frames": MAX_FRAMES, "encoding_seconds": ENCODE_SECONDS},
		"engine_pacing": {"max_fps": Engine.max_fps, "time_scale": Engine.time_scale, "physics_ticks_per_second": Engine.physics_ticks_per_second},
		"capture_timing": {"capture_msec": capture_msec, "encoding_msec": float(Time.get_ticks_usec() - encoding_started) / 1000.0},
		"retained_rgba_bytes": _retained_bytes,
		"captured_in_memory_count": captured_in_memory,
		"observed_draw_count": _draw_ordinal + 1,
		"stop_condition_matched": _stop_matched,
		"stop_condition_sample_count": _stop_samples,
		"stop_condition_stopped_at": _stop_at,
		"stop_condition_last_error": _stop_last_error,
		"frames": encoded,
		"failures": _failures,
	}


func _on_post_draw() -> void:
	if _done:
		return
	_draw_ordinal += 1
	var elapsed := float(Time.get_ticks_usec() - _started_usec) / 1000000.0
	if elapsed >= float(_params.get("max_seconds", 3.0)):
		_fail("screenshot_burst.capture_deadline", "burst capture deadline exhausted", {})
		return
	var schedule: Array = _params.get("capture_frames", [])
	var sequence_index := _pending.size()
	var time_mode := str(_params.get("timing_mode", "frames")) == "seconds"
	var due := float(sequence_index) / float(_params.get("fps", 1.0)) if time_mode else float(schedule[sequence_index])
	var texture := _root.get_texture()
	if texture == null:
		_fail("screenshot_burst.viewport_unavailable", "root viewport has no rendered texture", {})
		return
	var window_size := _root.size
	if _window_size != Vector2i.ZERO and window_size != _window_size:
		_fail("screenshot_burst.viewport_resized", "root window resized during burst", {"expected_window_size": [_window_size.x, _window_size.y], "actual_window_size": [window_size.x, window_size.y]})
		return
	var size := Vector2i(texture.get_width(), texture.get_height())
	if size.x <= 0 or size.y <= 0:
		_fail("screenshot_burst.viewport_unavailable", "root viewport dimensions must be positive", {"size": [size.x, size.y]})
		return
	if _size != Vector2i.ZERO and size != _size:
		_fail("screenshot_burst.viewport_resized", "root viewport resized during burst", {"expected": [_size.x, _size.y], "actual": [size.x, size.y]})
		return
	if (time_mode and elapsed < due) or (not time_mode and _draw_ordinal < int(due)):
		return
	if sequence_index >= MAX_FRAMES:
		_fail("screenshot_burst.frame_limit", "burst reached its retained frame limit", {"max_frames": MAX_FRAMES})
		return
	var available_bytes := int(_params.get("max_bytes", 536870912)) - _retained_bytes
	# Divide before multiplying so an extreme viewport cannot overflow geometry.
	if size.x > available_bytes / 4 / size.y:
		_fail("screenshot_burst.memory_limit", "burst RGBA retention budget exhausted before readback", {"retained_bytes": _retained_bytes, "next_size": [size.x, size.y], "max_bytes": _params.get("max_bytes")})
		return
	var image_bytes := size.x * size.y * 4
	var guards := _check_guards()
	if not bool(guards.get("ok", false)):
		_fail("screenshot_state_guard_failed", "burst state guard failed", guards)
		return
	var image := texture.get_image()
	if image == null or image.is_empty():
		_fail("screenshot_burst.viewport_unavailable", "root viewport returned no pixels", {})
		return
	if image.get_size() != size:
		_fail("screenshot_burst.viewport_resized", "root viewport dimensions changed during readback", {})
		return
	image.convert(Image.FORMAT_RGBA8)
	_size = size
	_window_size = window_size
	_retained_bytes += image_bytes
	var record := {
		"index": int(schedule[sequence_index]),
		"image": image,
		"engine_frame": Engine.get_process_frames(),
		"draw_ordinal": _draw_ordinal,
		"time_msec": Time.get_ticks_msec(),
		"elapsed_msec": float(Time.get_ticks_usec() - _started_usec) / 1000.0,
		"elapsed_usec": Time.get_ticks_usec() - _started_usec,
		"requested_time_usec": int(ceil(due * 1000000.0)) if time_mode else null,
		"requested_time_msec": due * 1000.0 if time_mode else null,
		"capture_source": "godot_viewport",
		"draw_forced": false,
		"state_guards": guards,
	}
	_pending.append(record)
	if float(Time.get_ticks_usec() - _started_usec) / 1000000.0 >= float(_params.get("max_seconds", 3.0)):
		_fail("screenshot_burst.capture_deadline", "burst capture deadline exhausted during guard evaluation or pixel readback", {"elapsed_usec": record["elapsed_usec"]})
		return
	var predicate = _params.get("stop_after_state")
	if predicate is Dictionary:
		_stop_samples += 1
		var check: Dictionary = _assertions._assert_state_matches(predicate)
		if float(Time.get_ticks_usec() - _started_usec) / 1000000.0 >= float(_params.get("max_seconds", 3.0)):
			_fail("screenshot_burst.capture_deadline", "burst capture deadline exhausted during stop predicate evaluation", {})
			return
		if bool(check.get("ok", false)):
			_stop_matched = true
			_stop_at = {"index": record["index"], "sequence_index": sequence_index}
			_stop_reason = "stop_condition_matched"
			_done = true
			return
		_stop_last_error = check.get("error", {})
		if str((_stop_last_error as Dictionary).get("type", "")) != "state_matches_mismatch":
			_fail("screenshot_burst.stop_condition_failed", "burst stop predicate could not be evaluated", check)
			return
	if _pending.size() == schedule.size():
		_done = true


func _check_guards() -> Dictionary:
	var guards: Dictionary = _params.get("state_guards", {})
	var checks: Array = []
	for field in ["require_visible", "require_hidden"]:
		for target in guards.get(field, []):
			if str(target).strip_edges().is_empty():
				continue
			var check: Dictionary = _assertions._assert_visible(str(target).strip_edges()) if field == "require_visible" else _assertions._assert_hidden(str(target).strip_edges())
			checks.append({"type": field, "target": target, "result": check})
			if not bool(check.get("ok", false)):
				return {"ok": false, "requirements": guards, "requirements_identity": _guard_identity, "checks": checks}
	var state = guards.get("require_state")
	if state is Dictionary:
		var check: Dictionary = _assertions._assert_state_matches(state)
		checks.append({"type": "require_state", "result": check})
		if not bool(check.get("ok", false)):
			return {"ok": false, "requirements": guards, "requirements_identity": _guard_identity, "checks": checks}
	return {"ok": true, "requirements": guards, "requirements_identity": _guard_identity, "checks": checks}


func _fail(kind: String, message: String, details: Dictionary) -> void:
	_failures.append({"type": kind, "message": message, "command": "screenshot sequence", "actual": details})
	_stop_reason = kind
	_done = true
