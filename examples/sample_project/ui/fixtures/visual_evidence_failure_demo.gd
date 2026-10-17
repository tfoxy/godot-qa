extends Control


class WorldSpaceVisualProxy:
	extends Node

	var visible := true

	func qa_visual_rect() -> Dictionary:
		return {
			"coordinate_space": "world_2d",
			"rect": {
				"x": 120.0,
				"y": 96.0,
				"width": 180.0,
				"height": 120.0,
			},
		}

	func qa_set_visual_visible(next_visible: bool) -> void:
		visible = next_visible

	func qa_visual_visible() -> bool:
		return visible


class UnsupportedVisibleTarget:
	extends Node

	var visible := true


func _ready() -> void:
	set_meta("qa_id", "visual_evidence_failure_demo")
	var world_space_target := WorldSpaceVisualProxy.new()
	world_space_target.name = "WorldSpaceVisualProxy"
	world_space_target.set_meta("qa_id", "world_space_visual_actor")
	add_child(world_space_target)
	var unsupported_target := UnsupportedVisibleTarget.new()
	unsupported_target.name = "UnsupportedVisibleTarget"
	unsupported_target.set_meta("qa_id", "unsupported_visible_visual_actor")
	add_child(unsupported_target)
