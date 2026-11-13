extends RefCounted

var _runtime: WeakRef


func _init(runtime: RefCounted) -> void:
	_runtime = weakref(runtime)


func release_held_actions() -> Array[Dictionary]:
	var failures: Array[Dictionary] = []
	var held_actions: Array = _runtime.get_ref()._held_actions.keys()
	for action_name in held_actions:
		var result: Dictionary = await _runtime.get_ref().execute(
			"input.release_action",
			{
				"action": str(action_name),
			},
		)
		if not bool(result.get("ok", false)):
			failures.append((result.get("error", {}) as Dictionary).duplicate(true))
	return failures


func release_held_keys() -> Array[Dictionary]:
	var failures: Array[Dictionary] = []
	for key_name in _runtime.get_ref()._held_keys.keys():
		var result := await _release_key(str(key_name))
		if not bool(result.get("ok", false)):
			failures.append((result.get("error", {}) as Dictionary).duplicate(true))
	return failures


# Zero every non-zero joypad axis injected during this process so a held
# trigger/stick cannot leak into the next warm scenario. Joypad axis state is
# process-global, so this clears the shared tracking set across runtime
# instances. Safe (and a no-op for untouched axes) on the cold one-shot path.
func release_held_axes() -> Array[Dictionary]:
	var axes: Array = _runtime.get_ref()._touched_joy_axes.values()
	for entry in axes:
		if not entry is Dictionary:
			continue
		_emit_joy_motion_event(
			int((entry as Dictionary).get("device", 0)),
			int((entry as Dictionary).get("axis_index", -1)),
			0.0,
		)
	_runtime.get_ref()._touched_joy_axes.clear()
	if not axes.is_empty():
		await _runtime.get_ref()._process_frames(1)
	return []


# Release every joypad button still pressed by injected events (a
# policy.drive_tick hold that never released, or an aborted step). Same
# process-global cleanup contract as release_held_axes.
func release_held_buttons() -> Array[Dictionary]:
	var buttons: Array = _runtime.get_ref()._touched_joy_buttons.values()
	for entry in buttons:
		if not entry is Dictionary:
			continue
		_emit_joy_button_event(
			int((entry as Dictionary).get("device", 0)),
			int((entry as Dictionary).get("button_index", -1)),
			false,
		)
	_runtime.get_ref()._touched_joy_buttons.clear()
	if not buttons.is_empty():
		await _runtime.get_ref()._process_frames(1)
	return []


# A virtual disconnect must release only the events injected for those
# devices. Real controller state and other QA pads stay untouched.
func _release_injected_joypad_devices(devices: Array[int]) -> void:
	var released := false
	for key in _runtime.get_ref()._touched_joy_axes.keys():
		var entry: Dictionary = _runtime.get_ref()._touched_joy_axes[key]
		if int(entry.get("device", -1)) in devices:
			_emit_joy_motion_event(int(entry["device"]), int(entry["axis_index"]), 0.0)
			released = true
	for key in _runtime.get_ref()._touched_joy_buttons.keys():
		var entry: Dictionary = _runtime.get_ref()._touched_joy_buttons[key]
		if int(entry.get("device", -1)) in devices:
			_emit_joy_button_event(int(entry["device"]), int(entry["button_index"]), false)
			released = true
	if released:
		await _runtime.get_ref()._process_frames(1)


# Lift every screen touch still held down by injected events (a peek/drag that
# never lifted, or an aborted step) at its last tracked position. A normal
# pressed=false release, not a cancel. Same process-global cleanup contract as
# release_held_buttons.
func release_held_touches() -> Array[Dictionary]:
	var indices: Array = _runtime.get_ref()._touched_screen_points.keys()
	indices.sort()
	for index in indices:
		var last_position: Vector2 = (_runtime.get_ref()._touched_screen_points[index] as Dictionary).get("position", Vector2.ZERO)
		_runtime.get_ref()._pointer._emit_screen_touch_event(int(index), last_position, false)
	_runtime.get_ref()._touched_screen_points.clear()
	if not indices.is_empty():
		await _runtime.get_ref()._process_frames(1)
	return []


# Fire a DISCONNECT joy_connection_changed signal for every joypad announced via
# setup.joy_connect and clear the tracking set, so a QA-announced pad cannot leak
# into the next warm scenario sharing this process — a long-lived game listener
# gets a clean disconnect for each fake, and the static tracking set (which drives
# joy_disconnect's real-pad guard) starts empty next scenario. Same process-global
# cleanup contract as release_held_axes/release_held_buttons; a no-op when nothing
# was faked.
func release_faked_joypads() -> Array[Dictionary]:
	var devices: Array = _runtime.get_ref()._faked_joypads.keys()
	var device_ids: Array[int] = []
	for device in devices:
		if int(device) not in Input.get_connected_joypads():
			device_ids.append(int(device))
	await _release_injected_joypad_devices(device_ids)
	for device in devices:
		_runtime.get_ref()._faked_joypads.erase(device)
		if int(device) in device_ids:
			Input.joy_connection_changed.emit(int(device), false)
	if not device_ids.is_empty():
		await _runtime.get_ref()._process_frames(1)
	return []


# Full input reset: release all held actions and keys, zero all touched joypad axes,
# release all pressed joypad buttons, lift all held screen touches, and
# disconnect all QA-faked joypads. Returns any release failures. Exposed as the
# input.reset bridge command and called from scenario finalize so each scenario
# self-cleans its injected input and faked pad connections.
func reset_input() -> Array[Dictionary]:
	var failures: Array[Dictionary] = await release_held_actions()
	failures.append_array(await release_held_keys())
	failures.append_array(await release_held_axes())
	failures.append_array(await release_held_buttons())
	failures.append_array(await release_held_touches())
	failures.append_array(await release_faked_joypads())
	return failures


