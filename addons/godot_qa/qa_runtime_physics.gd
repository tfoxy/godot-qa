extends RefCounted

var _runtime: WeakRef


func _init(runtime: RefCounted) -> void:
	_runtime = weakref(runtime)


func _physics_inspect(params: Dictionary) -> Dictionary:
	var target := str(params.get("target", ""))
	var resolution: Dictionary = _runtime.get_ref().resolve_target(target)
	if not bool(resolution.get("ok", false)):
		return {"ok": false, "error": (resolution.get("error", {}) as Dictionary).duplicate(true)}
	var target_node := resolution.get("node") as Node
	var radius := clampf(float(params.get("radius", 256.0)), 1.0, 4096.0)
	var frames := clampi(int(params.get("frames", 1)), 1, 240)
	if frames > 1:
		for _index in range(frames - 1):
			await _runtime.get_ref()._process_frames(1)

	var target_position: Vector2 = _runtime.get_ref()._shapes._global_position_vector2_for_node(target_node)
	var target_visual_rect: Dictionary = _runtime.get_ref()._shapes._visual_rect_for_node(target_node)
	var target_info := _physics_target_info(target, resolution, target_node, target_position, target_visual_rect)
	var collision_objects: Array[Dictionary] = []
	var target_shapes: Array[Dictionary] = []
	_collect_collision_object_reports(_runtime.get_ref()._scene_root, target_node, target_position, radius, target_visual_rect, collision_objects, target_shapes)
	var space_queries := _physics_space_queries(target_node, target_position, radius, target_shapes)
	var project_state := _collect_physics_project_state(
		target_node,
		params,
		bool(params.get("include_project_state", true)),
	)
	var diagnostics := _physics_diagnostics(target_info, collision_objects, target_shapes, project_state)
	return _runtime.get_ref()._ok({
		"target": target,
		"resolved": str(resolution.get("resolved", "")),
		"frame": Engine.get_process_frames(),
		"time_msec": Time.get_ticks_msec(),
		"sample": {
			"frames": frames,
			"radius": radius,
		},
		"target_node": target_info,
		"collision_objects": collision_objects,
		"target_shapes": target_shapes,
		"space_queries": space_queries,
		"project_state": project_state,
		"diagnostics": diagnostics,
	})


