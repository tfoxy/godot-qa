extends RefCounted

const QaSnapshot = preload("res://addons/godot_qa/qa_snapshot.gd")

var _runtime: WeakRef


func _init(runtime: RefCounted) -> void:
	_runtime = weakref(runtime)


func _visual_audit_targets(params: Dictionary) -> Dictionary:
	var raw_targets: Array = params.get("targets", []) if params.get("targets", []) is Array else []
	if raw_targets.is_empty():
		return _runtime.get_ref()._error(
			"invalid_params",
			"visual.audit_targets requires at least one target",
			{
				"command": "visual.audit_targets",
				"field": "targets",
				"expected": "non-empty target locator array",
				"actual": params.get("targets", null),
			},
		)

	var include_state := bool(params.get("include_state", false))
	var requested_state_methods: Array = []
	if params.has("state_methods"):
		if not (params.get("state_methods") is Array):
			return _runtime.get_ref()._error(
				"invalid_params",
				"visual.audit_targets state_methods must be an array",
				{
					"command": "visual.audit_targets",
					"field": "state_methods",
					"expected": "array of approved read-only visual state method names",
					"actual": params.get("state_methods", null),
				},
			)
		for raw_method in params.get("state_methods"):
			var method := str(raw_method).strip_edges()
			if not _approved_read_only_state_method(method):
				return _runtime.get_ref()._error(
					"invalid_params",
					"visual.audit_targets state method is not approved",
					{
						"command": "visual.audit_targets",
						"field": "state_methods",
						"expected": ["get_qa_visual_state", "get_visual_qa_state", "get_qa_readability_state"],
						"actual": method,
					},
				)
			if not requested_state_methods.has(method):
				requested_state_methods.append(method)
		include_state = true

	var viewport: Rect2 = _runtime.get_ref().viewport_rect()
	var targets: Array = []
	for raw_target in raw_targets:
		var locator := str(raw_target).strip_edges()
		if locator == "":
			targets.append({
				"locator": locator,
				"resolved": "",
				"visible": false,
				"viewport": _visual_audit_viewport_metadata(viewport),
				"visual_evidence": QaSnapshot.visual_evidence_for_node(null),
				"error": _runtime.get_ref()._error_payload(
					"invalid_target",
					"visual.audit_targets target locator must be non-empty",
					{"expected": "non-empty target locator", "actual": raw_target},
				),
			})
			continue
		var resolution: Dictionary = _runtime.get_ref().resolve_target(locator)
		if not bool(resolution.get("ok", false)):
			targets.append({
				"locator": locator,
				"resolved": str(resolution.get("resolved", "")),
				"visible": false,
				"viewport": _visual_audit_viewport_metadata(viewport),
				"visual_evidence": QaSnapshot.visual_evidence_for_node(null),
				"error": (resolution.get("error", {}) as Dictionary).duplicate(true),
			})
			continue
		var node := resolution.get("node") as Node
		var evidence := QaSnapshot.visual_evidence_for_node(node)
		var target_payload := {
			"locator": locator,
			"resolved": str(resolution.get("resolved", "")),
			"path": str(node.get_path()) if node != null else "",
			"visible": _runtime.get_ref().is_target_visible(node),
			"viewport": _visual_audit_viewport_metadata(viewport),
			"visual_evidence": evidence,
			"inside_viewport": _visual_evidence_inside_viewport(evidence, viewport),
		}
		if include_state:
			target_payload["state"] = _visual_audit_state_for_node(node, requested_state_methods)
		targets.append(target_payload)

	return _runtime.get_ref()._ok({
		"targets": targets,
		"target_count": targets.size(),
		"viewport": _visual_audit_viewport_metadata(viewport),
	})


func _visual_audit_state_for_node(node: Node, requested_methods: Array) -> Dictionary:
	var state := {
		"available": false,
		"methods": {},
	}
	if node == null:
		return state
	var methods := requested_methods.duplicate()
	if methods.is_empty():
		methods = ["get_qa_visual_state", "get_visual_qa_state", "get_qa_readability_state"]
	for raw_method in methods:
		var method := str(raw_method).strip_edges()
		if not _approved_read_only_state_method(method) or not node.has_method(method):
			continue
		if not _runtime.get_ref()._method_has_zero_required_arguments(node, method):
			(state["methods"] as Dictionary)[method] = {
				"available": false,
				"unsupported_reason": "method_requires_arguments",
			}
			continue
		var value = node.call(method)
		(state["methods"] as Dictionary)[method] = {
			"available": true,
			"value": _runtime.get_ref()._json_compatible_value(value),
		}
		state["available"] = true
	return state


