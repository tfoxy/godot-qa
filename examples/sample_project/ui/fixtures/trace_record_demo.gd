extends Node2D

const QaRuntime = preload("res://addons/godot_qa/qa_runtime.gd")

@onready var probe: CharacterBody2D = $TraceProbe
@onready var hidden_blocker: StaticBody2D = $HiddenBlocker
@onready var layer_mismatch_body: StaticBody2D = $LayerMismatchBody

var frame_index := 0
var phase := "approach"


func _ready() -> void:
	set_meta("qa_id", "trace_scene")
	add_to_group("qa.id.trace_scene")
	probe.set_meta("qa_id", "trace_probe")
	probe.add_to_group("qa.id.trace_probe")
	hidden_blocker.set_meta("qa_id", "trace_hidden_blocker")
	hidden_blocker.add_to_group("qa.id.trace_hidden_blocker")
	layer_mismatch_body.set_meta("qa_id", "trace_layer_mismatch")
	layer_mismatch_body.add_to_group("qa.id.trace_layer_mismatch")
	QaRuntime.register_fixture_driver("trace_driver", self, "trace_fixture_driver", {"fixture": "trace_record_demo"})
	_update_probe_visual_rect()


func _physics_process(_delta: float) -> void:
	frame_index += 1
	var cycle_frame := frame_index % 72
	if cycle_frame < 36:
		phase = "approach"
		probe.velocity = Vector2(180.0, 0.0)
		probe.global_position.x = 240.0 + float(cycle_frame) * 3.0
	else:
		phase = "reverse"
		probe.velocity = Vector2(-220.0, 0.0)
		probe.global_position.x = 348.0 - float(cycle_frame - 36) * (220.0 / 60.0)
	probe.global_position.y = 240.0
	_update_probe_visual_rect()


func get_qa_physics_state() -> Dictionary:
	return {
		"fixture": "trace_record_demo",
		"frame_index": frame_index,
		"phase": phase,
		"probe": {
			"qa_id": "trace_probe",
			"position": probe.global_position,
			"velocity": probe.velocity,
		},
		"hidden_blocker": {
			"qa_id": "trace_hidden_blocker",
			"position": hidden_blocker.global_position,
			"visible": hidden_blocker.visible,
		},
		"cases": ["motion", "reverse", "hidden_collider", "layer_mask_mismatch"],
	}


func trace_fixture_driver(request: Dictionary) -> Dictionary:
	var action := str(request.get("action", ""))
	if action == "prepare_trace":
		frame_index = 0
		phase = "approach"
		probe.global_position = Vector2(240.0, 240.0)
		probe.velocity = Vector2.ZERO
		_update_probe_visual_rect()
		return {"ok": true, "data": {"phase": phase, "frame_index": frame_index}}
	return {
		"ok": false,
		"error": {
			"type": "unknown_fixture_action",
			"message": "trace fixture driver action is not supported",
			"data": {"action": action},
		},
	}


func _update_probe_visual_rect() -> void:
	probe.set_meta(
		"qa_visual_rect",
		{
			"x": probe.global_position.x - 18.0,
			"y": probe.global_position.y - 18.0,
			"width": 36.0,
			"height": 36.0,
		},
	)
