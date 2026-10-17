extends RefCounted

const QaPatch = preload("res://addons/godot_qa/qa_patch.gd")

var _operations: Array[Dictionary] = []
var _next_sequence := 1


func set_property(runtime, scene_root: Node, target: String, property_name: String, value_input: String) -> Dictionary:
	var resolution: Dictionary = runtime.resolve_target(target)
	if not bool(resolution.get("ok", false)):
		return {"ok": false, "error": (resolution.get("error", {}) as Dictionary).duplicate(true)}
	var node := resolution.get("node") as Node
	if node == null:
		return runtime.error(
			"target_not_found",
			"No node matched the requested target",
			{
				"target": target,
				"expected": "existing runtime node",
				"actual": "missing",
			},
		)
	if not QaPatch.is_property_supported(node, property_name):
		return runtime.error(
			"edit_property_unsupported",
			"Live edit property is not allowed for the resolved node",
			{
				"target": target,
				"property": property_name,
				"expected": "supported live-edit property",
				"actual": node.get_class(),
			},
		)
	var current_value = node.get(property_name)
	var before_encoded = QaPatch.encode_value(current_value)
	var coerced: Dictionary = QaPatch.coerce_live_value(current_value, value_input)
	if not bool(coerced.get("ok", false)):
		return runtime.error(
			str(coerced.get("type", "edit_property_value_invalid")),
			str(coerced.get("message", "Live edit value could not be coerced")),
			{
				"target": target,
				"property": property_name,
				"expected": coerced.get("expected"),
				"actual": coerced.get("actual"),
			},
		)
	node.set(property_name, coerced.get("value"))
	await runtime.process_frames(1)
	var after_encoded = QaPatch.encode_value(node.get(property_name))
	if before_encoded == null or after_encoded == null:
		return runtime.error(
			"edit_property_value_unsupported",
			"Live edit property type is not supported for JSON export",
			{
				"target": target,
				"property": property_name,
				"expected": ["String", "bool", "int", "float", "Vector2", "Vector2i", "Color"],
				"actual": type_string(typeof(node.get(property_name))),
			},
		)
	var entry := {
		"sequence": _next_sequence,
		"op": "set_property",
		"target": target,
		"resolved": str(resolution.get("resolved", "")),
		"scene": _scene_path(scene_root),
		"property": property_name,
		"before": before_encoded,
		"after": after_encoded,
		"exportable": true,
	}
	_append_operation(entry)
	return runtime.ok(entry)


func call_method(runtime, scene_root: Node, target: String, method_name: String) -> Dictionary:
	var resolution: Dictionary = runtime.resolve_target(target)
	if not bool(resolution.get("ok", false)):
		return {"ok": false, "error": (resolution.get("error", {}) as Dictionary).duplicate(true)}
	var node := resolution.get("node") as Node
	if node == null or not QaPatch.is_method_supported(node, method_name):
		return runtime.error(
			"edit_method_unsupported",
			"Live edit method is not allowed for the resolved node",
			{
				"target": target,
				"method": method_name,
				"expected": ["grab_focus"],
				"actual": node.get_class() if node != null else "missing",
			},
		)
	node.call(method_name)
	await runtime.process_frames(1)
	var entry := {
		"sequence": _next_sequence,
		"op": "call_method",
		"target": target,
		"resolved": str(resolution.get("resolved", "")),
		"scene": _scene_path(scene_root),
		"method": method_name,
		"exportable": false,
	}
	_append_operation(entry)
	return runtime.ok(entry)


func diff(runtime) -> Dictionary:
	return runtime.ok({"operations": _operations.duplicate(true)})


func export_patch(runtime, scene_root: Node) -> Dictionary:
	var exportable_ops: Array[Dictionary] = []
	var skipped_ops: Array[Dictionary] = []
	for operation in _operations:
		if bool(operation.get("exportable", false)):
			exportable_ops.append(
				{
					"op": "set_property",
					"target": str(operation.get("target", "")),
					"property": str(operation.get("property", "")),
					"value": operation.get("after"),
				}
			)
		else:
			skipped_ops.append(
				{
					"op": str(operation.get("op", "")),
					"target": str(operation.get("target", "")),
					"method": str(operation.get("method", "")),
					"reason": "live_only",
				}
			)
	if exportable_ops.is_empty():
		return runtime.error(
			"patch_empty",
			"No exportable live edits are available for patch generation",
			{
				"expected": "at least one exportable set_property live edit",
				"actual": "none",
			},
		)
	return runtime.ok(
		{
			"format": QaPatch.PATCH_FORMAT,
			"scene": _scene_path(scene_root),
			"operations": exportable_ops,
			"skipped_operations": skipped_ops,
		}
	)


func _append_operation(entry: Dictionary) -> void:
	_operations.append(entry.duplicate(true))
	_next_sequence += 1


func _scene_path(scene_root: Node) -> String:
	if scene_root == null:
		return ""
	return str(scene_root.scene_file_path)