func _approved_read_only_state_method(method: String) -> bool:
	return method == "get_qa_visual_state" \
		or method == "get_visual_qa_state" \
		or method == "get_qa_readability_state"


func _visual_audit_viewport_metadata(viewport: Rect2) -> Dictionary:
	return {
		"rect": _rect2_payload(viewport),
		"size": _vector2_payload(viewport.size),
		"coordinate_space": "viewport_2d",
	}


func _visual_evidence_inside_viewport(evidence: Dictionary, viewport: Rect2) -> Variant:
	var raw_rect = evidence.get("rect", null)
	if raw_rect == null:
		return null
	var rect := _rect2_from_visual_evidence(raw_rect)
	if rect == null:
		return null
	if rect.size.x <= 0.0 or rect.size.y <= 0.0 or viewport.size.x <= 0.0 or viewport.size.y <= 0.0:
		return false
	return viewport.intersects(rect, true)


func _rect2_from_visual_evidence(raw_rect: Variant) -> Variant:
	if raw_rect is Array:
		var values: Array = raw_rect
		if values.size() >= 4 and _runtime.get_ref()._is_number(values[0]) and _runtime.get_ref()._is_number(values[1]) and _runtime.get_ref()._is_number(values[2]) and _runtime.get_ref()._is_number(values[3]):
			return Rect2(Vector2(float(values[0]), float(values[1])), Vector2(float(values[2]), float(values[3])))
	if raw_rect is Dictionary:
		var dictionary: Dictionary = raw_rect
		if _runtime.get_ref()._is_number(dictionary.get("x")) and _runtime.get_ref()._is_number(dictionary.get("y")) and _runtime.get_ref()._is_number(dictionary.get("width")) and _runtime.get_ref()._is_number(dictionary.get("height")):
			return Rect2(Vector2(float(dictionary.get("x")), float(dictionary.get("y"))), Vector2(float(dictionary.get("width")), float(dictionary.get("height"))))
	return null


func _global_position_vector2_for_node(node: Node) -> Vector2:
	if node is Node2D:
		return (node as Node2D).global_position
	if node is Control:
		return (node as Control).global_position
	return Vector2.ZERO


func _physics_velocity_payload(node: Node) -> Variant:
	if node is CharacterBody2D:
		return _vector2_payload((node as CharacterBody2D).velocity)
	if node is RigidBody2D:
		return _vector2_payload((node as RigidBody2D).linear_velocity)
	if node is CharacterBody3D:
		var velocity_3d := (node as CharacterBody3D).velocity
		return {"x": velocity_3d.x, "y": velocity_3d.y, "z": velocity_3d.z, "length": velocity_3d.length()}
	if node is RigidBody3D:
		var rigid_velocity_3d := (node as RigidBody3D).linear_velocity
		return {"x": rigid_velocity_3d.x, "y": rigid_velocity_3d.y, "z": rigid_velocity_3d.z, "length": rigid_velocity_3d.length()}
	return null


func _physics_angular_velocity_payload(node: Node) -> Variant:
	if node is RigidBody2D:
		return (node as RigidBody2D).angular_velocity
	if node is RigidBody3D:
		var angular_velocity_3d := (node as RigidBody3D).angular_velocity
		return {"x": angular_velocity_3d.x, "y": angular_velocity_3d.y, "z": angular_velocity_3d.z, "length": angular_velocity_3d.length()}
	return null


func _node_transform_payload(node: Node) -> Variant:
	if node is Node2D:
		return _transform2d_payload((node as Node2D).global_transform)
	if node is Control:
		return _transform2d_payload((node as Control).get_global_transform())
	if node is Node3D:
		var transform_3d := (node as Node3D).global_transform
		return {
			"origin": {"x": transform_3d.origin.x, "y": transform_3d.origin.y, "z": transform_3d.origin.z},
			"basis_x": {"x": transform_3d.basis.x.x, "y": transform_3d.basis.x.y, "z": transform_3d.basis.x.z},
			"basis_y": {"x": transform_3d.basis.y.x, "y": transform_3d.basis.y.y, "z": transform_3d.basis.y.z},
			"basis_z": {"x": transform_3d.basis.z.x, "y": transform_3d.basis.z.y, "z": transform_3d.basis.z.z},
		}
	return null


