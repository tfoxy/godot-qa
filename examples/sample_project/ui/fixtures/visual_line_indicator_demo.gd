extends Control

@onready var line_indicator: Line2D = $Canvas/LineIndicator
@onready var zero_length_indicator: Line2D = $Canvas/ZeroLengthIndicator


func _ready() -> void:
	set_meta("qa_id", "visual_line_indicator_demo")
	line_indicator.set_meta("qa_id", "line_indicator")
	zero_length_indicator.set_meta("qa_id", "zero_length_line_indicator")
	qa_show_valid_line()


func qa_show_valid_line() -> String:
	line_indicator.visible = true
	line_indicator.width = 18.0
	line_indicator.points = PackedVector2Array([
		Vector2(260.0, 420.0),
		Vector2(500.0, 260.0),
		Vector2(760.0, 360.0),
		Vector2(980.0, 180.0),
	])
	zero_length_indicator.visible = false
	return "valid"


func qa_show_zero_length_line() -> String:
	line_indicator.visible = false
	zero_length_indicator.visible = true
	zero_length_indicator.width = 18.0
	zero_length_indicator.points = PackedVector2Array([
		Vector2(640.0, 360.0),
		Vector2(640.0, 360.0),
	])
	return "zero_length"
