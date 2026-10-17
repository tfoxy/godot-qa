extends Node

# The recorder reverse-maps raw joypad indexes through the SAME named tables
# qa_runtime_impl uses to validate input.joy_button / input.joy_motion, so a
# recorded event is replayable by construction.
const QaRuntimeImpl = preload("res://addons/godot_qa/qa_runtime_impl.gd")

# Analog quantization defaults for recorded joypad motion. All three are
# configurable via `scenario record` (--joy-motion-step / --joy-motion-deadzone
# / --joy-motion-frame-gap), delivered through the record request's
# session.recorder options.
const DEFAULT_JOY_MOTION_STEP := 0.25
const DEFAULT_JOY_MOTION_DEADZONE := 0.1
const DEFAULT_JOY_MOTION_FRAME_GAP := 6

var _scene_root: Node = null
var _root_window: Window = null
var _runtime: RefCounted = null
var _events: Array[Dictionary] = []
var _unconverted_events: Array[Dictionary] = []
var _input_moments: Array[Dictionary] = []
var _frame_index := 0
var _sequence := 0
var _action_start_frames := {}
var _last_focus_target := ""
var _suppress_mouse_release_frame := -1
# Joypad capture state: press frames per "device:button_index", and per
# "device:axis_index" quantization state {last_recorded_value,
# last_recorded_frame, current_value}. Reported-once guards keep unsupported
# index noise out of unconverted_events.
var _joy_button_start_frames := {}
var _joy_axis_state := {}
var _reported_unknown_joy_buttons := {}
var _reported_unknown_joy_axes := {}
var _joy_button_names_by_index := {}
var _joy_axis_names_by_index := {}
var _joy_motion_step := DEFAULT_JOY_MOTION_STEP
var _joy_motion_deadzone := DEFAULT_JOY_MOTION_DEADZONE
var _joy_motion_frame_gap := DEFAULT_JOY_MOTION_FRAME_GAP
# Frame of the last event that anchored the replay timeline; the gap to the
# next raw joypad event is exported as a wait_frames event so the candidate
# scenario replays on the human's timeline.
var _last_timed_event_frame := 0


func _init(scene_root: Node, root_window: Window, runtime: RefCounted, options: Dictionary = {}) -> void:
	_scene_root = scene_root
	_root_window = root_window
	_runtime = runtime
	for entry in QaRuntimeImpl.SUPPORTED_JOY_BUTTONS.values():
		if entry is Dictionary:
			_joy_button_names_by_index[int((entry as Dictionary).get("button_index", -1))] = str((entry as Dictionary).get("name", ""))
	for entry in QaRuntimeImpl.SUPPORTED_JOY_AXES.values():
		if entry is Dictionary:
			_joy_axis_names_by_index[int((entry as Dictionary).get("axis_index", -1))] = str((entry as Dictionary).get("name", ""))
	var step = options.get("joy_motion_step", DEFAULT_JOY_MOTION_STEP)
	if (step is float or step is int) and float(step) > 0.0 and float(step) <= 1.0:
		_joy_motion_step = float(step)
	var deadzone = options.get("joy_motion_deadzone", DEFAULT_JOY_MOTION_DEADZONE)
	if (deadzone is float or deadzone is int) and float(deadzone) >= 0.0 and float(deadzone) < 0.5:
		_joy_motion_deadzone = float(deadzone)
	var frame_gap = options.get("joy_motion_frame_gap", DEFAULT_JOY_MOTION_FRAME_GAP)
	if (frame_gap is float or frame_gap is int) and int(frame_gap) >= 0 and int(frame_gap) <= 240:
		_joy_motion_frame_gap = int(frame_gap)


func _ready() -> void:
	process_mode = Node.PROCESS_MODE_ALWAYS
	set_process(true)
	set_process_input(true)