func _node_visible_for_physics(node: Node) -> bool:
	if node is CanvasItem:
		return (node as CanvasItem).is_visible_in_tree()
	return true


func _visual_rect_for_node(node: Node) -> Dictionary:
	var explicit_rect := _explicit_visual_rect_for_node(node)
	if not explicit_rect.is_empty():
		return explicit_rect
	if node is Control:
		return _rect2_payload((node as Control).get_global_rect())
	if node is Sprite2D:
		var sprite := node as Sprite2D
		var texture := sprite.texture
		if texture != null:
			var size := texture.get_size() * sprite.global_scale.abs()
			return _rect2_payload(Rect2(sprite.global_position - size * 0.5, size))
	return {}


func _explicit_visual_rect_for_node(node: Node) -> Dictionary:
	var value: Variant = null
	if node.has_method("qa_visual_rect"):
		value = node.call("qa_visual_rect")
	elif node.has_meta("qa_visual_rect"):
		value = node.get_meta("qa_visual_rect")
	return _rect2_from_variant(value)


func _rect2_from_variant(value: Variant) -> Dictionary:
	if value is Rect2:
		return _rect2_payload(value)
	if value is Array:
		var array: Array = value
		if array.size() >= 4 and _runtime.get_ref()._is_number(array[0]) and _runtime.get_ref()._is_number(array[1]) and _runtime.get_ref()._is_number(array[2]) and _runtime.get_ref()._is_number(array[3]):
			return _rect2_payload(Rect2(Vector2(float(array[0]), float(array[1])), Vector2(float(array[2]), float(array[3]))))
	if value is Dictionary:
		var dictionary: Dictionary = value
		if dictionary.has("rect"):
			return _rect2_from_variant(dictionary.get("rect"))
		if _runtime.get_ref()._is_number(dictionary.get("x")) and _runtime.get_ref()._is_number(dictionary.get("y")) and _runtime.get_ref()._is_number(dictionary.get("width")) and _runtime.get_ref()._is_number(dictionary.get("height")):
			return _rect2_payload(Rect2(Vector2(float(dictionary.get("x")), float(dictionary.get("y"))), Vector2(float(dictionary.get("width")), float(dictionary.get("height")))))
	return {}


func _shape2d_aabb_payload(shape: Shape2D, transform: Transform2D) -> Dictionary:
	var points := _shape2d_local_outline_points(shape)
	if points.is_empty():
		return {}
	return _rect2_payload(_rect2_from_points(_transform_points(points, transform)))


func _shape2d_projection_payload(shape: Shape2D, world_transform: Transform2D, screen_transform: Transform2D) -> Dictionary:
	var local_points := _shape2d_local_outline_points(shape)
	if local_points.is_empty():
		return {
			"world_points": [],
			"screen_points": [],
			"world_aabb": {},
			"screen_aabb": {},
		}
	var world_points := _transform_points(local_points, world_transform)
	var screen_points := _transform_points(local_points, screen_transform * world_transform)
	return {
		"world_points": _vector2_payloads(world_points),
		"screen_points": _vector2_payloads(screen_points),
		"world_aabb": _rect2_payload(_rect2_from_points(world_points)),
		"screen_aabb": _rect2_payload(_rect2_from_points(screen_points)),
	}


