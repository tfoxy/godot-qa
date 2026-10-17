extends Node2D

@onready var probe: CharacterBody2D = $PhysicsProbe
@onready var visible_blocker: StaticBody2D = $VisibleBlocker
@onready var hidden_blocker: StaticBody2D = $HiddenBlocker
@onready var disabled_blocker: StaticBody2D = $DisabledBlocker
@onready var oversized_blocker: StaticBody2D = $OversizedBlocker
@onready var layer_mismatch_body: StaticBody2D = $LayerMismatchBody


func _ready() -> void:
	set_meta("qa_id", "physics_scene")
	add_to_group("qa.id.physics_scene")
	probe.set_meta("qa_id", "physics_probe")
	probe.set_meta("qa_visual_rect", {"x": 276.0, "y": 276.0, "width": 48.0, "height": 48.0})
	probe.add_to_group("qa.id.physics_probe")
	visible_blocker.set_meta("qa_id", "visible_blocker")
	visible_blocker.add_to_group("qa.id.visible_blocker")
	hidden_blocker.set_meta("qa_id", "hidden_blocker")
	hidden_blocker.add_to_group("qa.id.hidden_blocker")
	disabled_blocker.set_meta("qa_id", "disabled_blocker")
	disabled_blocker.add_to_group("qa.id.disabled_blocker")
	oversized_blocker.set_meta("qa_id", "oversized_blocker")
	oversized_blocker.add_to_group("qa.id.oversized_blocker")
	layer_mismatch_body.set_meta("qa_id", "layer_mismatch_body")
	layer_mismatch_body.add_to_group("qa.id.layer_mismatch_body")


func get_qa_physics_state() -> Dictionary:
	return {
		"fixture": "physics_inspect_demo",
		"probe": {"qa_id": "physics_probe", "expected_engine_shape": "CircleShape2D"},
		"cases": ["visible_shape", "hidden_collider", "disabled_collider", "oversized_collider", "layer_mask_mismatch"],
	}


func get_qa_collision_footprint_state() -> Dictionary:
	return {
		"fixture": "physics_inspect_demo",
		"target": "qa:physics_probe",
		"visual_width": 48.0,
		"collision_radius": 24.0,
	}
