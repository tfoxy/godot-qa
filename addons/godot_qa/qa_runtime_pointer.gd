extends RefCounted

const QaSnapshot = preload("res://addons/godot_qa/qa_snapshot.gd")

var _runtime: WeakRef


func _init(runtime: RefCounted) -> void:
	_runtime = weakref(runtime)


func _click_target(target: String) -> Dictionary:
	var resolution: Dictionary = _runtime.get_ref().resolve_target(target)
	if not bool(resolution.get("ok", false)):
		return {
			"ok": false,
			"error": (resolution.get("error", {}) as Dictionary).duplicate(true),
		}

	var target_node = resolution.get("node")
	if not target_node is Control:
		return _runtime.get_ref()._error(
			"target_not_clickable",
			"Resolved target is not a clickable Control",
			{
				"target": target,
				"resolved": resolution.get("resolved", ""),
				"actual": _runtime.get_ref().describe_node(target_node),
			},
		)

	var control := target_node as Control
	if control.get_viewport() != _runtime.get_ref()._root_window:
		return _runtime.get_ref()._error(
			"target_not_clickable",
			"Locator clicks require a Control in the active root window viewport",
			{
				"target": target,
				"resolved": resolution.get("resolved", ""),
				"expected": str(_runtime.get_ref()._root_window.get_path()),
				"actual": str(control.get_viewport().get_path()),
			},
		)
	var rect := control.get_global_rect()
	if not control.is_visible_in_tree() or rect.size.x <= 0.0 or rect.size.y <= 0.0:
		return _runtime.get_ref()._error(
			"target_not_clickable",
			"Resolved target is not a visible Control with positive size",
			{
				"target": target,
				"resolved": resolution.get("resolved", ""),
				"actual": {
					"visible": control.is_visible_in_tree(),
					"global_rect": QaSnapshot.control_to_dict(control).get("global_rect", []),
				},
			},
		)

	# Input.parse_input_event enters through the window, before Godot applies
	# its inverse stretch transform. Include the canvas and viewport transforms
	# (including letterboxing), but exclude the desktop window position.
	var center := control.get_viewport_transform() * control.get_global_transform() * (control.size / 2.0)
	await _emit_left_click(center)
	return _runtime.get_ref()._ok(
		{
			"target": target,
			"resolved": resolution.get("resolved", ""),
		}
	)


func _mouse_click(raw_x: Variant, raw_y: Variant) -> Dictionary:
	var x: Variant = _runtime.get_ref()._coerce_coordinate(raw_x)
	var y: Variant = _runtime.get_ref()._coerce_coordinate(raw_y)
	if x == null or y == null:
		return _runtime.get_ref()._error(
			"invalid_params",
			"input.mouse_click requires integer viewport coordinates",
			{
				"command": "input.mouse_click",
				"expected": "integer --x and --y viewport coordinates",
				"actual": {
					"x": raw_x,
					"y": raw_y,
				},
			},
		)
	var position := Vector2(float(x), float(y))
	var visible_rect: Rect2 = _runtime.get_ref().viewport_rect()
	if visible_rect.size.x <= 0.0 or visible_rect.size.y <= 0.0 or not visible_rect.has_point(position):
		return _runtime.get_ref()._error(
			"invalid_params",
			"input.mouse_click requires coordinates inside the visible viewport",
			{
				"command": "input.mouse_click",
				"expected": _runtime.get_ref()._rect_to_array(visible_rect),
				"actual": {
					"x": x,
					"y": y,
				},
			},
		)

	var hovered := _hovered_control_at_position(position)
	await _emit_left_click(position)
	var data := {
		"device": "mouse",
		"button": "left",
		"position": {
			"x": x,
			"y": y,
		},
	}
	if hovered != null:
		data["target"] = _runtime.get_ref().stable_locator_for_node(hovered)
		data["resolved"] = "path:%s" % str(hovered.get_path())
	return _runtime.get_ref()._ok(data)