func _shape2d_local_outline_points(shape: Shape2D) -> Array[Vector2]:
	var points: Array[Vector2] = []
	if shape is CircleShape2D:
		var radius := (shape as CircleShape2D).radius
		for index in range(24):
			var angle := TAU * float(index) / 24.0
			points.append(Vector2(cos(angle), sin(angle)) * radius)
	elif shape is RectangleShape2D:
		var half_size := (shape as RectangleShape2D).size * 0.5
		points = [Vector2(-half_size.x, -half_size.y), Vector2(half_size.x, -half_size.y), Vector2(half_size.x, half_size.y), Vector2(-half_size.x, half_size.y)]
	elif shape is CapsuleShape2D:
		var capsule := shape as CapsuleShape2D
		var straight_half := maxf(capsule.height * 0.5 - capsule.radius, 0.0)
		for index in range(12):
			var top_angle := PI + PI * float(index) / 11.0
			points.append(Vector2(cos(top_angle) * capsule.radius, -straight_half + sin(top_angle) * capsule.radius))
		for index in range(12):
			var bottom_angle := PI * float(index) / 11.0
			points.append(Vector2(cos(bottom_angle) * capsule.radius, straight_half + sin(bottom_angle) * capsule.radius))
	elif shape is SegmentShape2D:
		var segment := shape as SegmentShape2D
		points = [segment.a, segment.b]
	elif shape is SeparationRayShape2D:
		points = [Vector2.ZERO, Vector2(0.0, (shape as SeparationRayShape2D).length)]
	elif shape is ConvexPolygonShape2D:
		for point in (shape as ConvexPolygonShape2D).points:
			points.append(point)
	elif shape is ConcavePolygonShape2D:
		for point in (shape as ConcavePolygonShape2D).segments:
			points.append(point)
	if points.is_empty() and shape != null:
		points = [Vector2.ZERO]
	return points


func _transform_points(points: Array[Vector2], transform: Transform2D) -> Array[Vector2]:
	var transformed_points: Array[Vector2] = []
	for point in points:
		transformed_points.append(transform * point)
	return transformed_points


func _vector2_payloads(points: Array[Vector2]) -> Array[Dictionary]:
	var payloads: Array[Dictionary] = []
	for point in points:
		payloads.append(_vector2_payload(point))
	return payloads


func _rect2_from_points(points: Array[Vector2]) -> Rect2:
	if points.is_empty():
		return Rect2()
	var min_point := points[0]
	var max_point := points[0]
	for point in points:
		min_point.x = minf(min_point.x, point.x)
		min_point.y = minf(min_point.y, point.y)
		max_point.x = maxf(max_point.x, point.x)
		max_point.y = maxf(max_point.y, point.y)
	return Rect2(min_point, max_point - min_point)


func _screen_transform_for_collision_object(collision_object: CollisionObject2D) -> Transform2D:
	var viewport := collision_object.get_viewport()
	if viewport != null:
		return viewport.get_canvas_transform()
	if _runtime.get_ref()._scene_root != null and _runtime.get_ref()._scene_root.get_viewport() != null:
		return _runtime.get_ref()._scene_root.get_viewport().get_canvas_transform()
	return Transform2D.IDENTITY


func _shape2d_overlay_type_name(shape: Shape2D) -> String:
	if shape is CircleShape2D:
		return "circle"
	if shape is RectangleShape2D:
		return "rect"
	if shape is CapsuleShape2D:
		return "capsule"
	if shape is SegmentShape2D or shape is SeparationRayShape2D:
		return "segment"
	if shape is ConvexPolygonShape2D or shape is ConcavePolygonShape2D:
		return "polygon"
	return "unknown"


func _shape_type_name(shape: Shape2D) -> String:
	if shape == null:
		return "null"
	return shape.get_class()


func _shape_large_vs_visual(shape_aabb: Dictionary, visual_rect: Dictionary) -> bool:
	if shape_aabb.is_empty() or visual_rect.is_empty():
		return false
	var shape_area := maxf(float(shape_aabb.get("width", 0.0)), 0.0) * maxf(float(shape_aabb.get("height", 0.0)), 0.0)
	var visual_area := maxf(float(visual_rect.get("width", 0.0)), 0.0) * maxf(float(visual_rect.get("height", 0.0)), 0.0)
	return visual_area > 0.0 and shape_area > visual_area * 1.75


func _world_2d_for_node(node: Node) -> World2D:
	if node is CanvasItem:
		return (node as CanvasItem).get_world_2d()
	if _runtime.get_ref()._scene_root is CanvasItem:
		return (_runtime.get_ref()._scene_root as CanvasItem).get_world_2d()
	if _runtime.get_ref()._root_window != null:
		return _runtime.get_ref()._root_window.world_2d
	return null


