extends RefCounted

const QaSnapshot = preload("res://addons/godot_qa/qa_snapshot.gd")
const QaJsonEquality = preload("res://addons/godot_qa/qa_json_equality.gd")
const QaArgs = preload("res://addons/godot_qa/qa_args.gd")

const QaAssertionsState = preload("res://addons/godot_qa/qa_assertions_state.gd")
const QaAssertionsTemporal = preload("res://addons/godot_qa/qa_assertions_temporal.gd")
const QaAssertionsVisual = preload("res://addons/godot_qa/qa_assertions_visual.gd")
const QaAssertionsSurface = preload("res://addons/godot_qa/qa_assertions_surface.gd")

var _state: QaAssertionsState = QaAssertionsState.new(self)
var _temporal: QaAssertionsTemporal = QaAssertionsTemporal.new(self)
var _visual: QaAssertionsVisual = QaAssertionsVisual.new(self)
var _surface: QaAssertionsSurface = QaAssertionsSurface.new(self)
var _runtime: RefCounted = null
var _root_window: Window = null
var _runtime_errors: Array = []
var _session_fixed_fps := 0
const SURFACE_ALLOWED_SPACES = QaAssertionsSurface.SURFACE_ALLOWED_SPACES
const SURFACE_MAX_POINTS = QaAssertionsSurface.SURFACE_MAX_POINTS
const SURFACE_MAX_RADIUS = QaAssertionsSurface.SURFACE_MAX_RADIUS
const SURFACE_AUDIT_PROVIDER_METHODS = QaAssertionsSurface.SURFACE_AUDIT_PROVIDER_METHODS
# Kept in sync with TREND_DIRECTIONS in scenario_doc.rs.
const TREND_DIRECTIONS: Array[String] = QaAssertionsTemporal.TREND_DIRECTIONS
# Cap on how many raw samples temporal assertions echo back for debuggability.
const TEMPORAL_SAMPLE_ECHO_CAP = QaAssertionsTemporal.TEMPORAL_SAMPLE_ECHO_CAP
# Serialized-byte cap on a single `state.record` value. A recorded value is one
# controlled read (already required to be JSON-compatible), so it is naturally
# bounded, but a runaway provider/method could still return a large structure;
# capping keeps the `state_records` evidence map and sweep pass-through bounded.
# Exceeding the cap fails the step (a truncated JSON value would be corrupt),
# mirroring the read-only failure-first contract of the state-source path.
const STATE_RECORD_MAX_VALUE_BYTES = QaAssertionsState.STATE_RECORD_MAX_VALUE_BYTES


func _init(runtime: RefCounted, root_window: Window, runtime_errors := []) -> void:
	_runtime = runtime
	_root_window = root_window
	_runtime_errors = runtime_errors if runtime_errors is Array else []
	var options: Dictionary = QaArgs.parse_user_args(OS.get_cmdline_user_args()).get("options", {})
	var session_path := str(options.get("session_request", ""))
	if session_path != "" and FileAccess.file_exists(session_path):
		var request: Variant = JSON.parse_string(FileAccess.get_file_as_string(session_path))
		if request is Dictionary:
			var pacing: Variant = request.get("session", {}).get("frame_pacing", {})
			if pacing is Dictionary:
				_session_fixed_fps = int(pacing.get("fixed_fps", 0))