func _mouse_wheel(params: Dictionary) -> Dictionary:
	var raw_x: Variant = params.get("x", null)
	var raw_y: Variant = params.get("y", null)
	var x: Variant = _runtime.get_ref()._coerce_number_coordinate(raw_x)
	var y: Variant = _runtime.get_ref()._coerce_number_coordinate(raw_y)
	if x == null or y == null:
		return _runtime.get_ref()._error(
			"invalid_params",
			"input.mouse_wheel requires numeric viewport coordinates",
			{
				"command": "input.mouse_wheel",
				"expected": "numeric --x and --y viewport coordinates",
				"actual": {
					"x": raw_x,
					"y": raw_y,
				},
			},
		)
	var dy := float(params.get("dy", 0.0))
	var dx := float(params.get("dx", 0.0))
	if is_zero_approx(dy) and is_zero_approx(dx):
		return _runtime.get_ref()._error(
			"invalid_params",
			"input.mouse_wheel requires a non-zero --dy or --dx delta",
			{
				"command": "input.mouse_wheel",
				"expected": "non-zero scroll delta on --dy or --dx",
				"actual": {
					"dy": dy,
					"dx": dx,
				},
			},
		)
	var position := Vector2(float(x), float(y))
	var visible_rect: Rect2 = _runtime.get_ref().viewport_rect()
	if visible_rect.size.x <= 0.0 or visible_rect.size.y <= 0.0 or not visible_rect.has_point(position):
		return _runtime.get_ref()._error(
			"invalid_params",
			"input.mouse_wheel requires coordinates inside the visible viewport",
			{
				"command": "input.mouse_wheel",
				"expected": _runtime.get_ref()._rect_to_array(visible_rect),
				"actual": {
					"x": x,
					"y": y,
				},
			},
		)

	var buttons: Array[String] = []
	if not is_zero_approx(dy):
		var vertical_down := dy > 0.0
		var vertical_button := MOUSE_BUTTON_WHEEL_DOWN if vertical_down else MOUSE_BUTTON_WHEEL_UP
		var vertical_name := "wheel_down" if vertical_down else "wheel_up"
		var vertical_pairs := int(max(1, ceil(absf(dy))))
		var vertical_factor := absf(dy) / float(vertical_pairs)
		for _i in range(vertical_pairs):
			_emit_mouse_wheel_event(position, vertical_button, vertical_factor, true)
			_emit_mouse_wheel_event(position, vertical_button, vertical_factor, false)
			await _runtime.get_ref()._process_frames(1)
			buttons.append(vertical_name)
	if not is_zero_approx(dx):
		var horizontal_right := dx > 0.0
		var horizontal_button := MOUSE_BUTTON_WHEEL_RIGHT if horizontal_right else MOUSE_BUTTON_WHEEL_LEFT
		var horizontal_name := "wheel_right" if horizontal_right else "wheel_left"
		var horizontal_pairs := int(max(1, ceil(absf(dx))))
		var horizontal_factor := absf(dx) / float(horizontal_pairs)
		for _i in range(horizontal_pairs):
			_emit_mouse_wheel_event(position, horizontal_button, horizontal_factor, true)
			_emit_mouse_wheel_event(position, horizontal_button, horizontal_factor, false)
			await _runtime.get_ref()._process_frames(1)
			buttons.append(horizontal_name)

	var data := {
		"device": "mouse",
		"position": {
			"x": x,
			"y": y,
		},
		"dx": dx,
		"dy": dy,
		"buttons": buttons,
	}
	return _runtime.get_ref()._ok(data)


