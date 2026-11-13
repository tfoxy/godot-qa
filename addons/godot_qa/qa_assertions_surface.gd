extends RefCounted

const QaAssertionsTemporal = preload("res://addons/godot_qa/qa_assertions_temporal.gd")
const SURFACE_ALLOWED_SPACES := {"world_2d": true, "viewport_2d": true, "world_3d": true}
const SURFACE_MAX_POINTS := 32
const SURFACE_MAX_RADIUS := 4096.0
const SURFACE_AUDIT_PROVIDER_METHODS := {
	"qa_surface_probe": true,
	"get_qa_surface_state": true,
	"get_surface_qa_state": true,
	"qa_probe_surface": true,
}

var _owner: WeakRef
var _runtime: RefCounted:
	get:
		return _owner.get_ref()._runtime
var _temporal: QaAssertionsTemporal:
	get:
		return _owner.get_ref()._temporal


func _init(owner: RefCounted) -> void:
	_owner = weakref(owner)


func surface_audit_probe(params: Dictionary) -> Dictionary:
	var provider_id := str(params.get("provider_id", "")).strip_edges()
	var provider_target := str(params.get("provider", params.get("provider_target", ""))).strip_edges()
	if provider_id == "" and provider_target == "":
		return _surface_failure("surface_audit.provider_missing", "surface audit requires a provider target", params, {"expected": "provider target", "actual": provider_target})
	var provider_method := str(params.get("provider_method", "")).strip_edges()
	if provider_id == "" and not SURFACE_AUDIT_PROVIDER_METHODS.has(provider_method):
		return _surface_failure("surface_audit.provider_invalid", "surface audit provider method is not approved", params, {"provider": provider_target, "provider_method": provider_method, "expected": SURFACE_AUDIT_PROVIDER_METHODS.keys()})
	var provider_node: Node = null
	var resolved_provider := provider_id
	if provider_id == "":
		var provider_resolution: Dictionary = _owner.get_ref()._require_target(provider_target)
		if not bool(provider_resolution.get("ok", false)):
			return provider_resolution
		provider_node = provider_resolution.get("node")
		resolved_provider = str(provider_resolution.get("resolved", provider_target))
	var coordinate_space := str(params.get("coordinate_space", ""))
	if coordinate_space == "":
		if provider_node != null:
			var provider_position := _node_position_payload(provider_node)
			coordinate_space = str(provider_position.get("coordinate_space", "world_2d"))
		else:
			coordinate_space = "world_2d"
	if not SURFACE_ALLOWED_SPACES.has(coordinate_space):
		return _surface_failure("unsupported_coordinate_space", "surface audit does not support the requested coordinate space", params, {"provider": provider_target, "coordinate_space": coordinate_space})
	var anchors := _resolve_surface_audit_anchors(params)
	var regions := _resolve_surface_audit_regions(params)
	var points_value = params.get("points", params.get("sample_points", []))
	if not (points_value is Array) or (points_value as Array).is_empty():
		points_value = _surface_audit_generated_points(params, anchors, regions)
	if not (points_value is Array):
		return _surface_failure("surface_audit.sample_limit_exceeded", "surface audit requires a bounded points array", params, {"provider": provider_target, "expected": "points array", "actual": points_value})
	var points: Array = points_value
	if points.is_empty() or points.size() > SURFACE_MAX_POINTS:
		return _surface_failure("surface_audit.sample_limit_exceeded", "surface audit point count exceeded bounds", params, {"provider": provider_target, "expected": {"min_points": 1, "max_points": SURFACE_MAX_POINTS}, "actual": {"point_count": points.size()}})
	for point in points:
		if not _valid_surface_audit_point(point):
			return _surface_failure("surface_audit.sample_limit_exceeded", "surface audit point must include finite coordinates", params, {"provider": provider_target, "expected": "point object with finite x/y and optional z", "actual": point})
	var provider_params := {"provider_method": provider_method, "provider_id": provider_id, "approved_provider_methods": SURFACE_AUDIT_PROVIDER_METHODS.keys()}
	var provider := _resolve_surface_provider(provider_node, provider_params)
	if not bool(provider.get("ok", false)):
		return provider
	var provider_result := _call_surface_provider(provider, provider_node, coordinate_space, points, true)
	if not bool(provider_result.get("ok", false)):
		return provider_result
	return _owner.get_ref()._ok({
		"command": "surface.audit_probe",
		"provider": resolved_provider,
		"provider_id": provider_id,
		"provider_method": provider.get("provider_method", provider_method),
		"coordinate_space": coordinate_space,
		"anchors": anchors,
		"regions": regions,
		"samples": provider_result.get("points", []),
	})


