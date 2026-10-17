extends Node2D

const FRAME_MSEC := 1000.0 / 60.0

@onready var actor: Node2D = $Actor
@onready var impact_marker: Node2D = $ImpactMarker
@onready var ui_anchor: Node2D = $UiAnchor

var frame_index := 0
var _events: Array = []
var _last_provider_call_msec := -1


func _ready() -> void:
	set_meta("qa_id", "event_source")
	add_to_group("qa.id.event_source")
	add_to_group("qa:event_source")
	actor.set_meta("qa_id", "event_actor")
	actor.add_to_group("qa.id.event_actor")
	impact_marker.set_meta("qa_id", "event_impact_marker")
	impact_marker.add_to_group("qa.id.event_impact_marker")
	ui_anchor.set_meta("qa_id", "event_ui_anchor")
	ui_anchor.add_to_group("qa.id.event_ui_anchor")


func _physics_process(_delta: float) -> void:
	frame_index += 1
	actor.position.x = 160.0 + float(frame_index % 90) * 2.0
	_record_events_for_frame(frame_index)


func get_qa_event_stream_state() -> Dictionary:
	var new_events := []
	var now_msec := Time.get_ticks_msec()
	var should_emit_events := _last_provider_call_msec < 0 or now_msec - _last_provider_call_msec >= 50
	_last_provider_call_msec = now_msec
	if should_emit_events:
		for event in _events:
			new_events.append(event.duplicate(true))
	return {
		"events": new_events,
		"frame_index": frame_index,
		"fixture": "event_trace_demo",
		"bounded": true,
	}


func _record_events_for_frame(frame: int) -> void:
	match frame:
		6:
			_append_event(frame, "gameplay", "gameplay.spawned", "event_actor", actor.global_position, {"phase": "spawn"})
		12:
			_append_event(frame, "ui", "ui.button_confirm", "event_ui_anchor", ui_anchor.global_position, {"control": "start"})
		14:
			_append_event(frame, "sfx", "sfx.ui_confirm", "event_ui_anchor", ui_anchor.global_position, {"cue": "confirm"})
		18:
			_append_event(frame, "collision", "collision.contact", "event_actor", impact_marker.global_position, {"surface": "demo_wall"})
		20:
			_append_event(frame, "vfx", "vfx.collision_spark", "event_impact_marker", impact_marker.global_position, {"particles": 8})
		24:
			_append_event(frame, "sfx", "sfx.collision_hit", "event_impact_marker", impact_marker.global_position, {"cue": "soft_hit"})
		42:
			_append_event(frame, "gameplay", "gameplay.powerup_collected", "event_actor", actor.global_position, {"powerup": "demo_boost"})
		54:
			_append_event(frame, "custom", "custom.telemetry_marker", "event_actor", actor.global_position, {"marker": "checkpoint_a"})
		_:
			if frame >= 72 and frame < 84:
				_append_event(frame, "sfx", "sfx.weapon_tick", "event_actor", actor.global_position, {"burst_index": frame - 72})


func _append_event(frame: int, category: String, event_type: String, logical_id: String, world_position: Vector2, payload: Dictionary) -> void:
	_events.append({
		"event_id": "%s:%03d" % [event_type, frame],
		"frame": frame,
		"time_msec": int(round(float(frame) * FRAME_MSEC)),
		"category": category,
		"event_type": event_type,
		"logical_id": logical_id,
		"position": {
			"x": world_position.x,
			"y": world_position.y,
		},
		"payload": payload.duplicate(true),
	})