func _touch_down(params: Dictionary) -> Dictionary:
	var raw_x: Variant = params.get("x", null)
	var raw_y: Variant = params.get("y", null)
	var x: Variant = _runtime.get_ref()._coerce_number_coordinate(raw_x)
	var y: Variant = _runtime.get_ref()._coerce_number_coordinate(raw_y)
	if x == null or y == null:
		return _runtime.get_ref()._error(
			"invalid_params",
			"input.touch_down requires numeric viewport coordinates",
			{
				"command": "input.touch_down",
				"expected": "numeric --x and --y viewport coordinates",
				"actual": {
					"x": raw_x,
					"y": raw_y,
				},
			},
		)
	var position := Vector2(float(x), float(y))
	var visible_rect: Rect2 = _runtime.get_ref().viewport_rect()
	if visible_rect.size.x <= 0.0 or visible_rect.size.y <= 0.0 or not visible_rect.has_point(position):
		return _runtime.get_ref()._error(
			"invalid_params",
			"input.touch_down requires coordinates inside the visible viewport",
			{
				"command": "input.touch_down",
				"expected": _runtime.get_ref()._rect_to_array(visible_rect),
				"actual": {
					"x": x,
					"y": y,
				},
			},
		)
	var index: Dictionary = _runtime.get_ref()._non_negative_int_param(params, "index", 0, "input.touch_down")
	if not bool(index.get("ok", false)):
		return index
	var finger := int(index.get("value", 0))
	if _runtime.get_ref()._touched_screen_points.has(finger):
		return _runtime.get_ref()._error(
			"touch_index_held",
			"input.touch_down index %d is already held" % finger,
			{
				"command": "input.touch_down",
				"index": finger,
			},
		)

	_note_touch_emulation()
	_emit_screen_touch_event(finger, position, true)
	await _runtime.get_ref()._process_frames(1)
	return _runtime.get_ref()._ok({
		"device": "touch",
		"index": finger,
		"position": {
			"x": x,
			"y": y,
		},
		"pressed": true,
	})


func _touch_move(params: Dictionary) -> Dictionary:
	var raw_x: Variant = params.get("x", null)
	var raw_y: Variant = params.get("y", null)
	var x: Variant = _runtime.get_ref()._coerce_number_coordinate(raw_x)
	var y: Variant = _runtime.get_ref()._coerce_number_coordinate(raw_y)
	if x == null or y == null:
		return _runtime.get_ref()._error(
			"invalid_params",
			"input.touch_move requires numeric viewport coordinates",
			{
				"command": "input.touch_move",
				"expected": "numeric --x and --y viewport coordinates",
				"actual": {
					"x": raw_x,
					"y": raw_y,
				},
			},
		)
	var position := Vector2(float(x), float(y))
	var visible_rect: Rect2 = _runtime.get_ref().viewport_rect()
	if visible_rect.size.x <= 0.0 or visible_rect.size.y <= 0.0 or not visible_rect.has_point(position):
		return _runtime.get_ref()._error(
			"invalid_params",
			"input.touch_move requires coordinates inside the visible viewport",
			{
				"command": "input.touch_move",
				"expected": _runtime.get_ref()._rect_to_array(visible_rect),
				"actual": {
					"x": x,
					"y": y,
				},
			},
		)
	var index: Dictionary = _runtime.get_ref()._non_negative_int_param(params, "index", 0, "input.touch_move")
	if not bool(index.get("ok", false)):
		return index
	var frames_param: Dictionary = _runtime.get_ref()._non_negative_int_param(params, "frames", 1, "input.touch_move")
	if not bool(frames_param.get("ok", false)):
		return frames_param
	var frames := int(frames_param.get("value", 1))
	if frames > _runtime.get_ref().MAX_INPUT_GESTURE_FRAMES:
		return _runtime.get_ref()._error(
			"invalid_params",
			"input.touch_move frames must not exceed %d" % _runtime.get_ref().MAX_INPUT_GESTURE_FRAMES,
			{
				"command": "input.touch_move",
				"field": "frames",
				"expected": "frames <= %d" % _runtime.get_ref().MAX_INPUT_GESTURE_FRAMES,
				"actual": frames,
			},
		)
	var finger := int(index.get("value", 0))
	if not _runtime.get_ref()._touched_screen_points.has(finger):
		return _runtime.get_ref()._error(
			"touch_index_not_held",
			"input.touch_move index %d is not held" % finger,
			{
				"command": "input.touch_move",
				"index": finger,
			},
		)

	_note_touch_emulation()
	var last_position: Vector2 = (_runtime.get_ref()._touched_screen_points[finger] as Dictionary).get("position", position)
	var steps := maxi(frames, 1)
	var prev_position := last_position
	var final_relative := Vector2.ZERO
	for i in range(1, steps + 1):
		var target_position := last_position.lerp(position, float(i) / float(steps))
		if i == steps:
			target_position = position
		var relative := target_position - prev_position
		var velocity := relative / _effective_frame_delta()
		_emit_screen_drag_event(finger, target_position, relative, velocity)
		final_relative = relative
		prev_position = target_position
		await _runtime.get_ref()._process_frames(1)
	return _runtime.get_ref()._ok({
		"device": "touch",
		"index": finger,
		"position": {
			"x": x,
			"y": y,
		},
		"relative": {
			"x": final_relative.x,
			"y": final_relative.y,
		},
		"frames": frames,
	})


