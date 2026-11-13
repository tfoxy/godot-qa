extends RefCounted

const QaRuntimeProviders = preload("res://addons/godot_qa/qa_runtime_providers.gd")

var _runtime: WeakRef


func _init(runtime: RefCounted) -> void:
	_runtime = weakref(runtime)


func _teleport_node(params: Dictionary) -> Dictionary:
	var target := str(params.get("target", ""))
	var resolution: Dictionary = _runtime.get_ref().resolve_target(target)
	if not bool(resolution.get("ok", false)):
		return {"ok": false, "error": (resolution.get("error", {}) as Dictionary).duplicate(true)}
	var target_node := resolution.get("node") as Node
	var position_result := _resolve_setup_position(params, "to")
	if not bool(position_result.get("ok", false)):
		return position_result
	var applied := _apply_global_position(
		target_node,
		position_result,
		bool(params.get("preserve_velocity", false)),
	)
	if not bool(applied.get("ok", false)):
		return applied
	var applied_data: Dictionary = applied.get("data", {})
	await _runtime.get_ref()._process_frames(1)
	return _runtime.get_ref()._ok(
		{
			"target": target,
			"resolved": str(resolution.get("resolved", "")),
			"position": applied_data,
		}
	)


func _fixture_teleport_to_target(params: Dictionary) -> Dictionary:
	var target := str(params.get("target", ""))
	var destination := str(params.get("to", ""))
	var has_point := params.has("to_point")
	if target == "" or (destination == "" and not has_point and not params.has("position")):
		return _runtime.get_ref()._error(
			"invalid_params",
			"fixture.teleport_to_target requires non-empty target and destination",
			{
				"command": "fixture.teleport_to_target",
				"expected": {"target": "non-empty locator", "to": "locator, to_point provider, or position"},
				"actual": {"target": target, "to": destination, "to_point": params.get("to_point", null), "position": params.get("position", null)},
			},
		)
	return await _teleport_node(params)


func _spawn_scene(params: Dictionary) -> Dictionary:
	var scene_path := str(params.get("scene", ""))
	if scene_path == "" or not scene_path.begins_with("res://"):
		return _runtime.get_ref()._error(
			"invalid_params",
			"setup.spawn_scene requires a non-empty res:// scene path",
			{
				"command": "setup.spawn_scene",
				"expected": "res:// PackedScene path",
				"actual": scene_path,
			},
		)
	var scene_resource = load(scene_path)
	if scene_resource == null or not scene_resource is PackedScene:
		return _runtime.get_ref()._error(
			"scene_load_failed",
			"setup.spawn_scene could not load a PackedScene",
			{
				"scene": scene_path,
				"expected": "loadable PackedScene",
				"actual": "missing_or_invalid",
			},
		)

	var parent: Node = _runtime.get_ref()._scene_root
	var parent_target := str(params.get("parent", ""))
	var parent_resolved: String = _runtime.get_ref().stable_locator_for_node(parent)
	if parent_target != "":
		var parent_resolution: Dictionary = _runtime.get_ref().resolve_target(parent_target)
		if not bool(parent_resolution.get("ok", false)):
			return {"ok": false, "error": (parent_resolution.get("error", {}) as Dictionary).duplicate(true)}
		parent = parent_resolution.get("node") as Node
		parent_resolved = str(parent_resolution.get("resolved", ""))
	if parent == null:
		return _runtime.get_ref()._error(
			"target_not_found",
			"setup.spawn_scene parent is not available",
			{
				"parent": parent_target,
				"expected": "existing parent node",
				"actual": "missing",
			},
		)

	var instance := (scene_resource as PackedScene).instantiate()
	scene_resource = null
	if not instance is Node:
		return _runtime.get_ref()._error(
			"scene_instantiate_failed",
			"setup.spawn_scene did not instantiate a Node",
			{
				"scene": scene_path,
				"expected": "PackedScene root Node",
				"actual": type_string(typeof(instance)),
			},
		)
	var instance_node := instance as Node
	var requested_name := str(params.get("name", ""))
	if requested_name != "":
		instance_node.name = requested_name
	parent.add_child(instance_node)
	var qa_id := str(params.get("qa_id", ""))
	if qa_id != "":
		_runtime.get_ref().register_qa_id(instance_node, qa_id)

	var position_payload: Dictionary = {}
	if params.has("at") or params.has("position"):
		var position_result := _resolve_setup_position(params, "at")
		if not bool(position_result.get("ok", false)):
			instance_node.queue_free()
			return position_result
		var applied := _apply_global_position(instance_node, position_result)
		if not bool(applied.get("ok", false)):
			instance_node.queue_free()
			return applied
		position_payload = applied.get("data", {})
	await _runtime.get_ref()._process_frames(1)
	return _runtime.get_ref()._ok(
		{
			"scene": scene_path,
			"parent": parent_resolved,
			"resolved": "path:%s" % str(instance_node.get_path()),
			"target": _runtime.get_ref().stable_locator_for_node(instance_node),
			"position": position_payload,
		}
	)