func _physics_overlay_collect(params: Dictionary) -> Dictionary:
	var viewport_wide := bool(params.get("viewport", false))
	var target := str(params.get("target", "")).strip_edges()
	var target_node: Node = _runtime.get_ref()._scene_root
	var resolution := {
		"ok": true,
		"resolved": "scene-root",
		"node": _runtime.get_ref()._scene_root,
	}
	if not viewport_wide or target != "":
		if target == "":
			return _runtime.get_ref()._error(
				"invalid_params",
				"physics.overlay.collect requires target unless viewport collection is enabled",
				{
					"command": "physics.overlay.collect",
					"field": "target",
					"expected": "non-empty target locator or viewport=true",
					"actual": params.get("target", null),
				},
			)
		resolution = _runtime.get_ref().resolve_target(target)
		if not bool(resolution.get("ok", false)):
			return {"ok": false, "error": (resolution.get("error", {}) as Dictionary).duplicate(true)}
		target_node = resolution.get("node") as Node
	if target_node == null:
		return _runtime.get_ref()._error(
			"scene_unavailable",
			"physics.overlay.collect target is not available",
			{
				"command": "physics.overlay.collect",
				"target": target,
			},
		)

	var radius := clampf(float(params.get("radius", 256.0)), 1.0, 4096.0)
	if viewport_wide and not params.has("radius"):
		radius = 4096.0
	var frames := clampi(int(params.get("frames", 1)), 1, 240)
	if frames > 1:
		for _index in range(frames - 1):
			await _runtime.get_ref()._process_frames(1)

	var target_position: Vector2 = _runtime.get_ref()._shapes._global_position_vector2_for_node(target_node)
	var target_visual_rect: Dictionary = _runtime.get_ref()._shapes._visual_rect_for_node(target_node)
	var collision_objects: Array[Dictionary] = []
	var target_shapes: Array[Dictionary] = []
	var collection_metadata := {
		"include_screen": true,
		"max_shapes": clampi(int(params.get("max_shapes", 512)), 1, 4096),
		"shape_count_total": 0,
		"shape_count_returned": 0,
		"shapes_truncated": false,
	}
	_collect_collision_object_reports(
		_runtime.get_ref()._scene_root,
		target_node,
		target_position,
		radius,
		target_visual_rect,
		collision_objects,
		target_shapes,
		collection_metadata,
	)
	var overlay_shapes := _overlay_shape_entries(collision_objects, bool(params.get("include_aabb", true)))
	var project_state := _collect_physics_project_state(
		target_node,
		params,
		bool(params.get("include_project_state", true)),
	)
	var contacts := _collect_physics_contacts(collision_objects, bool(params.get("include_contacts", false)))
	var warnings: Array[Dictionary] = []
	if overlay_shapes.is_empty():
		warnings.append({
			"id": "physics_overlay.no_collision_shapes_found",
			"severity": "warning",
			"target": target,
			"radius": radius,
		})
	if bool(collection_metadata.get("shapes_truncated", false)):
		warnings.append({
			"id": "physics_overlay.unbounded_shape_count_truncated",
			"severity": "warning",
			"max_shapes": collection_metadata.get("max_shapes", 0),
			"shape_count_total": collection_metadata.get("shape_count_total", 0),
			"shape_count_returned": collection_metadata.get("shape_count_returned", 0),
		})

	return _runtime.get_ref()._ok({
		"command": "physics.overlay.collect",
		"target": target,
		"viewport": viewport_wide,
		"resolved": str(resolution.get("resolved", "")),
		"frame": Engine.get_process_frames(),
		"time_msec": Time.get_ticks_msec(),
		"sample": {
			"frames": frames,
			"radius": radius,
		},
		"metadata": {
			"shape_count": overlay_shapes.size(),
			"shape_count_total": int(collection_metadata.get("shape_count_total", overlay_shapes.size())),
			"shape_count_returned": int(collection_metadata.get("shape_count_returned", overlay_shapes.size())),
			"truncated": bool(collection_metadata.get("shapes_truncated", false)),
			"max_shapes": int(collection_metadata.get("max_shapes", 0)),
			"body_count": _overlay_object_count(collision_objects, false),
			"area_count": _overlay_object_count(collision_objects, true),
			"contact_count": contacts.size(),
			"warnings": warnings,
		},
		"collision_objects": collision_objects,
		"shapes": overlay_shapes,
		"contacts": contacts,
		"project_state": project_state,
	})


func _physics_target_info(target: String, resolution: Dictionary, node: Node, position: Vector2, visual_rect: Dictionary) -> Dictionary:
	var payload := {
		"target": target,
		"resolved": str(resolution.get("resolved", "")),
		"path": str(node.get_path()),
		"name": str(node.name),
		"type": node.get_class(),
		"qa_id": str(node.get_meta("qa_id", "")),
		"position": _runtime.get_ref()._shapes._vector2_payload(position),
		"transform": _runtime.get_ref()._shapes._node_transform_payload(node),
		"rotation": _runtime.get_ref().rotation_for_node(node),
		"velocity": _runtime.get_ref()._shapes._physics_velocity_payload(node),
		"angular_velocity": _runtime.get_ref()._shapes._physics_angular_velocity_payload(node),
		"visible": _runtime.get_ref()._shapes._node_visible_for_physics(node),
		"visual_rect": visual_rect,
		"is_collision_object_2d": node is CollisionObject2D,
	}
	if node is CollisionObject2D:
		var collision_object := node as CollisionObject2D
		payload["collision_layer"] = collision_object.collision_layer
		payload["collision_mask"] = collision_object.collision_mask
	return payload


