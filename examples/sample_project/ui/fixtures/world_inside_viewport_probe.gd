extends Node2D

@onready var world_player: CharacterBody2D = $WorldPlayer
@onready var offscreen_marker: Node2D = $OffscreenMarker


func _ready() -> void:
	set_meta("qa_id", "world_probe")
	world_player.set_meta("qa_id", "world_probe_player")
	offscreen_marker.set_meta("qa_id", "world_probe_offscreen_marker")