func _set_property(params: Dictionary) -> Dictionary:
	var target := str(params.get("target", ""))
	var property_name := str(params.get("property", ""))
	if property_name == "":
		return _runtime.get_ref()._error(
			"invalid_params",
			"setup.set_property requires a non-empty property name",
			{
				"command": "setup.set_property",
				"expected": "non-empty property",
				"actual": property_name,
			},
		)
	var resolution: Dictionary = _runtime.get_ref().resolve_target(target)
	if not bool(resolution.get("ok", false)):
		return {"ok": false, "error": (resolution.get("error", {}) as Dictionary).duplicate(true)}
	var node := resolution.get("node") as Node
	if node == null or not _object_has_property(node, property_name):
		return _runtime.get_ref()._error(
			"property_not_found",
			"setup.set_property could not find the requested property",
			{
				"target": target,
				"property": property_name,
				"expected": "property on resolved node",
				"actual": node.get_class() if node != null else "missing",
			},
		)
	var before_value = _encode_setup_value(node.get(property_name))
	node.set(property_name, _decode_setup_value(params.get("value")))
	await _runtime.get_ref()._process_frames(1)
	return _runtime.get_ref()._ok(
		{
			"target": target,
			"resolved": str(resolution.get("resolved", "")),
			"property": property_name,
			"before": before_value,
			"after": _encode_setup_value(node.get(property_name)),
		}
	)


func _call_method(params: Dictionary) -> Dictionary:
	var target := str(params.get("target", ""))
	var method_name := str(params.get("method", ""))
	if not method_name.begins_with("qa_"):
		return _runtime.get_ref()._error(
			"fixture_method_not_allowed",
			"setup.call_method only allows qa_ fixture methods",
			{
				"target": target,
				"method": method_name,
				"expected": "method name beginning with qa_",
				"actual": method_name,
			},
		)
	var resolution: Dictionary = _runtime.get_ref().resolve_target(target)
	if not bool(resolution.get("ok", false)):
		return {"ok": false, "error": (resolution.get("error", {}) as Dictionary).duplicate(true)}
	var node := resolution.get("node") as Node
	if node == null or not node.has_method(method_name):
		return _runtime.get_ref()._error(
			"fixture_method_missing",
			"setup.call_method could not find the requested qa_ method",
			{
				"target": target,
				"method": method_name,
				"expected": "method on resolved node",
				"actual": node.get_class() if node != null else "missing",
			},
		)
	var args_value = params.get("args", [])
	var args: Array = args_value if args_value is Array else []
	var decoded_args: Array = []
	for arg in args:
		decoded_args.append(_decode_setup_value(arg))
	var result = node.callv(method_name, decoded_args)
	await _runtime.get_ref()._process_frames(1)
	return _runtime.get_ref()._ok(
		{
			"target": target,
			"resolved": str(resolution.get("resolved", "")),
			"method": method_name,
			"result": _encode_setup_value(result),
		}
	)


