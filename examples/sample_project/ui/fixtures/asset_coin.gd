extends Node2D

var _sound_finished_count := 0


func _ready() -> void:
	set_meta("qa_id", "asset_coin")
	add_to_group("qa.id.asset_coin")


func qa_on_sound_finished() -> void:
	_sound_finished_count += 1


func qa_get_sound_finished_count() -> int:
	return _sound_finished_count
