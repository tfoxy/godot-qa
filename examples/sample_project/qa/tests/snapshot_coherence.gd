extends SceneTree

const QaSnapshot = preload("res://addons/godot_qa/qa_snapshot.gd")

class DynamicControl extends Control:
	var property_reads := 0
	var dynamic_text := "first"

	func _get_property_list() -> Array[Dictionary]:
		property_reads += 1
		return [{"name": "text", "type": TYPE_STRING}]

	func _get(property: StringName) -> Variant:
		return dynamic_text if property == &"text" else null

class MovingControl extends ColorRect:
	var frame := 0
	var errors: Array = []
	var focus_targets: Array[Button] = []
	var removed: Node
	var visibility_target: Control

	func _process(_delta: float) -> void:
		frame += 1
		position = Vector2(40 + (frame % 9) * 80, 40)
		if visibility_target != null:
			visibility_target.visible = frame % 3 == 0
		set_meta("frame", frame)
		if not focus_targets.is_empty():
			focus_targets[frame % focus_targets.size()].grab_focus()
		errors.clear()
		errors.append({"frame": frame})
		if is_instance_valid(removed):
			removed.free()
			removed = null

var failures: Array[String] = []
var timings: Array[Dictionary] = []

func _initialize() -> void:
	_run.call_deferred()

func _run() -> void:
	var scene := Control.new()
	scene.name = "SnapshotCoherence"
	root.add_child(scene)
	var background := ColorRect.new()
	background.color = Color(0.1, 0.2, 0.3)
	background.size = Vector2(3840, 2160)
	scene.add_child(background)
	var moving := MovingControl.new()
	moving.name = "Moving"
	moving.color = Color.RED
	moving.size = Vector2(64, 64)
	moving.set_meta("qa_id", "moving")
	scene.add_child(moving)
	for index in range(2):
		var button := Button.new()
		button.name = "Focus%s" % index
		button.text = str(index)
		button.position = Vector2(40 + index * 120, 200)
		button.size = Vector2(100, 40)
		scene.add_child(button)
		moving.focus_targets.append(button)
	var removed := Control.new()
	removed.name = "RemovedWhileSettling"
	scene.add_child(removed)
	moving.removed = removed
	var visibility_target := ColorRect.new()
	visibility_target.name = "VisibilityTarget"
	visibility_target.color = Color.GREEN
	visibility_target.position = Vector2(900, 40)
	visibility_target.size = Vector2(64, 64)
	scene.add_child(visibility_target)
	moving.visibility_target = visibility_target

	var dynamic := DynamicControl.new()
	dynamic.name = "Dynamic"
	scene.add_child(dynamic)
	dynamic.property_reads = 0
	var serialized: Dictionary = QaSnapshot.control_to_dict(dynamic)
	_expect(serialized.get("text") == "first" and dynamic.property_reads == 1, "dynamic properties must be enumerated once per serialization")
	dynamic.dynamic_text = "second"
	_expect(QaSnapshot.control_to_dict(dynamic).get("text") == "second", "dynamic properties must not use a persistent class cache")
	_expect(QaSnapshot.all_controls_to_dict(scene, []).controls.is_empty(), "explicit empty controls must remain empty")
	_expect(QaSnapshot.visible_controls_to_dict(scene, []).controls.is_empty(), "explicit empty visible controls must remain empty")
	_expect(not QaSnapshot.all_controls_to_dict(scene).controls.is_empty(), "omitted controls must still trigger collection")

	# Verify exact existing filtering/ordering, including retained QA ancestors,
	# metadata selection, hidden controls, and bounded depth.
	var ancestor := Control.new()
	ancestor.name = "Ancestor"
	scene.add_child(ancestor)
	var descendant := Control.new()
	descendant.name = "TaggedDescendant"
	descendant.set_meta("qa_id", "descendant")
	descendant.set_meta("other", 7)
	ancestor.add_child(descendant)
	descendant.hide()
	for options in [{}, {"max_depth": 0}, {"max_depth": 1}, {"qa_only": true}, {"qa_only": true, "max_depth": 1}, {"metadata_keys": ["qa_id"]}, {"include_metadata": false}]:
		var actual: Dictionary = QaSnapshot._snapshot_payloads(scene, root, moving.errors, options)
		var expected := _legacy_payloads(scene, moving.errors, options)
		_expect(actual == expected, "combined traversal changed payloads: %s" % options)

	var artifact_dir := "user://godot_qa/snapshot-coherence-%s" % OS.get_process_id()
	for size in [Vector2i(1280, 720), Vector2i(1920, 1080), Vector2i(3840, 2160)]:
		root.size = size
		var started := Time.get_ticks_usec()
		var result: Dictionary = await QaSnapshot.capture_snapshot_bundle(scene, root, moving.errors, "%s/%sx%s" % [artifact_dir, size.x, size.y])
		var capture_us := Time.get_ticks_usec() - started
		_expect(result.ok, "bundle failed: %s" % result)
		if not result.ok:
			continue
		var ui: Dictionary = JSON.parse_string(FileAccess.get_file_as_string(result.artifacts.ui))
		var tree: Dictionary = JSON.parse_string(FileAccess.get_file_as_string(result.artifacts.tree))
		var focus: Dictionary = JSON.parse_string(FileAccess.get_file_as_string(result.artifacts.focus))
		var errors: Dictionary = JSON.parse_string(FileAccess.get_file_as_string(result.artifacts.errors))
		var moving_entry := {}
		var visibility_entry := {}
		var focused_path := ""
		for entry in ui.controls:
			if entry.qa_id == "moving":
				moving_entry = entry
			if entry.name == "VisibilityTarget":
				visibility_entry = entry
			if entry.has_focus:
				focused_path = entry.path
			_expect(entry.name != "RemovedWhileSettling", "snapshot retained a node removed during settling")
		_expect(focus.focus_owner_path == focused_path, "focus owner and UI focus disagree")
		var moving_tree: Dictionary = _find_tree(tree.root, "Moving")
		_expect(not moving_tree.is_empty(), "moving node missing from tree")
		var captured_frame := int(moving_tree.get("metadata", {}).get("frame", -1))
		_expect(captured_frame >= 0 and errors.count > 0, "snapshot omitted state established while settling")
		if captured_frame >= 0 and errors.count > 0:
			_expect(captured_frame == int(errors.errors[0].frame), "tree and runtime errors describe different frames")
			var visibility_tree: Dictionary = _find_tree(tree.root, "VisibilityTarget")
			_expect(visibility_tree.visible == (captured_frame % 3 == 0), "visibility and frame metadata disagree")
			var expected_focus: Button = moving.focus_targets[captured_frame % moving.focus_targets.size()]
			_expect(focus.focus_owner_path == str(expected_focus.get_path()), "focus and frame metadata disagree")
		if DisplayServer.get_name() != "headless" and not moving_entry.is_empty():
			var image := Image.load_from_file(result.artifacts.screenshot)
			_expect(image.get_size() == size, "screenshot did not use requested physical window size")
			# UI rects are logical viewport coordinates. Content stretching maps
			# them into screenshot pixels through the saved visual evidence.
			var center := _screenshot_center(moving_entry)
			var pixel := image.get_pixelv(center)
			_expect(pixel.r > 0.9 and pixel.g < 0.1 and pixel.b < 0.1, "pixels disagree with moving control rect at %s: %s" % [center, pixel])
			var visibility_pixel := image.get_pixelv(_screenshot_center(visibility_entry))
			var visibly_green := visibility_pixel.g > 0.9 and visibility_pixel.r < 0.1
			_expect(visibly_green == (captured_frame % 3 == 0), "pixels disagree with visibility metadata")
		timings.append({"size": [size.x, size.y], "capture_us": capture_us})

	# Dense trees make traversal costs visible independently of image encoding.
	for index in range(400):
		var branch := Control.new()
		branch.name = "Branch%s" % index
		scene.add_child(branch)
		var leaf := Control.new()
		leaf.name = "Leaf"
		leaf.set_meta("qa_id", "leaf%s" % index)
		branch.add_child(leaf)
	for options in [{}, {"max_depth": 1}]:
		var started := Time.get_ticks_usec()
		for iteration in range(5):
			_legacy_payloads(scene, moving.errors, options)
		var legacy_us := Time.get_ticks_usec() - started
		started = Time.get_ticks_usec()
		for iteration in range(5):
			QaSnapshot._snapshot_payloads(scene, root, moving.errors, options)
		timings.append({"options": options, "iterations": 5, "legacy_traversal_us": legacy_us, "combined_traversal_us": Time.get_ticks_usec() - started})
	print(JSON.stringify({"test": "snapshot_coherence", "ok": failures.is_empty(), "failures": failures, "timings": timings, "artifacts": ProjectSettings.globalize_path(artifact_dir)}))
	quit(0 if failures.is_empty() else 1)