func _assert_surface_probe(params: Dictionary) -> Dictionary:
	if params.has("probe_id"):
		return _assert_named_surface_probe(params)
	var target := str(params.get("target", ""))
	var target_resolution: Dictionary = _owner.get_ref()._require_target(target)
	if not bool(target_resolution.get("ok", false)):
		return target_resolution
	var point_resolution: Dictionary = _owner.get_ref()._resolve_target_position(target)
	if not bool(point_resolution.get("ok", false)):
		return point_resolution
	var coordinate_space := str(params.get("coordinate_space", point_resolution.get("coordinate_space", "")))
	if not SURFACE_ALLOWED_SPACES.has(coordinate_space):
		return _surface_failure("unsupported_coordinate_space", "surface probe does not support the requested coordinate space", params, {"target": target, "coordinate_space": coordinate_space})
	var points_value = params.get("points", null)
	if not (points_value is Array):
		return _surface_failure("probe_request_out_of_bounds", "surface probe requires an array of points", params, {"target": target, "coordinate_space": coordinate_space, "expected": "points array", "actual": points_value})
	var points: Array = points_value
	if points.is_empty() or points.size() > SURFACE_MAX_POINTS:
		return _surface_failure("probe_request_out_of_bounds", "surface probe point count exceeded bounds", params, {"target": target, "coordinate_space": coordinate_space, "expected": {"min_points": 1, "max_points": SURFACE_MAX_POINTS}, "actual": {"point_count": points.size()}})
	var radius := float(params.get("max_radius", params.get("radius", SURFACE_MAX_RADIUS)))
	if not is_finite(radius) or radius < 0.0 or radius > SURFACE_MAX_RADIUS:
		return _surface_failure("probe_request_out_of_bounds", "surface probe radius exceeded bounds", params, {"target": target, "coordinate_space": coordinate_space, "expected": {"max_radius": SURFACE_MAX_RADIUS}, "actual": {"radius": radius}})
	var origin: Dictionary = point_resolution.get("position", {})
	for point in points:
		if not (point is Dictionary):
			return _surface_failure("probe_request_out_of_bounds", "surface probe point must be an object", params, {"target": target, "coordinate_space": coordinate_space, "expected": "point object", "actual": point})
		if _surface_point_distance(origin, point) > radius:
			return _surface_failure("probe_request_out_of_bounds", "surface probe point exceeded radius bounds", params, {"target": target, "coordinate_space": coordinate_space, "point": _surface_point_payload(point), "expected": {"max_radius": radius}, "actual": {"distance": _surface_point_distance(origin, point)}})
	var declared_labels := _declared_surface_labels(params)
	var provider := _resolve_surface_provider(target_resolution.get("node"), params)
	if not bool(provider.get("ok", false)):
		return provider
	var provider_result := _call_surface_provider(provider, target_resolution.get("node"), coordinate_space, points)
	if not bool(provider_result.get("ok", false)):
		return provider_result
	var sample_points: Array = provider_result.get("points", [])
	var mismatches: Array = []
	for point in sample_points:
		if not (point is Dictionary):
			return _surface_failure("invalid_probe_result", "surface provider returned invalid point payload", params, {"target": target, "provider": provider.get("provider", ""), "provider_method": provider.get("provider_method", ""), "actual": point})
		var point_payload: Dictionary = point
		var label := str(point_payload.get("label", ""))
		if not declared_labels.is_empty() and not declared_labels.has(label):
			return _surface_failure("unknown_surface_label", "surface provider returned unknown surface label", params, {"target": target, "coordinate_space": coordinate_space, "actual": label, "expected": declared_labels.keys(), "provider": provider.get("provider", ""), "provider_method": provider.get("provider_method", "")})
		var expected_label := str(point_payload.get("expected", ""))
		if expected_label != "" and expected_label != label:
			var mismatch := point_payload.duplicate(true)
			mismatch["expected"] = expected_label
			mismatch["actual"] = label
			mismatches.append(mismatch)
	if not mismatches.is_empty():
		return _surface_failure("surface_mismatch", "surface probe labels did not match expected labels", params, {"command": "assert.surface_probe", "target": target, "resolved": target_resolution.get("resolved", ""), "coordinate_space": coordinate_space, "expected": "matching point labels", "actual": mismatches, "provider": provider.get("provider", ""), "provider_method": provider.get("provider_method", ""), "points": sample_points})
	return _owner.get_ref()._ok({"command": "assert.surface_probe", "target": target, "resolved": target_resolution.get("resolved", ""), "coordinate_space": coordinate_space, "provider": provider.get("provider", ""), "provider_method": provider.get("provider_method", ""), "point_count": sample_points.size(), "points": sample_points})


