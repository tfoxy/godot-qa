extends SceneTree

const Probe = preload("res://addons/godot_qa/qa_asset_trial_probe.gd")
const Scene = preload("res://addons/godot_qa/qa_asset_trial_scene.gd")
const CAPABILITIES := ["asset_trial_v1"]


func _initialize() -> void:
	var args := OS.get_cmdline_user_args()
	var request_path := ""
	for index in args.size() - 1:
		if args[index] in ["--request", "--asset-trial-request"]:
			request_path = args[index + 1]
	var request = JSON.parse_string(FileAccess.get_file_as_string(request_path)) if request_path != "" else null
	if not request is Dictionary:
		quit(1)
		return
	var result: Dictionary
	if int(request.get("version", 0)) != 1:
		result = failure("asset_trial_request_invalid", "unsupported trial request version")
	elif OS.has_feature("editor") and Engine.is_editor_hint():
		result = failure("asset_trial_request_invalid", "run trial scripts as plain runtime, never editor")
	else:
		match str(request.get("operation", "")):
			"write": result = write_scene(request)
			"probe": result = probe_resources(request)
			"uid": result = probe_uid(request)
			"references": result = probe_references(request)
			_: result = failure("asset_trial_request_invalid", "unsupported trial operation")
	result["capabilities"] = CAPABILITIES
	result["backend"] = Probe.backend()
	var output := FileAccess.open(str(request.get("result_path", "")), FileAccess.WRITE)
	if output == null:
		quit(1)
		return
	output.store_string(JSON.stringify(result, "\t"))
	output.flush()
	output.close()
	quit(0 if bool(result.get("ok", false)) else 1)


func failure(kind: String, message: String, data: Dictionary = {}) -> Dictionary:
	return {"ok": false, "error": {"type": kind, "message": message, "data": data}}


func probe_resources(request: Dictionary) -> Dictionary:
	var resources: Array = []
	for path in request.get("resources", []):
		var resource := ResourceLoader.load(str(path), "", ResourceLoader.CACHE_MODE_IGNORE)
		if resource == null:
			return failure("asset_fingerprint_unavailable", "resource could not be loaded", {"target": path})
		var facts := Probe.resource_facts(resource)
		var config := ConfigFile.new()
		if config.load(str(path) + ".import") == OK:
			var settings := {}
			for section in config.get_sections():
				var values := {}
				for key in config.get_section_keys(section):
					values[key] = config.get_value(section, key)
				settings[section] = values
			facts["import"] = settings
			if resource is Texture2D and int(config.get_value("params", "compress/mode", -1)) != 0:
				return failure("asset_import_mode_unsupported", "texture trials require lossless import compress/mode=0", {"target": path, "actual": config.get_value("params", "compress/mode", -1)})
		if facts.has("fingerprint_error"):
			return failure("asset_fingerprint_unavailable", "loaded resource content is unavailable", {"target": path, "actual": facts})
		resources.append(facts)
	return {"ok": true, "data": {"resources": resources}}


func probe_references(request: Dictionary) -> Dictionary:
	var matches := {}
	for path in request.get("resources", []):
		matches[str(path)] = []
	var pending: Array[String] = ["res://"]
	var skipped: Array[String] = []
	var extensions := ResourceLoader.get_recognized_extensions_for_type("")
	while not pending.is_empty():
		var directory: String = pending.pop_back()
		var access := DirAccess.open(directory)
		if access == null:
			return failure("asset_trial_reference_check_unavailable", "resource directory could not be read", {"target": directory})
		access.include_hidden = true
		access.list_dir_begin()
		var entry := access.get_next()
		while entry != "":
			var path := directory.path_join(entry)
			if entry not in [".", "..", ".qa", ".godot", ".git"]:
				if access.is_link(entry):
					skipped.append(path)
				elif access.current_is_dir():
					pending.append(path)
				elif extensions.has(entry.get_extension().to_lower()):
					for dependency in ResourceLoader.get_dependencies(path):
						var parts := str(dependency).split("::")
						var target := str(parts[parts.size() - 1])
						if target.begins_with("uid://"):
							var id := ResourceUID.text_to_id(target)
							target = ResourceUID.get_id_path(id) if ResourceUID.has_id(id) else target
						if matches.has(target):
							matches[target].append(path)
			entry = access.get_next()
		access.list_dir_end()
	var resources: Array = []
	for path in matches:
		resources.append({"path": path, "referenced": not matches[path].is_empty(), "references": matches[path]})
	return {"ok": true, "data": {"resources": resources, "complete": skipped.is_empty(), "skipped_symlinks": skipped}}


func probe_uid(request: Dictionary) -> Dictionary:
	var scene := str(request.get("scene", ""))
	var actual := Scene.uid(scene)
	var expected := str(request.get("expected_uid", ""))
	var expected_id := ResourceUID.text_to_id(expected) if expected != "" else -1
	var mapped := ResourceUID.get_id_path(expected_id) if expected_id >= 0 and ResourceUID.has_id(expected_id) else ""
	if actual != expected or (expected != "" and mapped != scene):
		return failure("asset_trial_scene_uid_mapping_changed", "scene UID must retain its original header and real scene mapping", {"target": scene, "expected": {"uid": expected, "mapped_path": scene if expected != "" else ""}, "actual": {"uid": actual, "mapped_path": mapped}})
	return {"ok": true, "data": {"scene_uid": actual, "mapped_path": mapped}}