func execute(command_name: String, params := {}) -> Dictionary:
	var command_params: Dictionary = params if params is Dictionary else {}
	match command_name:
		"assert.exists":
			return _assert_exists(str(command_params.get("target", "")))
		"assert.not_exists":
			return _assert_not_exists(str(command_params.get("target", "")))
		"assert.visible":
			return _assert_visible(str(command_params.get("target", "")))
		"assert.hidden":
			return _assert_hidden(str(command_params.get("target", "")))
		"assert.enabled":
			return _assert_enabled(str(command_params.get("target", "")))
		"assert.disabled":
			return _assert_disabled(str(command_params.get("target", "")))
		"assert.focused":
			return _assert_focused(str(command_params.get("target", "")))
		"assert.text_equals":
			return _assert_text_equals(
				str(command_params.get("target", "")),
				str(command_params.get("expected", "")),
			)
		"assert.text_contains":
			return _assert_text_contains(
				str(command_params.get("target", "")),
				str(command_params.get("expected", "")),
			)
		"assert.inside_viewport":
			return _assert_inside_viewport(str(command_params.get("target", "")))
		"assert.no_overlap":
			return _assert_no_overlap(
				str(command_params.get("a", "")),
				str(command_params.get("b", "")),
			)
		"assert.no_errors":
			return _assert_no_errors()
		"assert.distance":
			return _assert_distance(command_params)
		"assert.state_matches":
			return _assert_state_matches(command_params)
		"qa_state.read":
			return _qa_state_read(command_params)
		"state.record":
			return _record_state(command_params)
		"assert.observe_until":
			return await _assert_observe_until(command_params)
		"assert.surface_probe":
			return _assert_surface_probe(command_params)
		"assert.surface_matches":
			return await _assert_surface_matches(command_params)
		"assert.temporal.max_delta_position":
			return await _assert_temporal_max_delta_position(command_params)
		"assert.temporal.follows":
			return await _assert_temporal_follows(command_params)
		"assert.temporal.never":
			return await _assert_temporal_never(command_params)
		"assert.temporal.within_limits":
			return await _assert_temporal_within_limits(command_params)
		"assert.temporal.trend":
			return await _assert_temporal_trend(command_params)
		"assert.temporal.frame_time":
			return await _assert_temporal_frame_time(command_params)
		"assert.temporal.distance_changed":
			return await _assert_temporal_distance_changed(command_params)
		"assert.visible_pixels":
			return await _assert_visible_pixels(command_params)
		"assert.not_occluded":
			return await _assert_not_occluded(command_params)
		"assert.renders_above":
			return await _assert_renders_above(command_params)
		"assert.renders_below":
			return await _assert_renders_below(command_params)

	return _error(
		"unsupported_command",
		"QA bridge command is not supported in this milestone",
		{
			"command": command_name,
			"params": command_params.duplicate(true),
		},
	)


func surface_audit_probe(params: Dictionary) -> Dictionary:
	return _surface.surface_audit_probe(params)


func teardown() -> void:
	_runtime = null
	_root_window = null
	_runtime_errors = []


func _assert_exists(target: String) -> Dictionary:
	var resolution = _runtime.resolve_target(target)
	if not bool(resolution.get("ok", false)):
		return {
			"ok": false,
			"error": (resolution.get("error", {}) as Dictionary).duplicate(true),
		}
	return _success(target, resolution)


func _assert_not_exists(target: String) -> Dictionary:
	var matches: Array = _runtime.find_target_matches(target)
	if matches.size() > 1:
		return _ambiguous_target(target, matches)
	if matches.is_empty():
		return _ok({"target": target})
	return _error(
		"not_exists_mismatch",
		"target unexpectedly existed in the runtime",
		{
			"target": target,
			"expected": "missing",
			"actual": _runtime.describe_node(matches[0]),
		},
	)


func _assert_visible(target: String) -> Dictionary:
	var resolution = _require_target(target)
	if not resolution.get("ok", false):
		return resolution
	var node: Node = resolution.get("node")
	if _runtime.is_target_visible(node):
		return _success(target, resolution)
	return _error(
		"visible_mismatch",
		"target is not visible",
		{
			"target": target,
			"expected": true,
			"actual": false,
		},
	)


func _assert_hidden(target: String) -> Dictionary:
	var resolution = _require_target(target)
	if not resolution.get("ok", false):
		return resolution
	var node: Node = resolution.get("node")
	if not _runtime.is_target_visible(node):
		return _success(target, resolution)
	return _error(
		"hidden_mismatch",
		"target is not hidden",
		{
			"target": target,
			"expected": false,
			"actual": true,
		},
	)


func _assert_enabled(target: String) -> Dictionary:
	var control_resolution = _require_control(target)
	if not control_resolution.get("ok", false):
		return control_resolution
	var control: Control = control_resolution.get("control")
	var disabled := _control_disabled(control)
	if disabled == false:
		return _success(target, control_resolution)
	return _error(
		"enabled_mismatch",
		"target is not enabled",
		{
			"target": target,
			"expected": true,
			"actual": false if disabled == true else null,
		},
	)