func _collect_collision_object_reports(
	root: Node,
	target_node: Node,
	target_position: Vector2,
	radius: float,
	target_visual_rect: Dictionary,
	reports: Array,
	target_shapes: Array,
	collection_options := {}
) -> void:
	if root == null:
		return
	if root is CollisionObject2D:
		var collision_object := root as CollisionObject2D
		var object_position: Vector2 = _runtime.get_ref()._shapes._global_position_vector2_for_node(collision_object)
		var object_shapes := _collision_object_shapes_report(collision_object, target_position, target_visual_rect, collection_options)
		var nearest_distance := target_position.distance_to(object_position)
		for shape_report in object_shapes:
			if shape_report is Dictionary:
				nearest_distance = minf(nearest_distance, float((shape_report as Dictionary).get("distance_to_target", nearest_distance)))
		if nearest_distance <= radius or root == target_node:
			var reported_shapes := _bounded_collision_object_shapes(object_shapes, collection_options)
			var object_report := {
				"path": str(collision_object.get_path()),
				"name": str(collision_object.name),
				"type": collision_object.get_class(),
				"qa_id": str(collision_object.get_meta("qa_id", "")),
				"position": _runtime.get_ref()._shapes._vector2_payload(object_position),
				"visible": _runtime.get_ref()._shapes._node_visible_for_physics(collision_object),
				"collision_layer": collision_object.collision_layer,
				"collision_mask": collision_object.collision_mask,
				"distance_to_target": nearest_distance,
				"is_target": root == target_node,
				"is_area": collision_object is Area2D,
				"is_body": collision_object is PhysicsBody2D,
				"shapes": reported_shapes,
			}
			reports.append(object_report)
			if root == target_node:
				for shape_report in reported_shapes:
					target_shapes.append((shape_report as Dictionary).duplicate(true))
	for child in root.get_children():
		if child is Node:
			_collect_collision_object_reports(child as Node, target_node, target_position, radius, target_visual_rect, reports, target_shapes, collection_options)


func _collision_object_shapes_report(collision_object: CollisionObject2D, target_position: Vector2, target_visual_rect: Dictionary, collection_options := {}) -> Array[Dictionary]:
	var shapes: Array[Dictionary] = []
	var options: Dictionary = collection_options if collection_options is Dictionary else {}
	var include_screen := bool(options.get("include_screen", false))
	for owner_id in collision_object.get_shape_owners():
		var owner = collision_object.shape_owner_get_owner(owner_id)
		var owner_path := ""
		if owner is Node:
			owner_path = str((owner as Node).get_path())
		var owner_transform: Transform2D = collision_object.global_transform * collision_object.shape_owner_get_transform(owner_id)
		var disabled := false
		if collision_object.has_method("is_shape_owner_disabled"):
			disabled = bool(collision_object.call("is_shape_owner_disabled", owner_id))
		var one_way := false
		if collision_object.has_method("shape_owner_get_one_way_collision"):
			one_way = bool(collision_object.call("shape_owner_get_one_way_collision", owner_id))
		var shape_count := collision_object.shape_owner_get_shape_count(owner_id)
		for shape_index in range(shape_count):
			var shape := collision_object.shape_owner_get_shape(owner_id, shape_index)
			var aabb: Dictionary = _runtime.get_ref()._shapes._shape2d_aabb_payload(shape, owner_transform)
			var shape_payload := {
				"node_path": str(collision_object.get_path()),
				"node_type": collision_object.get_class(),
				"qa_id": str(collision_object.get_meta("qa_id", "")),
				"layer": collision_object.collision_layer,
				"mask": collision_object.collision_mask,
				"is_area": collision_object is Area2D,
				"is_body": collision_object is PhysicsBody2D,
				"is_soft": false,
				"owner_id": owner_id,
				"shape_index": shape_index,
				"shape_type": _runtime.get_ref()._shapes._shape_type_name(shape),
				"overlay_shape_type": _runtime.get_ref()._shapes._shape2d_overlay_type_name(shape),
				"owner_path": owner_path,
				"disabled": disabled,
				"one_way_collision": one_way,
				"transform": _runtime.get_ref()._shapes._transform2d_payload(owner_transform),
				"aabb": aabb,
				"world_aabb": aabb,
				"distance_to_target": _runtime.get_ref()._shapes._distance_from_rect_to_point(aabb, target_position),
				"large_shape_vs_visual": _runtime.get_ref()._shapes._shape_large_vs_visual(aabb, target_visual_rect),
			}
			if include_screen:
				var projection: Dictionary = _runtime.get_ref()._shapes._shape2d_projection_payload(shape, owner_transform, _runtime.get_ref()._shapes._screen_transform_for_collision_object(collision_object))
				shape_payload["world_points"] = projection.get("world_points", [])
				shape_payload["screen_points"] = projection.get("screen_points", [])
				shape_payload["screen_aabb"] = projection.get("screen_aabb", {})
			shapes.append(shape_payload)
	return shapes


