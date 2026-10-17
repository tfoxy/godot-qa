extends RefCounted

const FOCUS_MODE_NAMES := {
	Control.FOCUS_NONE: "none",
	Control.FOCUS_CLICK: "click",
	Control.FOCUS_ALL: "all",
	3: "accessibility",
}
const HEADLESS_FALLBACK_PNG_BASE64 := "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVQIHWP4////fwAJ+wP9KobjigAAAABJRU5ErkJggg=="
const MOUSE_FILTER_NAMES := {
	Control.MOUSE_FILTER_STOP: "stop",
	Control.MOUSE_FILTER_PASS: "pass",
	Control.MOUSE_FILTER_IGNORE: "ignore",
}
const FOCUS_NEIGHBOR_SIDES := {
	"left": SIDE_LEFT,
	"top": SIDE_TOP,
	"right": SIDE_RIGHT,
	"bottom": SIDE_BOTTOM,
}

static func node_to_dict(node: Node) -> Dictionary:
	var result := {
		"path": str(node.get_path()),
		"name": str(node.name),
		"type": node.get_class(),
		"qa_id": str(node.get_meta("qa_id", "")),
		"groups": _groups_to_array(node.get_groups()),
		"inside_tree": node.is_inside_tree(),
		"child_count": node.get_child_count(),
	}
	var recorder_meta_keys := [
		"qa_recorder_role",
		"qa_recorder_bounds",
		"qa_recorder_playfield",
		"qa_recorder_occluder",
		"qa_recorder_proximity_target",
		"qa_recorder_proximity_range",
	]
	for key in recorder_meta_keys:
		if node.has_meta(key):
			result[key] = _json_safe_value(node.get_meta(key))
	if node is Node2D:
		var node2d := node as Node2D
		result["global_position"] = _vector2_to_array(node2d.global_position)
		result["position"] = _vector2_to_array(node2d.position)
	if node is CanvasItem:
		result["visible"] = (node as CanvasItem).is_visible_in_tree()
	else:
		result["visible"] = true
	result["visual_evidence"] = visual_evidence_for_node(node)
	return result

static func node_summary_to_dict(node: Node, locator := "") -> Dictionary:
	var result := {
		"path": str(node.get_path()),
		"name": str(node.name),
		"type": node.get_class(),
		"qa_id": str(node.get_meta("qa_id", "")),
	}
	if locator != "":
		result["locator"] = locator
	return result

static func _has_property(object: Object, property_name: String) -> bool:
	for property in object.get_property_list():
		if property.get("name", "") == property_name:
			return true
	return false

static func _groups_to_array(groups: Array) -> Array[String]:
	var normalized: Array[String] = []
	for group_name in groups:
		var group_string := str(group_name)
		if group_string.begins_with("_"):
			continue
		normalized.append(group_string)
	normalized.sort()
	return normalized


static func _metadata_to_dict(node: Node, options := {}) -> Dictionary:
	var metadata := {}
	if not bool((options as Dictionary).get("include_metadata", true)):
		return metadata
	var allowlist: Array[String] = []
	if (options as Dictionary).has("metadata_keys"):
		var raw_keys = (options as Dictionary).get("metadata_keys", [])
		if raw_keys is Array or raw_keys is PackedStringArray:
			for raw_key in raw_keys:
				var key_string := str(raw_key)
				if key_string != "" and not allowlist.has(key_string):
					allowlist.append(key_string)
	var keys: Array[String] = []
	for key in node.get_meta_list():
		var key_string := str(key)
		if not allowlist.is_empty() and not allowlist.has(key_string):
			continue
		keys.append(key_string)
	keys.sort()
	for key in keys:
		metadata[key] = _json_safe_value(node.get_meta(key))
	return metadata


static func _json_safe_value(value: Variant) -> Variant:
	match typeof(value):
		TYPE_NIL, TYPE_BOOL, TYPE_INT, TYPE_FLOAT, TYPE_STRING:
			return value
		TYPE_STRING_NAME, TYPE_NODE_PATH:
			return str(value)
		TYPE_VECTOR2:
			return _vector2_to_array(value as Vector2)
		TYPE_VECTOR2I:
			var vector := value as Vector2i
			return [int(vector.x), int(vector.y)]
		TYPE_RECT2:
			return _rect2_to_array(value as Rect2)
		TYPE_ARRAY:
			var items: Array = []
			for item in value:
				items.append(_json_safe_value(item))
			return items
		TYPE_DICTIONARY:
			var result := {}
			var keys: Array[String] = []
			for key in (value as Dictionary).keys():
				keys.append(str(key))
			keys.sort()
			for key in keys:
				result[key] = _json_safe_value((value as Dictionary).get(key))
			return result
	return str(value)


