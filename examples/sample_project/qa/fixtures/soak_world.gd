extends Node2D

const QaArgs = preload("res://addons/godot_qa/qa_args.gd")
const QaRuntime = preload("res://addons/godot_qa/qa_runtime.gd")

var duration_seconds := 600.0
var mode := "normal"
var started_usec := 0
var epoch := 1
var tick := 0
var active := true
var phase := "running"
var node_count := 4
var reset_done := false
var spike_done := false

func _ready() -> void:
	for arg in QaArgs.effective_user_args():
		if arg.begins_with("--qa-soak-duration="):
			duration_seconds = maxf(1.0, float(arg.get_slice("=", 1)))
		if arg.begins_with("--qa-soak-mode="):
			mode = arg.get_slice("=", 1)
	started_usec = Time.get_ticks_usec()
	QaRuntime.register_state_provider("soak_fixture", self, "qa_soak_state")

func _process(_delta: float) -> void:
	var elapsed := float(Time.get_ticks_usec() - started_usec) / 1000000.0
	if elapsed >= duration_seconds * 0.5 and not reset_done:
		epoch += 1
		tick = 0
		reset_done = true
	var pause_seconds := minf(5.0, duration_seconds * 0.1)
	active = elapsed < duration_seconds * 0.5 or elapsed >= duration_seconds * 0.5 + pause_seconds
	phase = "paused" if not active else ("post_reset" if reset_done else "running")
	if mode == "spike" and elapsed >= duration_seconds * 0.25 and not spike_done:
		spike_done = true
		OS.delay_msec(180)
	if mode == "leak" and elapsed >= duration_seconds * 0.5:
		node_count = 20
	queue_redraw()

func _physics_process(_delta: float) -> void:
	var elapsed := float(Time.get_ticks_usec() - started_usec) / 1000000.0
	if mode == "stall" and elapsed >= duration_seconds * 0.25 and elapsed < duration_seconds * 0.4:
		phase = "stall"
		return
	if active:
		tick += 1

func _draw() -> void:
	var size := get_viewport_rect().size
	draw_rect(Rect2(Vector2.ZERO, size), Color(0.08, 0.11, 0.16), true)
	var progress := fmod(float(Time.get_ticks_usec() - started_usec) / 1000000.0, 5.0) / 5.0
	draw_circle(Vector2(size.x * (0.2 + 0.6 * progress), size.y * 0.5), minf(size.x, size.y) * 0.08, Color(0.3, 0.78, 0.9))

func qa_soak_state() -> Dictionary:
	return {"epoch": epoch, "tick": tick, "active": active, "phase": phase, "counts": {"nodes": node_count}}