func _bounded_collision_object_shapes(object_shapes: Array[Dictionary], collection_options := {}) -> Array[Dictionary]:
	if not collection_options is Dictionary or (collection_options as Dictionary).is_empty():
		return object_shapes
	var options: Dictionary = collection_options
	options["shape_count_total"] = int(options.get("shape_count_total", 0)) + object_shapes.size()
	var max_shapes := int(options.get("max_shapes", 0))
	if max_shapes <= 0:
		options["shape_count_returned"] = int(options.get("shape_count_returned", 0)) + object_shapes.size()
		return object_shapes
	var remaining := max_shapes - int(options.get("shape_count_returned", 0))
	if remaining <= 0:
		options["shapes_truncated"] = true
		return []
	var returned: Array[Dictionary] = []
	var returned_count := mini(remaining, object_shapes.size())
	for index in range(returned_count):
		returned.append(object_shapes[index])
	options["shape_count_returned"] = int(options.get("shape_count_returned", 0)) + returned.size()
	if returned.size() < object_shapes.size():
		options["shapes_truncated"] = true
	return returned


func _physics_space_queries(target_node: Node, target_position: Vector2, radius: float, target_shapes: Array) -> Dictionary:
	var world: World2D = _runtime.get_ref()._shapes._world_2d_for_node(target_node)
	if world == null:
		return {"available": false, "reason": "world_2d_unavailable"}
	var space_state := world.direct_space_state
	var radius_shape := CircleShape2D.new()
	radius_shape.radius = radius
	var radius_query := PhysicsShapeQueryParameters2D.new()
	radius_query.shape = radius_shape
	radius_query.transform = Transform2D(0.0, target_position)
	radius_query.collision_mask = 0xFFFFFFFF
	radius_query.collide_with_areas = true
	radius_query.collide_with_bodies = true
	var radius_hits: Array = _runtime.get_ref()._shapes._serialize_intersect_shape_results(space_state.intersect_shape(radius_query, 64), target_node)
	var target_overlap_hits: Array = []
	for shape_report in target_shapes:
		if not shape_report is Dictionary:
			continue
		var source_shape: Shape2D = _runtime.get_ref()._shapes._shape_from_target_report(target_node, shape_report as Dictionary)
		if source_shape == null:
			continue
		var target_query := PhysicsShapeQueryParameters2D.new()
		target_query.shape = source_shape
		target_query.transform = _runtime.get_ref()._shapes._transform2d_from_payload((shape_report as Dictionary).get("transform", {}))
		target_query.collision_mask = 0xFFFFFFFF
		target_query.collide_with_areas = true
		target_query.collide_with_bodies = true
		if target_node is CollisionObject2D:
			target_query.exclude = [(target_node as CollisionObject2D).get_rid()]
		target_overlap_hits.append({
			"owner_id": (shape_report as Dictionary).get("owner_id", null),
			"shape_index": (shape_report as Dictionary).get("shape_index", null),
			"hits": _runtime.get_ref()._shapes._serialize_intersect_shape_results(space_state.intersect_shape(target_query, 32), target_node),
		})
	return {
		"available": true,
		"radius_query": {
			"radius": radius,
			"hit_count": radius_hits.size(),
			"hits": radius_hits,
		},
		"target_shape_overlaps": target_overlap_hits,
	}


