extends SceneTree

const QaArgs = preload("res://addons/godot_qa/qa_args.gd")
const QaPatch = preload("res://addons/godot_qa/qa_patch.gd")
const QaRuntime = preload("res://addons/godot_qa/qa_runtime.gd")

var _request := {}


func _initialize() -> void:
	var parsed_args := QaArgs.parse_user_args(OS.get_cmdline_user_args())
	var options: Dictionary = parsed_args.get("options", {})
	var request_path := str(options.get("patch_request", ""))
	if request_path == "":
		quit(1)
		return
	_request = _read_json_file(request_path)
	if _request.is_empty():
		quit(1)
		return
	await _run()


func _run() -> void:
	var artifacts: Dictionary = _request.get("artifacts", {})
	var result_path := str(artifacts.get("result", ""))
	var result := await _apply_patch(_request.get("patch", {}))
	if result_path != "":
		_write_json_file(result_path, result)
	quit(0 if bool(result.get("ok", false)) else 1)


func _apply_patch(patch: Variant) -> Dictionary:
	if not patch is Dictionary:
		return _failure("patch_invalid", "patch request must be a JSON object", "", "JSON object", type_string(typeof(patch)))
	var patch_dict := patch as Dictionary
	if str(patch_dict.get("format", "")) != QaPatch.PATCH_FORMAT:
		return _failure("patch_invalid", "patch format is not supported", "", QaPatch.PATCH_FORMAT, patch_dict.get("format"))
	var scene_path := str(patch_dict.get("scene", ""))
	var packed_scene := load(scene_path)
	if packed_scene == null or not packed_scene is PackedScene:
		return _failure("patch_scene_load_failed", "patch target scene could not be loaded", scene_path, "loadable PackedScene", "missing")
	var root := (packed_scene as PackedScene).instantiate()
	if root == null:
		return _failure("patch_apply_failed", "patch target scene could not be instantiated", scene_path, "instantiated root node", "missing")
	get_root().add_child(root)
	await process_frame
	var runtime := QaRuntime.new(root, get_root(), [], {})
	var raw_operations = patch_dict.get("operations", [])
	if not raw_operations is Array:
		return _failure("patch_invalid", "patch operations must be an array", scene_path, "JSON array", type_string(typeof(raw_operations)))
	var prepared: Array[Dictionary] = []
	for operation in raw_operations:
		if not operation is Dictionary:
			return _failure("patch_invalid", "patch operations entries must be objects", scene_path, "JSON object entries", type_string(typeof(operation)))
		var op_dict := operation as Dictionary
		if str(op_dict.get("op", "")) != "set_property":
			return _failure("patch_invalid", "patch operation is not supported", scene_path, ["set_property"], op_dict.get("op"))
		var target := str(op_dict.get("target", ""))
		var property_name := str(op_dict.get("property", ""))
		var resolution: Dictionary = runtime.resolve_target(target)
		if not bool(resolution.get("ok", false)):
			var error_data := ((resolution.get("error", {}) as Dictionary).get("data", {}) as Dictionary).duplicate(true)
			return _failure(
				str((resolution.get("error", {}) as Dictionary).get("type", "target_not_found")),
				str((resolution.get("error", {}) as Dictionary).get("message", "Patch target resolution failed")),
				scene_path,
				error_data.get("expected"),
				error_data.get("actual"),
				target,
			)
		var node := resolution.get("node") as Node
		if node == null or not QaPatch.is_property_supported(node, property_name):
			return _failure(
				"edit_property_unsupported",
				"Patch property is not allowed for the resolved node",
				scene_path,
				"supported scene property",
				property_name,
				target,
			)
		var decoded := QaPatch.decode_patch_value(op_dict.get("value"))
		if not bool(decoded.get("ok", false)):
			return _failure(
				str(decoded.get("type", "patch_invalid")),
				str(decoded.get("message", "Patch value is invalid")),
				scene_path,
				decoded.get("expected"),
				decoded.get("actual"),
				target,
			)
		prepared.append(
			{
				"node": node,
				"target": target,
				"property": property_name,
				"value": decoded.get("value"),
			}
		)
	for item in prepared:
		var item_node := item.get("node") as Node
		item_node.set(str(item.get("property", "")), item.get("value"))
	await process_frame
	var packed := PackedScene.new()
	var pack_error := packed.pack(root)
	if pack_error != OK:
		return _failure("patch_apply_failed", "patched scene could not be packed", scene_path, OK, pack_error)
	var save_error := ResourceSaver.save(packed, scene_path)
	if save_error != OK:
		return _failure("patch_save_failed", "patched scene could not be saved", scene_path, OK, save_error)
	return {
		"ok": true,
		"scene": scene_path,
		"applied_operations": prepared.size(),
		"saved_scene_path": ProjectSettings.globalize_path(scene_path),
		"failures": [],
	}


func _failure(error_type: String, message: String, path: String, expected: Variant, actual: Variant, target := "") -> Dictionary:
	var failure := {
		"type": error_type,
		"message": message,
	}
	if path != "":
		failure["path"] = path
	if expected != null:
		failure["expected"] = expected
	if actual != null:
		failure["actual"] = actual
	if target != "":
		failure["target"] = target
	return {
		"ok": false,
		"scene": path,
		"applied_operations": 0,
		"saved_scene_path": "",
		"failures": [failure],
	}


func _read_json_file(path: String) -> Dictionary:
	var file := FileAccess.open(path, FileAccess.READ)
	if file == null:
		return {}
	var parser := JSON.new()
	var parse_error := parser.parse(file.get_as_text())
	if parse_error != OK or not parser.data is Dictionary:
		return {}
	return parser.data


func _write_json_file(path: String, payload: Dictionary) -> void:
	var file := FileAccess.open(path, FileAccess.WRITE)
	if file == null:
		return
	file.store_string(JSON.stringify(payload, "\t"))