func _assert_named_surface_probe(params: Dictionary) -> Dictionary:
	var provider_target := str(params.get("provider", params.get("target", "")))
	var probe_id := str(params.get("probe_id", ""))
	if provider_target == "" or probe_id == "":
		return _surface_failure("invalid_assertion_args", "named surface probe requires provider and probe_id", params, {"expected": "provider and probe_id", "actual": params})
	var provider_resolution: Dictionary = _owner.get_ref()._require_target(provider_target)
	if not bool(provider_resolution.get("ok", false)):
		return provider_resolution
	var provider_node: Node = provider_resolution.get("node")
	if provider_node == null or not provider_node.has_method("qa_surface_probe"):
		return _surface_failure("missing_probe_provider", "surface probe provider does not expose qa_surface_probe", params, {"provider": provider_target, "resolved": provider_resolution.get("resolved", ""), "expected": "qa_surface_probe(probe_id)", "actual": _runtime.describe_node(provider_node)})
	var result = provider_node.call("qa_surface_probe", probe_id)
	if not result is Dictionary:
		return _surface_failure("invalid_probe_result", "surface probe provider returned non-dictionary result", params, {"provider": provider_target, "probe_id": probe_id, "actual": result})
	var probe_result: Dictionary = result
	if probe_result.has("ok") and not bool(probe_result.get("ok", false)):
		return _surface_failure("invalid_probe_result", "surface probe provider reported failure", params, {"provider": provider_target, "probe_id": probe_id, "actual": probe_result})
	var mismatches: Array = []
	if params.has("expected_surface"):
		var expected_surface := str(params.get("expected_surface", ""))
		var actual_surface := str(probe_result.get("surface", ""))
		if expected_surface != actual_surface:
			mismatches.append({"field": "surface", "expected": expected_surface, "actual": actual_surface})
	if params.has("expected_walkable"):
		var expected_walkable := bool(params.get("expected_walkable", false))
		var actual_walkable := bool(probe_result.get("walkable", false))
		if expected_walkable != actual_walkable:
			mismatches.append({"field": "walkable", "expected": expected_walkable, "actual": actual_walkable})
	if not mismatches.is_empty():
		return _surface_failure("surface_mismatch", "named surface probe did not match expected values", params, {"command": "assert.surface_probe", "provider": provider_target, "resolved": provider_resolution.get("resolved", ""), "probe_id": probe_id, "expected": {"surface": params.get("expected_surface", null), "walkable": params.get("expected_walkable", null)}, "actual": probe_result, "mismatches": mismatches})
	return _owner.get_ref()._ok({"command": "assert.surface_probe", "provider": provider_target, "resolved": provider_resolution.get("resolved", ""), "probe_id": probe_id, "actual": probe_result})


func _assert_surface_matches(params: Dictionary) -> Dictionary:
	if _surface_match_temporal_requested(params):
		return await _assert_surface_matches_temporal(params)
	return _assert_surface_matches_once(params)


