extends SceneTree

# Standalone rendered acceptance: godot --path <project> --script this_file.gd.
const Burst = preload("res://addons/godot_qa/qa_screenshot_burst.gd")
const Runtime = preload("res://addons/godot_qa/qa_runtime.gd")
class SlowProvider extends RefCounted:
	func get_qa_state() -> Dictionary:
		OS.delay_msec(120)
		return {"ready": true}


var fixture: Control
var ticks := 0
var resize_on_tick := 0
var resize_content := false
var output_root := ""
var results: Dictionary = {}


func _initialize() -> void:
	call_deferred("_run")


func _tick() -> void:
	ticks += 1
	fixture.set_meta("counter", ticks)
	if resize_on_tick > 0 and ticks == resize_on_tick:
		root.size = Vector2i(360, 180)
		if resize_content:
			root.content_scale_size = Vector2i(360, 180)


func _options(frames: Array) -> Dictionary:
	return {"timing_mode": "frames", "capture_frames": frames, "max_bytes": 536870912, "max_seconds": 3.0, "state_guards": {}}


func _record(name: String, result: Dictionary, valid: bool) -> void:
	results[name] = {"valid": valid, "result": result}


func _run() -> void:
	root.size = Vector2i(320, 180)
	root.content_scale_size = Vector2i(320, 180)
	Engine.max_fps = 60
	fixture = Control.new()
	fixture.name = "BurstFixture"
	root.add_child(fixture)
	Runtime.register_qa_id(fixture, "burst_fixture")
	var left := ColorRect.new()
	left.color = Color(0.8, 0.1, 0.2)
	left.size = Vector2(160, 180)
	fixture.add_child(left)
	var right := ColorRect.new()
	right.color = Color(0.1, 0.4, 0.8)
	right.position = Vector2(160, 0)
	right.size = Vector2(160, 180)
	fixture.add_child(right)
	process_frame.connect(_tick)
	await process_frame
	await process_frame
	var runtime := Runtime.new(fixture, root)
	output_root = "user://godot_qa/burst_acceptance_%d" % OS.get_process_id()
	var regular: Dictionary = await Burst.new().capture(root, runtime, _options([0, 2, 4]), output_root + "/regular")
	var frames: Array = regular.get("frames", [])
	var valid := bool(regular.get("complete", false)) and frames.size() == 3
	if valid:
		valid = frames[0].get("draw_ordinal") == 0 and frames[1].get("draw_ordinal") == 2 and frames[2].get("draw_ordinal") == 4
		var image := Image.load_from_file(str(frames[0].get("artifacts", {}).get("screenshot", "")))
		valid = valid and image != null and image.get_size() == Vector2i(320, 180) and image.get_pixel(20, 20).r > image.get_pixel(250, 20).r
	_record("frame_schedule", regular, valid)
	var timed_options := _options([0, 1, 2])
	timed_options["timing_mode"] = "seconds"
	timed_options["seconds"] = 0.1
	timed_options["fps"] = 20.0
	var timed: Dictionary = await Burst.new().capture(root, runtime, timed_options, output_root + "/timed")
	var timed_frames: Array = timed.get("frames", [])
	valid = bool(timed.get("complete", false)) and timed_frames.size() == 3
	if valid:
		valid = float(timed_frames[1].get("elapsed_msec", 0)) >= 50.0 and float(timed_frames[2].get("elapsed_msec", 0)) >= 100.0
	_record("monotonic_schedule", timed, valid)
	var many_frames: Array = []
	for index in range(97):
		many_frames.append(index)
	var frame_limit: Dictionary = await Burst.new().capture(root, runtime, _options(many_frames), output_root + "/frame_limit")
	_record("frame_limit_partial", frame_limit, not bool(frame_limit.get("complete", true)) and frame_limit.get("frames", []).size() == 96 and frame_limit.get("stop_reason") == "screenshot_burst.frame_limit")
	var memory_options := _options([0, 1, 2])
	memory_options["max_bytes"] = 320 * 180 * 4
	var memory: Dictionary = await Burst.new().capture(root, runtime, memory_options, output_root + "/memory")
	_record("memory_partial", memory, not bool(memory.get("complete", true)) and memory.get("frames", []).size() == 1 and memory.get("stop_reason") == "screenshot_burst.memory_limit")
	var guarded_options := _options([0, 1])
	guarded_options["state_guards"] = {"require_hidden": ["qa:burst_fixture"]}
	var guarded: Dictionary = await Burst.new().capture(root, runtime, guarded_options, output_root + "/guard")
	_record("guard_failure", guarded, not bool(guarded.get("complete", true)) and guarded.get("frames", []).is_empty() and guarded.get("stop_reason") == "screenshot_state_guard_failed")
	var stop_options := _options([0, 1, 2, 3, 4, 5])
	stop_options["stop_after_state"] = {"target": "qa:burst_fixture", "source": "metadata", "path": "counter", "operator": "greater_than", "expected": ticks + 2}
	var stopped: Dictionary = await Burst.new().capture(root, runtime, stop_options, output_root + "/stop")
	_record("stop_predicate", stopped, bool(stopped.get("complete", false)) and bool(stopped.get("stop_condition_matched", false)) and stopped.get("frames", []).size() < 6)
	var slow_provider := SlowProvider.new()
	Runtime.register_state_provider("burst_slow_guard", slow_provider, "get_qa_state")
	var slow_options := _options([0])
	slow_options["max_seconds"] = 0.05
	slow_options["state_guards"] = {"require_state": {"source": "provider", "provider": "burst_slow_guard", "path": "ready", "operator": "equals", "expected": true}}
	var slow: Dictionary = await Burst.new().capture(root, runtime, slow_options, output_root + "/slow_guard")
	_record("final_guard_deadline", slow, not bool(slow.get("complete", true)) and slow.get("frames", []).size() == 1 and slow.get("stop_reason") == "screenshot_burst.capture_deadline")
	var slow_stop_options := _options([0])
	slow_stop_options["max_seconds"] = 0.05
	slow_stop_options["stop_after_state"] = slow_options["state_guards"]["require_state"]
	var slow_stop: Dictionary = await Burst.new().capture(root, runtime, slow_stop_options, output_root + "/slow_stop")
	_record("final_stop_deadline", slow_stop, not bool(slow_stop.get("complete", true)) and slow_stop.get("frames", []).size() == 1 and slow_stop.get("stop_reason") == "screenshot_burst.capture_deadline")
	Runtime.unregister_state_provider("burst_slow_guard")
	var deadline_options := _options([0, 10000])
	deadline_options["max_seconds"] = 0.1
	var deadline: Dictionary = await Burst.new().capture(root, runtime, deadline_options, output_root + "/deadline")
	_record("deadline_partial", deadline, not bool(deadline.get("complete", true)) and deadline.get("frames", []).size() == 1 and deadline.get("stop_reason") == "screenshot_burst.capture_deadline")
	resize_on_tick = ticks + 3
	var resized: Dictionary = await Burst.new().capture(root, runtime, _options([0, 4, 8]), output_root + "/resize")
	_record("resize_partial", resized, not bool(resized.get("complete", true)) and resized.get("frames", []).size() == 1 and resized.get("stop_reason") == "screenshot_burst.viewport_resized")
	resize_on_tick = 0
	root.size = Vector2i(320, 180)
	await process_frame
	await process_frame
	resize_content = true
	resize_on_tick = ticks + 3
	var pixel_resized: Dictionary = await Burst.new().capture(root, runtime, _options([0, 4, 8]), output_root + "/pixel_resize")
	var before_pixels: Image = null
	if pixel_resized.get("frames", []).size() == 1:
		before_pixels = Image.load_from_file(str(pixel_resized.frames[0].artifacts.screenshot))
	var after_pixels := root.get_texture().get_image()
	var after_path := output_root + "/pixel_resize_after.png"
	var after_write := after_pixels.save_png(after_path)
	var after_decoded := Image.load_from_file(after_path) if after_write == OK else null
	pixel_resized["resize_pixel_evidence"] = {"before_size": before_pixels.get_size() if before_pixels != null else Vector2i.ZERO, "after_size": after_decoded.get_size() if after_decoded != null else Vector2i.ZERO, "after_path": after_path}
	_record("pixel_resize_partial", pixel_resized, not bool(pixel_resized.get("complete", true)) and pixel_resized.get("frames", []).size() == 1 and pixel_resized.get("stop_reason") == "screenshot_burst.viewport_resized" and before_pixels != null and before_pixels.get_size() == Vector2i(320, 180) and after_decoded != null and after_decoded.get_size() == Vector2i(360, 180))
	resize_on_tick = 0
	resize_content = false
	root.size = Vector2i(320, 180)
	root.content_scale_size = Vector2i(320, 180)
	await process_frame
	await process_frame
	var blocked_path := output_root + "/blocked"
	var blocked_file := FileAccess.open(blocked_path, FileAccess.WRITE)
	blocked_file.store_string("not a directory")
	blocked_file.close()
	var blocked: Dictionary = await Burst.new().capture(root, runtime, _options([0]), blocked_path)
	_record("write_failure", blocked, not bool(blocked.get("complete", true)) and blocked.get("stop_reason") == "artifact_write_failed")
	var ok := true
	for name in results:
		ok = ok and bool(results[name].get("valid", false))
	runtime.teardown()
	print(JSON.stringify({"ok": ok, "output": ProjectSettings.globalize_path(output_root), "cases": results}))
	quit(0 if ok else 1)