static func _vector2_to_array(value: Vector2) -> Array[float]:
	return [float(value.x), float(value.y)]

static func _rect2_to_array(value: Rect2) -> Array[float]:
	var position := _vector2_to_array(value.position)
	var size := _vector2_to_array(value.size)
	return [position[0], position[1], size[0], size[1]]

static func visual_evidence_for_node(node: Node) -> Dictionary:
	if node == null:
		return _unsupported_visual_evidence("missing_node", false)
	var explicit := _explicit_visual_rect(node)
	if bool(explicit.get("ok", false)):
		return _supported_visual_evidence(explicit.get("rect"), "qa_visual_rect", explicit.get("coordinate_space", "viewport_2d"), node)
	if not bool(explicit.get("missing", false)):
		return _unsupported_visual_evidence(
			str(explicit.get("unsupported_reason", "invalid_qa_visual_rect")),
			_target_hidden_supported(node),
			explicit.get("coordinate_space", explicit.get("actual_space", "")),
			explicit.get("actual", null),
		)
	if node is CanvasItem:
		var rect_evidence := _canvas_item_visual_rect(node as CanvasItem)
		if bool(rect_evidence.get("ok", false)):
			return _supported_visual_evidence(rect_evidence.get("rect"), rect_evidence.get("rect_source", "canvas_item"), "viewport_2d", node)
		return _unsupported_visual_evidence(
			str(rect_evidence.get("unsupported_reason", "unsupported_canvas_item_geometry")),
			_target_hidden_supported(node),
			"viewport_2d",
			rect_evidence.get("actual", null),
		)
	return _unsupported_visual_evidence("target_is_not_canvas_item_and_has_no_qa_visual_rect", _target_hidden_supported(node), "", {"class": node.get_class()})

static func _supported_visual_evidence(rect: Rect2, rect_source: String, coordinate_space: String, node: Node) -> Dictionary:
	var transform := _canvas_to_screenshot_transform(node)
	return {
		"coordinate_space": coordinate_space,
		"rect": _rect2_to_array(rect),
		# Same canvas rect mapped into window/screenshot pixel space so saved-pixel
		# consumers can index the screenshot correctly under content scaling. Equal
		# to `rect` when there is no scaling (identity transform).
		"rect_screenshot": _rect2_to_array(transform * rect),
		"rect_source": rect_source,
		"target_hidden_supported": _target_hidden_supported(node),
	}


# Maps 2D canvas coordinates into the window/screenshot pixel space of the node's
# viewport (Viewport.get_final_transform() folds in content scaling). Identity when
# the node is not in a tree or content scaling is disabled.
static func _canvas_to_screenshot_transform(node: Node) -> Transform2D:
	if node != null and node.is_inside_tree():
		var viewport := node.get_viewport()
		if viewport != null:
			return viewport.get_final_transform()
	return Transform2D.IDENTITY

static func _unsupported_visual_evidence(reason: String, target_hidden_supported: bool, coordinate_space := "", actual: Variant = null) -> Dictionary:
	var evidence := {
		"coordinate_space": coordinate_space,
		"rect": null,
		"rect_source": "",
		"target_hidden_supported": target_hidden_supported,
		"unsupported_reason": reason,
	}
	if actual != null:
		evidence["actual"] = _json_safe_value(actual)
	return evidence

static func _target_hidden_supported(node: Node) -> bool:
	return node is CanvasItem or (node != null and node.has_method("qa_set_visual_visible"))

static func _explicit_visual_rect(node: Node) -> Dictionary:
	var value: Variant = null
	if node.has_method("qa_visual_rect"):
		value = node.call("qa_visual_rect")
	elif node.has_meta("qa_visual_rect"):
		value = node.get_meta("qa_visual_rect")
	else:
		return {"ok": false, "missing": true}
	var parsed := _parse_visual_rect(value)
	if not bool(parsed.get("ok", false)):
		parsed["actual"] = value
	return parsed