func _fixture_call(params: Dictionary) -> Dictionary:
	var driver_id := str(params.get("driver", "")).strip_edges()
	var action := str(params.get("action", "")).strip_edges()
	if driver_id == "" or action == "":
		return _runtime.get_ref()._error(
			"invalid_fixture_call",
			"fixture.call requires non-empty driver and action",
			{
				"command": "fixture.call",
				"expected": {"driver": "non-empty provider id", "action": "non-empty action"},
				"actual": params.duplicate(true),
			},
		)
	var call_params = params.get("params", {})
	if not (call_params is Dictionary):
		return _runtime.get_ref()._error(
			"invalid_fixture_call",
			"fixture.call params must be a dictionary when present",
			{
				"command": "fixture.call",
				"driver": driver_id,
				"action": action,
				"expected": "Dictionary",
				"actual": call_params,
			},
		)
	var driver: Dictionary = _runtime.get_ref()._providers._provider_entry(_runtime.get_ref()._fixture_drivers, driver_id, "fixture_driver")
	if not bool(driver.get("ok", false)):
		return driver
	var entry: Dictionary = driver.get("entry", {})
	var owner: Object = entry.get("owner")
	var method_name := str(entry.get("method", ""))
	var request := {
		"driver": driver_id,
		"action": action,
		"params": (call_params as Dictionary).duplicate(true),
	}
	var result = owner.call(method_name, request)
	await _runtime.get_ref()._process_frames(1)
	if not (result is Dictionary):
		return _runtime.get_ref()._error(
			"invalid_fixture_result",
			"fixture driver must return a protocol dictionary",
			{
				"command": "fixture.call",
				"driver": driver_id,
				"action": action,
				"expected": {"ok": "boolean", "data": "Dictionary on success", "error": "Dictionary on failure"},
				"actual": _encode_setup_value(result),
			},
		)
	var result_dict: Dictionary = result
	if result_dict.has("ok") and not bool(result_dict.get("ok", false)):
		var error_payload = result_dict.get("error", {})
		if error_payload is Dictionary:
			return {"ok": false, "error": (error_payload as Dictionary).duplicate(true)}
		return _runtime.get_ref()._error(
			"fixture_driver_failed",
			"fixture driver reported failure",
			{"command": "fixture.call", "driver": driver_id, "action": action, "actual": result_dict.duplicate(true)},
		)
	if not bool(result_dict.get("ok", false)):
		return _runtime.get_ref()._error(
			"invalid_fixture_result",
			"fixture driver result must include ok: true on success",
			{"command": "fixture.call", "driver": driver_id, "action": action, "actual": result_dict.duplicate(true)},
		)
	var data = result_dict.get("data", {})
	if not (data is Dictionary):
		return _runtime.get_ref()._error(
			"invalid_fixture_result",
			"fixture driver success data must be a dictionary",
			{"command": "fixture.call", "driver": driver_id, "action": action, "actual": result_dict.duplicate(true)},
		)
	var payload: Dictionary = (data as Dictionary).duplicate(true)
	payload["driver"] = driver_id
	payload["action"] = action
	return _runtime.get_ref()._ok(payload)


func _resolve_setup_position(params: Dictionary, locator_field: String) -> Dictionary:
	var has_locator := params.has(locator_field)
	var has_position := params.has("position")
	var has_point := params.has("to_point")
	var destination_count := (1 if has_locator else 0) + (1 if has_position else 0) + (1 if has_point else 0)
	if destination_count > 1:
		return _runtime.get_ref()._error(
			"invalid_params",
			"setup position requires exactly one destination source",
			{
				"expected": "only one of %s, position, or to_point" % locator_field,
				"actual": {
					locator_field: params.get(locator_field, null),
					"position": params.get("position", null),
					"to_point": params.get("to_point", null),
				},
			},
		)
	if has_point:
		return _resolve_setup_point_provider(params.get("to_point"), params.get("offset", null))
	if params.has(locator_field):
		var target := str(params.get(locator_field, ""))
		var resolution: Dictionary = _runtime.get_ref().resolve_target(target)
		if not bool(resolution.get("ok", false)):
			return {"ok": false, "error": (resolution.get("error", {}) as Dictionary).duplicate(true)}
		var node := resolution.get("node") as Node
		var position := _global_position_for_node(node)
		if position.is_empty():
			return _runtime.get_ref()._error(
				"target_position_unavailable",
				"Resolved target does not expose a supported global position",
				{
					"target": target,
					"resolved": str(resolution.get("resolved", "")),
					"expected": "Node2D, Control, or Node3D target",
					"actual": node.get_class() if node != null else "missing",
				},
			)
		position["source"] = target
		position["source_resolved"] = str(resolution.get("resolved", ""))
		return _runtime.get_ref()._ok(_position_with_offset(position, params.get("offset", null)))
	if params.has("position"):
		var position_value = params.get("position")
		var position := _coerce_position_dictionary(position_value)
		if position.is_empty():
			return _runtime.get_ref()._error(
				"invalid_params",
				"setup position requires numeric x and y fields",
				{
					"expected": {"x": "number", "y": "number", "z": "optional number"},
					"actual": position_value,
				},
			)
		return _runtime.get_ref()._ok(_position_with_offset(position, params.get("offset", null)))
	return _runtime.get_ref()._error(
		"invalid_params",
		"setup position requires a locator or explicit position",
		{
			"expected": "%s locator, position object, or to_point provider" % locator_field,
			"actual": params.duplicate(true),
		},
	)