func _collect_physics_project_state(target_node: Node, params: Dictionary, include_project_state: bool) -> Dictionary:
	if not include_project_state:
		return {"enabled": false, "available": false, "providers": []}
	var raw_methods: Array = params.get("state_methods", []) if params.get("state_methods", []) is Array else []
	var methods: Array[String] = []
	if raw_methods.is_empty():
		methods = [
			"get_qa_collision_state",
			"get_collision_qa_state",
			"get_qa_physics_state",
			"get_physics_qa_state",
			"qa_collision_probe",
			"get_qa_collision_footprint_state",
			"get_qa_visual_footprint_state",
			"get_qa_collision_visual_state",
		]
	else:
		for raw_method in raw_methods:
			var method := str(raw_method)
			if _runtime.get_ref()._shapes._is_allowed_physics_state_method(method):
				methods.append(method)
	var providers: Array[Node] = []
	if raw_methods.is_empty():
		if _runtime.get_ref()._scene_root != null:
			providers.append(_runtime.get_ref()._scene_root)
		if target_node != null and target_node != _runtime.get_ref()._scene_root:
			providers.append(target_node)
	else:
		var provider_locator := str(params.get("state_provider", "scene-root"))
		if provider_locator == "scene-root":
			if _runtime.get_ref()._scene_root != null:
				providers.append(_runtime.get_ref()._scene_root)
		else:
			var provider_resolution: Dictionary = _runtime.get_ref().resolve_target(provider_locator)
			if bool(provider_resolution.get("ok", false)):
				providers.append(provider_resolution.get("node") as Node)
			else:
				return {
					"enabled": true,
					"available": false,
					"providers": [],
					"error": provider_resolution.get("error", {}),
				}
	var provider_payloads: Array = []
	var available := false
	for provider in providers:
		var methods_payload: Array = []
		for method in methods:
			var method_payload := _call_physics_state_method(provider, method)
			method_payload = _bounded_visual_footprint_method_payload(method_payload, params)
			methods_payload.append(method_payload)
			available = available or bool(method_payload.get("ok", false))
		provider_payloads.append({
			"resolved": "path:%s" % str(provider.get_path()),
			"path": str(provider.get_path()),
			"type": provider.get_class(),
			"methods": methods_payload,
		})
	return {
		"enabled": true,
		"available": available,
		"providers": provider_payloads,
	}


func _bounded_visual_footprint_method_payload(method_payload: Dictionary, params: Dictionary) -> Dictionary:
	if not bool(method_payload.get("ok", false)):
		return method_payload
	var value: Variant = method_payload.get("value", null)
	if not value is Dictionary:
		return method_payload
	var dictionary: Dictionary = value
	if not dictionary.has("visual_footprints") or not dictionary.get("visual_footprints") is Array:
		return method_payload
	var max_entries := clampi(int(params.get("max_shapes", 512)), 1, 4096)
	var footprints: Array = dictionary.get("visual_footprints")
	var bounded: Array = []
	var returned_count := mini(footprints.size(), max_entries)
	for index in range(returned_count):
		bounded.append(footprints[index])
	var next_value := dictionary.duplicate(true)
	next_value["visual_footprints"] = bounded
	next_value["visual_footprint_count"] = footprints.size()
	next_value["visual_footprint_returned_count"] = bounded.size()
	next_value["visual_footprints_truncated"] = footprints.size() > bounded.size()
	method_payload = method_payload.duplicate(true)
	method_payload["value"] = next_value
	method_payload["visual_footprint_count"] = footprints.size()
	method_payload["visual_footprint_returned_count"] = bounded.size()
	method_payload["visual_footprints_truncated"] = footprints.size() > bounded.size()
	return method_payload