func _assert_surface_matches_once(params: Dictionary) -> Dictionary:
	var target := str(params.get("target", ""))
	var target_resolution: Dictionary = _owner.get_ref()._require_target(target)
	if not bool(target_resolution.get("ok", false)):
		return target_resolution
	var expected_label := str(params.get("surface", params.get("expected", "")))
	if not params.has("provider") and not params.has("provider_target") and not params.has("provider_method"):
		var actor_only_result := _actor_surface_label(target_resolution.get("node"), params)
		if not bool(actor_only_result.get("ok", false)):
			return actor_only_result
		var actor_only_label := str(actor_only_result.get("label", ""))
		if expected_label == "" or actor_only_label == expected_label:
			return _owner.get_ref()._ok({"command": "assert.surface_matches", "target": target, "resolved": target_resolution.get("resolved", ""), "expected": expected_label, "actual": actor_only_label, "provider": "actor"})
		return _surface_failure("surface_mismatch", "actor surface does not match expected surface", params, {"command": "assert.surface_matches", "target": target, "resolved": target_resolution.get("resolved", ""), "expected": expected_label, "actual": actor_only_label, "provider": "actor"})
	var point_resolution: Dictionary = _owner.get_ref()._resolve_target_position(target)
	if not bool(point_resolution.get("ok", false)):
		return point_resolution
	var coordinate_space := str(params.get("coordinate_space", point_resolution.get("coordinate_space", "")))
	if not SURFACE_ALLOWED_SPACES.has(coordinate_space):
		return _surface_failure("unsupported_coordinate_space", "surface match does not support the resolved coordinate space", params, {"target": target, "coordinate_space": coordinate_space})
	var provider := _resolve_surface_provider(target_resolution.get("node"), params)
	if not bool(provider.get("ok", false)):
		return provider
	var probe_point := _surface_point_payload(point_resolution.get("position", {}))
	var provider_result := _call_surface_provider(provider, target_resolution.get("node"), coordinate_space, [probe_point])
	if not bool(provider_result.get("ok", false)):
		return provider_result
	var points: Array = provider_result.get("points", [])
	if points.is_empty():
		return _surface_failure("invalid_probe_result", "surface provider returned no probe point for surface match", params, {"command": "assert.surface_matches", "target": target, "provider": provider.get("provider", ""), "provider_method": provider.get("provider_method", "")})
	var world_point: Dictionary = points[0]
	var world_label := str(world_point.get("label", ""))
	if expected_label != "" and expected_label != world_label:
		return _surface_failure("surface_mismatch", "sampled world surface does not match expected surface", params, {"command": "assert.surface_matches", "target": target, "resolved": target_resolution.get("resolved", ""), "point": world_point, "coordinate_space": coordinate_space, "expected": expected_label, "actual": world_label, "provider": provider.get("provider", ""), "provider_method": provider.get("provider_method", "")})
	var actor_result := _actor_surface_label(target_resolution.get("node"), params)
	if not bool(actor_result.get("ok", false)):
		return actor_result
	var actor_label := str(actor_result.get("label", ""))
	if actor_label == world_label:
		return _owner.get_ref()._ok({"command": "assert.surface_matches", "target": target, "resolved": target_resolution.get("resolved", ""), "coordinate_space": coordinate_space, "point": world_point, "expected": world_label, "actual": actor_label, "provider": provider.get("provider", ""), "provider_method": provider.get("provider_method", "")})
	return _surface_failure("surface_mismatch", "actor surface does not match sampled world surface", params, {"command": "assert.surface_matches", "target": target, "resolved": target_resolution.get("resolved", ""), "point": world_point, "coordinate_space": coordinate_space, "expected": world_label, "actual": actor_label, "provider": provider.get("provider", ""), "provider_method": provider.get("provider_method", "")})


func _surface_match_temporal_requested(params: Dictionary) -> bool:
	return params.has("window_frames") or params.has("window_seconds") or params.has("sample_every_frames") or params.has("dwell_frames") or params.has("dwell_seconds")


func _assert_surface_matches_temporal(params: Dictionary) -> Dictionary:
	var window_frames := _temporal._temporal_frame_count(params.get("window_frames", null), params.get("window_seconds", null), 1)
	var sample_every := max(1, int(params.get("sample_every_frames", 1)))
	var dwell_frames := _temporal._temporal_frame_count(params.get("dwell_frames", null), params.get("dwell_seconds", null), 1)
	var required_dwell_samples := max(1, int(ceil(float(dwell_frames) / float(sample_every))))
	var samples: Array = []
	var consecutive_matches := 0
	var last_failure: Dictionary = {}
	var frame_index := 0
	while frame_index <= window_frames:
		var result := _assert_surface_matches_once(params)
		var sample := _surface_match_sample_payload(frame_index, result)
		samples.append(sample)
		if bool(result.get("ok", false)):
			consecutive_matches += 1
			if consecutive_matches >= required_dwell_samples:
				var ok_data: Dictionary = result.get("data", {})
				ok_data["temporal"] = {
					"window_frames": window_frames,
					"sample_every_frames": sample_every,
					"dwell_frames": dwell_frames,
					"required_dwell_samples": required_dwell_samples,
					"matched_samples": consecutive_matches,
					"samples": samples,
				}
				return _owner.get_ref()._ok(ok_data)
		else:
			consecutive_matches = 0
			last_failure = result
			var error := result.get("error", {})
			var error_type := str(error.get("type", ""))
			if error_type != "surface_mismatch":
				return result
		if frame_index >= window_frames:
			break
		await _runtime.process_frames(sample_every)
		frame_index += sample_every
	var details := {
		"command": "assert.surface_matches",
		"target": str(params.get("target", "")),
		"expected": str(params.get("surface", params.get("expected", ""))),
		"actual": _surface_match_last_actual(samples),
		"window_frames": window_frames,
		"sample_every_frames": sample_every,
		"dwell_frames": dwell_frames,
		"required_dwell_samples": required_dwell_samples,
		"samples": samples,
	}
	if params.has("temporal_dir"):
		details["artifacts"] = {"temporal_dir": str(params.get("temporal_dir", ""))}
	var last_error := last_failure.get("error", {})
	if last_error is Dictionary and (last_error as Dictionary).has("data"):
		var last_data: Dictionary = (last_error as Dictionary).get("data", {})
		for key in ["point", "coordinate_space", "provider", "provider_method", "resolved"]:
			if last_data.has(key):
				details[key] = last_data.get(key)
	return _surface_failure("surface_mismatch", "surface match did not satisfy the requested temporal window", params, details)