func _resolve_setup_point_provider(raw_point_request: Variant, raw_offset: Variant = null) -> Dictionary:
	if not raw_point_request is Dictionary:
		return _runtime.get_ref()._error(
			"invalid_params",
			"to_point requires a provider request object",
			{
				"expected": {"provider": "qa:<id>", "method": "qa_point_probe", "id": "logical-point-id"},
				"actual": raw_point_request,
			},
		)
	var point_request: Dictionary = raw_point_request
	var provider_target := str(point_request.get("provider", "")).strip_edges()
	var method_name := str(point_request.get("method", "qa_point_probe")).strip_edges()
	var point_id := str(point_request.get("id", point_request.get("point_id", ""))).strip_edges()
	if provider_target == "" or method_name == "" or point_id == "":
		return _runtime.get_ref()._error(
			"invalid_params",
			"to_point requires non-empty provider, method, and id",
			{
				"expected": {"provider": "non-empty locator", "method": "qa_ method", "id": "non-empty logical id"},
				"actual": point_request.duplicate(true),
			},
		)
	var method_check := _validate_readonly_qa_probe_method(method_name)
	if not bool(method_check.get("ok", false)):
		return _runtime.get_ref()._error(
			"point_provider_not_allowed",
			"to_point provider method must be a read-only QA probe method",
			{
				"provider": provider_target,
				"method": method_name,
				"expected": "qa_ method without mutating prefix",
				"actual": method_check.get("reason", "invalid_method"),
			},
		)
	var provider_resolution: Dictionary = _runtime.get_ref().resolve_target(provider_target)
	if not bool(provider_resolution.get("ok", false)):
		return {"ok": false, "error": (provider_resolution.get("error", {}) as Dictionary).duplicate(true)}
	var provider_node := provider_resolution.get("node") as Node
	if provider_node == null or not provider_node.has_method(method_name):
		return _runtime.get_ref()._error(
			"point_provider_missing",
			"to_point provider does not expose the requested method",
			{
				"provider": provider_target,
				"resolved": str(provider_resolution.get("resolved", "")),
				"method": method_name,
				"expected": "method on resolved provider",
				"actual": provider_node.get_class() if provider_node != null else "missing",
			},
		)
	var result = provider_node.call(method_name, point_id)
	if not result is Dictionary:
		return _runtime.get_ref()._error(
			"invalid_point_provider_result",
			"to_point provider must return a Dictionary",
			{
				"provider": provider_target,
				"method": method_name,
				"id": point_id,
				"expected": "Dictionary",
				"actual": type_string(typeof(result)),
			},
		)
	var point_result: Dictionary = result
	if not bool(point_result.get("ok", false)):
		return _runtime.get_ref()._error(
			"point_provider_rejected",
			"to_point provider could not resolve the logical point",
			{
				"provider": provider_target,
				"method": method_name,
				"id": point_id,
				"expected": "ok point result",
				"actual": point_result.duplicate(true),
			},
		)
	var coordinate_space := str(point_result.get("coordinate_space", "world_2d")).strip_edges()
	if coordinate_space != "world_2d":
		return _runtime.get_ref()._error(
			"unsupported_coordinate_space",
			"to_point provider must return world_2d coordinates",
			{
				"provider": provider_target,
				"method": method_name,
				"id": point_id,
				"expected": "world_2d",
				"actual": coordinate_space,
			},
		)
	var raw_point = point_result.get("point", point_result.get("position", point_result.get("global_position", null)))
	var position := _coerce_position_dictionary(raw_point)
	if position.is_empty():
		return _runtime.get_ref()._error(
			"invalid_point_provider_result",
			"to_point provider result must include numeric point coordinates",
			{
				"provider": provider_target,
				"method": method_name,
				"id": point_id,
				"expected": {"point": {"x": "number", "y": "number"}},
				"actual": point_result.duplicate(true),
			},
		)
	position["source"] = "to_point:%s" % point_id
	position["source_resolved"] = str(provider_resolution.get("resolved", ""))
	position["provider"] = provider_target
	position["provider_method"] = method_name
	position["logical_id"] = str(point_result.get("logical_id", point_id))
	return _runtime.get_ref()._ok(_position_with_offset(position, raw_offset))