func _assert_disabled(target: String) -> Dictionary:
	var control_resolution = _require_control(target)
	if not control_resolution.get("ok", false):
		return control_resolution
	var control: Control = control_resolution.get("control")
	var disabled := _control_disabled(control)
	if disabled == true:
		return _success(target, control_resolution)
	return _error(
		"disabled_mismatch",
		"target is not disabled",
		{
			"target": target,
			"expected": true,
			"actual": disabled,
		},
	)


func _assert_focused(target: String) -> Dictionary:
	var resolution = _require_target(target)
	if not resolution.get("ok", false):
		return resolution
	var focus_owner := _root_window.gui_get_focus_owner()
	var node: Node = resolution.get("node")
	if focus_owner == node:
		return _success(target, resolution)
	return _error(
		"focused_mismatch",
		"focus owner did not match the expected target",
		{
			"target": target,
			"expected": target,
			"actual": _runtime.describe_node(focus_owner),
		},
	)


func _assert_text_equals(target: String, expected: String) -> Dictionary:
	var control_resolution = _require_control(target)
	if not control_resolution.get("ok", false):
		return control_resolution
	var control: Control = control_resolution.get("control")
	var actual := _control_text(control)
	if actual == expected:
		return _success(target, control_resolution)
	return _error(
		"text_equals_mismatch",
		"target text did not match the expected value",
		{
			"target": target,
			"expected": expected,
			"actual": actual,
		},
	)


func _assert_text_contains(target: String, expected: String) -> Dictionary:
	var control_resolution = _require_control(target)
	if not control_resolution.get("ok", false):
		return control_resolution
	var control: Control = control_resolution.get("control")
	var actual := _control_text(control)
	if actual.contains(expected):
		return _success(target, control_resolution)
	return _error(
		"text_contains_mismatch",
		"target text did not contain the expected substring",
		{
			"target": target,
			"expected": expected,
			"actual": actual,
		},
	)


func _assert_inside_viewport(target: String) -> Dictionary:
	var resolution = _require_target(target)
	if not resolution.get("ok", false):
		return resolution
	var node = resolution.get("node")
	if node is Control:
		var control: Control = node
		var rect := control.get_global_rect()
		var viewport_rect := control.get_viewport_rect()
		if (
			rect.position.x >= viewport_rect.position.x
			and rect.position.y >= viewport_rect.position.y
			and rect.end.x <= viewport_rect.end.x
			and rect.end.y <= viewport_rect.end.y
		):
			return _success(target, resolution)
		return _error(
			"inside_viewport_mismatch",
			"target rectangle was not fully inside the viewport",
			{
				"target": target,
				"expected": true,
				"actual": [rect.position.x, rect.position.y, rect.size.x, rect.size.y],
			},
		)
	if node is CanvasItem:
		var canvas_item: CanvasItem = node
		var screen_position := canvas_item.get_global_transform_with_canvas().origin
		var viewport_rect := canvas_item.get_viewport().get_visible_rect()
		if viewport_rect.has_point(screen_position):
			return _success(target, resolution)
		return _error(
			"inside_viewport_mismatch",
			"target position was not inside the viewport",
			{
				"target": target,
				"expected": true,
				"actual": [screen_position.x, screen_position.y],
			},
		)
	return _error(
		"target_not_canvas_item",
		"Resolved target is not a Control or CanvasItem",
		{
			"target": target,
			"actual": _runtime.describe_node(node),
		},
	)


func _assert_no_overlap(a: String, b: String) -> Dictionary:
	var left_resolution = _require_control(a)
	if not left_resolution.get("ok", false):
		return left_resolution
	var right_resolution = _require_control(b)
	if not right_resolution.get("ok", false):
		return right_resolution
	var left: Control = left_resolution.get("control")
	var right: Control = right_resolution.get("control")
	if not left.get_global_rect().intersects(right.get_global_rect()):
		return _ok(
			{
				"target": "%s vs %s" % [a, b],
				"a_resolved": left_resolution.get("resolved", ""),
				"b_resolved": right_resolution.get("resolved", ""),
			}
		)
	return _error(
		"no_overlap_mismatch",
		"targets overlapped within the viewport",
		{
			"target": "%s vs %s" % [a, b],
			"expected": false,
			"actual": true,
		},
	)


func _assert_no_errors() -> Dictionary:
	if _runtime_errors.is_empty():
		return _ok({"count": 0})
	return _error(
		"runtime_errors_present",
		"runner recorded runtime errors",
		{
			"target": "runtime",
			"expected": [],
			"actual": _runtime_errors.duplicate(true),
		},
	)