func _surface_match_sample_payload(frame_index: int, result: Dictionary) -> Dictionary:
	var payload := {
		"frame": frame_index,
		"ok": bool(result.get("ok", false)),
	}
	if bool(result.get("ok", false)):
		var data: Dictionary = result.get("data", {})
		for key in ["expected", "actual", "point", "coordinate_space", "provider", "provider_method", "resolved"]:
			if data.has(key):
				payload[key] = data.get(key)
		return payload
	var error: Dictionary = result.get("error", {})
	payload["type"] = str(error.get("type", ""))
	payload["message"] = str(error.get("message", ""))
	var details: Dictionary = error.get("data", {})
	for key in ["expected", "actual", "point", "coordinate_space", "provider", "provider_method", "resolved"]:
		if details.has(key):
			payload[key] = details.get(key)
	return payload


func _surface_match_last_actual(samples: Array) -> Variant:
	for index in range(samples.size() - 1, -1, -1):
		var sample = samples[index]
		if sample is Dictionary and (sample as Dictionary).has("actual"):
			return (sample as Dictionary).get("actual")
	return null


func _declared_surface_labels(params: Dictionary) -> Dictionary:
	var labels_raw = params.get("surface_labels", params.get("surfaces", []))
	var labels: Dictionary = {}
	if labels_raw is Array:
		for value in labels_raw:
			var label := str(value).strip_edges()
			if label != "":
				labels[label] = true
	return labels


func _resolve_surface_provider(target_node: Node, params: Dictionary) -> Dictionary:
	var provider_id := str(params.get("provider_id", "")).strip_edges()
	if provider_id != "":
		var probe: Dictionary = _runtime.describe_probe_provider(provider_id)
		if not bool(probe.get("ok", false)):
			return probe
		var data: Dictionary = probe.get("data", {})
		var metadata: Dictionary = data.get("metadata", {}) if data.get("metadata", {}) is Dictionary else {}
		var provider_payload := {
			"ok": true,
			"provider_id": provider_id,
			"provider": provider_id,
			"registered": true,
			"provider_method": metadata.get("provider_method", ""),
		}
		return provider_payload
	var provider_method := str(params.get("provider_method", ""))
	if provider_method == "" and target_node != null and target_node.has_meta("qa_surface_probe_method"):
		provider_method = str(target_node.get_meta("qa_surface_probe_method"))
	if provider_method.strip_edges() == "":
		return _surface_failure("missing_probe_provider", "surface probe provider method is required", params, {})
	provider_method = provider_method.strip_edges()
	if not provider_method.begins_with("qa_") and not _approved_surface_provider_method(provider_method, params):
		return _surface_failure("missing_probe_provider", "surface probe provider must use a QA-prefixed method name", params, {"provider_method": provider_method, "expected": "qa_*"})
	var provider_node := target_node
	var provider_target := str(params.get("provider_target", params.get("provider", "")))
	if provider_target.strip_edges() != "":
		var provider_resolution: Dictionary = _owner.get_ref()._require_target(provider_target)
		if not bool(provider_resolution.get("ok", false)):
			return provider_resolution
		provider_node = provider_resolution.get("node")
	if provider_node == null or not provider_node.has_method(provider_method):
		return _surface_failure("missing_probe_provider", "surface probe provider method was not found", params, {"provider": _runtime.describe_node(provider_node), "provider_method": provider_method})
	return {"ok": true, "provider_node": provider_node, "provider": _runtime.describe_node(provider_node), "provider_method": provider_method}


func _approved_surface_provider_method(provider_method: String, params: Dictionary) -> bool:
	var approved = params.get("approved_provider_methods", [])
	if approved is Array:
		for method in approved:
			if str(method) == provider_method:
				return true
	return false