static func _parse_visual_rect(value: Variant) -> Dictionary:
	if value is Rect2:
		return {"ok": true, "rect": value, "coordinate_space": "viewport_2d"}
	if value is Array:
		var values: Array = value
		if values.size() >= 4:
			return {"ok": true, "rect": Rect2(float(values[0]), float(values[1]), float(values[2]), float(values[3])), "coordinate_space": "viewport_2d"}
	if value is Dictionary:
		var rect: Dictionary = value
		var coordinate_space := str(rect.get("coordinate_space", rect.get("space", "viewport_2d"))).strip_edges()
		if coordinate_space == "":
			coordinate_space = "viewport_2d"
		if coordinate_space != "viewport_2d":
			return {"ok": false, "missing": false, "unsupported_reason": "unsupported_coordinate_space", "coordinate_space": coordinate_space, "actual_space": coordinate_space}
		if rect.has("rect"):
			var parsed_rect := _parse_visual_rect(rect.get("rect"))
			if not bool(parsed_rect.get("ok", false)):
				parsed_rect["coordinate_space"] = coordinate_space
				return parsed_rect
			parsed_rect["coordinate_space"] = coordinate_space
			return parsed_rect
		if rect.has("x") and rect.has("y") and (rect.has("width") or rect.has("w")) and (rect.has("height") or rect.has("h")):
			return {"ok": true, "rect": Rect2(float(rect.get("x")), float(rect.get("y")), float(rect.get("width", rect.get("w"))), float(rect.get("height", rect.get("h")))), "coordinate_space": coordinate_space}
		if rect.has("position") and rect.has("size") and rect.get("position") is Array and rect.get("size") is Array:
			var position: Array = rect.get("position")
			var size: Array = rect.get("size")
			if position.size() >= 2 and size.size() >= 2:
				return {"ok": true, "rect": Rect2(float(position[0]), float(position[1]), float(size[0]), float(size[1])), "coordinate_space": coordinate_space}
	return {"ok": false, "missing": false, "unsupported_reason": "invalid_qa_visual_rect"}

static func _canvas_item_visual_rect(item: CanvasItem) -> Dictionary:
	if item is Control:
		return {"ok": true, "rect": (item as Control).get_global_rect(), "rect_source": "control_global_rect"}
	if item is Line2D:
		return _line2d_visual_rect(item as Line2D)
	return {"ok": true, "rect": Rect2(item.get_global_transform_with_canvas().origin - Vector2(2, 2), Vector2(4, 4)), "rect_source": "canvas_item_origin"}

static func _line2d_visual_rect(line: Line2D) -> Dictionary:
	var points := line.points
	var actual := {"class": line.get_class(), "point_count": points.size(), "width": float(line.width)}
	if points.size() < 2:
		return {"ok": false, "unsupported_reason": "nonzero_line2d_points", "actual": actual}
	var transform := line.get_global_transform_with_canvas()
	var first := transform * points[0]
	var min_x := first.x
	var min_y := first.y
	var max_x := first.x
	var max_y := first.y
	var has_length := false
	var previous := first
	for index in range(1, points.size()):
		var current := transform * points[index]
		min_x = minf(min_x, current.x)
		min_y = minf(min_y, current.y)
		max_x = maxf(max_x, current.x)
		max_y = maxf(max_y, current.y)
		if previous.distance_to(current) > 0.001:
			has_length = true
		previous = current
	if not has_length:
		actual["bounds"] = _rect2_to_array(Rect2(Vector2(min_x, min_y), Vector2(max_x - min_x, max_y - min_y)))
		return {"ok": false, "unsupported_reason": "nonzero_line2d_length", "actual": actual}
	var stroke_padding := maxf(float(line.width) * 0.5, 0.5) + 1.0
	var rect := Rect2(Vector2(min_x, min_y), Vector2(max_x - min_x, max_y - min_y)).grow(stroke_padding)
	if rect.size.x <= 0.0 or rect.size.y <= 0.0:
		actual["bounds"] = _rect2_to_array(rect)
		return {"ok": false, "unsupported_reason": "measurable_line2d_rect", "actual": actual}
	return {"ok": true, "rect": rect, "rect_source": "line2d_points"}