func _tap_action(action: String) -> Dictionary:
	if not InputMap.has_action(action):
		return _runtime.get_ref()._action_missing_error(action)

	_emit_action_event(action, true)
	await _runtime.get_ref()._process_frames(1)
	_emit_action_event(action, false)
	await _runtime.get_ref()._process_frames(1)
	return _runtime.get_ref()._ok({"action": action})


func _hold_action(action: String, frames: Variant) -> Dictionary:
	if not InputMap.has_action(action):
		return _runtime.get_ref()._action_missing_error(action)
	var frame_count = _runtime.get_ref()._coerce_frame_count(frames)
	if frame_count == null:
		return _runtime.get_ref()._error(
			"invalid_params",
			"input.hold_action requires a non-negative integer frame count",
			{
				"command": "input.hold_action",
				"expected": "non-negative integer frames",
				"actual": frames,
			},
		)

	_emit_action_event(action, true)
	_runtime.get_ref()._held_actions[action] = true
	await _runtime.get_ref()._process_frames(frame_count)
	return _runtime.get_ref()._ok(
		{
			"action": action,
			"frames": frame_count,
			"held": true,
		}
	)


func _release_action(action: String) -> Dictionary:
	if not _runtime.get_ref()._held_actions.has(action):
		return _runtime.get_ref()._error(
			"action_not_held",
			"input.release_action requires a currently held action",
			{
				"action": action,
				"expected": "held action tracked by the runtime",
				"actual": "not_held",
			},
		)

	_emit_action_event(action, false)
	_runtime.get_ref()._held_actions.erase(action)
	await _runtime.get_ref()._process_frames(1)
	return _runtime.get_ref()._ok(
		{
			"action": action,
			"held": false,
		}
	)


# One drive_policy tick: call the registered (or builtin) policy with a
# read-only request, validate the returned desired-input list, then emit each
# input through the same joypad event paths as input.joy_motion /
# input.joy_button. Never mutates node state on the policy's behalf, and emits
# nothing when the returned shape is malformed. Frame pacing is owned by the
# scenario runner, so this command does not await frames.
func _policy_drive_tick(params: Dictionary) -> Dictionary:
	var policy_id := str(params.get("policy", "")).strip_edges()
	if policy_id == "":
		return _runtime.get_ref()._error(
			"invalid_policy_call",
			"policy.drive_tick requires a non-empty policy",
			{
				"command": "policy.drive_tick",
				"expected": {"policy": "non-empty policy provider id"},
				"actual": params.duplicate(true),
			},
		)
	var devices := _policy_device_allowlist(params.get("devices", null))
	if devices.is_empty():
		return _runtime.get_ref()._error(
			"invalid_policy_call",
			"policy.drive_tick requires a non-empty devices list of non-negative integers",
			{
				"command": "policy.drive_tick",
				"policy": policy_id,
				"expected": "non-empty list of non-negative integers",
				"actual": _runtime.get_ref()._fixtures._encode_setup_value(params.get("devices", null)),
			},
		)
	var policy_params = params.get("params", {})
	if not (policy_params is Dictionary):
		return _runtime.get_ref()._error(
			"invalid_policy_call",
			"policy.drive_tick params must be a dictionary when present",
			{
				"command": "policy.drive_tick",
				"policy": policy_id,
				"expected": "Dictionary",
				"actual": _runtime.get_ref()._fixtures._encode_setup_value(policy_params),
			},
		)
	var request := {
		"policy": policy_id,
		"tick": int(params.get("tick", 0)),
		"frame": int(params.get("frame", 0)),
		"devices": devices.duplicate(),
		"params": (policy_params as Dictionary).duplicate(true),
	}
	var result: Variant
	if policy_id.begins_with("builtin:"):
		var builtin := _call_builtin_policy(policy_id, request)
		if not bool(builtin.get("ok", false)):
			return builtin
		result = builtin.get("result", {})
	else:
		var provider: Dictionary = _runtime.get_ref()._providers._provider_entry(_runtime.get_ref()._policy_providers, policy_id, "policy_provider")
		if not bool(provider.get("ok", false)):
			return provider
		var entry: Dictionary = provider.get("entry", {})
		var owner: Object = entry.get("owner")
		var method_name := str(entry.get("method", ""))
		result = owner.call(method_name, request)
	if not (result is Dictionary):
		return _runtime.get_ref()._error(
			"invalid_policy_result",
			"policy provider must return a dictionary with an 'inputs' list",
			{
				"command": "policy.drive_tick",
				"policy": policy_id,
				"expected": {"inputs": "list of {device, motion|button} objects"},
				"actual": _runtime.get_ref()._fixtures._encode_setup_value(result),
			},
		)
	var raw_inputs = (result as Dictionary).get("inputs", null)
	if not (raw_inputs is Array):
		return _runtime.get_ref()._error(
			"invalid_policy_result",
			"policy result field 'inputs' must be a list",
			{
				"command": "policy.drive_tick",
				"policy": policy_id,
				"expected": "list of {device, motion|button} objects",
				"actual": _runtime.get_ref()._fixtures._encode_setup_value(raw_inputs),
			},
		)
	# Validate the whole list before emitting anything so a malformed policy
	# result injects no input at all.
	var planned: Array[Dictionary] = []
	for index in range((raw_inputs as Array).size()):
		var validated := _validate_policy_input(policy_id, index, (raw_inputs as Array)[index], devices)
		if not bool(validated.get("ok", false)):
			return validated
		planned.append(validated.get("input", {}))
	var motions := 0
	var buttons := 0
	for input in planned:
		if input.has("axis_index"):
			_emit_joy_motion_event(int(input.get("device", 0)), int(input.get("axis_index", -1)), float(input.get("value", 0.0)))
			motions += 1
		else:
			_emit_joy_button_event(int(input.get("device", 0)), int(input.get("button_index", -1)), bool(input.get("pressed", false)))
			buttons += 1
	return _runtime.get_ref()._ok({
		"policy": policy_id,
		"tick": int(request.get("tick", 0)),
		"inputs_applied": planned.size(),
		"motions": motions,
		"buttons": buttons,
	})


