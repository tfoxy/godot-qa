extends Control

const QaRuntime = preload("res://addons/godot_qa/qa_runtime.gd")

const TRACK_LEFT := 112.0
const TRACK_RIGHT := 1168.0
const CYCLE_FRAMES := 120.0

@onready var marker: ColorRect = $Canvas/Marker
@onready var phase_fill: ColorRect = $Canvas/PhaseBar/Fill
@onready var frame_label: Label = $Canvas/FrameLabel
@onready var pulse: ColorRect = $Canvas/Pulse

var fixture_status := "ready"


func _ready() -> void:
	set_meta("qa_id", "screenshot_sequence_demo")
	$Canvas.set_meta("qa_id", "screenshot_sequence_canvas")
	$Canvas/Track.set_meta("qa_id", "screenshot_sequence_track")
	$Canvas/Marker.set_meta("qa_id", "screenshot_sequence_marker")
	$Canvas/PhaseBar.set_meta("qa_id", "screenshot_sequence_phase_bar")
	$Canvas/PhaseBar/Fill.set_meta("qa_id", "screenshot_sequence_phase_fill")
	$Canvas/Pulse.set_meta("qa_id", "screenshot_sequence_pulse")
	$Canvas/FrameLabel.set_meta("qa_id", "screenshot_sequence_frame_label")
	QaRuntime.register_fixture_driver("screenshot_sequence_driver", self, "screenshot_sequence_fixture_driver", {"fixture": "screenshot_sequence_demo"})
	_update_frame_state()


func _process(_delta: float) -> void:
	_update_frame_state()


func _update_frame_state() -> void:
	var frame := Engine.get_process_frames()
	var phase := fmod(float(frame), CYCLE_FRAMES) / CYCLE_FRAMES
	var x: float = lerp(TRACK_LEFT, TRACK_RIGHT, phase)
	marker.position.x = x - marker.size.x * 0.5
	phase_fill.size.x = 880.0 * phase
	pulse.color = Color(0.95, 0.29 + 0.46 * phase, 0.18, 1.0)
	frame_label.text = "Frame %04d" % frame


func screenshot_sequence_fixture_driver(request: Dictionary) -> Dictionary:
	var action := str(request.get("action", ""))
	var params: Dictionary = request.get("params", {}) if request.get("params", {}) is Dictionary else {}
	if action == "set_status":
		fixture_status = str(params.get("status", "ready"))
		return {"ok": true, "data": {"status": fixture_status}}
	return {
		"ok": false,
		"error": {
			"type": "unknown_fixture_action",
			"message": "screenshot sequence fixture driver action is not supported",
			"data": {"action": action},
		},
	}
