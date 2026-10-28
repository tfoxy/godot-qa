extends SceneTree

const QaSnapshot = preload("res://addons/godot_qa/qa_snapshot.gd")


func _initialize() -> void:
	var failures: Array[String] = []
	var timings: Array[Dictionary] = []
	var artifact_dir := "user://godot_qa/snapshot-equivalence-%s" % OS.get_process_id()
	if DirAccess.make_dir_recursive_absolute(artifact_dir) != OK:
		push_error("could not create snapshot equivalence artifacts: %s" % artifact_dir)
		quit(1)
		return
	# Exercise overlapping borders, partial tiles, and the first interior pixel.
	for size in [
		Vector2i(1, 1), Vector2i(1, 90), Vector2i(90, 1), Vector2i(12, 12),
		Vector2i(23, 47), Vector2i(24, 24), Vector2i(25, 25), Vector2i(31, 33),
		Vector2i(32, 32), Vector2i(63, 65), Vector2i(257, 113),
		Vector2i(1280, 720), Vector2i(1920, 1080), Vector2i(3840, 2160),
	]:
		var started := Time.get_ticks_usec()
		var expected := _legacy_fallback_image(size)
		var legacy_us := Time.get_ticks_usec() - started
		started = Time.get_ticks_usec()
		var actual: Image = QaSnapshot._create_fallback_screenshot_image(size.x, size.y)
		var native_us := Time.get_ticks_usec() - started
		var artifact := "%s/%sx%s.png" % [artifact_dir, size.x, size.y]
		if actual.save_png(artifact) != OK:
			failures.append("could not write diagnostic screenshot: %s" % artifact)
			continue
		var decoded := Image.load_from_file(artifact)
		if decoded == null or decoded.get_data() != expected.get_data():
			failures.append("decoded pixels differ from legacy pattern at %s; artifact=%s" % [size, artifact])
		timings.append({"size": [size.x, size.y], "legacy_image_us": legacy_us, "native_image_us": native_us})

	# Exercise the actual writer and strict headless failure contract as well as
	# the image helper; use a detached viewport so no runtime QA session is needed.
	var viewport := SubViewport.new()
	viewport.size = Vector2i(127, 83)
	var written_path := "%s/writer.png" % artifact_dir
	var write_result: int = QaSnapshot._write_fallback_screenshot(viewport, written_path)
	if write_result != OK:
		failures.append("fallback writer failed: result=%s artifact=%s" % [write_result, written_path])
	else:
		var decoded_writer := Image.load_from_file(written_path)
		if decoded_writer == null or decoded_writer.get_data() != _legacy_fallback_image(viewport.size).get_data():
			failures.append("fallback writer changed decoded pixels: %s" % written_path)
	if DisplayServer.get_name() == "headless":
		var strict_path := "%s/strict.png" % artifact_dir
		if QaSnapshot.save_viewport_screenshot(viewport, strict_path, false) != ERR_UNAVAILABLE:
			failures.append("strict headless capture did not return ERR_UNAVAILABLE")
		if FileAccess.file_exists(strict_path):
			failures.append("strict headless capture wrote fallback pixels: %s" % strict_path)
	viewport.free()

	print(JSON.stringify({
		"test": "snapshot_image_equivalence", "ok": failures.is_empty(),
		"timings": timings, "failures": failures,
		"artifacts": ProjectSettings.globalize_path(artifact_dir),
	}))
	quit(0 if failures.is_empty() else 1)


# Preserve the former implementation independently to detect any rounding,
# border precedence, or checkerboard alignment changes in decoded PNG pixels.
func _legacy_fallback_image(size: Vector2i) -> Image:
	var image := Image.create(size.x, size.y, false, Image.FORMAT_RGBA8)
	for y in range(size.y):
		for x in range(size.x):
			var tile := int(floor(float(x) / 32.0)) + int(floor(float(y) / 32.0))
			var color := Color(0.12, 0.14, 0.18, 1.0)
			if tile % 2 == 0:
				color = Color(0.24, 0.28, 0.34, 1.0)
			if x < 12 or y < 12 or x >= size.x - 12 or y >= size.y - 12:
				color = Color(0.64, 0.72, 0.82, 1.0)
			image.set_pixel(x, y, color)
	return image