func _assert_distance(params: Dictionary) -> Dictionary:
	var a := str(params.get("a", params.get("target", "")))
	var b := str(params.get("b", params.get("other", "")))
	var min_raw = params.get("min", null)
	var max_raw = params.get("max", null)
	var has_min := params.has("min") and min_raw != null
	var has_max := params.has("max") and max_raw != null
	var min_distance := 0.0
	var max_distance := 0.0

	var has_a_point := params.has("a_point") or params.has("target_point")
	var has_b_point := params.has("b_point") or params.has("other_point")
	if (a.strip_edges() == "" and not has_a_point) or (b.strip_edges() == "" and not has_b_point):
		return _error(
			"invalid_distance_threshold",
			"distance assertion requires non-empty a/b targets or point providers",
			{
				"a": a,
				"b": b,
				"expected": "non-empty target locators or *_point provider objects",
				"actual": {"a": a, "b": b, "a_point": params.get("a_point", params.get("target_point", null)), "b_point": params.get("b_point", params.get("other_point", null))},
			},
		)
	if not has_min and not has_max:
		return _error(
			"invalid_distance_threshold",
			"distance assertion requires at least one threshold",
			{
				"a": a,
				"b": b,
				"expected": "min and/or max",
				"actual": {"min": null, "max": null},
			},
		)
	if has_min:
		if typeof(min_raw) != TYPE_INT and typeof(min_raw) != TYPE_FLOAT:
			return _error(
				"invalid_distance_threshold",
				"distance min threshold must be finite and non-negative",
				{
					"a": a,
					"b": b,
					"expected": "finite min >= 0",
					"actual": {"min": min_raw},
				},
			)
		min_distance = min_raw
	if has_min and (not is_finite(min_distance) or min_distance < 0.0):
		return _error(
			"invalid_distance_threshold",
			"distance min threshold must be finite and non-negative",
			{
				"a": a,
				"b": b,
				"expected": "finite min >= 0",
				"actual": {"min": min_distance},
			},
		)
	if has_max:
		if typeof(max_raw) != TYPE_INT and typeof(max_raw) != TYPE_FLOAT:
			return _error(
				"invalid_distance_threshold",
				"distance max threshold must be finite and non-negative",
				{
					"a": a,
					"b": b,
					"expected": "finite max >= 0",
					"actual": {"max": max_raw},
				},
			)
		max_distance = max_raw
	if has_max and (not is_finite(max_distance) or max_distance < 0.0):
		return _error(
			"invalid_distance_threshold",
			"distance max threshold must be finite and non-negative",
			{
				"a": a,
				"b": b,
				"expected": "finite max >= 0",
				"actual": {"max": max_distance},
			},
		)
	if has_min and has_max and min_distance > max_distance:
		return _error(
			"invalid_distance_threshold",
			"distance min threshold must be less than or equal to max threshold",
			{
				"a": a,
				"b": b,
				"expected": "min <= max",
				"actual": {"min": min_distance, "max": max_distance},
			},
		)

	var requested_space := str(params.get("coordinate_space", params.get("space", "auto"))).strip_edges()
	if requested_space == "":
		requested_space = "auto"
	if not ["auto", "world_2d", "viewport_2d", "world_3d"].has(requested_space):
		return _error(
			"unsupported_coordinate_space",
			"distance assertion does not support the requested coordinate space",
			{
				"mismatch_type": "unsupported_coordinate_space",
				"a": a,
				"b": b,
				"expected": ["auto", "world_2d", "viewport_2d", "world_3d"],
				"actual": requested_space,
			},
		)

	var left = _resolve_point_position(params.get("a_point", params.get("target_point", null)), "a_point") if has_a_point else _resolve_target_position(a)
	if not bool(left.get("ok", false)):
		return left
	var right = _resolve_point_position(params.get("b_point", params.get("other_point", null)), "b_point") if has_b_point else _resolve_target_position(b)
	if not bool(right.get("ok", false)):
		return right
	var space_result := _normalize_distance_spaces(left, right, requested_space)
	if not bool(space_result.get("ok", false)):
		return _distance_space_error(a, b, left, right, requested_space, space_result)
	left = space_result.get("left", left)
	right = space_result.get("right", right)

	var left_space := str(left.get("coordinate_space", ""))
	var right_space := str(right.get("coordinate_space", ""))
	if left_space != right_space:
		return _error(
			"unsupported_coordinate_space",
			"distance assertion requires matching coordinate spaces",
			{
				"mismatch_type": "unsupported_coordinate_space",
				"a": a,
				"b": b,
				"a_resolved": left.get("resolved", ""),
				"b_resolved": right.get("resolved", ""),
				"a_position": left.get("position", {}),
				"b_position": right.get("position", {}),
				"expected": {"coordinate_space": left_space},
				"actual": {"coordinate_space": right_space},
			},
		)

	var left_value = left.get("value", null)
	var right_value = right.get("value", null)
	var distance := _position_distance(left_value, right_value)
	if not is_finite(distance):
		return _error(
			"unsupported_coordinate_space",
			"distance assertion could not compare resolved positions",
			{
				"mismatch_type": "unsupported_coordinate_space",
				"a": a,
				"b": b,
				"a_resolved": left.get("resolved", ""),
				"b_resolved": right.get("resolved", ""),
				"a_position": left.get("position", {}),
				"b_position": right.get("position", {}),
				"expected": {"coordinate_space": left_space},
				"actual": {"coordinate_space": right_space},
			},
		)

	var payload := {
		"a": a,
		"b": b,
		"a_resolved": left.get("resolved", ""),
		"b_resolved": right.get("resolved", ""),
		"a_position": left.get("position", {}),
		"b_position": right.get("position", {}),
		"coordinate_space": left_space,
		"requested_coordinate_space": requested_space,
		"distance": distance,
		"threshold": {"min": min_distance if has_min else null, "max": max_distance if has_max else null},
	}
	if has_max and distance > max_distance:
		var too_far_payload := payload.duplicate(true)
		too_far_payload["mismatch_type"] = "distance_too_far"
		too_far_payload["expected"] = {"max": max_distance}
		too_far_payload["actual"] = {"distance": distance}
		return _error("distance_too_far", "distance exceeded max threshold", too_far_payload)
	if has_min and distance < min_distance:
		var too_near_payload := payload.duplicate(true)
		too_near_payload["mismatch_type"] = "distance_too_near"
		too_near_payload["expected"] = {"min": min_distance}
		too_near_payload["actual"] = {"distance": distance}
		return _error("distance_too_near", "distance was below min threshold", too_near_payload)
	return _ok(payload)