func _validate_readonly_qa_probe_method(method_name: String) -> Dictionary:
	if not method_name.begins_with("qa_"):
		return {"ok": false, "reason": "method_must_begin_with_qa_"}
	var mutating_prefixes := ["qa_set_", "qa_add_", "qa_remove_", "qa_delete_", "qa_clear_", "qa_queue_", "qa_free", "qa_mutate_"]
	for prefix in mutating_prefixes:
		if method_name.begins_with(prefix):
			return {"ok": false, "reason": "mutating_method_prefix"}
	return {"ok": true}


func _position_with_offset(position: Dictionary, raw_offset: Variant) -> Dictionary:
	var result := position.duplicate(true)
	if raw_offset == null:
		return result
	var offset := _coerce_position_dictionary(raw_offset)
	if offset.is_empty():
		return result
	result["x"] = float(result.get("x", 0.0)) + float(offset.get("x", 0.0))
	result["y"] = float(result.get("y", 0.0)) + float(offset.get("y", 0.0))
	if result.has("z") or offset.has("z"):
		result["z"] = float(result.get("z", 0.0)) + float(offset.get("z", 0.0))
	result["offset"] = offset
	return result


func _apply_global_position(node: Node, position_result: Dictionary, preserve_velocity := false) -> Dictionary:
	if node == null:
		return _runtime.get_ref()._error(
			"target_not_found",
			"No node matched the requested target",
			{
				"expected": "existing runtime node",
				"actual": "missing",
			},
		)
	var position_data: Dictionary = position_result.get("data", {})
	var x := float(position_data.get("x", 0.0))
	var y := float(position_data.get("y", 0.0))
	if node is Node2D:
		(node as Node2D).global_position = Vector2(x, y)
		return _runtime.get_ref()._ok({"x": x, "y": y, "velocity_reset": _reset_transient_physics_state(node, preserve_velocity)})
	if node is Control:
		(node as Control).global_position = Vector2(x, y)
		return _runtime.get_ref()._ok({"x": x, "y": y, "velocity_reset": false})
	if node is Node3D:
		var current := (node as Node3D).global_position
		var z := float(position_data.get("z", current.z))
		(node as Node3D).global_position = Vector3(x, y, z)
		return _runtime.get_ref()._ok({"x": x, "y": y, "z": z, "velocity_reset": _reset_transient_physics_state(node, preserve_velocity)})
	return _runtime.get_ref()._error(
		"target_position_unsupported",
		"Resolved target does not support global-position setup",
		{
			"target": _runtime.get_ref().describe_node(node),
			"expected": "Node2D, Control, or Node3D target",
			"actual": node.get_class(),
		},
	)


func _reset_transient_physics_state(node: Node, preserve_velocity: bool) -> bool:
	if preserve_velocity:
		return false
	if node is CharacterBody2D:
		(node as CharacterBody2D).velocity = Vector2.ZERO
		return true
	if node is CharacterBody3D:
		(node as CharacterBody3D).velocity = Vector3.ZERO
		return true
	if node is RigidBody2D:
		(node as RigidBody2D).linear_velocity = Vector2.ZERO
		(node as RigidBody2D).angular_velocity = 0.0
		return true
	if node is RigidBody3D:
		(node as RigidBody3D).linear_velocity = Vector3.ZERO
		(node as RigidBody3D).angular_velocity = Vector3.ZERO
		return true
	return false