func _legacy_payloads(scene: Node, errors: Array, options: Dictionary) -> Dictionary:
	var all: Array[Control] = []
	var visible: Array[Control] = []
	for control in QaSnapshot.all_controls(scene):
		if QaSnapshot._control_allowed(control, scene, options):
			all.append(control)
	for control in QaSnapshot.visible_controls(scene):
		if QaSnapshot._control_allowed(control, scene, options):
			visible.append(control)
	return {"tree": QaSnapshot.node_tree_to_dict(scene, options), "ui": QaSnapshot.all_controls_to_dict(scene, all, options), "focus": QaSnapshot.focus_to_dict(root, visible, options), "errors": QaSnapshot.errors_to_dict(errors)}

func _find_tree(entry: Dictionary, name_value: String) -> Dictionary:
	if entry.name == name_value:
		return entry
	for child in entry.children:
		var found := _find_tree(child, name_value)
		if not found.is_empty():
			return found
	return {}

func _expect(condition: bool, message: String) -> void:
	if not condition:
		failures.append(message)

func _screenshot_center(entry: Dictionary) -> Vector2i:
	var rect: Array = entry.visual_evidence.rect_screenshot
	return Vector2i(int(float(rect[0]) + float(rect[2]) / 2.0), int(float(rect[1]) + float(rect[3]) / 2.0))