static func normalize_filter_options(options := {}) -> Dictionary:
	var raw_options: Dictionary = options if options is Dictionary else {}
	var normalized := {}
	if raw_options.has("target") and str(raw_options.get("target", "")).strip_edges() != "":
		normalized["target"] = str(raw_options.get("target", "")).strip_edges()
	if raw_options.has("resolved_target") and str(raw_options.get("resolved_target", "")).strip_edges() != "":
		normalized["resolved_target"] = str(raw_options.get("resolved_target", "")).strip_edges()
	if raw_options.has("max_depth"):
		normalized["max_depth"] = max(int(raw_options.get("max_depth", 0)), 0)
	if raw_options.has("qa_only"):
		normalized["qa_only"] = bool(raw_options.get("qa_only", false))
	if raw_options.has("include_metadata"):
		normalized["include_metadata"] = bool(raw_options.get("include_metadata", true))
	if raw_options.has("metadata_keys"):
		var metadata_keys: Array[String] = []
		var raw_keys = raw_options.get("metadata_keys", [])
		if raw_keys is Array or raw_keys is PackedStringArray:
			for raw_key in raw_keys:
				var key := str(raw_key).strip_edges()
				if key != "" and not metadata_keys.has(key):
					metadata_keys.append(key)
		metadata_keys.sort()
		normalized["metadata_keys"] = metadata_keys
	return normalized

static func validate_filter_options(options := {}) -> Dictionary:
	if not (options is Dictionary):
		return _filter_validation_error("invalid_filter_options", "snapshot filter options must be a dictionary", "dictionary", options)
	var raw_options: Dictionary = options
	if raw_options.has("max_depth"):
		var raw_depth = raw_options.get("max_depth")
		if not (raw_depth is int or raw_depth is float):
			return _filter_validation_error("invalid_max_depth", "snapshot max_depth must be a non-negative integer", "integer >= 0", raw_depth)
		if int(raw_depth) < 0:
			return _filter_validation_error("invalid_max_depth", "snapshot max_depth must be a non-negative integer", "integer >= 0", raw_depth)
	if raw_options.has("metadata_keys"):
		var raw_keys = raw_options.get("metadata_keys")
		if not (raw_keys is Array or raw_keys is PackedStringArray):
			return _filter_validation_error("invalid_metadata_keys", "snapshot metadata_keys must be an array of key strings", "array of strings", raw_keys)
		for key in raw_keys:
			if not (key is String or key is StringName):
				return _filter_validation_error("invalid_metadata_keys", "snapshot metadata_keys must contain only strings", "array of strings", raw_keys)
	if raw_options.has("include_metadata") and not (raw_options.get("include_metadata") is bool):
		return _filter_validation_error("invalid_include_metadata", "snapshot include_metadata must be a boolean", "boolean", raw_options.get("include_metadata"))
	if raw_options.has("qa_only") and not (raw_options.get("qa_only") is bool):
		return _filter_validation_error("invalid_qa_only", "snapshot qa_only must be a boolean", "boolean", raw_options.get("qa_only"))
	return {"ok": true, "options": normalize_filter_options(raw_options)}

static func node_tree_to_dict(root: Node, options := {}) -> Dictionary:
	var normalized := normalize_filter_options(options)
	var result := {
		"root": _node_tree_entry(root, normalized, 0),
	}
	if not normalized.is_empty():
		result["filter"] = normalized.duplicate(true)
	return result

static func visible_controls(root: Node) -> Array[Control]:
	var controls: Array[Control] = []
	_collect_visible_controls(root, controls)
	return controls

static func all_controls(root: Node) -> Array[Control]:
	var controls: Array[Control] = []
	_collect_all_controls(root, controls)
	return controls

static func control_to_dict(control: Control) -> Dictionary:
	var result := node_to_dict(control)
	if _has_property(control, "disabled"):
		result["disabled"] = bool(control.get("disabled"))
	if _has_property(control, "text"):
		result["text"] = str(control.get("text"))
	result["global_rect"] = _rect2_to_array(control.get_global_rect())
	result["rect"] = _rect2_to_array(Rect2(control.position, control.size))
	result["global_position"] = _vector2_to_array(control.global_position)
	result["size"] = _vector2_to_array(control.size)
	result["position"] = _vector2_to_array(control.position)
	result["minimum_size"] = _vector2_to_array(control.get_combined_minimum_size())
	result["anchors"] = _anchors_to_array(control)
	result["offsets"] = _offsets_to_array(control)
	result["focus_mode"] = _focus_mode_to_string(control.focus_mode)
	result["has_focus"] = control.has_focus()
	result["mouse_filter"] = _mouse_filter_to_string(control.mouse_filter)
	if control.theme_type_variation != "":
		result["theme_type_variation"] = control.theme_type_variation
	return result

