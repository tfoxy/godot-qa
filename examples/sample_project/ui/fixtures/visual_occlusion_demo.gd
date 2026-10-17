extends Control

@onready var important_actor: ColorRect = $Canvas/ImportantActor
@onready var partial_occluder: ColorRect = $Canvas/PartialOccluder
@onready var full_occluder: ColorRect = $Canvas/FullOccluder

class MetadataVisualProxy:
	extends Node

	var actor: ColorRect

	func qa_visual_rect() -> Dictionary:
		if actor == null:
			return {}
		var rect := actor.get_global_rect()
		return {
			"x": rect.position.x,
			"y": rect.position.y,
			"width": rect.size.x,
			"height": rect.size.y,
		}

	func qa_visual_visible() -> bool:
		return actor != null and actor.visible

	func qa_set_visual_visible(visible: bool) -> void:
		if actor != null:
			actor.visible = visible


class InvalidMetadataVisualProxy:
	extends Node

	func qa_visual_rect() -> Dictionary:
		return {"x": 12.0}

	func qa_set_visual_visible(_visible: bool) -> void:
		pass


func _ready() -> void:
	set_meta("qa_id", "visual_occlusion_demo")
	important_actor.set_meta("qa_id", "important_actor")
	partial_occluder.set_meta("qa_id", "partial_occluder")
	full_occluder.set_meta("qa_id", "full_occluder")
	var metadata_proxy := MetadataVisualProxy.new()
	metadata_proxy.name = "MetadataVisualProxy"
	metadata_proxy.actor = important_actor
	metadata_proxy.set_meta("qa_id", "metadata_visual_actor")
	add_child(metadata_proxy)
	var invalid_metadata_proxy := InvalidMetadataVisualProxy.new()
	invalid_metadata_proxy.name = "InvalidMetadataVisualProxy"
	invalid_metadata_proxy.set_meta("qa_id", "invalid_metadata_visual_actor")
	add_child(invalid_metadata_proxy)
	qa_show_partial_occlusion()


func qa_show_partial_occlusion() -> String:
	important_actor.visible = true
	important_actor.z_index = 1
	partial_occluder.visible = true
	partial_occluder.z_index = 2
	full_occluder.visible = false
	full_occluder.z_index = 3
	return "partial"


func qa_show_full_occlusion() -> String:
	important_actor.visible = true
	important_actor.z_index = 1
	partial_occluder.visible = false
	partial_occluder.z_index = 2
	full_occluder.visible = true
	full_occluder.z_index = 3
	return "full"


func qa_swap_z_order() -> String:
	important_actor.visible = true
	partial_occluder.visible = true
	full_occluder.visible = false
	var actor_z := important_actor.z_index
	important_actor.z_index = partial_occluder.z_index
	partial_occluder.z_index = actor_z
	return "swapped"
