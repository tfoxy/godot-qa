extends SceneTree
## Run rendered for capture, transforms, write failures, and visibility cleanup:
## godot --path examples/sample_project --script res://qa/tests/visual_diagnostics_test.gd
## --headless also runs decoded equivalence and scan benchmarks.

const Assertions = preload("res://addons/godot_qa/qa_assertions_impl.gd")
const QaRuntime = preload("res://addons/godot_qa/qa_runtime.gd")
const QaSnapshot = preload("res://addons/godot_qa/qa_snapshot.gd")
var _failures: Array[String] = []
var _timings: Array = []
var _checks := 0
var _dir := "user://godot_qa/visual-diagnostics-%s" % OS.get_process_id()


func _initialize() -> void:
	call_deferred("_run")


func _check(ok: bool, label: String) -> void:
	_checks += 1
	if not ok:
		_failures.append(label)
		push_error(label)


func _legacy(before: Image, hidden: Image, rect: Rect2) -> Dictionary:
	var x0 := maxi(0, int(floor(rect.position.x)))
	var y0 := maxi(0, int(floor(rect.position.y)))
	var x1 := mini(before.get_width(), int(ceil(rect.end.x)))
	var y1 := mini(before.get_height(), int(ceil(rect.end.y)))
	var total := maxi((x1 - x0) * (y1 - y0), 1)
	var count := 0
	for y in range(y0, y1):
		for x in range(x0, x1):
			if _differ(before.get_pixel(x, y), hidden.get_pixel(x, y)):
				count += 1
	var mask := Image.create(before.get_width(), before.get_height(), false, Image.FORMAT_RGBA8)
	var overlay := Image.create(before.get_width(), before.get_height(), false, Image.FORMAT_RGBA8)
	overlay.blit_rect(before, Rect2i(Vector2i.ZERO, before.get_size()), Vector2i.ZERO)
	for y in range(y0, y1):
		for x in range(x0, x1):
			if _differ(before.get_pixel(x, y), hidden.get_pixel(x, y)):
				mask.set_pixel(x, y, Color.WHITE)
				overlay.set_pixel(x, y, overlay.get_pixel(x, y).lerp(Color(1.0, 0.0, 0.0, 1.0), 0.55))
	return {"changed": count, "ratio": float(count) / float(total), "mask": mask, "overlay": overlay}


func _differ(a: Color, b: Color) -> bool:
	return abs(a.r - b.r) > 0.01 or abs(a.g - b.g) > 0.01 or abs(a.b - b.b) > 0.01 or abs(a.a - b.a) > 0.01


func _equivalence(assertions: RefCounted, size: Vector2i, rect: Rect2, identical: bool, label: String) -> void:
	var before := Image.create(size.x, size.y, false, Image.FORMAT_RGBA8)
	before.fill(Color(0.12, 0.3, 0.6, 1.0))
	var hidden := before.duplicate() as Image
	if not identical:
		hidden.fill_rect(Rect2i(Vector2i.ZERO, size / 2), Color(0.8, 0.1, 0.2, 0.4))
	var started := Time.get_ticks_usec()
	var legacy := _legacy(before, hidden, rect)
	var legacy_mask_path := "%s/%s_legacy_mask.png" % [_dir, label]
	var legacy_overlay_path := "%s/%s_legacy_overlay.png" % [_dir, label]
	_check((legacy.get("mask") as Image).save_png(legacy_mask_path) == OK, "%s legacy mask write" % label)
	_check((legacy.get("overlay") as Image).save_png(legacy_overlay_path) == OK, "%s legacy overlay write" % label)
	var legacy_usec := Time.get_ticks_usec() - started
	started = Time.get_ticks_usec()
	var mask_path := "%s/%s_mask.png" % [_dir, label]
	var overlay_path := "%s/%s_overlay.png" % [_dir, label]
	var fused: Dictionary = assertions.call("_write_visual_diagnostics", before, hidden, rect, mask_path, overlay_path)
	var fused_usec := Time.get_ticks_usec() - started
	_check(fused.get("changed") == legacy.get("changed") and fused.get("ratio") == legacy.get("ratio"), "%s metrics remain exact" % label)
	_check((fused.get("artifact_failures") as Array).is_empty(), "%s artifacts write successfully" % label)
	for pair in [[mask_path, legacy_mask_path], [overlay_path, legacy_overlay_path]]:
		_check(Image.load_from_file(pair[0]).get_data() == Image.load_from_file(pair[1]).get_data(), "%s decoded %s equivalent" % [label, pair[0]])
	_timings.append({"size": [size.x, size.y], "case": label, "legacy_usec": legacy_usec, "fused_usec": fused_usec})