func _touch_up(params: Dictionary) -> Dictionary:
	var index: Dictionary = _runtime.get_ref()._non_negative_int_param(params, "index", 0, "input.touch_up")
	if not bool(index.get("ok", false)):
		return index
	var finger := int(index.get("value", 0))
	if not _runtime.get_ref()._touched_screen_points.has(finger):
		return _runtime.get_ref()._error(
			"touch_index_not_held",
			"input.touch_up index %d is not held" % finger,
			{
				"command": "input.touch_up",
				"index": finger,
			},
		)

	var position: Vector2 = (_runtime.get_ref()._touched_screen_points[finger] as Dictionary).get("position", Vector2.ZERO)
	var raw_x: Variant = params.get("x", null)
	var raw_y: Variant = params.get("y", null)
	if raw_x != null or raw_y != null:
		var x: Variant = _runtime.get_ref()._coerce_number_coordinate(raw_x)
		var y: Variant = _runtime.get_ref()._coerce_number_coordinate(raw_y)
		if x == null or y == null:
			return _runtime.get_ref()._error(
				"invalid_params",
				"input.touch_up requires numeric viewport coordinates",
				{
					"command": "input.touch_up",
					"expected": "numeric --x and --y viewport coordinates",
					"actual": {
						"x": raw_x,
						"y": raw_y,
					},
				},
			)
		position = Vector2(float(x), float(y))
		var visible_rect: Rect2 = _runtime.get_ref().viewport_rect()
		if visible_rect.size.x <= 0.0 or visible_rect.size.y <= 0.0 or not visible_rect.has_point(position):
			return _runtime.get_ref()._error(
				"invalid_params",
				"input.touch_up requires coordinates inside the visible viewport",
				{
					"command": "input.touch_up",
					"expected": _runtime.get_ref()._rect_to_array(visible_rect),
					"actual": {
						"x": x,
						"y": y,
					},
				},
			)

	_note_touch_emulation()
	_emit_screen_touch_event(finger, position, false)
	await _runtime.get_ref()._process_frames(1)
	return _runtime.get_ref()._ok({
		"device": "touch",
		"index": finger,
		"position": {
			"x": position.x,
			"y": position.y,
		},
		"pressed": false,
	})