func _global_position_for_node(node: Node) -> Dictionary:
	if node is Node2D:
		var position_2d := (node as Node2D).global_position
		return {"x": position_2d.x, "y": position_2d.y}
	if node is Control:
		var position_control := (node as Control).global_position
		return {"x": position_control.x, "y": position_control.y}
	if node is Node3D:
		var position_3d := (node as Node3D).global_position
		return {"x": position_3d.x, "y": position_3d.y, "z": position_3d.z}
	return {}


func _coerce_position_dictionary(value: Variant) -> Dictionary:
	if not value is Dictionary:
		return {}
	var position: Dictionary = value
	if not _runtime.get_ref()._is_number(position.get("x")) or not _runtime.get_ref()._is_number(position.get("y")):
		return {}
	var result := {
		"x": float(position.get("x")),
		"y": float(position.get("y")),
	}
	if position.has("z"):
		if not _runtime.get_ref()._is_number(position.get("z")):
			return {}
		result["z"] = float(position.get("z"))
	return result


func _object_has_property(object: Object, property_name: String) -> bool:
	for property in object.get_property_list():
		if not property is Dictionary:
			continue
		if str((property as Dictionary).get("name", "")) == property_name:
			return true
	return false


func _decode_setup_value(value: Variant) -> Variant:
	if value is Dictionary:
		var dictionary: Dictionary = value
		var value_type := str(dictionary.get("type", ""))
		match value_type:
			"Vector2":
				if _runtime.get_ref()._is_number(dictionary.get("x")) and _runtime.get_ref()._is_number(dictionary.get("y")):
					return Vector2(float(dictionary.get("x")), float(dictionary.get("y")))
			"Vector2i":
				if _runtime.get_ref()._is_number(dictionary.get("x")) and _runtime.get_ref()._is_number(dictionary.get("y")):
					return Vector2i(int(dictionary.get("x")), int(dictionary.get("y")))
			"Vector3":
				if _runtime.get_ref()._is_number(dictionary.get("x")) and _runtime.get_ref()._is_number(dictionary.get("y")) and _runtime.get_ref()._is_number(dictionary.get("z")):
					return Vector3(float(dictionary.get("x")), float(dictionary.get("y")), float(dictionary.get("z")))
			"Color":
				if _runtime.get_ref()._is_number(dictionary.get("r")) and _runtime.get_ref()._is_number(dictionary.get("g")) and _runtime.get_ref()._is_number(dictionary.get("b")):
					return Color(
						float(dictionary.get("r")),
						float(dictionary.get("g")),
						float(dictionary.get("b")),
						float(dictionary.get("a", 1.0))
					)
		var decoded := {}
		for key in dictionary.keys():
			decoded[key] = _decode_setup_value(dictionary.get(key))
		return decoded
	if value is Array:
		var decoded_array: Array = []
		for item in value:
			decoded_array.append(_decode_setup_value(item))
		return decoded_array
	return value


func _encode_setup_value(value: Variant) -> Variant:
	match typeof(value):
		TYPE_NIL, TYPE_BOOL, TYPE_INT, TYPE_FLOAT, TYPE_STRING:
			return value
		TYPE_VECTOR2:
			var vector2: Vector2 = value
			return {"type": "Vector2", "x": vector2.x, "y": vector2.y}
		TYPE_VECTOR2I:
			var vector2i: Vector2i = value
			return {"type": "Vector2i", "x": vector2i.x, "y": vector2i.y}
		TYPE_VECTOR3:
			var vector3: Vector3 = value
			return {"type": "Vector3", "x": vector3.x, "y": vector3.y, "z": vector3.z}
		TYPE_COLOR:
			var color: Color = value
			return {"type": "Color", "r": color.r, "g": color.g, "b": color.b, "a": color.a}
		TYPE_ARRAY:
			var encoded_array: Array = []
			for item in value:
				encoded_array.append(_encode_setup_value(item))
			return encoded_array
		TYPE_DICTIONARY:
			var encoded_dictionary := {}
			for key in value.keys():
				encoded_dictionary[str(key)] = _encode_setup_value(value.get(key))
			return encoded_dictionary
	if value is Node:
		return _runtime.get_ref().stable_locator_for_node(value as Node)
	return str(value)
