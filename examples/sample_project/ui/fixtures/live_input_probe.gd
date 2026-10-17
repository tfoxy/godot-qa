extends Control

const QaRuntime = preload("res://addons/godot_qa/qa_runtime.gd")

@onready var title_label: Label = $Canvas/TitleLabel
@onready var instruction_label: Label = $Canvas/InstructionLabel
@onready var input_probe_button: PanelContainer = $Canvas/InputProbeButton
@onready var input_probe_label: Label = $Canvas/InputProbeButton/Margin/InputProbeLabel
@onready var result_label: Label = $Canvas/ResultLabel

# Devices this lobby has seen CONNECT via the Input.joy_connection_changed signal
# and not yet seen disconnect, keyed by int device -> true. This mirrors exactly
# how a real local-multiplayer lobby tracks joined pads from the connect/disconnect
# signal (rather than polling Input.get_connected_joypads()), which is the path
# joy_connect/joy_disconnect exercises under headless QA.
var _connected_pads: Dictionary = {}
var _connection_events: Array[Dictionary] = []


func _ready() -> void:
	set_meta("qa_id", "live_input_probe")
	title_label.set_meta("qa_id", "input_probe_title")
	instruction_label.set_meta("qa_id", "input_probe_instruction")
	input_probe_button.set_meta("qa_id", "input_probe_button")
	input_probe_label.set_meta("qa_id", "input_probe_button_label")
	result_label.set_meta("qa_id", "input_probe_result")
	result_label.text = "ready"
	input_probe_button.grab_focus()
	Input.joy_connection_changed.connect(_on_joy_connection_changed)


func _on_joy_connection_changed(device: int, connected: bool) -> void:
	if connected:
		_connected_pads[device] = true
	else:
		_connected_pads.erase(device)
	_connection_events.append({
		"device": device,
		"connected": connected,
		"effective_roster": QaRuntime.effective_connected_joypads(),
	})


func _input(event: InputEvent) -> void:
	if event is InputEventKey:
		var key_event := event as InputEventKey
		if key_event.pressed and not key_event.echo and key_event.keycode == KEY_ENTER:
			result_label.text = "key:Enter"
	elif event is InputEventJoypadButton:
		var joy_event := event as InputEventJoypadButton
		if joy_event.pressed and joy_event.button_index == JOY_BUTTON_START:
			result_label.text = "joy:start:device=%d" % joy_event.device
	elif event is InputEventJoypadMotion:
		var motion_event := event as InputEventJoypadMotion
		if motion_event.axis == JOY_AXIS_LEFT_X:
			result_label.text = "joy:motion:device=%d:left_x=%.2f" % [motion_event.device, motion_event.axis_value]


# Read-only concurrent joypad-state probe for the input_joy_state demo. Unlike
# the _input handler above (which only records the latest event), this reads live
# Input state so a scenario can assert that two devices' buttons are held in the
# SAME frame plus a latched axis, which open-loop input.joy_state makes possible.
func get_qa_joy_state() -> Dictionary:
	var device0_a := Input.is_joy_button_pressed(0, JOY_BUTTON_A)
	var device3_x := Input.is_joy_button_pressed(3, JOY_BUTTON_X)
	return {
		"device0_a": device0_a,
		"device3_x": device3_x,
		"both_buttons_held": device0_a and device3_x,
		"device3_left_x": Input.get_joy_axis(3, JOY_AXIS_LEFT_X),
	}


# Read-only joypad CONNECTION roster for the joy_connect demo, covering both
# lobby styles:
# - `connected`/`count`: the roster this fixture tracks from the
#   Input.joy_connection_changed signal (see _ready) — how an event-driven lobby
#   sees joy_connect/joy_disconnect.
# - `effective_roster_count`: the polling path via the
#   QaRuntime.effective_connected_joypads() shim (the QA-aware replacement for
#   Input.get_connected_joypads() that unions the engine roster with QA-faked
#   pads) — how a polling lobby adopts the tool on Godot 4.6.2.
# - `engine_roster_count`: the raw Input.get_connected_joypads() size, echoed to
#   keep the engine limitation visible: it cannot be faked from script, so it
#   stays 0 under headless QA while the other two move.
func get_qa_joypad_roster() -> Dictionary:
	var connected: Array = _connected_pads.keys()
	connected.sort()
	var last_roster: Array = _connection_events.back().get("effective_roster", []) if not _connection_events.is_empty() else []
	var last_roster_labels: Array[String] = []
	for device in last_roster:
		last_roster_labels.append(str(device))
	return {
		"connected": connected,
		"count": connected.size(),
		"effective_roster_count": QaRuntime.effective_connected_joypads().size(),
		"engine_roster_count": Input.get_connected_joypads().size(),
		"connection_events": _connection_events.duplicate(true),
		"last_event_roster": ",".join(last_roster_labels),
	}


func get_qa_virtual_pad_cleanup() -> Dictionary:
	return {
		"effective_roster_count": QaRuntime.effective_connected_joypads().size(),
		"device40_b_pressed": Input.is_joy_button_pressed(40, JOY_BUTTON_B),
		"device40_left_x": Input.get_joy_axis(40, JOY_AXIS_LEFT_X),
	}
