extends Control

@onready var visible_target: ColorRect = $Canvas/VisibleTarget
@onready var hidden_tree_visible_target: ColorRect = $Canvas/HiddenButTreeVisibleTarget
@onready var low_information_target: ColorRect = $Canvas/LowInformationTarget


class UnsupportedCustomTarget:
	extends Node

	var visible := true


func _ready() -> void:
	set_meta("qa_id", "saved_snapshot_pixels_demo")
	visible_target.set_meta("qa_id", "saved_pixels_visible_target")
	hidden_tree_visible_target.set_meta("qa_id", "saved_pixels_tree_visible_empty_target")
	low_information_target.set_meta("qa_id", "saved_pixels_low_information_target")
	var unsupported_target := UnsupportedCustomTarget.new()
	unsupported_target.name = "UnsupportedCustomTarget"
	unsupported_target.set_meta("qa_id", "saved_pixels_unsupported_target")
	add_child(unsupported_target)


func qa_show_tree_visible_empty_target() -> String:
	visible_target.visible = false
	hidden_tree_visible_target.visible = true
	low_information_target.visible = false
	return "tree_visible_empty"


func qa_show_visible_target() -> String:
	visible_target.visible = true
	hidden_tree_visible_target.visible = false
	low_information_target.visible = false
	return "visible"


func qa_show_low_information_target() -> String:
	visible_target.visible = false
	hidden_tree_visible_target.visible = false
	low_information_target.visible = true
	return "low_information"