func _touch_tap(params: Dictionary) -> Dictionary:
	var raw_x: Variant = params.get("x", null)
	var raw_y: Variant = params.get("y", null)
	var x: Variant = _runtime.get_ref()._coerce_number_coordinate(raw_x)
	var y: Variant = _runtime.get_ref()._coerce_number_coordinate(raw_y)
	if x == null or y == null:
		return _runtime.get_ref()._error(
			"invalid_params",
			"input.touch_tap requires numeric viewport coordinates",
			{
				"command": "input.touch_tap",
				"expected": "numeric --x and --y viewport coordinates",
				"actual": {
					"x": raw_x,
					"y": raw_y,
				},
			},
		)
	var position := Vector2(float(x), float(y))
	var visible_rect: Rect2 = _runtime.get_ref().viewport_rect()
	if visible_rect.size.x <= 0.0 or visible_rect.size.y <= 0.0 or not visible_rect.has_point(position):
		return _runtime.get_ref()._error(
			"invalid_params",
			"input.touch_tap requires coordinates inside the visible viewport",
			{
				"command": "input.touch_tap",
				"expected": _runtime.get_ref()._rect_to_array(visible_rect),
				"actual": {
					"x": x,
					"y": y,
				},
			},
		)
	var index: Dictionary = _runtime.get_ref()._non_negative_int_param(params, "index", 0, "input.touch_tap")
	if not bool(index.get("ok", false)):
		return index
	var finger := int(index.get("value", 0))
	if _runtime.get_ref()._touched_screen_points.has(finger):
		return _runtime.get_ref()._error(
			"touch_index_held",
			"input.touch_tap index %d is already held" % finger,
			{
				"command": "input.touch_tap",
				"index": finger,
			},
		)

	_note_touch_emulation()
	var hovered := _hovered_control_at_position(position)
	_emit_screen_touch_event(finger, position, true)
	await _runtime.get_ref()._process_frames(1)
	_emit_screen_touch_event(finger, position, false)
	await _runtime.get_ref()._process_frames(1)
	var data := {
		"device": "touch",
		"index": finger,
		"position": {
			"x": x,
			"y": y,
		},
	}
	if hovered != null:
		data["target"] = _runtime.get_ref().stable_locator_for_node(hovered)
		data["resolved"] = "path:%s" % str(hovered.get_path())
	return _runtime.get_ref()._ok(data)