func _serialize_intersect_shape_results(results: Array, target_node: Node) -> Array:
	var hits: Array = []
	for result in results:
		if not result is Dictionary:
			continue
		var entry: Dictionary = result
		var collider = entry.get("collider", null)
		var collider_payload := {
			"path": "",
			"name": "",
			"type": type_string(typeof(collider)),
			"qa_id": "",
			"is_target": false,
		}
		if collider is Node:
			var collider_node := collider as Node
			collider_payload["path"] = str(collider_node.get_path())
			collider_payload["name"] = str(collider_node.name)
			collider_payload["type"] = collider_node.get_class()
			collider_payload["qa_id"] = str(collider_node.get_meta("qa_id", ""))
			collider_payload["is_target"] = collider_node == target_node
		hits.append({
			"collider": collider_payload,
			"collider_id": entry.get("collider_id", null),
			"shape": entry.get("shape", null),
			"rid": str(entry.get("rid", "")),
		})
	return hits


func _shape_from_target_report(target_node: Node, shape_report: Dictionary) -> Shape2D:
	if not target_node is CollisionObject2D:
		return null
	var collision_object := target_node as CollisionObject2D
	var owner_id := int(shape_report.get("owner_id", -1))
	var shape_index := int(shape_report.get("shape_index", -1))
	if owner_id < 0 or shape_index < 0:
		return null
	if shape_index >= collision_object.shape_owner_get_shape_count(owner_id):
		return null
	return collision_object.shape_owner_get_shape(owner_id, shape_index)


func _transform2d_from_payload(payload: Variant) -> Transform2D:
	if not payload is Dictionary:
		return Transform2D.IDENTITY
	var transform_payload: Dictionary = payload
	var x_axis := _vector2_from_payload(transform_payload.get("x", {}), Vector2.RIGHT)
	var y_axis := _vector2_from_payload(transform_payload.get("y", {}), Vector2.DOWN)
	var origin := _vector2_from_payload(transform_payload.get("origin", {}), Vector2.ZERO)
	return Transform2D(x_axis, y_axis, origin)


func _vector2_from_payload(payload: Variant, fallback: Vector2) -> Vector2:
	if payload is Dictionary:
		var dictionary: Dictionary = payload
		if _runtime.get_ref()._is_number(dictionary.get("x")) and _runtime.get_ref()._is_number(dictionary.get("y")):
			return Vector2(float(dictionary.get("x")), float(dictionary.get("y")))
	return fallback


func _distance_from_rect_to_point(rect_payload: Dictionary, point: Vector2) -> float:
	if rect_payload.is_empty():
		return INF
	var rect := Rect2(
		Vector2(float(rect_payload.get("x", 0.0)), float(rect_payload.get("y", 0.0))),
		Vector2(float(rect_payload.get("width", 0.0)), float(rect_payload.get("height", 0.0))),
	)
	if rect.has_point(point):
		return 0.0
	var clamped := Vector2(
		clampf(point.x, rect.position.x, rect.position.x + rect.size.x),
		clampf(point.y, rect.position.y, rect.position.y + rect.size.y),
	)
	return point.distance_to(clamped)


func _vector2_payload(value: Vector2) -> Dictionary:
	return {
		"x": value.x,
		"y": value.y,
		"length": value.length(),
	}


func _rect2_payload(rect: Rect2) -> Dictionary:
	return {
		"x": rect.position.x,
		"y": rect.position.y,
		"width": rect.size.x,
		"height": rect.size.y,
		"center": _vector2_payload(rect.get_center()),
	}


func _transform2d_payload(transform: Transform2D) -> Dictionary:
	return {
		"x": _vector2_payload(transform.x),
		"y": _vector2_payload(transform.y),
		"origin": _vector2_payload(transform.origin),
	}


func _is_allowed_physics_state_method(method: String) -> bool:
	if method == "" \
			or method.begins_with("qa_prepare_") \
			or method.begins_with("qa_apply_") \
			or method.begins_with("qa_mutate_") \
			or method.begins_with("qa_set_"):
		return false
	for character in method:
		var codepoint := character.unicode_at(0)
		if not ((codepoint >= 48 and codepoint <= 57) or (codepoint >= 65 and codepoint <= 90) or (codepoint >= 97 and codepoint <= 122) or character == "_"):
			return false
	if method == "qa_collision_probe":
		return true
	return (method.begins_with("get_qa_") and method.ends_with("_state")) \
		or (method.begins_with("get_") and method.ends_with("_qa_state")) \
		or (method.begins_with("qa_") and method.contains("diagnostic"))
