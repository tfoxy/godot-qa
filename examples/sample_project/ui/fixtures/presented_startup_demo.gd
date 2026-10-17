extends Node2D

## Static fixture for compositor-versus-viewport startup checks.
## The negative variant changes viewport pixels without swapping buffers.
@export var stale_no_swap := false

var _stale_red := false


func _ready() -> void:
	OS.low_processor_usage_mode = true
	_stale_red = stale_no_swap
	queue_redraw()
	if stale_no_swap:
		_recreate_stale_presentation()


func _draw() -> void:
	var area := get_viewport().get_visible_rect().size
	draw_rect(Rect2(Vector2.ZERO, area), Color(0.45, 0.10, 0.17) if _stale_red else Color(0.08, 0.24, 0.37))
	for index in range(4):
		var x := area.x * (0.10 + float(index) * 0.21)
		var width := area.x * 0.14
		draw_rect(Rect2(x, area.y * 0.20, width, area.y * 0.60), Color.from_hsv(float(index) / 4.0, 0.65, 0.85))


func _recreate_stale_presentation() -> void:
	# Let the normal QA runner present red, then reproduce old no-swap drawing.
	await get_tree().create_timer(0.8).timeout
	RenderingServer.set_render_loop_enabled(false)
	_stale_red = false
	queue_redraw()
	RenderingServer.force_draw(false)