func _drag(params: Dictionary) -> Dictionary:
	var raw_from_x: Variant = params.get("from_x", null)
	var raw_from_y: Variant = params.get("from_y", null)
	var raw_to_x: Variant = params.get("to_x", null)
	var raw_to_y: Variant = params.get("to_y", null)
	var from_x: Variant = _runtime.get_ref()._coerce_number_coordinate(raw_from_x)
	var from_y: Variant = _runtime.get_ref()._coerce_number_coordinate(raw_from_y)
	var to_x: Variant = _runtime.get_ref()._coerce_number_coordinate(raw_to_x)
	var to_y: Variant = _runtime.get_ref()._coerce_number_coordinate(raw_to_y)
	if from_x == null or from_y == null or to_x == null or to_y == null:
		return _runtime.get_ref()._error(
			"invalid_params",
			"input.drag requires numeric viewport coordinates",
			{
				"command": "input.drag",
				"expected": "numeric --from-x/--from-y/--to-x/--to-y viewport coordinates",
				"actual": {
					"from_x": raw_from_x,
					"from_y": raw_from_y,
					"to_x": raw_to_x,
					"to_y": raw_to_y,
				},
			},
		)
	var from_position := Vector2(float(from_x), float(from_y))
	var to_position := Vector2(float(to_x), float(to_y))
	var visible_rect: Rect2 = _runtime.get_ref().viewport_rect()
	if visible_rect.size.x <= 0.0 or visible_rect.size.y <= 0.0 or not visible_rect.has_point(from_position):
		return _runtime.get_ref()._error(
			"invalid_params",
			"input.drag requires coordinates inside the visible viewport",
			{
				"command": "input.drag",
				"expected": _runtime.get_ref()._rect_to_array(visible_rect),
				"actual": {
					"endpoint": "from",
					"x": from_x,
					"y": from_y,
				},
			},
		)
	if not visible_rect.has_point(to_position):
		return _runtime.get_ref()._error(
			"invalid_params",
			"input.drag requires coordinates inside the visible viewport",
			{
				"command": "input.drag",
				"expected": _runtime.get_ref()._rect_to_array(visible_rect),
				"actual": {
					"endpoint": "to",
					"x": to_x,
					"y": to_y,
				},
			},
		)
	var frames_param: Dictionary = _runtime.get_ref()._non_negative_int_param(params, "frames", 8, "input.drag")
	if not bool(frames_param.get("ok", false)):
		return frames_param
	var frames := int(frames_param.get("value", 8))
	if frames < 1:
		return _runtime.get_ref()._error(
			"invalid_params",
			"input.drag frames must be at least 1",
			{
				"command": "input.drag",
				"field": "frames",
				"expected": "frames >= 1",
				"actual": frames,
			},
		)
	if frames > _runtime.get_ref().MAX_INPUT_GESTURE_FRAMES:
		return _runtime.get_ref()._error(
			"invalid_params",
			"input.drag frames must not exceed %d" % _runtime.get_ref().MAX_INPUT_GESTURE_FRAMES,
			{
				"command": "input.drag",
				"field": "frames",
				"expected": "frames <= %d" % _runtime.get_ref().MAX_INPUT_GESTURE_FRAMES,
				"actual": frames,
			},
		)
	var index_param: Dictionary = _runtime.get_ref()._non_negative_int_param(params, "index", 0, "input.drag")
	if not bool(index_param.get("ok", false)):
		return index_param
	var finger := int(index_param.get("value", 0))
	var pointer := str(params.get("pointer", "touch"))
	if pointer != "touch" and pointer != "mouse":
		return _runtime.get_ref()._error(
			"invalid_params",
			"input.drag pointer must be touch or mouse",
			{
				"command": "input.drag",
				"field": "pointer",
				"expected": ["touch", "mouse"],
				"actual": pointer,
			},
		)
	var steps := maxi(frames, 1)

	if pointer == "touch":
		if _runtime.get_ref()._touched_screen_points.has(finger):
			return _runtime.get_ref()._error(
				"touch_index_held",
				"input.drag index %d is already held" % finger,
				{
					"command": "input.drag",
					"index": finger,
				},
			)
		_note_touch_emulation()
		_emit_screen_touch_event(finger, from_position, true)
		await _runtime.get_ref()._process_frames(1)
		var prev_position := from_position
		for i in range(1, steps + 1):
			var target_position := from_position.lerp(to_position, float(i) / float(steps))
			if i == steps:
				target_position = to_position
			var relative := target_position - prev_position
			var velocity := relative / _effective_frame_delta()
			_emit_screen_drag_event(finger, target_position, relative, velocity)
			prev_position = target_position
			await _runtime.get_ref()._process_frames(1)
		_emit_screen_touch_event(finger, to_position, false)
		await _runtime.get_ref()._process_frames(1)
		return _runtime.get_ref()._ok({
			"device": "touch",
			"pointer": pointer,
			"index": finger,
			"from": {
				"x": from_x,
				"y": from_y,
			},
			"to": {
				"x": to_x,
				"y": to_y,
			},
			"frames": frames,
			"steps": steps,
		})

	# pointer == "mouse": drive Control drag detection with a left-button-masked
	# motion; the plain _emit_mouse_motion_event carries no mask.
	_emit_mouse_motion_event(from_position)
	await _runtime.get_ref()._process_frames(1)
	_emit_mouse_button_event(from_position, true)
	await _runtime.get_ref()._process_frames(1)
	var mouse_prev := from_position
	for j in range(1, steps + 1):
		var mouse_target := from_position.lerp(to_position, float(j) / float(steps))
		if j == steps:
			mouse_target = to_position
		var mouse_relative := mouse_target - mouse_prev
		var mouse_velocity := mouse_relative / _effective_frame_delta()
		_emit_mouse_drag_motion_event(mouse_target, mouse_relative, mouse_velocity)
		mouse_prev = mouse_target
		await _runtime.get_ref()._process_frames(1)
	_emit_mouse_button_event(to_position, false)
	await _runtime.get_ref()._process_frames(1)
	return _runtime.get_ref()._ok({
		"device": "mouse",
		"pointer": pointer,
		"from": {
			"x": from_x,
			"y": from_y,
		},
		"to": {
			"x": to_x,
			"y": to_y,
		},
		"frames": frames,
		"steps": steps,
	})


func _emit_mouse_button_event(position: Vector2, pressed: bool) -> void:
	var event := InputEventMouseButton.new()
	event.button_index = MOUSE_BUTTON_LEFT
	event.pressed = pressed
	event.position = position
	event.global_position = position
	event.button_mask = MOUSE_BUTTON_MASK_LEFT if pressed else 0
	Input.parse_input_event(event)


func _emit_mouse_motion_event(position: Vector2) -> void:
	var event := InputEventMouseMotion.new()
	event.position = position
	event.global_position = position
	Input.parse_input_event(event)