# Zero touched joypad axes and release pressed joypad buttons for exactly the
# devices a drive_policy step drove, so a stopped policy leaves no input
# behind while inputs injected for other devices stay untouched.
func _policy_release_inputs(params: Dictionary) -> Dictionary:
	var devices := _policy_device_allowlist(params.get("devices", null))
	if devices.is_empty():
		return _runtime.get_ref()._error(
			"invalid_policy_call",
			"policy.release_inputs requires a non-empty devices list of non-negative integers",
			{
				"command": "policy.release_inputs",
				"expected": "non-empty list of non-negative integers",
				"actual": _runtime.get_ref()._fixtures._encode_setup_value(params.get("devices", null)),
			},
		)
	var released_axes := 0
	for entry in _runtime.get_ref()._touched_joy_axes.values():
		if not entry is Dictionary:
			continue
		var device := int((entry as Dictionary).get("device", -1))
		if not devices.has(device):
			continue
		_emit_joy_motion_event(device, int((entry as Dictionary).get("axis_index", -1)), 0.0)
		released_axes += 1
	var released_buttons := 0
	for entry in _runtime.get_ref()._touched_joy_buttons.values():
		if not entry is Dictionary:
			continue
		var device := int((entry as Dictionary).get("device", -1))
		if not devices.has(device):
			continue
		_emit_joy_button_event(device, int((entry as Dictionary).get("button_index", -1)), false)
		released_buttons += 1
	if released_axes + released_buttons > 0:
		await _runtime.get_ref()._process_frames(1)
	return _runtime.get_ref()._ok({
		"released_axes": released_axes,
		"released_buttons": released_buttons,
	})


func _policy_device_allowlist(raw_devices: Variant) -> Array[int]:
	var devices: Array[int] = []
	if not (raw_devices is Array):
		return devices
	for entry in raw_devices:
		var device := -1
		if entry is int:
			device = int(entry)
		elif entry is float and is_equal_approx(float(entry), roundf(float(entry))):
			device = int(roundf(float(entry)))
		if device < 0:
			return []
		if not devices.has(device):
			devices.append(device)
	return devices


func _validate_policy_input(policy_id: String, index: int, raw_input: Variant, devices: Array[int]) -> Dictionary:
	# drive_policy results carry the policy id in every error and restrict inputs
	# to the step's `devices` allowlist; the shared entry validator handles both.
	return _validate_joy_input_entry(
		index, raw_input, devices, "invalid_policy_result", "policy.drive_tick", "policy input", {"policy": policy_id}
	)


# Validate one `{device, motion|button}` joypad input entry, the exact per-entry
# shape shared by policy.drive_tick results and the open-loop input.joy_state
# batch. `devices` is an allowlist when non-empty (drive_policy); an empty list
# allows any non-negative device (input.joy_state). `error_type`/`command`/`label`
# shape the failure envelope and messages, and `identity` (e.g. {"policy": id})
# is merged into every error's data. On success returns
# {"ok": true, "input": {device, axis|button name, axis_index|button_index, value|pressed}}.
func _validate_joy_input_entry(
	index: int,
	raw_input: Variant,
	devices: Array[int],
	error_type: String,
	command: String,
	label: String,
	identity: Dictionary,
) -> Dictionary:
	var device_expected := "integer from the step's devices list" if not devices.is_empty() else "non-negative integer"
	var expected := {
		"device": device_expected,
		"motion": "{axis, value} with a named axis and value from -1.0 to 1.0",
		"button": "{name, pressed} with a named button and boolean pressed",
	}
	if not (raw_input is Dictionary):
		return _joy_input_error(
			error_type, command, identity,
			"%s %d must be an object" % [label, index],
			expected, _runtime.get_ref()._fixtures._encode_setup_value(raw_input),
		)
	var input: Dictionary = raw_input
	var raw_device = input.get("device", null)
	var device := -1
	if raw_device is int:
		device = int(raw_device)
	elif raw_device is float and is_equal_approx(float(raw_device), roundf(float(raw_device))):
		device = int(roundf(float(raw_device)))
	if device < 0:
		return _joy_input_error(
			error_type, command, identity,
			"%s %d field 'device' must be a non-negative integer" % [label, index],
			expected, _runtime.get_ref()._fixtures._encode_setup_value(raw_device),
		)
	if not devices.is_empty() and not devices.has(device):
		return _joy_input_error(
			error_type, command, identity,
			"%s %d drives device %d outside the step's devices list" % [label, index, device],
			devices, device,
		)
	var has_motion := input.has("motion")
	var has_button := input.has("button")
	if has_motion == has_button:
		return _joy_input_error(
			error_type, command, identity,
			"%s %d must use exactly one of 'motion' or 'button'" % [label, index],
			expected, input.duplicate(true),
		)
	if has_motion:
		var motion = input.get("motion", null)
		if not (motion is Dictionary):
			return _joy_input_error(
				error_type, command, identity,
				"%s %d field 'motion' must be an object" % [label, index],
				expected, _runtime.get_ref()._fixtures._encode_setup_value(motion),
			)
		var normalized_axis := _normalize_joy_axis(str((motion as Dictionary).get("axis", "")))
		if normalized_axis.is_empty():
			return _joy_input_error(
				error_type, command, identity,
				"%s %d motion 'axis' must be a supported named joypad axis" % [label, index],
				_supported_joy_axis_names(), _runtime.get_ref()._fixtures._encode_setup_value((motion as Dictionary).get("axis", null)),
			)
		var raw_value = (motion as Dictionary).get("value", null)
		if not (raw_value is int or raw_value is float) or float(raw_value) < -1.0 or float(raw_value) > 1.0:
			return _joy_input_error(
				error_type, command, identity,
				"%s %d motion 'value' must be a number from -1.0 to 1.0" % [label, index],
				"number between -1.0 and 1.0", _runtime.get_ref()._fixtures._encode_setup_value(raw_value),
			)
		return {"ok": true, "input": {
			"device": device,
			"axis": str(normalized_axis.get("name", "")),
			"axis_index": int(normalized_axis.get("axis_index", -1)),
			"value": float(raw_value),
		}}
	var button = input.get("button", null)
	if not (button is Dictionary):
		return _joy_input_error(
			error_type, command, identity,
			"%s %d field 'button' must be an object" % [label, index],
			expected, _runtime.get_ref()._fixtures._encode_setup_value(button),
		)
	var normalized_button := _normalize_joy_button(str((button as Dictionary).get("name", "")))
	if normalized_button.is_empty():
		return _joy_input_error(
			error_type, command, identity,
			"%s %d button 'name' must be a supported named joypad button" % [label, index],
			_supported_joy_button_names(), _runtime.get_ref()._fixtures._encode_setup_value((button as Dictionary).get("name", null)),
		)
	var raw_pressed = (button as Dictionary).get("pressed", null)
	if not (raw_pressed is bool):
		return _joy_input_error(
			error_type, command, identity,
			"%s %d button 'pressed' must be a boolean" % [label, index],
			"boolean", _runtime.get_ref()._fixtures._encode_setup_value(raw_pressed),
		)
	return {"ok": true, "input": {
		"device": device,
		"button": str(normalized_button.get("name", "")),
		"button_index": int(normalized_button.get("button_index", -1)),
		"pressed": bool(raw_pressed),
	}}