func _assert_state_matches(params: Dictionary) -> Dictionary:
	return _state._assert_state_matches(params)


func _qa_state_read(params: Dictionary) -> Dictionary:
	return _state._qa_state_read(params)


func _record_state(params: Dictionary) -> Dictionary:
	return _state._record_state(params)


func _assert_observe_until(params: Dictionary) -> Dictionary:
	return await _temporal._assert_observe_until(params)


func _resolve_target_position(target: String) -> Dictionary:
	var resolution = _require_target(target)
	if not bool(resolution.get("ok", false)):
		return resolution
	var node: Node = resolution.get("node")
	var resolved := str(resolution.get("resolved", ""))
	if node is Node2D:
		var p2 := (node as Node2D).global_position
		return {
			"ok": true,
			"target": target,
			"resolved": resolved,
			"coordinate_space": "world_2d",
			"position": {"x": p2.x, "y": p2.y},
			"value": p2,
			"node": node,
		}
	if node is Control:
		var center := (node as Control).get_global_rect().get_center()
		return {
			"ok": true,
			"target": target,
			"resolved": resolved,
			"coordinate_space": "viewport_2d",
			"position": {"x": center.x, "y": center.y},
			"value": center,
			"node": node,
		}
	if node is Node3D:
		var p3 := (node as Node3D).global_position
		return {
			"ok": true,
			"target": target,
			"resolved": resolved,
			"coordinate_space": "world_3d",
			"position": {"x": p3.x, "y": p3.y, "z": p3.z},
			"value": p3,
			"node": node,
		}
	return _error(
		"unsupported_coordinate_space",
		"resolved target does not provide a supported coordinate space",
		{
			"mismatch_type": "unsupported_coordinate_space",
			"target": target,
			"resolved": resolved,
			"actual": _runtime.describe_node(node),
		},
	)