func _params(label: String, min_count: int) -> Dictionary:
	return {"target": "qa:visual_probe", "snapshot": label, "visual_dir": _dir, "min_count": min_count}


func _blocked_path(assertions: RefCounted, label: String, suffix: String, command := "assert.visible_pixels") -> String:
	var name: String = assertions.call("_sanitize_visual_name", "%s_%s_qa:visual_probe" % [command, label])
	var path := "%s/%s_%s.png" % [_dir, name, suffix]
	_check(DirAccess.make_dir_recursive_absolute(path) == OK, "create blocked %s PNG path" % suffix)
	return path


func _rendered(runtime: RefCounted, assertions: RefCounted, scene: Node) -> void:
	root.content_scale_size = Vector2i(1280, 720)
	root.content_scale_mode = Window.CONTENT_SCALE_MODE_CANVAS_ITEMS
	root.content_scale_aspect = Window.CONTENT_SCALE_ASPECT_KEEP
	root.mode = Window.MODE_WINDOWED
	root.size = Vector2i(1920, 1080)
	var background := ColorRect.new()
	background.color = Color(0.1, 0.2, 0.3, 1.0)
	background.size = Vector2(1280, 720)
	scene.add_child(background)
	var accent := ColorRect.new()
	accent.color = Color(0.4, 0.7, 0.2, 1.0)
	accent.size = Vector2(150, 150)
	scene.add_child(accent)
	var probe := ColorRect.new()
	probe.set_meta("qa_id", "visual_probe")
	probe.color = Color(0.8, 0.2, 0.1, 1.0)
	probe.position = Vector2(300, 200)
	probe.size = Vector2(100, 80)
	scene.add_child(probe)
	for _i in range(4):
		await process_frame
	var result: Dictionary = await runtime.execute("assert.visible_pixels", _params("scaled", 1))
	_check(bool(result.get("ok", false)), "scaled target passes with retained image: %s" % result)
	_check(probe.visible, "successful capture restores target visibility")
	var screenshot: String = result.get("data", {}).get("artifacts", {}).get("original", "")
	var reported_size: Array = result.get("data", {}).get("screenshot_size", [])
	var image_size := Image.load_from_file(screenshot).get_size()
	_check(reported_size == [image_size.x, image_size.y] and image_size != root.content_scale_size, "scaled screenshot dimensions match reported captured dimensions %s" % image_size)
	var captured: Dictionary = await assertions.call("_capture_visual_image", "%s/direct.png" % _dir)
	_check(bool(captured.get("ok", false)) and (captured.get("image") as Image).get_data() == Image.load_from_file("%s/direct.png" % _dir).get_data(), "retained capture exactly matches decoded written PNG")
	for suffix in ["original", "hidden", "mask", "overlay"]:
		var label := "blocked_%s" % suffix
		var path := _blocked_path(assertions, label, suffix)
		result = await runtime.execute("assert.visible_pixels", _params(label, 1))
		var error: Dictionary = result.get("error", {})
		_check(not bool(result.get("ok", true)) and error.get("type") == "screenshot_write_failed", "required %s write failure prevents passing verdict: %s" % [suffix, result])
		var failures: Array = error.get("data", {}).get("artifact_failures", [])
		_check(not failures.is_empty() and failures[0].get("attempted_screenshot_path") == path and int(failures[0].get("screenshot_error", OK)) != OK, "required %s write failure identifies attempted path and Godot error" % suffix)
		_check(probe.visible, "failed %s write restores target visibility" % suffix)
	_blocked_path(assertions, "mismatch", "mask")
	result = await runtime.execute("assert.visible_pixels", _params("mismatch", 999999))
	_check(result.get("error", {}).get("type") == "visible_pixels_mismatch" and not (result.get("error", {}).get("data", {}).get("artifact_failures", []) as Array).is_empty(), "mismatch remains primary when artifact write also fails")
	_blocked_path(assertions, "occlusion", "mask", "assert.not_occluded")
	result = await runtime.execute("assert.not_occluded", _params("occlusion", 1))
	_check(result.get("error", {}).get("type") == "screenshot_write_failed" and probe.visible, "required occlusion diagnostics fail passing assertion and restore visibility")
	result = await runtime.execute("assert.not_occluded", _params("occlusion", 999999))
	_check(result.get("error", {}).get("type") == "not_occluded_mismatch" and not (result.get("error", {}).get("data", {}).get("artifact_failures", []) as Array).is_empty(), "occlusion mismatch remains primary when artifact write also fails")
	probe.visible = false
	result = await runtime.execute("assert.visible_pixels", _params("already_hidden", 0))
	_check(bool(result.get("ok", false)) and not probe.visible and int(result.get("data", {}).get("visible_pixel_count", -1)) == 0, "identical rendered images preserve hidden visibility and zero contribution")
	probe.visible = true
	probe.position = Vector2(-500, -500)
	result = await runtime.execute("assert.visible_pixels", _params("offscreen", 1))
	_check(result.get("error", {}).get("type") == "visible_pixels_mismatch" and probe.visible, "offscreen target fails threshold and restores visibility")
	probe.position = Vector2(300, 200)
	var order_name: String = assertions.call("_sanitize_visual_name", "assert.renders_above_qa:visual_probe_qa:visual_background")
	background.set_meta("qa_id", "visual_background")
	_check(DirAccess.make_dir_recursive_absolute("%s/%s_order_overlay.png" % [_dir, order_name]) == OK, "block optional order overlay")
	result = await runtime.execute("assert.renders_above", {"target": "qa:visual_probe", "other": "qa:visual_background", "snapshot": "order", "visual_dir": _dir})
	_check(bool(result.get("ok", false)) and result.get("data", {}).has("capture_warning"), "optional canvas-order overlay failure remains capture warning: %s" % result)