func _overlay_shape_entries(collision_objects: Array, include_aabb: bool) -> Array[Dictionary]:
	var entries: Array[Dictionary] = []
	for object_report in collision_objects:
		if not object_report is Dictionary:
			continue
		var object_payload: Dictionary = object_report
		for shape in object_payload.get("shapes", []):
			if not shape is Dictionary:
				continue
			var shape_payload: Dictionary = shape
			var entry := {
				"logical_id": _overlay_logical_id(object_payload, shape_payload),
				"qa_id": str(object_payload.get("qa_id", "")),
				"node_path": str(object_payload.get("path", shape_payload.get("node_path", ""))),
				"node_type": str(object_payload.get("type", shape_payload.get("node_type", ""))),
				"shape_type": str(shape_payload.get("overlay_shape_type", shape_payload.get("shape_type", "unknown"))),
				"layer": int(object_payload.get("collision_layer", shape_payload.get("layer", 0))),
				"mask": int(object_payload.get("collision_mask", shape_payload.get("mask", 0))),
				"owner_id": shape_payload.get("owner_id", null),
				"shape_index": shape_payload.get("shape_index", null),
				"disabled": bool(shape_payload.get("disabled", false)),
				"world_aabb": shape_payload.get("world_aabb", shape_payload.get("aabb", {})),
				"screen_aabb": shape_payload.get("screen_aabb", {}),
				"screen_points": shape_payload.get("screen_points", []),
				"is_area": bool(object_payload.get("is_area", false)),
				"is_body": bool(object_payload.get("is_body", false)),
				"is_soft": bool(shape_payload.get("is_soft", false)),
			}
			if include_aabb:
				entry["aabb"] = shape_payload.get("aabb", {})
			entries.append(entry)
	return entries


func _overlay_logical_id(object_payload: Dictionary, shape_payload: Dictionary) -> String:
	var qa_id := str(object_payload.get("qa_id", ""))
	var base := qa_id if qa_id != "" else str(object_payload.get("path", shape_payload.get("node_path", "")))
	return "%s#%s:%s" % [base, str(shape_payload.get("owner_id", "")), str(shape_payload.get("shape_index", ""))]


func _overlay_object_count(collision_objects: Array, count_areas: bool) -> int:
	var count := 0
	for object_report in collision_objects:
		if not object_report is Dictionary:
			continue
		var report: Dictionary = object_report
		if count_areas and bool(report.get("is_area", false)):
			count += 1
		elif not count_areas and bool(report.get("is_body", false)):
			count += 1
	return count