static func control_state_to_dict(control: Control) -> Dictionary:
	var result := control_to_dict(control)
	for key in ["path", "name", "type", "qa_id", "groups", "inside_tree", "child_count", "visible"]:
		result.erase(key)
	return result

static func visible_controls_to_dict(root: Node, controls: Array[Control] = [], options := {}) -> Dictionary:
	var normalized := normalize_filter_options(options)
	var visible := controls
	if visible.is_empty():
		visible = _filtered_visible_controls(root, normalized)
	var viewport_rect := root.get_viewport().get_visible_rect()
	var entries: Array[Dictionary] = []
	for control in visible:
		entries.append(_filtered_control_to_dict(control, normalized))
	var result := {
		"viewport": _vector2_to_array(viewport_rect.size),
		"controls": entries,
	}
	if not normalized.is_empty():
		result["filter"] = normalized.duplicate(true)
	return result

static func all_controls_to_dict(root: Node, controls: Array[Control] = [], options := {}) -> Dictionary:
	var normalized := normalize_filter_options(options)
	var all := controls
	if all.is_empty():
		all = _filtered_all_controls(root, normalized)
	var viewport_rect := root.get_viewport().get_visible_rect()
	var entries: Array[Dictionary] = []
	for control in all:
		entries.append(_filtered_control_to_dict(control, normalized))
	var result := {
		"viewport": _vector2_to_array(viewport_rect.size),
		"controls": entries,
	}
	if not normalized.is_empty():
		result["filter"] = normalized.duplicate(true)
	return result

static func focus_to_dict(root_window: Window, controls: Array[Control], options := {}) -> Dictionary:
	var normalized := normalize_filter_options(options)
	var focus_owner := root_window.gui_get_focus_owner()
	var control_lookup := {}
	for control in controls:
		control_lookup[control.get_instance_id()] = true
	if focus_owner != null and not control_lookup.has(focus_owner.get_instance_id()):
		focus_owner = null
	var result := {
		"focus_owner": _describe_control_target(focus_owner),
		"focus_owner_path": str(focus_owner.get_path()) if focus_owner != null else "",
		"graph": focus_graph_to_dict(controls),
	}
	if not normalized.is_empty():
		result["filter"] = normalized.duplicate(true)
	return result

static func focus_graph_to_dict(controls: Array[Control]) -> Array[Dictionary]:
	var visible_lookup := {}
	for control in controls:
		visible_lookup[control.get_instance_id()] = control

	var graph: Array[Dictionary] = []
	for control in controls:
		if int(control.focus_mode) == Control.FOCUS_NONE:
			continue
		var neighbors := _focus_neighbors_to_dict(control, visible_lookup)
		if neighbors.is_empty():
			continue
		graph.append({
			"target": _describe_control_target(control),
			"path": str(control.get_path()),
			"neighbors": neighbors,
		})
	return graph

static func errors_to_dict(runtime_errors: Array) -> Dictionary:
	var errors: Array[Dictionary] = []
	for entry in runtime_errors:
		if entry is Dictionary:
			errors.append((entry as Dictionary).duplicate(true))
	return {
		"count": errors.size(),
		"errors": errors,
	}

static func save_viewport_screenshot(viewport: Viewport, path: String, allow_fallback := true) -> int:
	if OS.has_feature("headless") or DisplayServer.get_name() == "headless":
		if not allow_fallback:
			return ERR_UNAVAILABLE
		return _write_fallback_screenshot(viewport, path)
	var texture := viewport.get_texture()
	if texture == null:
		if not allow_fallback:
			return ERR_UNAVAILABLE
		return _write_fallback_screenshot(viewport, path)
	var image := texture.get_image()
	if image == null:
		if not allow_fallback:
			return ERR_UNAVAILABLE
		return _write_fallback_screenshot(viewport, path)
	if image.get_width() <= 0 or image.get_height() <= 0:
		if not allow_fallback:
			return ERR_UNAVAILABLE
		return _write_fallback_screenshot(viewport, path)
	if image.get_format() != Image.FORMAT_RGBA8:
		image.convert(Image.FORMAT_RGBA8)
	# Strict callers retain real blank pixels as evidence; only the portable
	# capture path substitutes its diagnostic image for an empty viewport.
	if _image_is_blank_or_transparent(image) and allow_fallback:
		return _write_fallback_screenshot(viewport, path)
	var png_image := Image.create(image.get_width(), image.get_height(), false, Image.FORMAT_RGBA8)
	png_image.blit_rect(
		image,
		Rect2i(0, 0, image.get_width(), image.get_height()),
		Vector2i.ZERO,
	)
	var png_buffer := png_image.save_png_to_buffer()
	if png_buffer.is_empty():
		if not allow_fallback:
			return ERR_UNAVAILABLE
		return _write_fallback_screenshot(viewport, path)
	var file := FileAccess.open(path, FileAccess.WRITE)
	if file == null:
		return ERR_CANT_OPEN
	file.store_buffer(png_buffer)
	return OK