func _call_surface_provider(provider: Dictionary, target_node: Node, coordinate_space: String, points: Array, preserve_sample_fields := false) -> Dictionary:
	var payload_points: Array = []
	for point in points:
		if point is Dictionary:
			payload_points.append((point as Dictionary).duplicate(true))
	var request := {"target": _runtime.describe_node(target_node), "coordinate_space": coordinate_space, "points": payload_points}
	if bool(provider.get("registered", false)):
		var provider_id := str(provider.get("provider_id", ""))
		var registered_result: Dictionary = _runtime.call_probe_provider(provider_id, request)
		if not bool(registered_result.get("ok", false)):
			return registered_result
		var registered_payload: Dictionary = registered_result.get("result", {})
		return _normalize_surface_provider_result(registered_payload, request, provider, preserve_sample_fields)
	var provider_node: Node = provider.get("provider_node")
	var provider_method := str(provider.get("provider_method", ""))
	var result = provider_node.call(provider_method, request)
	if not _owner.get_ref()._json_compatible(result):
		return _surface_failure("invalid_probe_result", "surface provider returned non JSON-compatible data", request, {"provider": provider.get("provider", ""), "provider_method": provider_method, "actual_type": typeof(result)})
	if not (result is Dictionary):
		return _surface_failure("invalid_probe_result", "surface provider must return a dictionary", request, {"provider": provider.get("provider", ""), "provider_method": provider_method, "actual": result})
	return _normalize_surface_provider_result(result, request, provider, preserve_sample_fields)


func _normalize_surface_provider_result(result_dict: Dictionary, request: Dictionary, provider: Dictionary, preserve_sample_fields: bool) -> Dictionary:
	var points_value = result_dict.get("points", null)
	if not (points_value is Array):
		return _surface_failure("invalid_probe_result", "surface provider result must include points array", request, {"provider": provider.get("provider", ""), "provider_id": provider.get("provider_id", ""), "provider_method": provider.get("provider_method", ""), "actual": result_dict})
	var normalized_points: Array = []
	for entry in points_value:
		if not (entry is Dictionary):
			return _surface_failure("invalid_probe_result", "surface provider point entry must be an object", request, {"provider": provider.get("provider", ""), "provider_id": provider.get("provider_id", ""), "provider_method": provider.get("provider_method", ""), "actual": entry})
		if preserve_sample_fields:
			normalized_points.append(_surface_sample_payload(entry))
		else:
			normalized_points.append(_surface_point_payload(entry))
	return {"ok": true, "points": normalized_points, "provider_id": provider.get("provider_id", "")}


func _actor_surface_label(node: Node, params: Dictionary) -> Dictionary:
	var method_name := str(params.get("actor_surface_method", ""))
	if method_name == "" and node != null and node.has_meta("qa_surface_state_method"):
		method_name = str(node.get_meta("qa_surface_state_method"))
	method_name = method_name.strip_edges()
	if method_name == "":
		if node != null and node.has_meta("qa_player_surface"):
			return {"ok": true, "label": str(node.get_meta("qa_player_surface"))}
		if node != null and node.has_meta("qa_surface"):
			return {"ok": true, "label": str(node.get_meta("qa_surface"))}
	if method_name == "" or not method_name.begins_with("qa_") or node == null or not node.has_method(method_name):
		return _surface_failure("missing_probe_provider", "actor surface state method is required and must be QA-prefixed", params, {"provider_method": method_name})
	var value = node.call(method_name)
	if not _owner.get_ref()._json_compatible(value):
		return _surface_failure("invalid_probe_result", "actor surface state was not JSON-compatible", params, {"provider_method": method_name, "actual_type": typeof(value)})
	return {"ok": true, "label": str(value)}


func _surface_point_payload(point: Variant) -> Dictionary:
	var payload: Dictionary = {}
	if point is Dictionary:
		payload = (point as Dictionary).duplicate(true)
	var coordinate_source := payload
	if payload.get("point", null) is Dictionary:
		coordinate_source = payload.get("point")
	var normalized: Dictionary = {}
	if coordinate_source.has("x"):
		normalized["x"] = float(coordinate_source.get("x", 0.0))
	if coordinate_source.has("y"):
		normalized["y"] = float(coordinate_source.get("y", 0.0))
	if coordinate_source.has("z"):
		normalized["z"] = float(coordinate_source.get("z", 0.0))
	var label := str(payload.get("label", payload.get("surface", "")))
	normalized["label"] = label
	if payload.has("surface"):
		normalized["surface"] = str(payload.get("surface", ""))
	elif label != "":
		normalized["surface"] = label
	if payload.has("expected"):
		normalized["expected"] = str(payload.get("expected", ""))
	if payload.has("coordinate_space"):
		normalized["coordinate_space"] = str(payload.get("coordinate_space", ""))
	if payload.has("provider"):
		normalized["provider"] = str(payload.get("provider", ""))
	if payload.has("provider_method"):
		normalized["provider_method"] = str(payload.get("provider_method", ""))
	return normalized


func _surface_sample_payload(point: Variant) -> Dictionary:
	var payload := _surface_point_payload(point)
	if not (point is Dictionary):
		return payload
	var source: Dictionary = point
	for key in source.keys():
		if typeof(key) != TYPE_STRING:
			continue
		var key_name := str(key)
		if key_name == "point":
			continue
		var value = source.get(key)
		if _owner.get_ref()._json_compatible(value):
			payload[key_name] = value
	return payload