func write_scene(request: Dictionary) -> Dictionary:
	var scene := str(request.get("scene", ""))
	var output := str(request.get("output_path", ""))
	var global_output := ProjectSettings.globalize_path(output).simplify_path()
	var journal := ProjectSettings.globalize_path("res://.qa/asset-trials/")
	if not scene.begins_with("res://") or not scene.ends_with(".tscn") or not global_output.begins_with(journal) or not output.ends_with(".tscn") or FileAccess.file_exists(output):
		return failure("asset_trial_request_invalid", "writer requires a new hidden journal .tscn output and project scene", {"target": output})
	var packed := ResourceLoader.load(scene, "PackedScene", ResourceLoader.CACHE_MODE_IGNORE) as PackedScene
	if packed == null:
		return failure("asset_trial_scene_unsupported", "edited scene could not be loaded", {"target": scene})
	if packed.get_state().get_base_scene_state() != null:
		return failure("asset_trial_scene_unsupported", "inherited edited scenes are unsupported", {"target": scene})
	var excluded := {}
	for binding in request.get("bindings", []):
		if not binding is Dictionary:
			return failure("asset_trial_request_invalid", "binding must be an object")
		var path := str(binding.get("edit_path", "")).simplify_path().trim_prefix("./")
		excluded[path + ":" + str(binding.get("property", ""))] = true
	# Snapshot authored storage BEFORE constructors/setters can alter shared
	# embedded objects referenced by the PackedScene's SceneState.
	var original := Scene.state(packed, excluded)
	var original_uid := Scene.uid(scene)
	# Edit-state preserves unrelated instance boundaries, off-tree prevents
	# _enter_tree/_ready/process. _init/setters are constrained by the save gate.
	var root := packed.instantiate(PackedScene.GEN_EDIT_STATE_INSTANCE)
	if root == null:
		return failure("asset_trial_scene_unsupported", "edited scene could not be instantiated", {"target": scene})
	var seen_slots := {}
	var prepared: Array = []
	for binding in request.get("bindings", []):
		var path := str(binding.get("edit_path", ""))
		var property := str(binding.get("property", ""))
		var node := root.get_node_or_null(NodePath(path)) if path != "" and not path.begins_with("/") and not path.split("/").has("..") else null
		if node == null or not Probe.slot_supported(node, property) or (node != root and node.owner != root):
			root.free()
			return failure("asset_trial_slot_unsupported", "binding requires a direct scene-owned native slot", {"target": path, "property": property})
		var ancestor := node
		while ancestor != root:
			if ancestor.scene_file_path != "":
				root.free()
				return failure("asset_trial_slot_unsupported", "targets owned by scene instances are unsupported", {"target": path})
			ancestor = ancestor.get_parent()
		var slot := str(root.get_path_to(node)) + ":" + property
		if seen_slots.has(slot):
			root.free()
			return failure("asset_trial_slot_unsupported", "duplicate binding slot", {"target": slot})
		seen_slots[slot] = true
		var destination := str(binding.get("destination", ""))
		var resource := ResourceLoader.load(destination, "", ResourceLoader.CACHE_MODE_IGNORE)
		if (property == "texture" and not resource is Texture2D) or (property == "stream" and not resource is AudioStreamWAV):
			root.free()
			return failure("asset_trial_slot_unsupported", "candidate resource is incompatible with native slot", {"target": path, "destination": destination})
		prepared.append({"id": str(binding.get("id", "")), "node": node, "edit_path": path, "property": property, "destination": destination, "resource": resource, "original": Probe.resource_facts(node.get(property) as Resource)})
	if prepared.is_empty():
		root.free()
		return failure("asset_trial_request_invalid", "writer requires at least one binding")
	for item in prepared:
		item.node.set(item.property, item.resource)
	var candidate := PackedScene.new()
	var error := candidate.pack(root)
	if error != OK:
		root.free()
		return failure("asset_trial_roundtrip_unsupported", "scene pack failed", {"actual": error})
	error = ResourceSaver.save(candidate, output)
	if error == OK and original_uid != "":
		error = ResourceSaver.set_uid(output, ResourceUID.text_to_id(original_uid))
	if error != OK or Scene.uid(output) != original_uid:
		root.free()
		return failure("asset_trial_roundtrip_unsupported", "scene UID could not be preserved", {"expected": original_uid, "actual": Scene.uid(output), "save_error": error})
	var saved := ResourceLoader.load(output, "PackedScene", ResourceLoader.CACHE_MODE_IGNORE) as PackedScene
	if saved == null:
		root.free()
		return failure("asset_trial_roundtrip_unsupported", "saved scene could not be reloaded")
	var saved_state := Scene.state(saved, excluded)
	var gate := Scene.compare(original, saved_state, root)
	if not bool(gate.get("ok", false)):
		root.free()
		return failure("asset_trial_roundtrip_unsupported", "scene roundtrip changed unrelated authored semantics", gate)
	var reloaded := saved.instantiate(PackedScene.GEN_EDIT_STATE_INSTANCE)
	var facts: Array = []
	for item in prepared:
		var node := reloaded.get_node_or_null(NodePath(item.edit_path))
		var resource := node.get(item.property) as Resource if node != null else null
		if resource == null or resource.resource_path != item.destination:
			reloaded.free()
			root.free()
			return failure("asset_binding_unconfirmed", "saved scene did not retain candidate assignment", {"target": item.edit_path, "expected": item.destination})
		facts.append({"id": item.id, "edit_path": item.edit_path, "property": item.property, "original": item.original, "resource": Probe.resource_facts(resource)})
	reloaded.free()
	root.free()
	return {"ok": true, "data": {"scene_uid": original_uid, "original_state": original, "candidate_state": saved_state, "bindings": facts, "semantic_gate": gate}}