# Mouse-drag motion carrying the left-button mask so Control drag detection fires;
# the plain _emit_mouse_motion_event sets no mask. Used by the input.drag mouse path.
func _emit_mouse_drag_motion_event(position: Vector2, relative: Vector2, velocity: Vector2) -> void:
	var event := InputEventMouseMotion.new()
	event.position = position
	event.global_position = position
	event.relative = relative
	event.velocity = velocity
	event.button_mask = MOUSE_BUTTON_MASK_LEFT
	Input.parse_input_event(event)


func _emit_mouse_wheel_event(position: Vector2, button_index: int, factor: float, pressed: bool) -> void:
	var event := InputEventMouseButton.new()
	event.button_index = button_index
	event.pressed = pressed
	event.factor = factor
	event.position = position
	event.global_position = position
	event.button_mask = 0
	Input.parse_input_event(event)


func _emit_screen_touch_event(index: int, position: Vector2, pressed: bool) -> void:
	var event := InputEventScreenTouch.new()
	event.index = index
	event.position = position
	event.pressed = pressed
	Input.parse_input_event(event)
	if pressed:
		_runtime.get_ref()._touched_screen_points[index] = {"position": position}
	else:
		_runtime.get_ref()._touched_screen_points.erase(index)


func _emit_screen_drag_event(index: int, position: Vector2, relative: Vector2, velocity: Vector2) -> void:
	var event := InputEventScreenDrag.new()
	event.index = index
	event.position = position
	event.relative = relative
	event.velocity = velocity
	Input.parse_input_event(event)
	_runtime.get_ref()._touched_screen_points[index] = {"position": position}


# Frame delta used to derive InputEventScreenDrag.velocity. Uses the live root
# window's process delta when available (deterministic under --fixed-fps), else a
# 1/60 fallback so velocity is finite even before the first frame is measured.
func _effective_frame_delta() -> float:
	if _runtime.get_ref()._root_window != null:
		var delta: float = _runtime.get_ref()._root_window.get_process_delta_time()
		if delta > 0.0:
			return delta
	return 1.0 / 60.0


# One-time informational note: while emulate_mouse_from_touch (Godot default on)
# is enabled, injected touch also synthesizes mouse events. Never mutates the
# setting; logs once per process through the addon's existing print channel.
func _note_touch_emulation() -> void:
	if _runtime.get_ref()._touch_emulation_note_emitted:
		return
	_runtime.get_ref()._touch_emulation_note_emitted = true
	if bool(ProjectSettings.get_setting("input_devices/pointing/emulate_mouse_from_touch", true)):
		print("[godot-qa] injected touch also synthesizes mouse events (input_devices/pointing/emulate_mouse_from_touch is enabled)")


func _emit_left_click(position: Vector2) -> void:
	_emit_mouse_motion_event(position)
	await _runtime.get_ref()._process_frames(1)
	_emit_mouse_button_event(position, true)
	await _runtime.get_ref()._process_frames(1)
	_emit_mouse_button_event(position, false)
	await _runtime.get_ref()._process_frames(1)


func _hovered_control_at_position(position: Vector2) -> Control:
	var visible_controls: Array[Control] = QaSnapshot.visible_controls(_runtime.get_ref()._scene_root)
	var best_control: Control = null
	var best_area := INF
	var best_depth := -1
	for control in visible_controls:
		if control == null:
			continue
		var rect := control.get_global_rect()
		if not rect.has_point(position):
			continue
		var area := rect.size.x * rect.size.y
		var depth := str(control.get_path()).count("/")
		if best_control == null or area < best_area or (is_equal_approx(area, best_area) and depth > best_depth):
			best_control = control
			best_area = area
			best_depth = depth
	if best_control != null:
		return best_control
	if _runtime.get_ref()._root_window != null and _runtime.get_ref()._root_window.has_method("gui_get_hovered_control"):
		var hovered = _runtime.get_ref()._root_window.gui_get_hovered_control()
		if hovered is Control:
			return hovered as Control
	return best_control