func _process(_delta: float) -> void:
	_frame_index += 1
	var focus_owner := _root_window.gui_get_focus_owner() if _root_window != null else null
	var focus_target := _stable_locator(focus_owner)
	if focus_target != _last_focus_target:
		_last_focus_target = focus_target
		_append_event(
			{
				"type": "focus_changed",
				"target": focus_target,
			}
		)


func _input(event: InputEvent) -> void:
	if event is InputEventAction:
		var action_event := event as InputEventAction
		var action_name := str(action_event.action)
		if action_name == "":
			return
		if action_event.pressed:
			_action_start_frames[action_name] = _frame_index
			return
		if not _action_start_frames.has(action_name):
			return
		var start_frame := int(_action_start_frames[action_name])
		_action_start_frames.erase(action_name)
		var frame_span := maxi(_frame_index - start_frame, 0)
		if frame_span <= 1:
			_append_event(
				{
					"type": "tap_action",
					"action": action_name,
				}
			)
			return
		_append_event(
			{
				"type": "hold_action",
				"action": action_name,
				"frames": frame_span,
			}
		)
		_append_event(
			{
				"type": "release_action",
				"action": action_name,
			}
		)
		return

	if event is InputEventJoypadButton:
		_capture_joy_button(event as InputEventJoypadButton)
		return

	if event is InputEventJoypadMotion:
		_capture_joy_motion(event as InputEventJoypadMotion)
		return

	if event is InputEventMouseButton:
		var mouse_event := event as InputEventMouseButton
		if mouse_event.button_index != MOUSE_BUTTON_LEFT or mouse_event.pressed:
			return
		if _suppress_mouse_release_frame == _frame_index:
			return
		var hovered := _hovered_control()
		var locator := _stable_locator(hovered)
		if locator == "":
			_append_unconverted_event(
				{
					"type": "click",
					"reason": "no stable locator",
					"position": [float(mouse_event.position.x), float(mouse_event.position.y)],
				}
			)
			return
		_append_event(
			{
				"type": "click",
				"target": locator,
				"resolved": locator,
			}
		)


func _capture_joy_button(event: InputEventJoypadButton) -> void:
	var button_index := int(event.button_index)
	var button_name := str(_joy_button_names_by_index.get(button_index, ""))
	if button_name == "":
		if not _reported_unknown_joy_buttons.has(button_index):
			_reported_unknown_joy_buttons[button_index] = true
			_append_unconverted_event(
				{
					"type": "input_joy_button",
					"reason": "unsupported joypad button index",
					"button_index": button_index,
					"device": event.device,
				}
			)
		return
	var key := "%d:%d" % [event.device, button_index]
	if event.pressed:
		if not _joy_button_start_frames.has(key):
			_joy_button_start_frames[key] = _frame_index
		return
	if not _joy_button_start_frames.has(key):
		return
	var start_frame := int(_joy_button_start_frames[key])
	_joy_button_start_frames.erase(key)
	_append_joy_button_event(event.device, button_name, start_frame)


# Mirror the tap/hold pairing used for action events: a release within one
# frame replays as a one-frame input_joy_button tap, a longer press replays
# with frames = the observed span (input.joy_button presses, holds `frames`
# frames, then releases).
func _append_joy_button_event(device: int, button_name: String, start_frame: int) -> void:
	var frame_span := maxi(_frame_index - start_frame, 0)
	_append_timeline_gap(start_frame)
	var payload := {
		"type": "input_joy_button",
		"button": button_name,
		"device": device,
	}
	if frame_span > 1:
		payload["frames"] = frame_span
	_append_event(payload)
	_last_timed_event_frame = maxi(_last_timed_event_frame, _frame_index)