# Build a joypad-entry validation error, merging provider identity (e.g.
# {"policy": id}) into the data envelope right after `command` so the policy
# path keeps its historic {command, policy, expected, actual} field order.
func _joy_input_error(error_type: String, command: String, identity: Dictionary, message: String, expected: Variant, actual: Variant) -> Dictionary:
	var data := {"command": command}
	for key in identity:
		data[key] = identity[key]
	data["expected"] = expected
	data["actual"] = actual
	return _runtime.get_ref()._error(error_type, message, data)


func _call_builtin_policy(policy_id: String, request: Dictionary) -> Dictionary:
	match policy_id:
		"builtin:steer_toward":
			return _builtin_policy_steer_toward(request)
	return _runtime.get_ref()._error(
		"unknown_builtin_policy",
		"policy.drive_tick does not provide this builtin policy",
		{
			"command": "policy.drive_tick",
			"policy": policy_id,
			"expected": ["builtin:steer_toward"],
		},
	)


# Minimal builtin policy for smoke tests and simple games: each tick it reads
# the steered node and target positions and returns normalized stick motion
# toward the target for the first driven device, or zeroed motion once the
# steered node is within tolerance.
func _builtin_policy_steer_toward(request: Dictionary) -> Dictionary:
	var params: Dictionary = request.get("params", {})
	var node_locator := str(params.get("node", "")).strip_edges()
	var target_locator := str(params.get("target", "")).strip_edges()
	if node_locator == "" or target_locator == "":
		return _runtime.get_ref()._error(
			"invalid_policy_call",
			"builtin:steer_toward requires non-empty 'node' and 'target' locator params",
			{
				"command": "policy.drive_tick",
				"policy": "builtin:steer_toward",
				"expected": {"node": "steered node locator", "target": "destination locator"},
				"actual": params.duplicate(true),
			},
		)
	var stick := str(params.get("stick", "left")).strip_edges().to_lower()
	if stick != "left" and stick != "right":
		return _runtime.get_ref()._error(
			"invalid_policy_call",
			"builtin:steer_toward param 'stick' must be 'left' or 'right'",
			{
				"command": "policy.drive_tick",
				"policy": "builtin:steer_toward",
				"expected": ["left", "right"],
				"actual": _runtime.get_ref()._fixtures._encode_setup_value(params.get("stick", null)),
			},
		)
	var tolerance := 8.0
	if params.has("tolerance"):
		var raw_tolerance = params.get("tolerance", null)
		if not (raw_tolerance is int or raw_tolerance is float) or float(raw_tolerance) <= 0.0:
			return _runtime.get_ref()._error(
				"invalid_policy_call",
				"builtin:steer_toward param 'tolerance' must be a positive number",
				{
					"command": "policy.drive_tick",
					"policy": "builtin:steer_toward",
					"expected": "positive number",
					"actual": _runtime.get_ref()._fixtures._encode_setup_value(raw_tolerance),
				},
			)
		tolerance = float(raw_tolerance)
	var node_result: Dictionary = _runtime.get_ref().resolve_target(node_locator)
	if not bool(node_result.get("ok", false)):
		return node_result
	var target_result: Dictionary = _runtime.get_ref().resolve_target(target_locator)
	if not bool(target_result.get("ok", false)):
		return target_result
	var node = node_result.get("node", null)
	var target = target_result.get("node", null)
	if not (node is Node2D) or not (target is Node2D):
		return _runtime.get_ref()._error(
			"invalid_policy_call",
			"builtin:steer_toward requires Node2D 'node' and 'target' nodes",
			{
				"command": "policy.drive_tick",
				"policy": "builtin:steer_toward",
				"expected": "Node2D locators",
				"actual": {
					"node": (node as Object).get_class() if node is Object else "missing",
					"target": (target as Object).get_class() if target is Object else "missing",
				},
			},
		)
	var devices: Array = request.get("devices", [])
	var device := int(devices[0]) if not devices.is_empty() else 0
	var delta: Vector2 = (target as Node2D).global_position - (node as Node2D).global_position
	var direction := Vector2.ZERO
	if delta.length() > tolerance:
		direction = delta.normalized()
	return {"ok": true, "result": {"inputs": [
		{"device": device, "motion": {"axis": "%s_x" % stick, "value": clampf(direction.x, -1.0, 1.0)}},
		{"device": device, "motion": {"axis": "%s_y" % stick, "value": clampf(direction.y, -1.0, 1.0)}},
	]}}