static func snapshot_paths(snapshot_dir: String) -> Dictionary:
	return {
		"root": snapshot_dir,
		"screenshot": "%s/screenshot.png" % snapshot_dir,
		"tree": "%s/tree.json" % snapshot_dir,
		"ui": "%s/ui.json" % snapshot_dir,
		"focus": "%s/focus.json" % snapshot_dir,
		"errors": "%s/errors.json" % snapshot_dir,
	}

static func capture_screenshot(root_window: Window, output_path: String, allow_fallback := true) -> Dictionary:
	var screenshot_dir := output_path.get_base_dir()
	var dir_error := DirAccess.make_dir_recursive_absolute(screenshot_dir)
	if dir_error != OK:
		return {
			"ok": false,
			"failures": [
				{
					"type": "screenshot_write_failed",
					"message": "screenshot artifact directory could not be created",
					"expected": "writable screenshot artifact directory",
					"actual": screenshot_dir,
				},
			],
			"artifacts": {
				"screenshot": output_path,
			},
		}

	await root_window.get_tree().process_frame
	RenderingServer.force_draw(false)
	await root_window.get_tree().process_frame

	var screenshot_error := save_viewport_screenshot(root_window, output_path, allow_fallback)
	if screenshot_error != OK:
		var unavailable := screenshot_error == ERR_UNAVAILABLE
		return {
			"ok": false,
			"failures": [
				{
					"type": "screenshot_capture_failed" if unavailable else "screenshot_write_failed",
					"message": "screenshot viewport pixels were unavailable" if unavailable else "screenshot PNG could not be written",
					"expected": "writable screenshot artifact path",
					"actual": output_path,
					"attempted_screenshot_path": output_path,
					"screenshot_error": screenshot_error,
					"unavailable": unavailable,
					"allow_fallback": allow_fallback,
					"display_server": DisplayServer.get_name(),
					"headless": OS.has_feature("headless"),
				},
			],
			"artifacts": {
				"screenshot": output_path,
			},
		}

	return {
		"ok": true,
		"artifacts": {
			"screenshot": output_path,
		},
	}

static func capture_snapshot_bundle(
	scene_root: Node,
	root_window: Window,
	runtime_errors: Array,
	snapshot_dir: String,
	options := {},
) -> Dictionary:
	var normalized := normalize_filter_options(options)
	var paths := snapshot_paths(snapshot_dir)
	var dir_error := DirAccess.make_dir_recursive_absolute(snapshot_dir)
	if dir_error != OK:
		return {
			"ok": false,
			"failures": [
				{
					"type": "snapshot_write_failed",
					"message": "snapshot artifact directory could not be created",
					"target": "%s.root" % snapshot_dir.get_file(),
					"expected": "writable snapshot artifact directory",
					"actual": snapshot_dir,
				},
			],
			"artifacts": paths,
		}

	var visible := _filtered_visible_controls(scene_root, normalized)
	var all := _filtered_all_controls(scene_root, normalized)
	var failures: Array[Dictionary] = []
	for job in [
		{
			"key": "tree",
			"path": str(paths["tree"]),
			"payload": node_tree_to_dict(scene_root, normalized),
		},
		{
			"key": "ui",
			"path": str(paths["ui"]),
			"payload": all_controls_to_dict(scene_root, all, normalized),
		},
		{
			"key": "focus",
			"path": str(paths["focus"]),
			"payload": focus_to_dict(root_window, visible, normalized),
		},
		{
			"key": "errors",
			"path": str(paths["errors"]),
			"payload": errors_to_dict(runtime_errors),
		},
	]:
		var save_error := _write_json_file(str(job["path"]), job["payload"])
		if save_error != OK:
			failures.append(
				{
					"type": "snapshot_write_failed",
					"message": "snapshot artifact could not be written",
					"target": "%s.%s" % [snapshot_dir.get_file(), str(job["key"])],
					"expected": "writable snapshot artifact file",
					"actual": str(job["path"]),
				}
			)

	await root_window.get_tree().process_frame
	RenderingServer.force_draw(false)
	await root_window.get_tree().process_frame

	var screenshot_error := save_viewport_screenshot(root_window, str(paths["screenshot"]))
	if screenshot_error != OK:
		failures.append(
			{
				"type": "screenshot_capture_failed",
				"message": "screenshot artifact could not be written",
				"target": "%s.screenshot" % snapshot_dir.get_file(),
				"expected": "PNG screenshot written to the snapshot directory",
				"actual": screenshot_error,
			}
		)

	return {
		"ok": failures.is_empty(),
		"artifacts": paths,
		"failures": failures,
	}