func _capture_joy_motion(event: InputEventJoypadMotion) -> void:
	var axis_index := int(event.axis)
	var axis_name := str(_joy_axis_names_by_index.get(axis_index, ""))
	if axis_name == "":
		if not _reported_unknown_joy_axes.has(axis_index):
			_reported_unknown_joy_axes[axis_index] = true
			_append_unconverted_event(
				{
					"type": "input_joy_motion",
					"reason": "unsupported joypad axis index",
					"axis_index": axis_index,
					"device": event.device,
				}
			)
		return
	var key := "%d:%d" % [event.device, axis_index]
	var state: Dictionary = _joy_axis_state.get(
		key,
		{
			"device": event.device,
			"axis": axis_name,
			"last_recorded_value": 0.0,
			"last_recorded_frame": -1_000_000,
			"current_value": 0.0,
		}
	)
	var value := clampf(event.axis_value, -1.0, 1.0)
	state["current_value"] = value
	_joy_axis_state[key] = state
	var last_value := float(state.get("last_recorded_value", 0.0))
	var last_frame := int(state.get("last_recorded_frame", -1_000_000))
	var recorded := _quantized_motion_value(value)
	# Quantization/dedup: analog noise only becomes a recorded event when the
	# value crosses a step bucket, returns to the deadzone, or drifts to a new
	# value after the minimum frame gap for this axis.
	var crossed_bucket := _motion_bucket(value) != _motion_bucket(last_value)
	var returned_to_zero := absf(value) <= _joy_motion_deadzone and absf(last_value) > _joy_motion_deadzone
	var frame_gap_elapsed := (
		_frame_index - last_frame >= _joy_motion_frame_gap
		and not is_equal_approx(recorded, last_value)
	)
	if not (crossed_bucket or returned_to_zero or frame_gap_elapsed):
		return
	if is_equal_approx(recorded, last_value):
		return
	state["last_recorded_value"] = recorded
	state["last_recorded_frame"] = _frame_index
	_append_timeline_gap(_frame_index)
	_append_event(
		{
			"type": "input_joy_motion",
			"axis": axis_name,
			"device": event.device,
			"value": recorded,
		}
	)


func _motion_bucket(value: float) -> int:
	if absf(value) <= _joy_motion_deadzone:
		return 0
	return int(roundf(value / _joy_motion_step))


func _quantized_motion_value(value: float) -> float:
	if absf(value) <= _joy_motion_deadzone:
		return 0.0
	return value


# Interleave the frame gap since the previous timeline anchor as a wait_frames
# event, so joypad events replay on the human's timeline (the same wait_frames
# vocabulary the CLI-driven recording path already exports).
func _append_timeline_gap(event_frame: int) -> void:
	var gap := event_frame - _last_timed_event_frame
	if gap > 0:
		_append_event(
			{
				"type": "wait_frames",
				"frames": gap,
			}
		)
	_last_timed_event_frame = maxi(_last_timed_event_frame, event_frame)


# Flush pending joypad state at recording end: buttons still held become
# bounded holds, and every axis whose live value differs from its last
# recorded value emits one final input_joy_motion so replay ends on the
# human's final stick pose.
func _finalize_joypad_capture() -> void:
	var held_keys := _joy_button_start_frames.keys()
	held_keys.sort()
	for key in held_keys:
		var start_frame := int(_joy_button_start_frames[key])
		var parts := str(key).split(":")
		if parts.size() != 2:
			continue
		var button_name := str(_joy_button_names_by_index.get(int(parts[1]), ""))
		if button_name == "":
			continue
		_append_joy_button_event(int(parts[0]), button_name, start_frame)
	_joy_button_start_frames.clear()
	var axis_keys := _joy_axis_state.keys()
	axis_keys.sort()
	for key in axis_keys:
		var state: Dictionary = _joy_axis_state[key]
		var final_value := _quantized_motion_value(float(state.get("current_value", 0.0)))
		if is_equal_approx(final_value, float(state.get("last_recorded_value", 0.0))):
			continue
		state["last_recorded_value"] = final_value
		state["last_recorded_frame"] = _frame_index
		_append_timeline_gap(_frame_index)
		_append_event(
			{
				"type": "input_joy_motion",
				"axis": str(state.get("axis", "")),
				"device": int(state.get("device", 0)),
				"value": final_value,
			}
		)