func _press_key(params: Dictionary) -> Dictionary:
	var raw_key := str(params.get("key", ""))
	var normalized := _normalize_key(raw_key)
	if normalized.is_empty():
		return _runtime.get_ref()._error(
			"invalid_params",
			"input.key requires a supported named key",
			{
				"command": "input.key",
				"expected": _supported_key_names(),
				"actual": raw_key,
			},
		)

	var frames: Dictionary = _runtime.get_ref()._non_negative_int_param(params, "frames", 1, "input.key")
	if not bool(frames.get("ok", false)):
		return frames

	var key_name := str(normalized.get("name", ""))
	var keycode := int(normalized.get("keycode", 0))
	var unicode := int(normalized.get("unicode", 0))
	await _tap_key(keycode, unicode, int(frames.get("value", 1)))
	var data := {
		"device": "keyboard",
		"key": key_name,
		"unicode": unicode,
	}
	_runtime.get_ref()._append_focus_owner(data)
	return _runtime.get_ref()._ok(data)


func _hold_key(params: Dictionary) -> Dictionary:
	var raw_key := str(params.get("key", ""))
	var normalized := _normalize_key(raw_key)
	if normalized.is_empty():
		return _runtime.get_ref()._error(
			"invalid_params",
			"input.key_hold requires a supported named key",
			{"command": "input.key_hold", "expected": _supported_key_names(), "actual": raw_key},
		)
	var frames: Dictionary = _runtime.get_ref()._non_negative_int_param(params, "frames", 1, "input.key_hold")
	if not bool(frames.get("ok", false)):
		return frames

	var key_name := str(normalized.get("name", ""))
	_emit_key_event(int(normalized.get("keycode", 0)), true, int(normalized.get("unicode", 0)))
	_runtime.get_ref()._held_keys[key_name] = true
	await _runtime.get_ref()._process_frames(int(frames.get("value", 1)))
	var held_keys: Array = _runtime.get_ref()._held_keys.keys()
	held_keys.sort()
	return _runtime.get_ref()._ok({
		"device": "keyboard",
		"key": key_name,
		"frames": int(frames.get("value", 1)),
		"held": true,
		"held_keys": held_keys,
	})


func _release_key(raw_key: String) -> Dictionary:
	var normalized := _normalize_key(raw_key)
	if normalized.is_empty():
		return _runtime.get_ref()._error(
			"invalid_params",
			"input.key_release requires a supported named key",
			{"command": "input.key_release", "expected": _supported_key_names(), "actual": raw_key},
		)
	var key_name := str(normalized.get("name", ""))
	if not _runtime.get_ref()._held_keys.has(key_name):
		return _runtime.get_ref()._error(
			"key_not_held",
			"input.key_release requires a currently held key",
			{"key": key_name, "expected": "held key tracked by the runtime", "actual": "not_held"},
		)

	_emit_key_event(int(normalized.get("keycode", 0)), false, int(normalized.get("unicode", 0)))
	_runtime.get_ref()._held_keys.erase(key_name)
	await _runtime.get_ref()._process_frames(1)
	var held_keys: Array = _runtime.get_ref()._held_keys.keys()
	held_keys.sort()
	return _runtime.get_ref()._ok({"device": "keyboard", "key": key_name, "held": false, "held_keys": held_keys})


func _type_text(params: Dictionary) -> Dictionary:
	var raw_text = params.get("text", null)
	if not (raw_text is String) or (raw_text as String).is_empty():
		return _runtime.get_ref()._error(
			"invalid_params",
			"input.text requires a non-empty text string",
			{
				"command": "input.text",
				"field": "text",
				"expected": "non-empty string",
				"actual": raw_text,
			},
		)

	var text := raw_text as String
	var max_text_length: int = _runtime.get_ref().MAX_INPUT_GESTURE_FRAMES / 2
	if text.length() > max_text_length:
		return _runtime.get_ref()._error(
			"invalid_params",
			"input.text text is too long for one gesture",
			{
				"command": "input.text",
				"field": "text",
				"expected": "text length <= %d" % max_text_length,
				"actual": text.length(),
			},
		)

	for i in text.length():
		var codepoint := text.unicode_at(i)
		var keycode := OS.find_keycode_from_string(text.substr(i, 1).to_upper())
		await _tap_key(keycode, codepoint)

	var data := {
		"device": "keyboard",
		"text": text,
		"characters": text.length(),
	}
	_runtime.get_ref()._append_focus_owner(data)
	return _runtime.get_ref()._ok(data)


func _press_joy_button(params: Dictionary) -> Dictionary:
	var raw_button := str(params.get("button", ""))
	var normalized := _normalize_joy_button(raw_button)
	if normalized.is_empty():
		return _runtime.get_ref()._error(
			"invalid_params",
			"input.joy_button requires a supported named joypad button",
			{
				"command": "input.joy_button",
				"expected": _supported_joy_button_names(),
				"actual": raw_button,
			},
		)

	var device: Dictionary = _runtime.get_ref()._non_negative_int_param(params, "device", 0, "input.joy_button")
	if not bool(device.get("ok", false)):
		return device
	var frames: Dictionary = _runtime.get_ref()._non_negative_int_param(params, "frames", 1, "input.joy_button")
	if not bool(frames.get("ok", false)):
		return frames

	var button_name := str(normalized.get("name", ""))
	var button_index := int(normalized.get("button_index", -1))
	_emit_joy_button_event(int(device.get("value", 0)), button_index, true)
	await _runtime.get_ref()._process_frames(int(frames.get("value", 1)))
	_emit_joy_button_event(int(device.get("value", 0)), button_index, false)
	await _runtime.get_ref()._process_frames(1)
	var data := {
		"device": "joypad",
		"device_id": int(device.get("value", 0)),
		"button": button_name,
		"button_index": button_index,
		"frames": int(frames.get("value", 1)),
	}
	_runtime.get_ref()._append_focus_owner(data)
	return _runtime.get_ref()._ok(data)