static func _write_fallback_screenshot(viewport: Viewport, path: String) -> int:
	var fallback_size := viewport.get_visible_rect().size
	var width := max(int(fallback_size.x), 1)
	var height := max(int(fallback_size.y), 1)
	var fallback_image := Image.create(
		width,
		height,
		false,
		Image.FORMAT_RGBA8,
	)
	for y in range(height):
		for x in range(width):
			var tile := int(floor(float(x) / 32.0)) + int(floor(float(y) / 32.0))
			var color := Color(0.12, 0.14, 0.18, 1.0)
			if tile % 2 == 0:
				color = Color(0.24, 0.28, 0.34, 1.0)
			if x < 12 or y < 12 or x >= width - 12 or y >= height - 12:
				color = Color(0.64, 0.72, 0.82, 1.0)
			fallback_image.set_pixel(x, y, color)
	var png_buffer := fallback_image.save_png_to_buffer()
	if png_buffer.is_empty():
		png_buffer = Marshalls.base64_to_raw(HEADLESS_FALLBACK_PNG_BASE64)
	var file := FileAccess.open(path, FileAccess.WRITE)
	if file == null:
		return ERR_CANT_OPEN
	file.store_buffer(png_buffer)
	return OK

static func _image_is_blank_or_transparent(image: Image) -> bool:
	var width := image.get_width()
	var height := image.get_height()
	if width <= 0 or height <= 0:
		return true

	var first_visible := Color.TRANSPARENT
	var has_first_visible := false
	var has_bright_visible_pixel := false
	var has_variation := false
	for y in range(height):
		for x in range(width):
			var pixel := image.get_pixel(x, y)
			if pixel.a <= 0.02:
				continue
			if not has_first_visible:
				first_visible = pixel
				has_first_visible = true
			if max(pixel.r, pixel.g, pixel.b) > 0.03:
				has_bright_visible_pixel = true
			if (
				abs(pixel.r - first_visible.r) > 0.02
				or abs(pixel.g - first_visible.g) > 0.02
				or abs(pixel.b - first_visible.b) > 0.02
				or abs(pixel.a - first_visible.a) > 0.02
			):
				has_variation = true
			if has_bright_visible_pixel and has_variation:
				return false
	return true

static func _write_json_file(path: String, payload: Dictionary) -> int:
	var file := FileAccess.open(path, FileAccess.WRITE)
	if file == null:
		return ERR_CANT_OPEN
	file.store_string(JSON.stringify(payload))
	return OK

static func _node_tree_entry(node: Node, options := {}, depth := 0) -> Variant:
	var entry := {
		"path": str(node.get_path()),
		"name": str(node.name),
		"type": node.get_class(),
		"qa_id": str(node.get_meta("qa_id", "")),
		"visible": _node_visible(node),
		"visual_evidence": visual_evidence_for_node(node),
		"children": [],
	}
	if bool((options as Dictionary).get("include_metadata", true)):
		entry["metadata"] = _metadata_to_dict(node, options)
	var children: Array[Dictionary] = []
	var max_depth = (options as Dictionary).get("max_depth", null)
	if max_depth == null or depth < int(max_depth):
		for child in node.get_children():
			var child_node := child as Node
			if child_node == null:
				continue
			var child_entry = _node_tree_entry(child_node, options, depth + 1)
			if child_entry is Dictionary:
				children.append(child_entry)
	entry["children"] = children
	if bool((options as Dictionary).get("qa_only", false)) and children.is_empty() and not _node_matches_qa_filter(node):
		return null
	return entry

static func _filter_validation_error(error_type: String, message: String, expected: Variant, actual: Variant) -> Dictionary:
	return {
		"ok": false,
		"error": {
			"type": error_type,
			"message": message,
			"expected": expected,
			"actual": _json_safe_value(actual),
		},
	}

