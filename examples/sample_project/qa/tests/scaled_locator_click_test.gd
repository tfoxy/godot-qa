extends SceneTree
## Rendered input regression: run with a display, never --headless.
## godot --path examples/sample_project --script res://qa/tests/scaled_locator_click_test.gd

const QaRuntime = preload("res://addons/godot_qa/qa_runtime.gd")
var _failures: Array[String] = []
var _press_count := 0


func _initialize() -> void:
	call_deferred("_run")


func _check(ok: bool, label: String) -> void:
	if not ok:
		_failures.append(label)
		push_error(label)


func _settle() -> void:
	for _i in range(4):
		await process_frame
	await RenderingServer.frame_post_draw


func _button(parent: Node, qa_id: String) -> Button:
	var button := Button.new()
	button.set_meta("qa_id", qa_id)
	button.position = Vector2(900, 500)
	button.size = Vector2(180, 70)
	button.text = qa_id
	button.pressed.connect(func(): _press_count += 1)
	parent.add_child(button)
	return button


func _click(runtime: RefCounted, qa_id: String, label: String) -> void:
	var before := _press_count
	var result: Dictionary = await runtime.execute("input.click_target", {"target": "qa:%s" % qa_id})
	_check(bool(result.get("ok", false)), "%s command succeeds: %s" % [label, result])
	_check(_press_count == before + 1, "%s fires the button's pressed action exactly once" % label)


func _run() -> void:
	if DisplayServer.get_name() == "headless":
		push_error("scaled locator click regression requires rendered Godot with a display")
		quit(1)
		return
	root.content_scale_size = Vector2i(1280, 720)
	root.content_scale_mode = Window.CONTENT_SCALE_MODE_CANVAS_ITEMS
	root.content_scale_aspect = Window.CONTENT_SCALE_ASPECT_KEEP
	root.mode = Window.MODE_WINDOWED
	var scene := Node.new()
	root.add_child(scene)
	var button := _button(scene, "scaled_button")
	var runtime := QaRuntime.new(scene, root)
	for dimensions in [Vector2i(1280, 720), Vector2i(1920, 1080), Vector2i(1600, 1000), Vector2i(1280, 720)]:
		root.size = dimensions
		await _settle()
		_check(root.size == dimensions, "window adopts requested rendered size %s (actual %s)" % [dimensions, root.size])
		await _click(runtime, "scaled_button", "window %s" % dimensions)
	button.visible = false
	var layer := CanvasLayer.new()
	layer.offset = Vector2(130, 60)
	layer.scale = Vector2(0.8, 0.8)
	scene.add_child(layer)
	var layer_button := _button(layer, "layer_button")
	layer_button.rotation = 0.15
	root.size = Vector2i(1920, 1080)
	await _settle()
	await _click(runtime, "layer_button", "scaled rotated CanvasLayer target")
	var subviewport := SubViewport.new()
	subviewport.size = Vector2i(1280, 720)
	scene.add_child(subviewport)
	_button(subviewport, "nested_button")
	await _settle()
	var before := _press_count
	var nested_result: Dictionary = await runtime.execute("input.click_target", {"target": "qa:nested_button"})
	_check(not bool(nested_result.get("ok", false)) and str(nested_result.get("error", {}).get("type", "")) == "target_not_clickable",
		"nested viewport target explicitly rejects unsupported root-window dispatch: %s" % nested_result)
	_check(_press_count == before, "unsupported viewport emits no click")
	runtime.teardown()
	runtime = null
	scene.queue_free()
	await process_frame
	print(JSON.stringify({"ok": _failures.is_empty(), "failures": _failures, "pressed": _press_count}))
	quit(0 if _failures.is_empty() else 1)
