extends Control


const TARGETS := {
	"readable_target": {
		"path": "Canvas/ReadableZone/ReadableTarget",
		"background_path": "Canvas/ReadableZone",
		"expected_visual_risk": "none",
	},
	"low_contrast_target": {
		"path": "Canvas/LowContrastZone/LowContrastTarget",
		"background_path": "Canvas/LowContrastZone",
		"expected_visual_risk": "low_contrast",
	},
	"tiny_target": {
		"path": "Canvas/TinyTarget",
		"background_path": "Canvas",
		"expected_visual_risk": "too_small",
	},
	"offscreen_target": {
		"path": "Canvas/OffscreenTarget",
		"background_path": "Canvas",
		"expected_visual_risk": "offscreen",
	},
}


func _ready() -> void:
	set_meta("qa_id", "visual_audit_demo")
	add_to_group("qa.id.visual_audit_demo")

	for qa_id in TARGETS:
		var target := get_node(String(TARGETS[qa_id]["path"]))
		target.set_meta("qa_id", qa_id)
		target.add_to_group("qa.id.%s" % qa_id)


func get_qa_visual_state() -> Dictionary:
	return _build_visual_state()


func get_qa_readability_state() -> Dictionary:
	return _build_visual_state()


func _build_visual_state() -> Dictionary:
	var targets := {}
	for qa_id in TARGETS:
		var config: Dictionary = TARGETS[qa_id]
		var target := get_node(String(config["path"]))
		var background := get_node(String(config["background_path"]))
		targets[qa_id] = {
			"qa_id": qa_id,
			"node_path": str(target.get_path()),
			"global_rect": _rect_to_dictionary(target.get_global_rect()),
			"target_color": _color_rect_color(target),
			"local_background_color": _color_rect_color(background),
			"expected_visual_risk": config["expected_visual_risk"],
		}
	return {
		"fixture": "visual_audit_demo",
		"viewport_size": [
			int(get_viewport_rect().size.x),
			int(get_viewport_rect().size.y),
		],
		"targets": targets,
	}


func _rect_to_dictionary(rect: Rect2) -> Dictionary:
	return {
		"x": rect.position.x,
		"y": rect.position.y,
		"width": rect.size.x,
		"height": rect.size.y,
	}


func _color_rect_color(node: Node) -> Array:
	if node is ColorRect:
		var color: Color = node.color
		return [color.r, color.g, color.b, color.a]
	return []