func _resolve_point_position(point_request: Variant, field_name: String) -> Dictionary:
	var point_result: Dictionary = _runtime.resolve_point_provider(point_request)
	if not bool(point_result.get("ok", false)):
		return point_result
	var data: Dictionary = point_result.get("data", {})
	var position := Vector2(float(data.get("x", 0.0)), float(data.get("y", 0.0)))
	return {
		"ok": true,
		"target": field_name,
		"resolved": str(data.get("source_resolved", "")),
		"coordinate_space": "world_2d",
		"position": {"x": position.x, "y": position.y},
		"value": position,
		"node": null,
		"provider": data.get("provider", ""),
		"provider_method": data.get("provider_method", ""),
		"logical_id": data.get("logical_id", ""),
	}


func _normalize_distance_spaces(left: Dictionary, right: Dictionary, requested_space: String) -> Dictionary:
	var left_space := str(left.get("coordinate_space", ""))
	var right_space := str(right.get("coordinate_space", ""))
	if requested_space == "auto":
		if left_space == right_space:
			return {"ok": true, "left": left, "right": right}
		if _can_project_to_viewport(left) and _can_project_to_viewport(right) and (left_space == "viewport_2d" or right_space == "viewport_2d"):
			return {
				"ok": true,
				"left": _project_position_to_viewport(left),
				"right": _project_position_to_viewport(right),
			}
		return {"ok": false, "reason": "mismatched_coordinate_spaces"}
	if requested_space == left_space and requested_space == right_space:
		return {"ok": true, "left": left, "right": right}
	if requested_space == "viewport_2d" and _can_project_to_viewport(left) and _can_project_to_viewport(right):
		return {
			"ok": true,
			"left": _project_position_to_viewport(left),
			"right": _project_position_to_viewport(right),
		}
	return {"ok": false, "reason": "unsupported_coordinate_projection"}


func _can_project_to_viewport(position: Dictionary) -> bool:
	var space := str(position.get("coordinate_space", ""))
	if space == "viewport_2d":
		return true
	if space != "world_2d":
		return false
	var node = position.get("node", null)
	return node is CanvasItem


func _project_position_to_viewport(position: Dictionary) -> Dictionary:
	if str(position.get("coordinate_space", "")) == "viewport_2d":
		return position
	var node = position.get("node", null)
	if not (node is CanvasItem):
		return position
	var viewport_position := (node as CanvasItem).get_global_transform_with_canvas().origin
	var projected := position.duplicate(false)
	projected["original_coordinate_space"] = position.get("coordinate_space", "")
	projected["original_position"] = position.get("position", {})
	projected["coordinate_space"] = "viewport_2d"
	projected["position"] = {"x": viewport_position.x, "y": viewport_position.y}
	projected["value"] = viewport_position
	projected.erase("node")
	return projected


func _distance_space_error(a: String, b: String, left: Dictionary, right: Dictionary, requested_space: String, result: Dictionary) -> Dictionary:
	return _error(
		"unsupported_coordinate_space",
		"distance assertion could not compare targets in the requested coordinate space",
		{
			"mismatch_type": "unsupported_coordinate_space",
			"a": a,
			"b": b,
			"a_resolved": left.get("resolved", ""),
			"b_resolved": right.get("resolved", ""),
			"a_position": left.get("position", {}),
			"b_position": right.get("position", {}),
			"expected": {"coordinate_space": requested_space},
			"actual": {
				"a_coordinate_space": left.get("coordinate_space", ""),
				"b_coordinate_space": right.get("coordinate_space", ""),
				"reason": result.get("reason", ""),
			},
		},
	)


func _assert_temporal_max_delta_position(params: Dictionary) -> Dictionary:
	return await _temporal._assert_temporal_max_delta_position(params)


func _assert_temporal_follows(params: Dictionary) -> Dictionary:
	return await _temporal._assert_temporal_follows(params)


func _assert_temporal_never(params: Dictionary) -> Dictionary:
	return await _temporal._assert_temporal_never(params)


func _assert_temporal_within_limits(params: Dictionary) -> Dictionary:
	return await _temporal._assert_temporal_within_limits(params)


func _assert_temporal_trend(params: Dictionary) -> Dictionary:
	return await _temporal._assert_temporal_trend(params)