func _valid_surface_audit_point(point: Variant) -> bool:
	if not (point is Dictionary):
		return false
	var payload := _surface_point_payload(point)
	if not payload.has("x") or not payload.has("y"):
		return false
	if not is_finite(float(payload.get("x", 0.0))) or not is_finite(float(payload.get("y", 0.0))):
		return false
	if payload.has("z") and not is_finite(float(payload.get("z", 0.0))):
		return false
	return true


func _node_position_payload(node: Node) -> Dictionary:
	if node is Node2D:
		var p2 := (node as Node2D).global_position
		return {"coordinate_space": "world_2d", "position": {"x": p2.x, "y": p2.y}}
	if node is Control:
		var center := (node as Control).get_global_rect().get_center()
		return {"coordinate_space": "viewport_2d", "position": {"x": center.x, "y": center.y}}
	if node is Node3D:
		var p3 := (node as Node3D).global_position
		return {"coordinate_space": "world_3d", "position": {"x": p3.x, "y": p3.y, "z": p3.z}}
	return {"coordinate_space": "world_2d", "position": {}}


func _resolve_surface_audit_anchors(params: Dictionary) -> Dictionary:
	var locators: Dictionary = {}
	_add_surface_audit_locator(locators, "center", params.get("center", ""))
	_add_surface_audit_locator(locators, "mirror_axis", params.get("mirror_axis", ""))
	var route_value = params.get("route", [])
	if route_value is Array:
		var route_index := 0
		for locator in route_value:
			_add_surface_audit_locator(locators, "route_%d" % route_index, locator)
			route_index += 1
	var anchors_value = params.get("anchors", {})
	if anchors_value is Dictionary:
		for key in (anchors_value as Dictionary).keys():
			_add_surface_audit_locator(locators, str(key), (anchors_value as Dictionary).get(key))
	elif anchors_value is Array:
		var anchor_index := 0
		for locator in anchors_value:
			_add_surface_audit_locator(locators, "anchor_%d" % anchor_index, locator)
			anchor_index += 1
	return _resolve_surface_audit_locator_map(locators)


func _resolve_surface_audit_regions(params: Dictionary) -> Dictionary:
	var locators: Dictionary = {}
	_add_surface_audit_locator(locators, "left_region", params.get("left_region", ""))
	_add_surface_audit_locator(locators, "right_region", params.get("right_region", ""))
	var regions_value = params.get("regions", {})
	if regions_value is Dictionary:
		for key in (regions_value as Dictionary).keys():
			_add_surface_audit_locator(locators, str(key), (regions_value as Dictionary).get(key))
	elif regions_value is Array:
		var region_index := 0
		for locator in regions_value:
			_add_surface_audit_locator(locators, "region_%d" % region_index, locator)
			region_index += 1
	return _resolve_surface_audit_locator_map(locators)


func _surface_audit_generated_points(params: Dictionary, anchors: Dictionary, regions: Dictionary) -> Array:
	var mode := str(params.get("mode", ""))
	var sample_step := max(1.0, float(params.get("sample_step", 16.0)))
	var actor_radius := max(1.0, float(params.get("actor_radius", sample_step)))
	match mode:
		"region":
			return _surface_audit_region_points(params, anchors, sample_step, actor_radius)
		"route":
			return _surface_audit_route_points(anchors, sample_step, actor_radius)
		"mirror":
			return _surface_audit_mirror_points(regions, sample_step, actor_radius)
		_:
			return []


func _surface_audit_region_points(params: Dictionary, anchors: Dictionary, sample_step: float, actor_radius: float) -> Array:
	var center := _surface_audit_anchor_position(anchors, "center")
	if center.is_empty():
		return []
	var threshold := max(actor_radius, float(params.get("min_spawn_clearance", actor_radius)))
	var radii := [0.0, actor_radius, threshold, threshold + sample_step]
	var directions := [
		Vector2(1.0, 0.0),
		Vector2(-1.0, 0.0),
		Vector2(0.0, 1.0),
		Vector2(0.0, -1.0),
		Vector2(0.70710678, 0.70710678),
		Vector2(-0.70710678, 0.70710678),
		Vector2(0.70710678, -0.70710678),
		Vector2(-0.70710678, -0.70710678),
	]
	var points: Array = [_surface_audit_point(center.x, center.y)]
	for radius in radii:
		if float(radius) <= 0.0:
			continue
		for direction in directions:
			points.append(_surface_audit_point(center.x + direction.x * float(radius), center.y + direction.y * float(radius)))
			if points.size() >= SURFACE_MAX_POINTS:
				return points
	return points