func _collect_physics_contacts(
	collision_objects: Array,
	include_contacts: bool,
	target_node: Node = null,
	space_queries: Dictionary = {}
) -> Array[Dictionary]:
	var contacts: Array[Dictionary] = []
	if not include_contacts:
		return contacts
	for object_report in collision_objects:
		if not object_report is Dictionary:
			continue
		if target_node != null and not bool((object_report as Dictionary).get("is_target", false)):
			continue
		var path := str((object_report as Dictionary).get("path", ""))
		if path == "" or _runtime.get_ref()._scene_root == null:
			continue
		var node: Node = _runtime.get_ref()._scene_root.get_node_or_null(NodePath(path))
		if node == null:
			continue
		if node.has_method("get_contact_count"):
			var count := int(node.call("get_contact_count"))
			for index in range(count):
				var contact := {
					"node_path": path,
					"index": index,
					"kind": "body_contact",
					"available": true,
				}
				if node.has_method("get_contact_local_normal"):
					contact["normal"] = _runtime.get_ref()._shapes._vector2_payload(node.call("get_contact_local_normal", index))
				if node.has_method("get_contact_local_position"):
					contact["position"] = _runtime.get_ref()._shapes._vector2_payload(node.call("get_contact_local_position", index))
				if node.has_method("get_contact_impulse"):
					contact["impulse"] = _runtime.get_ref()._json_compatible_value(node.call("get_contact_impulse", index))
				if node.has_method("get_contact_collider_object"):
					var collider = node.call("get_contact_collider_object", index)
					contact.merge(_contact_collider_payload(collider), true)
				elif node.has_method("get_contact_collider"):
					contact["collider_rid"] = str(node.call("get_contact_collider", index))
				if node.has_method("get_contact_collider_id"):
					contact["collider_instance_id"] = int(node.call("get_contact_collider_id", index))
				if node.has_method("get_contact_collider_shape"):
					contact["collider_shape"] = int(node.call("get_contact_collider_shape", index))
				if node.has_method("get_contact_local_shape"):
					contact["local_shape"] = int(node.call("get_contact_local_shape", index))
				contact["contact_id"] = _contact_id_for_payload(node, contact, index)
				contacts.append(contact)
		elif node is CharacterBody2D and node.has_method("get_slide_collision_count"):
			var slide_count := int(node.call("get_slide_collision_count"))
			for index in range(slide_count):
				var collision = node.call("get_slide_collision", index)
				var contact := {
					"node_path": path,
					"index": index,
					"kind": "slide_collision",
					"available": true,
				}
				if collision != null:
					if collision.has_method("get_position"):
						contact["position"] = _runtime.get_ref()._shapes._vector2_payload(collision.call("get_position"))
					if collision.has_method("get_normal"):
						contact["normal"] = _runtime.get_ref()._shapes._vector2_payload(collision.call("get_normal"))
					if collision.has_method("get_depth"):
						contact["depth"] = float(collision.call("get_depth"))
					if collision.has_method("get_collider"):
						contact.merge(_contact_collider_payload(collision.call("get_collider")), true)
					if collision.has_method("get_collider_id"):
						contact["collider_instance_id"] = int(collision.call("get_collider_id"))
					if collision.has_method("get_collider_shape"):
						contact["collider_shape"] = int(collision.call("get_collider_shape"))
					if collision.has_method("get_collider_velocity"):
						contact["collider_velocity"] = _runtime.get_ref()._shapes._vector2_payload(collision.call("get_collider_velocity"))
					if collision.has_method("get_local_shape"):
						contact["local_shape"] = int(collision.call("get_local_shape"))
				contact["contact_id"] = _contact_id_for_payload(node, contact, index)
				contacts.append(contact)
	contacts.append_array(_collect_target_overlap_contacts(target_node, space_queries))
	return contacts


func _collect_target_overlap_contacts(target_node: Node, space_queries: Dictionary) -> Array[Dictionary]:
	var contacts: Array[Dictionary] = []
	if target_node == null or not space_queries.get("available", false):
		return contacts
	var overlaps: Array = space_queries.get("target_shape_overlaps", []) if space_queries.get("target_shape_overlaps", []) is Array else []
	for overlap_index in range(overlaps.size()):
		if not overlaps[overlap_index] is Dictionary:
			continue
		var overlap: Dictionary = overlaps[overlap_index]
		var hits: Array = overlap.get("hits", []) if overlap.get("hits", []) is Array else []
		for hit_index in range(hits.size()):
			if not hits[hit_index] is Dictionary:
				continue
			var hit: Dictionary = hits[hit_index]
			var contact := {
				"node_path": str(target_node.get_path()),
				"index": hit_index,
				"kind": "shape_overlap",
				"available": true,
				"owner_id": overlap.get("owner_id", null),
				"shape_index": overlap.get("shape_index", null),
				"depth_status": "not_evaluated",
			}
			contact.merge(hit, true)
			contact["contact_id"] = _contact_id_for_payload(target_node, contact, overlap_index * 1000 + hit_index)
			contacts.append(contact)
	return contacts


func _contact_collider_payload(collider: Variant) -> Dictionary:
	if collider is Node:
		var node := collider as Node
		return {
			"collider_path": str(node.get_path()),
			"collider_type": node.get_class(),
			"collider_qa_id": str(node.get_meta("qa_id", "")),
			"other_id": _stable_node_contact_id(node),
		}
	return {"other_id": str(collider)}