func _assert_temporal_frame_time(params: Dictionary) -> Dictionary:
	return await _temporal._assert_temporal_frame_time(params)


func _assert_temporal_distance_changed(params: Dictionary) -> Dictionary:
	return await _temporal._assert_temporal_distance_changed(params)


func _assert_visible_pixels(params: Dictionary) -> Dictionary:
	return await _visual._assert_visible_pixels(params)


func _assert_not_occluded(params: Dictionary) -> Dictionary:
	return await _visual._assert_not_occluded(params)


func _assert_surface_probe(params: Dictionary) -> Dictionary:
	return _surface._assert_surface_probe(params)


func _assert_surface_matches(params: Dictionary) -> Dictionary:
	return await _surface._assert_surface_matches(params)


func _assert_renders_above(params: Dictionary) -> Dictionary:
	return await _visual._assert_renders_above(params)


func _assert_renders_below(params: Dictionary) -> Dictionary:
	return await _visual._assert_renders_below(params)


func _require_target(target: String) -> Dictionary:
	var resolution = _runtime.resolve_target(target)
	if not bool(resolution.get("ok", false)):
		return {
			"ok": false,
			"error": (resolution.get("error", {}) as Dictionary).duplicate(true),
		}
	return resolution


func _temporal_window(params: Dictionary, target: String) -> Dictionary:
	return _temporal._temporal_window(params, target)


func _temporal_should_sample(window: Dictionary) -> bool:
	return _temporal._temporal_should_sample(window)


func _advance_temporal_sample(window: Dictionary) -> int:
	return await _temporal._advance_temporal_sample(window)


func _window_payload(window: Dictionary) -> Dictionary:
	return _temporal._window_payload(window)


func _position_distance(a: Variant, b: Variant) -> float:
	return _temporal._position_distance(a, b)


func _require_control(target: String) -> Dictionary:
	var resolution = _require_target(target)
	if not resolution.get("ok", false):
		return resolution
	var node = resolution.get("node")
	if node is Control:
		var success := resolution.duplicate(true)
		success["control"] = node
		return success
	return _error(
		"target_not_control",
		"Resolved target is not a Control",
		{
			"target": target,
			"actual": _runtime.describe_node(node),
		},
	)


func _success(target: String, resolution: Dictionary) -> Dictionary:
	return _ok(
		{
			"target": target,
			"resolved": resolution.get("resolved", ""),
		}
	)


func _ambiguous_target(target: String, matches: Array) -> Dictionary:
	return _error(
		"ambiguous_target",
		"Multiple nodes matched the requested target",
		{
			"target": target,
			"expected": "unique runtime node",
			"actual": _runtime.describe_matches(matches),
		},
	)


func _control_disabled(control: Control) -> Variant:
	if control == null:
		return null
	if _has_property(control, "disabled"):
		return bool(control.get("disabled"))
	return null


func _control_text(control: Control) -> String:
	if control == null:
		return ""
	if _has_property(control, "text"):
		return str(control.get("text"))
	return ""


func _has_property(object: Object, property_name: String) -> bool:
	for property in object.get_property_list():
		if property.get("name", "") == property_name:
			return true
	return false


func _temporal_frame_count(frame_value: Variant, second_value: Variant, default_frames: int) -> int:
	return _temporal._temporal_frame_count(frame_value, second_value, default_frames)


func _json_compatible(value: Variant) -> bool:
	match typeof(value):
		TYPE_NIL, TYPE_BOOL, TYPE_INT, TYPE_FLOAT, TYPE_STRING:
			return true
		TYPE_ARRAY:
			for item in value:
				if not _json_compatible(item):
					return false
			return true
		TYPE_DICTIONARY:
			for key in value.keys():
				if typeof(key) != TYPE_STRING:
					return false
				if not _json_compatible(value[key]):
					return false
			return true
		_:
			return false


func _ok(data := {}) -> Dictionary:
	return {
		"ok": true,
		"data": data if data is Dictionary else {},
	}


func _error(error_type: String, message: String, data := {}) -> Dictionary:
	var error_data: Dictionary = data if data is Dictionary else {}
	if not error_data.has("severity"):
		error_data["severity"] = "error"
	return {
		"ok": false,
		"error": {
			"type": error_type,
			"message": message,
			"data": error_data,
		},
	}