func _surface_audit_route_points(anchors: Dictionary, sample_step: float, actor_radius: float) -> Array:
	var route_positions: Array[Vector2] = []
	var route_index := 0
	while anchors.has("route_%d" % route_index):
		var position := _surface_audit_anchor_position(anchors, "route_%d" % route_index)
		if not position.is_empty():
			route_positions.append(Vector2(float(position.get("x", 0.0)), float(position.get("y", 0.0))))
		route_index += 1
	if route_positions.is_empty():
		return []
	var points: Array = []
	for position in route_positions:
		for offset in [0.0, actor_radius, -actor_radius, actor_radius + sample_step, -(actor_radius + sample_step)]:
			points.append(_surface_audit_point(position.x, position.y + float(offset)))
			if points.size() >= SURFACE_MAX_POINTS:
				return points
	for index in range(route_positions.size() - 1):
		var start := route_positions[index]
		var finish := route_positions[index + 1]
		var midpoint := start.lerp(finish, 0.5)
		for offset in [0.0, actor_radius, -actor_radius, actor_radius + sample_step, -(actor_radius + sample_step)]:
			points.append(_surface_audit_point(midpoint.x, midpoint.y + float(offset)))
			if points.size() >= SURFACE_MAX_POINTS:
				return points
	return points


func _surface_audit_mirror_points(regions: Dictionary, sample_step: float, actor_radius: float) -> Array:
	var points: Array = []
	for key in ["left_region", "right_region"]:
		var position := _surface_audit_anchor_position(regions, key)
		if position.is_empty():
			continue
		for offset in [Vector2.ZERO, Vector2(sample_step, 0.0), Vector2(-sample_step, 0.0), Vector2(0.0, actor_radius), Vector2(0.0, -actor_radius)]:
			points.append(_surface_audit_point(float(position.get("x", 0.0)) + offset.x, float(position.get("y", 0.0)) + offset.y))
			if points.size() >= SURFACE_MAX_POINTS:
				return points
	return points


func _surface_audit_anchor_position(anchors: Dictionary, key: String) -> Dictionary:
	var anchor_value = anchors.get(key, {})
	if not (anchor_value is Dictionary):
		return {}
	var anchor: Dictionary = anchor_value
	if not bool(anchor.get("ok", false)):
		return {}
	var position = anchor.get("position", {})
	if position is Dictionary:
		return position
	return {}


func _surface_audit_point(x: float, y: float) -> Dictionary:
	return {"x": x, "y": y}


func _add_surface_audit_locator(locators: Dictionary, key: String, value: Variant) -> void:
	var locator := str(value).strip_edges()
	if key.strip_edges() != "" and locator != "":
		locators[key] = locator


func _resolve_surface_audit_locator_map(locators: Dictionary) -> Dictionary:
	var resolved: Dictionary = {}
	for key in locators.keys():
		var locator := str(locators.get(key, ""))
		var resolution: Dictionary = _owner.get_ref()._require_target(locator)
		if not bool(resolution.get("ok", false)):
			resolved[str(key)] = {"locator": locator, "ok": false, "error": (resolution.get("error", {}) as Dictionary).duplicate(true)}
			continue
		var node: Node = resolution.get("node")
		var position := _node_position_payload(node)
		resolved[str(key)] = {
			"locator": locator,
			"ok": true,
			"resolved": str(resolution.get("resolved", "")),
			"coordinate_space": str(position.get("coordinate_space", "")),
			"position": position.get("position", {}),
		}
	return resolved


func _surface_point_distance(origin: Dictionary, point: Variant) -> float:
	if not (point is Dictionary):
		return INF
	var payload: Dictionary = point
	var dx := float(payload.get("x", 0.0)) - float(origin.get("x", 0.0))
	var dy := float(payload.get("y", 0.0)) - float(origin.get("y", 0.0))
	var dz := float(payload.get("z", origin.get("z", 0.0))) - float(origin.get("z", 0.0))
	return sqrt((dx * dx) + (dy * dy) + (dz * dz))


func _surface_failure(error_type: String, message: String, params: Dictionary, details: Dictionary) -> Dictionary:
	var payload := details.duplicate(true)
	if not payload.has("command"):
		payload["command"] = str(params.get("command", ""))
	if params.has("target") and not payload.has("target"):
		payload["target"] = str(params.get("target", ""))
	if params.has("coordinate_space") and not payload.has("coordinate_space"):
		payload["coordinate_space"] = str(params.get("coordinate_space", ""))
	return _owner.get_ref()._error(error_type, message, payload)
