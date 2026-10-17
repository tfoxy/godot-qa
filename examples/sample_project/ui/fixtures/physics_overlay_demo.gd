extends Node2D

@onready var overlay_actor: StaticBody2D = $OverlayActor
@onready var offset_collider_actor: StaticBody2D = $OffsetColliderActor
@onready var oversized_collider_actor: StaticBody2D = $OversizedColliderActor
@onready var trigger_area: Area2D = $TriggerArea
@onready var decorative_padding_actor: StaticBody2D = $DecorativePaddingActor


func _ready() -> void:
	_register_qa_node(self, "physics_overlay_demo")
	_register_qa_node(overlay_actor, "overlay_actor")
	_register_qa_node(offset_collider_actor, "offset_collider_actor")
	_register_qa_node(oversized_collider_actor, "oversized_collider_actor")
	_register_qa_node(trigger_area, "trigger_area")
	_register_qa_node(decorative_padding_actor, "decorative_padding_actor")


func _register_qa_node(node: Node, qa_id: String) -> void:
	node.set_meta("qa_id", qa_id)
	node.add_to_group("qa.id.%s" % qa_id)


func get_qa_visual_footprint_state() -> Dictionary:
	return {
		"fixture": "physics_overlay_demo",
		"bounded": true,
		"max_entries": 6,
		"visual_footprints": [
			_rect_footprint("overlay_actor", overlay_actor, Vector2(72.0, 48.0), true, false),
			_rect_footprint("offset_collider_actor", offset_collider_actor, Vector2(64.0, 40.0), true, false),
			_rect_footprint("oversized_collider_actor", oversized_collider_actor, Vector2(52.0, 52.0), true, false),
			_circle_footprint("trigger_area", trigger_area, 48.0, false, false),
			_rect_footprint("decorative_padding_actor", decorative_padding_actor, Vector2(46.0, 46.0), true, false),
			_rect_footprint("decorative_padding_actor", decorative_padding_actor, Vector2(100.0, 72.0), false, true),
		],
	}


func get_qa_collision_visual_state() -> Dictionary:
	return get_qa_visual_footprint_state()


func _rect_footprint(logical_id: String, node: Node2D, size: Vector2, solid_visual_body: bool, decorative_padding: bool) -> Dictionary:
	var center := node.global_position
	var half := size * 0.5
	return {
		"logical_id": logical_id,
		"qa_id": logical_id,
		"shape": "rect",
		"world_position": _point(center),
		"world_points": [
			_point(center + Vector2(-half.x, -half.y)),
			_point(center + Vector2(half.x, -half.y)),
			_point(center + Vector2(half.x, half.y)),
			_point(center + Vector2(-half.x, half.y)),
		],
		"size": {"x": size.x, "y": size.y},
		"solid_visual_body": solid_visual_body,
		"decorative_padding": decorative_padding,
	}


func _circle_footprint(logical_id: String, node: Node2D, radius: float, solid_visual_body: bool, decorative_padding: bool) -> Dictionary:
	return {
		"logical_id": logical_id,
		"qa_id": logical_id,
		"shape": "circle",
		"world_position": _point(node.global_position),
		"radius": radius,
		"solid_visual_body": solid_visual_body,
		"decorative_padding": decorative_padding,
	}


func _point(value: Vector2) -> Dictionary:
	return {"x": value.x, "y": value.y}