func _run() -> void:
	_check(DirAccess.make_dir_recursive_absolute(_dir) == OK, "create diagnostics artifact directory")
	var sample := Image.create(2, 2, false, Image.FORMAT_RGBA8)
	sample.fill(Color.RED)
	_check(QaSnapshot._save_captured_viewport_screenshot(root, sample, "%s/write-check.png" % _dir, false) == OK, "captured writer flushes valid PNG successfully")
	if OS.get_name() == "Linux":
		_check(QaSnapshot._save_captured_viewport_screenshot(root, sample, "/dev/full", false) != OK, "captured writer detects buffered disk-full write failure")
	var scene := Node.new()
	root.add_child(scene)
	var runtime := QaRuntime.new(scene, root)
	var assertions := Assertions.new(runtime, root)
	var rendered_only := OS.get_cmdline_user_args().has("--rendered-only")
	if not rendered_only:
		for size in [Vector2i(1280, 720), Vector2i(1920, 1080), Vector2i(3840, 2160)]:
			_equivalence(assertions, size, Rect2(Vector2.ZERO, size), false, "%sx%s" % [size.x, size.y])
		_equivalence(assertions, Vector2i(43, 31), Rect2(-3.5, 2.5, 30.7, 40), false, "clipped_fractional")
		_equivalence(assertions, Vector2i(43, 31), Rect2(100, 100, 20, 20), false, "offscreen")
		_equivalence(assertions, Vector2i(43, 31), Rect2(0, 0, 43, 31), true, "identical")
	if DisplayServer.get_name() != "headless":
		await _rendered(runtime, assertions, scene)
	elif rendered_only:
		_check(false, "--rendered-only requires a display backend")
	assertions.teardown()
	assertions = null
	runtime.teardown()
	runtime = null
	scene.queue_free()
	await process_frame
	print(JSON.stringify({"test": "visual_diagnostics", "ok": _failures.is_empty(), "checks": _checks, "failures": _failures, "timings": _timings, "artifacts": ProjectSettings.globalize_path(_dir)}))
	quit(0 if _failures.is_empty() else 1)