func _joy_motion(params: Dictionary) -> Dictionary:
	var raw_axis := str(params.get("axis", ""))
	var normalized := _normalize_joy_axis(raw_axis)
	if normalized.is_empty():
		return _runtime.get_ref()._error(
			"invalid_params",
			"input.joy_motion requires a supported named joypad axis",
			{
				"command": "input.joy_motion",
				"expected": _supported_joy_axis_names(),
				"actual": raw_axis,
			},
		)

	var raw_value = params.get("value", null)
	if not (raw_value is int or raw_value is float):
		return _runtime.get_ref()._error(
			"invalid_params",
			"input.joy_motion value must be a number from -1.0 to 1.0",
			{
				"command": "input.joy_motion",
				"field": "value",
				"expected": "number between -1.0 and 1.0",
				"actual": raw_value,
			},
		)
	var value := float(raw_value)
	if value < -1.0 or value > 1.0:
		return _runtime.get_ref()._error(
			"invalid_params",
			"input.joy_motion value must be between -1.0 and 1.0",
			{
				"command": "input.joy_motion",
				"field": "value",
				"expected": "number between -1.0 and 1.0",
				"actual": value,
			},
		)

	var device: Dictionary = _runtime.get_ref()._non_negative_int_param(params, "device", 0, "input.joy_motion")
	if not bool(device.get("ok", false)):
		return device
	var frames: Dictionary = _runtime.get_ref()._non_negative_int_param(params, "frames", 1, "input.joy_motion")
	if not bool(frames.get("ok", false)):
		return frames

	var axis_name := str(normalized.get("name", ""))
	var axis_index := int(normalized.get("axis_index", -1))
	_emit_joy_motion_event(int(device.get("value", 0)), axis_index, value)
	await _runtime.get_ref()._process_frames(int(frames.get("value", 1)))
	var data := {
		"device": "joypad",
		"device_id": int(device.get("value", 0)),
		"axis": axis_name,
		"axis_index": axis_index,
		"value": value,
		"frames": int(frames.get("value", 1)),
	}
	_runtime.get_ref()._append_focus_owner(data)
	return _runtime.get_ref()._ok(data)


# Open-loop batch joypad input: apply many devices' inputs in the SAME frame,
# including button holds and axis positions that persist across steps. The whole
# `inputs` list is validated before anything is emitted (a malformed entry fails
# the step and injects nothing); then every entry is emitted back-to-back with no
# frame awaits between them, so concurrent cross-device holds land on one frame.
# Buttons stay pressed and axes stay latched until a later step changes them;
# existing _touched_joy_* tracking plus finalize/input.reset release them (this
# command adds no cleanup of its own). `frames` (default 1) is the post-emit wait
# (0 = emit and continue same-frame). Unlike input.joy_button there is no device
# allowlist: any non-negative device is accepted.
func _input_joy_state(params: Dictionary) -> Dictionary:
	var raw_inputs = params.get("inputs", null)
	if not (raw_inputs is Array) or (raw_inputs as Array).is_empty():
		return _runtime.get_ref()._error(
			"invalid_params",
			"input.joy_state requires a non-empty 'inputs' list",
			{
				"command": "input.joy_state",
				"field": "inputs",
				"expected": "non-empty list of {device, motion|button} objects",
				"actual": _runtime.get_ref()._fixtures._encode_setup_value(raw_inputs),
			},
		)
	var frames: Dictionary = _runtime.get_ref()._non_negative_int_param(params, "frames", 1, "input.joy_state")
	if not bool(frames.get("ok", false)):
		return frames
	# Validate the WHOLE list before emitting anything so a malformed entry
	# injects no input at all. No device allowlist: any non-negative device.
	var no_allowlist: Array[int] = []
	var planned: Array[Dictionary] = []
	for index in range((raw_inputs as Array).size()):
		var validated := _validate_joy_input_entry(
			index, (raw_inputs as Array)[index], no_allowlist, "invalid_params", "input.joy_state", "input", {}
		)
		if not bool(validated.get("ok", false)):
			return validated
		planned.append(validated.get("input", {}))
	# Emit every entry back-to-back with no frame await between them so all
	# devices' inputs are applied in a single frame, mirroring the drive_policy
	# tick emit loop.
	for input in planned:
		if input.has("axis_index"):
			_emit_joy_motion_event(int(input.get("device", 0)), int(input.get("axis_index", -1)), float(input.get("value", 0.0)))
		else:
			_emit_joy_button_event(int(input.get("device", 0)), int(input.get("button_index", -1)), bool(input.get("pressed", false)))
	await _runtime.get_ref()._process_frames(int(frames.get("value", 1)))
	var data := {
		"device": "joypad",
		"inputs_applied": planned.size(),
		"inputs": planned,
		"frames": int(frames.get("value", 1)),
	}
	_runtime.get_ref()._append_focus_owner(data)
	return _runtime.get_ref()._ok(data)