func note_wait_frames(frames: int) -> void:
	if frames <= 0:
		return
	_append_event(
		{
			"type": "wait_frames",
			"frames": frames,
		}
	)
	_last_timed_event_frame = _frame_index


func note_snapshot(name: String) -> void:
	if name == "":
		return
	_append_event(
		{
			"type": "snapshot",
			"name": name,
		}
	)


func note_click_target(target: String, resolved: String) -> void:
	var locator := target if target != "" else resolved
	if locator == "":
		_append_unconverted_event(
			{
				"type": "click",
				"reason": "no stable locator",
			}
		)
		return
	_suppress_mouse_release_frame = _frame_index
	_append_event(
		{
			"type": "click",
			"target": locator,
			"resolved": resolved if resolved != "" else locator,
		}
	)

func note_runtime_command(command_name: String, params: Dictionary, result: Dictionary) -> void:
	if command_name == "":
		return
	var command_result := result if result is Dictionary else {}
	var ok := bool(command_result.get("ok", false))
	var payload := {
		"command": command_name,
		"params": params.duplicate(true),
		"ok": ok,
		"frame": _frame_index,
	}
	var snapshot := str(params.get("snapshot", ""))
	if snapshot != "":
		payload["snapshot"] = snapshot
	if ok:
		var data := command_result.get("data", {})
		if data is Dictionary:
			var data_dict: Dictionary = (data as Dictionary).duplicate(true)
			var target := str(data_dict.get("target", str(params.get("target", ""))))
			var resolved := str(data_dict.get("resolved", ""))
			var locator := target if target != "" else resolved
			if locator != "":
				payload["target"] = target if target != "" else locator
				payload["resolved"] = resolved if resolved != "" else locator
				payload["locator"] = locator
	else:
		var error := command_result.get("error", {})
		if error is Dictionary:
			var error_dict: Dictionary = (error as Dictionary).duplicate(true)
			payload["error_type"] = str(error_dict.get("type", "runtime_error"))
			var error_data := error_dict.get("data", {})
			if error_data is Dictionary:
				var error_data_dict: Dictionary = (error_data as Dictionary).duplicate(true)
				var target := str(error_data_dict.get("target", str(params.get("target", ""))))
				var resolved := str(error_data_dict.get("resolved", ""))
				var locator := target if target != "" else resolved
				if locator != "":
					payload["target"] = target if target != "" else locator
					payload["resolved"] = resolved if resolved != "" else locator
					payload["locator"] = locator
	_input_moments.append(_with_sequence(payload))


func export() -> Dictionary:
	_finalize_joypad_capture()
	return {
		"events": _duplicate_entries(_events),
		"unconverted_events": _duplicate_entries(_unconverted_events),
		"input_moments": _duplicate_entries(_input_moments),
		"frame_index": _frame_index,
	}


func _append_event(payload: Dictionary) -> void:
	var event := _with_sequence(payload)
	if not event.has("frame"):
		event["frame"] = _frame_index
	_events.append(event)


func _append_unconverted_event(payload: Dictionary) -> void:
	var event := _with_sequence(payload)
	if not event.has("frame"):
		event["frame"] = _frame_index
	_unconverted_events.append(event)

func _with_sequence(payload: Dictionary) -> Dictionary:
	_sequence += 1
	var entry := payload.duplicate(true)
	entry["seq"] = _sequence
	return entry


func _hovered_control() -> Control:
	if _root_window == null:
		return null
	if _root_window.has_method("gui_get_hovered_control"):
		return _root_window.call("gui_get_hovered_control") as Control
	return null


func _stable_locator(node: Variant) -> String:
	if node == null or _runtime == null or not _runtime.has_method("stable_locator_for_node"):
		return ""
	return str(_runtime.call("stable_locator_for_node", node))


func _duplicate_entries(entries: Array[Dictionary]) -> Array[Dictionary]:
	var duplicated: Array[Dictionary] = []
	for entry in entries:
		duplicated.append(entry.duplicate(true))
	return duplicated