static func _node_matches_qa_filter(node: Node) -> bool:
	if str(node.get_meta("qa_id", "")) != "":
		return true
	for group_name in node.get_groups():
		var group_string := str(group_name)
		if group_string.begins_with("qa.") or group_string == "qa":
			return true
	for meta_key in node.get_meta_list():
		if str(meta_key).begins_with("qa_"):
			return true
	return false

static func _control_allowed(control: Control, root: Node, options: Dictionary) -> bool:
	if not _node_within_depth(control, root, options):
		return false
	if bool(options.get("qa_only", false)) and not _node_matches_qa_filter(control):
		return false
	return true

static func _node_within_depth(node: Node, root: Node, options: Dictionary) -> bool:
	if not options.has("max_depth"):
		return true
	var depth := 0
	var current := node
	while current != null and current != root:
		depth += 1
		current = current.get_parent()
	return current == root and depth <= int(options.get("max_depth", 0))

static func _filtered_visible_controls(root: Node, options := {}) -> Array[Control]:
	var controls := visible_controls(root)
	var filtered: Array[Control] = []
	for control in controls:
		if _control_allowed(control, root, options as Dictionary):
			filtered.append(control)
	return filtered

static func _filtered_all_controls(root: Node, options := {}) -> Array[Control]:
	var controls := all_controls(root)
	var filtered: Array[Control] = []
	for control in controls:
		if _control_allowed(control, root, options as Dictionary):
			filtered.append(control)
	return filtered

static func _filtered_control_to_dict(control: Control, options := {}) -> Dictionary:
	var result := control_to_dict(control)
	if not bool((options as Dictionary).get("include_metadata", true)):
		result.erase("metadata")
	elif (options as Dictionary).has("metadata_keys"):
		result["metadata"] = _metadata_to_dict(control, options)
	return result

static func _node_visible(node: Node) -> bool:
	if node is CanvasItem:
		return (node as CanvasItem).is_visible_in_tree()
	return true

static func _collect_visible_controls(node: Node, controls: Array[Control]) -> void:
	if node is Control:
		var control := node as Control
		if control.is_visible_in_tree():
			controls.append(control)
	for child in node.get_children():
		var child_node := child as Node
		if child_node == null:
			continue
		_collect_visible_controls(child_node, controls)

static func _collect_all_controls(node: Node, controls: Array[Control]) -> void:
	if node is Control:
		controls.append(node as Control)
	for child in node.get_children():
		var child_node := child as Node
		if child_node == null:
			continue
		_collect_all_controls(child_node, controls)

static func _anchors_to_array(control: Control) -> Array[float]:
	return [
		float(control.get_anchor(SIDE_LEFT)),
		float(control.get_anchor(SIDE_TOP)),
		float(control.get_anchor(SIDE_RIGHT)),
		float(control.get_anchor(SIDE_BOTTOM)),
	]

static func _offsets_to_array(control: Control) -> Array[float]:
	return [
		float(control.get_offset(SIDE_LEFT)),
		float(control.get_offset(SIDE_TOP)),
		float(control.get_offset(SIDE_RIGHT)),
		float(control.get_offset(SIDE_BOTTOM)),
	]

static func _focus_mode_to_string(value: int) -> String:
	return str(FOCUS_MODE_NAMES.get(value, str(value)))

static func _mouse_filter_to_string(value: int) -> String:
	return str(MOUSE_FILTER_NAMES.get(value, str(value)))

static func _describe_control_target(control: Control) -> String:
	if control == null:
		return ""
	if control.has_meta("qa_id"):
		var qa_id := str(control.get_meta("qa_id"))
		if qa_id != "":
			return "qa:%s" % qa_id
	return "path:%s" % str(control.get_path())

static func _focus_neighbors_to_dict(control: Control, visible_lookup: Dictionary) -> Dictionary:
	var neighbors := {}
	for direction in FOCUS_NEIGHBOR_SIDES.keys():
		var side = int(FOCUS_NEIGHBOR_SIDES[direction])
		var neighbor_path := control.get_focus_neighbor(side)
		if neighbor_path == NodePath(""):
			continue
		var neighbor := control.get_node_or_null(neighbor_path) as Control
		if neighbor == null:
			continue
		if not visible_lookup.has(neighbor.get_instance_id()):
			continue
		neighbors[direction] = _describe_control_target(neighbor)
	return neighbors