# Announce one or more virtual joypads as CONNECTED by firing the engine's
# Input.joy_connection_changed signal — the SAME signal the platform layer emits
# on a real controller connect — so a local-multiplayer lobby that listens for it
# ("press A to join", spawn-a-player) runs its connect handler under headless QA,
# where no physical pad ever fires it. `devices` is a required non-empty list of
# non-negative ints (deduped/normalized via the shared _policy_device_allowlist
# helper); `name` is optional QA metadata (see the engine-limitation note below).
# Connect is idempotent: a device already faked is NOT re-emitted (so a listener
# cannot double-join) but the request still succeeds. Every faked device is
# tracked in the static _faked_joypads set so finalize/input.reset can announce a
# matching disconnect for exactly what was faked and joy_disconnect's real-pad
# guard knows what the QA layer owns. One delivery frame is awaited after the
# whole batch so listeners settle.
#
# ENGINE LIMITATION (Godot 4.6.2): `Input.joy_connection_changed` is a SIGNAL, not
# a bound method. The roster-mutating C++ entry point
# `Input::joy_connection_changed(device, connected, name, guid)` is not exposed to
# GDScript (`Input.has_method("joy_connection_changed")` == false), and the signal
# carries only `(device, connected)`. So this fires the signal but does NOT add
# the device to `Input.get_connected_joypads()`, and `name` is not delivered to
# the engine (`get_joy_name` is unaffected). Enumeration-based lobbies that poll
# `get_connected_joypads()` therefore cannot be faked on this engine — only
# signal-driven connect/disconnect handling can. `name` is kept as accepted,
# echoed QA metadata (and to keep the step contract stable) for forward
# compatibility if a future engine binds the four-argument method.
func _joy_connect(params: Dictionary) -> Dictionary:
	var devices := _validate_joy_setup_devices(params, "setup.joy_connect")
	if not bool(devices.get("ok", false)):
		return devices
	var device_list: Array[int] = devices.get("devices", [] as Array[int])
	for device in device_list:
		if device in Input.get_connected_joypads():
			return _runtime.get_ref()._error(
				"invalid_params",
				"setup.joy_connect cannot claim a real connected joypad",
				{"command": "setup.joy_connect", "device": device, "connected_joypads": _connected_joypads_array()},
			)
	var name := "QA Virtual Pad"
	if params.has("name"):
		var raw_name = params.get("name", null)
		if not (raw_name is String) or (raw_name as String).strip_edges() == "":
			return _runtime.get_ref()._error(
				"invalid_params",
				"setup.joy_connect 'name' must be a non-empty string",
				{
					"command": "setup.joy_connect",
					"field": "name",
					"expected": "non-empty string",
					"actual": _runtime.get_ref()._fixtures._encode_setup_value(raw_name),
				},
			)
		name = (raw_name as String).strip_edges()
	for device in device_list:
		# Idempotent: only fire `connected` for a device not already announced, so
		# a listener cannot double-join; refresh the tracked name either way.
		var already_faked: bool = _runtime.get_ref()._faked_joypads.has(device)
		_runtime.get_ref()._faked_joypads[device] = {"device": device, "name": name}
		if not already_faked:
			Input.joy_connection_changed.emit(device, true)
	await _runtime.get_ref()._process_frames(1)
	return _runtime.get_ref()._ok({
		"devices": device_list,
		"name": name,
		"announced_joypads": _announced_joypads_array(),
		"connected_joypads": _connected_joypads_array(),
	})


# Announce one or more QA-faked joypads as DISCONNECTED by firing the same
# Input.joy_connection_changed signal with connected == false, so a scenario can
# exercise mid-match controller-drop handling (pause-on-drop, the classic
# couch-game bug class). GUARD: only devices previously faked via setup.joy_connect
# may be disconnected. Firing a disconnect for a device the QA layer never
# announced could correspond to a real physical pad and wrongly pause the game or
# drop a real player, so a request naming any non-faked device fails with
# invalid_params and emits nothing. Disconnected devices are removed from
# _faked_joypads tracking. One delivery frame is awaited after the whole batch.
# See _joy_connect for the Godot 4.6.2 engine limitation (signal-only; the
# get_connected_joypads() roster is unaffected).
func _joy_disconnect(params: Dictionary) -> Dictionary:
	var devices := _validate_joy_setup_devices(params, "setup.joy_disconnect")
	if not bool(devices.get("ok", false)):
		return devices
	var device_list: Array[int] = devices.get("devices", [] as Array[int])
	var not_faked: Array[int] = []
	for device in device_list:
		if not _runtime.get_ref()._faked_joypads.has(device):
			not_faked.append(device)
	if not not_faked.is_empty():
		return _runtime.get_ref()._error(
			"invalid_params",
			"setup.joy_disconnect can only disconnect joypads faked via setup.joy_connect",
			{
				"command": "setup.joy_disconnect",
				"field": "devices",
				"expected": "devices previously faked via setup.joy_connect",
				"actual": not_faked,
				"faked_devices": _runtime.get_ref()._faked_joypads.keys(),
			},
		)
	for device in device_list:
		if device in Input.get_connected_joypads():
			return _runtime.get_ref()._error(
				"invalid_params",
				"setup.joy_disconnect cannot disconnect a real connected joypad",
				{"command": "setup.joy_disconnect", "device": device, "connected_joypads": _connected_joypads_array()},
			)
	await _release_injected_joypad_devices(device_list)
	for device in device_list:
		_runtime.get_ref()._faked_joypads.erase(device)
		Input.joy_connection_changed.emit(device, false)
	await _runtime.get_ref()._process_frames(1)
	return _runtime.get_ref()._ok({
		"devices": device_list,
		"announced_joypads": _announced_joypads_array(),
		"connected_joypads": _connected_joypads_array(),
	})


# Validate the shared `devices` field for setup.joy_connect/disconnect: a required
# non-empty list of non-negative ints. Reuses _policy_device_allowlist for the
# dedup/normalize pass (drops duplicates, coerces whole floats). Returns
# {"ok": true, "devices": Array[int]} or an invalid_params error dictionary.
func _validate_joy_setup_devices(params: Dictionary, command: String) -> Dictionary:
	var raw_devices = params.get("devices", null)
	if not (raw_devices is Array) or (raw_devices as Array).is_empty():
		return _runtime.get_ref()._error(
			"invalid_params",
			"%s requires a non-empty 'devices' list" % command,
			{
				"command": command,
				"field": "devices",
				"expected": "non-empty list of non-negative integers",
				"actual": _runtime.get_ref()._fixtures._encode_setup_value(raw_devices),
			},
		)
	var devices := _policy_device_allowlist(raw_devices)
	if devices.is_empty():
		return _runtime.get_ref()._error(
			"invalid_params",
			"%s 'devices' entries must be non-negative integers" % command,
			{
				"command": command,
				"field": "devices",
				"expected": "non-negative integers",
				"actual": _runtime.get_ref()._fixtures._encode_setup_value(raw_devices),
			},
		)
	return {"ok": true, "devices": devices}