func _contact_id_for_payload(node: Node, contact: Dictionary, index: int) -> String:
	var self_id := _stable_node_contact_id(node)
	var other_id := str(contact.get("other_id", ""))
	if other_id == "":
		other_id = str(contact.get("collider_qa_id", ""))
	if other_id == "":
		other_id = str(contact.get("collider_path", ""))
	if other_id == "":
		other_id = str(contact.get("rid", contact.get("collider_rid", "unknown")))
	return "%s->%s#%s" % [self_id, other_id, str(index)]


func _stable_node_contact_id(node: Node) -> String:
	if node == null:
		return ""
	var qa_id := str(node.get_meta("qa_id", ""))
	if qa_id != "":
		return "qa:%s" % qa_id
	return "path:%s" % str(node.get_path())


func _call_physics_state_method(provider: Node, method: String) -> Dictionary:
	if not _runtime.get_ref()._shapes._is_allowed_physics_state_method(method):
		return {"method": method, "ok": false, "status": "rejected_method"}
	if not provider.has_method(method):
		return {"method": method, "ok": false, "status": "missing_method"}
	if not _runtime.get_ref()._method_has_zero_required_arguments(provider, method):
		return {"method": method, "ok": false, "status": "requires_arguments"}
	var runtime_error_count_before: int = _runtime.get_ref()._runtime_errors.size()
	var value = provider.call(method)
	if _runtime.get_ref()._runtime_errors.size() > runtime_error_count_before:
		return {
			"method": method,
			"ok": false,
			"status": "runtime_error",
			"runtime_errors": _runtime.get_ref()._runtime_errors.slice(runtime_error_count_before).duplicate(true),
		}
	if not _runtime.get_ref()._is_json_compatible_project_state_value(value):
		return {
			"method": method,
			"ok": false,
			"status": "non_json_return",
			"actual_type": type_string(typeof(value)),
		}
	return {
		"method": method,
		"ok": true,
		"value": _runtime.get_ref()._json_compatible_value(value),
	}


func _physics_diagnostics(target_info: Dictionary, collision_objects: Array, target_shapes: Array, project_state: Dictionary) -> Array[Dictionary]:
	var diagnostics: Array[Dictionary] = []
	if target_shapes.is_empty():
		diagnostics.append({
			"type": "no_engine_target_shape",
			"severity": "info",
			"message": "target has no CollisionObject2D-owned shapes",
			"target": target_info.get("resolved", target_info.get("path", "")),
		})
	for object_report in collision_objects:
		if not object_report is Dictionary:
			continue
		var report: Dictionary = object_report
		if not bool(report.get("visible", true)):
			diagnostics.append({
				"type": "hidden_collision_object",
				"severity": "warning",
				"message": "collision object is not visible in tree",
				"target": report.get("path", ""),
			})
		if bool(report.get("is_target", false)):
			continue
		if target_info.get("collision_mask", null) != null and report.get("collision_layer", null) != null:
			var target_mask := int(target_info.get("collision_mask", 0))
			var target_layer := int(target_info.get("collision_layer", 0))
			var other_mask := int(report.get("collision_mask", 0))
			var other_layer := int(report.get("collision_layer", 0))
			if (target_mask & other_layer) == 0 or (other_mask & target_layer) == 0:
				diagnostics.append({
					"type": "layer_mask_cannot_collide",
					"severity": "info",
					"message": "target and nearby object layer/mask settings do not mutually collide",
					"target": report.get("path", ""),
				})
		for shape in report.get("shapes", []):
			if shape is Dictionary and bool((shape as Dictionary).get("large_shape_vs_visual", false)):
				diagnostics.append({
					"type": "large_shape_vs_visual",
					"severity": "warning",
					"message": "shape AABB is much larger than the target visual rect",
					"target": report.get("path", ""),
					"shape": {
						"owner_id": (shape as Dictionary).get("owner_id", null),
						"shape_index": (shape as Dictionary).get("shape_index", null),
					},
				})
	if bool(project_state.get("available", false)):
		diagnostics.append({
			"type": "project_state_available",
			"severity": "info",
			"message": "project-owned QA physics/collision state was captured",
		})
	return diagnostics
