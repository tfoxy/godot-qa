extends Node

const Runtime = preload("res://addons/godot_qa/qa_runtime.gd")
class ChildProvider extends Node:
	func _ready() -> void:
		Runtime.register_state_provider("child-provider", self, "qa_state")

	func qa_state() -> Dictionary:
		return {"instance": get_instance_id()}

var registration: Dictionary = {}


func _ready() -> void:
	registration = Runtime.register_state_provider("scene-provider", self, "qa_state")
	var child := ChildProvider.new()
	child.name = "ChildProvider"
	add_child(child)


func _exit_tree() -> void:
	Runtime.unregister_state_provider("scene-provider")


func qa_state() -> Dictionary:
	return {"instance": get_instance_id()}