# Sorted device indices the QA layer has announced-as-connected and not yet
# disconnected (the static _faked_joypads tracking set). This is the observable
# contract the connect/disconnect success payloads echo, since on Godot 4.6.2 the
# real Input.get_connected_joypads() roster cannot be faked (see _joy_connect).
func _announced_joypads_array() -> Array[int]:
	var announced: Array[int] = []
	for device in _runtime.get_ref()._faked_joypads.keys():
		announced.append(int(device))
	announced.sort()
	return announced


# The real, engine-truth Input.get_connected_joypads() copied into a plain
# JSON-compatible Array[int]. Echoed for transparency; on Godot 4.6.2 it does NOT
# reflect QA-announced pads (signal-only, no roster mutation), so
# `announced_joypads` is the field scenarios should assert against. Games that
# poll the roster should read effective_connected_joypads() instead.
func _connected_joypads_array() -> Array[int]:
	var connected: Array[int] = []
	for device in Input.get_connected_joypads():
		connected.append(int(device))
	return connected


func _emit_action_event(action: String, pressed: bool) -> void:
	var event := InputEventAction.new()
	event.action = action
	event.pressed = pressed
	Input.parse_input_event(event)


func _tap_key(keycode: int, unicode: int = 0, frames: int = 1) -> void:
	_emit_key_event(keycode, true, unicode)
	await _runtime.get_ref()._process_frames(frames)
	_emit_key_event(keycode, false, unicode)
	for key_name in _runtime.get_ref()._held_keys.keys():
		if int(_normalize_key(str(key_name)).get("keycode", 0)) == keycode:
			_runtime.get_ref()._held_keys.erase(key_name)
	await _runtime.get_ref()._process_frames(1)


func _emit_key_event(keycode: int, pressed: bool, unicode: int = 0) -> void:
	var event := InputEventKey.new()
	event.keycode = keycode
	event.physical_keycode = keycode
	event.unicode = unicode
	event.pressed = pressed
	event.echo = false
	Input.parse_input_event(event)


func _emit_joy_button_event(device: int, button_index: int, pressed: bool) -> void:
	var event := InputEventJoypadButton.new()
	event.device = device
	event.button_index = button_index
	event.pressed = pressed
	event.pressure = 1.0 if pressed else 0.0
	Input.parse_input_event(event)
	var key := "%d:%d" % [device, button_index]
	if pressed:
		_runtime.get_ref()._touched_joy_buttons[key] = {"device": device, "button_index": button_index}
	else:
		_runtime.get_ref()._touched_joy_buttons.erase(key)


func _emit_joy_motion_event(device: int, axis_index: int, value: float) -> void:
	var event := InputEventJoypadMotion.new()
	event.device = device
	event.axis = axis_index
	event.axis_value = value
	Input.parse_input_event(event)
	var key := "%d:%d" % [device, axis_index]
	if is_zero_approx(value):
		_runtime.get_ref()._touched_joy_axes.erase(key)
	else:
		_runtime.get_ref()._touched_joy_axes[key] = {"device": device, "axis_index": axis_index}


func _normalize_key(raw_key: String) -> Dictionary:
	var key_name := raw_key.strip_edges().to_lower()
	if key_name == "":
		return {}
	return (_runtime.get_ref().SUPPORTED_KEYS.get(key_name, {}) as Dictionary).duplicate(true)


func _normalize_joy_button(raw_button: String) -> Dictionary:
	var button_name := raw_button.strip_edges().to_lower()
	if button_name == "":
		return {}
	return (_runtime.get_ref().SUPPORTED_JOY_BUTTONS.get(button_name, {}) as Dictionary).duplicate(true)


func _supported_key_names() -> Array[String]:
	var canonical_names: Array[String] = []
	var seen := {}
	for entry in _runtime.get_ref().SUPPORTED_KEYS.values():
		if not entry is Dictionary:
			continue
		var name := str((entry as Dictionary).get("name", ""))
		if name == "" or seen.has(name):
			continue
		seen[name] = true
		canonical_names.append(name)
	canonical_names.sort()
	return canonical_names


func _supported_joy_button_names() -> Array[String]:
	var canonical_names: Array[String] = []
	var seen := {}
	for entry in _runtime.get_ref().SUPPORTED_JOY_BUTTONS.values():
		if not entry is Dictionary:
			continue
		var name := str((entry as Dictionary).get("name", ""))
		if name == "" or seen.has(name):
			continue
		seen[name] = true
		canonical_names.append(name)
	canonical_names.sort()
	return canonical_names


func _normalize_joy_axis(raw_axis: String) -> Dictionary:
	var axis_name := raw_axis.strip_edges().to_lower()
	if axis_name == "":
		return {}
	return (_runtime.get_ref().SUPPORTED_JOY_AXES.get(axis_name, {}) as Dictionary).duplicate(true)


func _supported_joy_axis_names() -> Array[String]:
	var canonical_names: Array[String] = []
	var seen := {}
	for entry in _runtime.get_ref().SUPPORTED_JOY_AXES.values():
		if not entry is Dictionary:
			continue
		var name := str((entry as Dictionary).get("name", ""))
		if name == "" or seen.has(name):
			continue
		seen[name] = true
		canonical_names.append(name)
	canonical_names.sort()
	return canonical_names
