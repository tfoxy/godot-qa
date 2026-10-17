extends RefCounted

const QaSnapshot = preload("res://addons/godot_qa/qa_snapshot.gd")

var _runtime: RefCounted = null
var _root_window: Window = null
var _runtime_errors: Array = []
const SURFACE_ALLOWED_SPACES := {"world_2d": true, "viewport_2d": true, "world_3d": true}
const SURFACE_MAX_POINTS := 32
const SURFACE_MAX_RADIUS := 4096.0
const SURFACE_AUDIT_PROVIDER_METHODS := {
	"qa_surface_probe": true,
	"get_qa_surface_state": true,
	"get_surface_qa_state": true,
	"qa_probe_surface": true,
}
# Kept in sync with TREND_DIRECTIONS in scenario_doc.rs.
const TREND_DIRECTIONS: Array[String] = ["increasing", "decreasing", "non_increasing", "non_decreasing"]
# Cap on how many raw samples temporal assertions echo back for debuggability.
const TEMPORAL_SAMPLE_ECHO_CAP := 32
# Serialized-byte cap on a single `state.record` value. A recorded value is one
# controlled read (already required to be JSON-compatible), so it is naturally
# bounded, but a runaway provider/method could still return a large structure;
# capping keeps the `state_records` evidence map and sweep pass-through bounded.
# Exceeding the cap fails the step (a truncated JSON value would be corrupt),
# mirroring the read-only failure-first contract of the state-source path.
const STATE_RECORD_MAX_VALUE_BYTES := 65536


func _init(runtime: RefCounted, root_window: Window, runtime_errors := []) -> void:
	_runtime = runtime
	_root_window = root_window
	_runtime_errors = runtime_errors if runtime_errors is Array else []


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
		var provider_resolution := _require_target(provider_target)
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
	return _ok({
		"command": "surface.audit_probe",
		"provider": resolved_provider,
		"provider_id": provider_id,
		"provider_method": provider.get("provider_method", provider_method),
		"coordinate_space": coordinate_space,
		"anchors": anchors,
		"regions": regions,
		"samples": provider_result.get("points", []),
	})


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
	var source := str(params.get("source", "auto")).strip_edges()
	if source == "provider":
		var state_result := _resolve_state_source(null, params, "", "")
		if not bool(state_result.get("ok", false)):
			return state_result
		return _assert_state_result_matches(params, state_result, "", "")
	var target := str(params.get("target", "")).strip_edges()
	if target == "":
		return _error("invalid_assertion_args", "state_matches requires a non-empty target", {"command": "assert.state_matches", "field": "target", "actual": params.get("target", null)})
	var resolution = _require_target(target)
	if not bool(resolution.get("ok", false)):
		return resolution
	var state_result := _resolve_state_source(resolution.get("node"), params, target, str(resolution.get("resolved", "")))
	if not bool(state_result.get("ok", false)):
		return state_result
	return _assert_state_result_matches(params, state_result, target, str(resolution.get("resolved", "")))


func _assert_state_result_matches(params: Dictionary, state_result: Dictionary, target: String, resolved: String) -> Dictionary:
	var state = state_result.get("state", {})
	var checks := _state_match_checks(params)
	if checks.is_empty():
		return _error("invalid_assertion_args", "state_matches requires path/operator or a non-empty matches array", {"command": "assert.state_matches", "target": target, "source": state_result.get("source", ""), "actual": params})
	var mismatches: Array = []
	var matched: Array = []
	for raw_check in checks:
		if not (raw_check is Dictionary):
			mismatches.append({"mismatch_type": "invalid_check", "expected": "check object", "actual": raw_check})
			continue
		var check: Dictionary = raw_check
		var check_result := _evaluate_state_match(state, check)
		if bool(check_result.get("ok", false)):
			matched.append(check_result.get("data", {}))
		else:
			mismatches.append(check_result.get("data", {}))
	var payload := {
		"command": "assert.state_matches",
		"target": target,
		"resolved": resolved,
		"source": state_result.get("source", ""),
		"provider": state_result.get("provider", ""),
		"method": state_result.get("method", ""),
		"matched": matched,
	}
	if mismatches.is_empty():
		return _ok(payload)
	payload["mismatches"] = mismatches
	payload["expected"] = "all state checks match"
	payload["actual"] = mismatches
	return _error("state_matches_mismatch", "runtime state did not match expected checks", payload)


func _qa_state_read(params: Dictionary) -> Dictionary:
	var provider := str(params.get("provider", params.get("provider_id", ""))).strip_edges()
	if provider != "":
		var provider_result: Dictionary = _runtime.read_state_provider(provider)
		if not bool(provider_result.get("ok", false)):
			return provider_result
		return _ok({
			"provider": provider_result.get("provider", provider),
			"source": "provider",
			"method": provider_result.get("method", ""),
			"state": provider_result.get("state", null),
		})
	var target := str(params.get("target", "")).strip_edges()
	if target == "":
		return _error("invalid_assertion_args", "qa_state.read requires a non-empty target", {"command": "qa_state.read", "field": "target", "actual": params.get("target", null)})
	var method := str(params.get("method", "")).strip_edges()
	if method == "":
		return _error("invalid_assertion_args", "qa_state.read requires a non-empty method", {"command": "qa_state.read", "target": target, "field": "method", "actual": params.get("method", null)})
	var resolution = _require_target(target)
	if not bool(resolution.get("ok", false)):
		return resolution
	var state_result := _call_state_method(resolution.get("node"), method, "method", target, str(resolution.get("resolved", "")))
	if not bool(state_result.get("ok", false)):
		return state_result
	return _ok({
		"target": target,
		"resolved": resolution.get("resolved", ""),
		"method": method,
		"state": state_result.get("state", null),
	})


# record_state captures a state VALUE as run evidence under a scenario-unique
# `name`, reusing the exact `_resolve_state_source` read path that state_matches
# and wait_until use (same provider/method allowlist, same read-only rules, same
# underlying error types on unresolved provider/target, invalid method, or a
# non-JSON-compatible value). It does not assert on the value; it records it. An
# optional dot-separated `path` selects a sub-value with the same
# `_state_path_value` helper state checks use (an absent path records null with
# `present: false` rather than failing). The recorded value is JSON-encodable via
# the shared read path and must stay within STATE_RECORD_MAX_VALUE_BYTES once
# serialized, otherwise the step fails with `state_record_value_too_large`.
func _record_state(params: Dictionary) -> Dictionary:
	var name := str(params.get("name", "")).strip_edges()
	if name == "":
		return _error("state_record_invalid_args", "state.record requires a non-empty name", {"command": "state.record", "field": "name", "actual": params.get("name", null)})
	var source := str(params.get("source", "auto")).strip_edges()
	var target := ""
	var resolved := ""
	var state_result: Dictionary
	if source == "provider":
		state_result = _resolve_state_source(null, params, "", "")
	else:
		target = str(params.get("target", "")).strip_edges()
		if target == "":
			return _error("state_record_invalid_args", "state.record method source requires a non-empty target", {"command": "state.record", "name": name, "field": "target", "actual": params.get("target", null)})
		var resolution = _require_target(target)
		if not bool(resolution.get("ok", false)):
			return resolution
		resolved = str(resolution.get("resolved", ""))
		state_result = _resolve_state_source(resolution.get("node"), params, target, resolved)
	if not bool(state_result.get("ok", false)):
		return state_result
	var value = state_result.get("state", null)
	var selection_path := str(params.get("path", "")).strip_edges()
	var present := true
	if selection_path != "":
		var selected := _state_path_value(value, selection_path)
		present = bool(selected.get("exists", false))
		value = selected.get("value", null)
	var encoded := JSON.stringify(value)
	var value_bytes := encoded.length()
	if value_bytes > STATE_RECORD_MAX_VALUE_BYTES:
		return _error("state_record_value_too_large", "state.record value exceeded the bounded evidence size", {
			"command": "state.record",
			"name": name,
			"source": state_result.get("source", source),
			"path": selection_path,
			"expected": {"max_value_bytes": STATE_RECORD_MAX_VALUE_BYTES},
			"actual": {"value_bytes": value_bytes},
		})
	var record := {
		"name": name,
		"source": state_result.get("source", source),
		"path": selection_path,
		"present": present,
		"value": value,
		"value_bytes": value_bytes,
	}
	var provider_id := str(state_result.get("provider", ""))
	if provider_id != "":
		record["provider"] = provider_id
	var method := str(state_result.get("method", ""))
	if method != "":
		record["method"] = method
	if target != "":
		record["target"] = target
		record["resolved"] = resolved
	return _ok(record)


func _assert_observe_until(params: Dictionary) -> Dictionary:
	var target := str(params.get("target", "")).strip_edges()
	if target == "":
		return _error("invalid_assertion_args", "observe_until requires a non-empty target", {"command": "assert.observe_until", "field": "target", "actual": params.get("target", null)})
	var window := _temporal_window(params, target)
	if not bool(window.get("ok", false)):
		return window
	var condition := _observe_until_condition_payload(params)
	if not bool(condition.get("ok", false)):
		return condition
	var resolution = _require_target(target)
	if not bool(resolution.get("ok", false)):
		return resolution
	var node: Node = resolution.get("node")
	var samples: Array = []
	var first_late_sample: Variant = null
	while _temporal_should_sample(window):
		await _advance_temporal_sample(window)
		var sample_result := _observe_until_sample_payload(node, params, condition.get("condition", {}), window, target, str(resolution.get("resolved", "")))
		if not bool(sample_result.get("ok", false)):
			return sample_result
		var sample: Dictionary = sample_result.get("sample", {})
		if _observe_until_sample_exceeds_window(window, sample):
			first_late_sample = sample.duplicate(true)
			break
		samples.append(sample)
		if bool(sample.get("matched", false)):
			var success_payload := {
				"command": "assert.observe_until",
				"target": target,
				"resolved": resolution.get("resolved", ""),
				"condition": condition.get("condition", {}),
				"window": _window_payload(window),
				"sample_every_frames": int(window.get("sample_every_frames", 1)),
				"first_matching_sample": sample.duplicate(true),
			}
			_attach_temporal_evidence("observe_until", target, params, success_payload)
			return _ok(success_payload)
	if first_late_sample == null:
		first_late_sample = await _observe_until_first_late_sample(node, params, condition.get("condition", {}), window, target, str(resolution.get("resolved", "")))
	var last_sample := samples.back() if not samples.is_empty() else {}
	var last_actual = null
	if last_sample is Dictionary:
		last_actual = (last_sample as Dictionary).get("actual", null)
	var failure_payload := {
		"command": "assert.observe_until",
		"target": target,
		"resolved": resolution.get("resolved", ""),
		"condition": condition.get("condition", {}),
		"window": _window_payload(window),
		"sample_every_frames": int(window.get("sample_every_frames", 1)),
		"observed_samples": samples,
		"first_late_sample": first_late_sample,
		"why_no_match": _observe_until_why_no_match(samples, first_late_sample),
		"expected": condition.get("expected", null),
		"actual": last_actual,
	}
	_attach_temporal_evidence("observe_until", target, params, failure_payload)
	return _error("observe_until_mismatch", "condition did not become true within the bounded window", failure_payload)


func _observe_until_condition_payload(params: Dictionary) -> Dictionary:
	var families: Array = []
	if params.has("visible"):
		families.append("visible")
	if params.has("property"):
		families.append("property")
	if params.has("matches"):
		families.append("state_matches")
	if families.size() != 1:
		return _error("invalid_assertion_args", "observe_until requires exactly one condition family", {"command": "assert.observe_until", "expected": ["visible", "property", "matches"], "actual": families})
	var family := str(families[0])
	if family == "visible":
		if typeof(params.get("visible", null)) != TYPE_BOOL:
			return _error("invalid_assertion_args", "observe_until visible condition must be a boolean", {"command": "assert.observe_until", "field": "visible", "actual": params.get("visible", null)})
		return {"ok": true, "condition": {"type": "visible", "visible": bool(params.get("visible", false))}, "expected": {"visible": bool(params.get("visible", false))}}
	if family == "property":
		var property := str(params.get("property", "")).strip_edges()
		var operator := str(params.get("operator", params.get("op", "equals"))).strip_edges()
		if operator == "":
			operator = "equals"
		if property == "" or not _state_match_operators().has(operator):
			return _error("invalid_assertion_args", "observe_until property conditions require a property and supported operator", {"command": "assert.observe_until", "expected": {"property": "non-empty", "operator": _state_match_operators()}, "actual": {"property": property, "operator": operator}})
		if ["equals", "contains", "within"].has(operator) and not params.has("expected"):
			return _error("invalid_assertion_args", "observe_until property condition requires expected", {"command": "assert.observe_until", "field": "expected", "actual": null})
		if operator == "within" and not bool(_state_match_within_tolerance(params).get("ok", false)):
			return _error("invalid_assertion_args", "observe_until within conditions require a positive numeric tolerance", {"command": "assert.observe_until", "field": "tolerance", "expected": "positive number", "actual": params.get("tolerance", null)})
		return {"ok": true, "condition": {"type": "property", "property": property, "operator": operator, "expected": params.get("expected", null), "tolerance": params.get("tolerance", null)}, "expected": {"property": property, "operator": operator, "value": params.get("expected", null)}}
	var state_params := params.duplicate(true)
	var checks := _state_match_checks(state_params)
	if checks.is_empty():
		return _error("invalid_assertion_args", "observe_until state condition requires matches", {"command": "assert.observe_until", "field": "matches", "actual": params.get("matches", null)})
	return {"ok": true, "condition": {"type": "state_matches", "source": str(params.get("source", "auto")), "method": str(params.get("method", "")), "matches": checks}, "expected": {"matches": checks}}


func _observe_until_sample_payload(node: Node, params: Dictionary, condition: Dictionary, window: Dictionary, target: String, resolved: String) -> Dictionary:
	if node == null or not is_instance_valid(node):
		return _error("target_freed", "observe_until target was freed before state could be read", {"command": "assert.observe_until", "target": target, "resolved": resolved})
	var condition_type := str(condition.get("type", ""))
	var sample := {"sample": int(window.get("sample", 0)), "frame": Engine.get_process_frames(), "time_msec": Time.get_ticks_msec(), "target": target, "resolved": resolved, "condition": condition.duplicate(true), "matched": false, "actual": null}
	if condition_type == "visible":
		var visible: bool = _runtime.is_target_visible(node)
		sample["actual"] = {"visible": visible}
		sample["matched"] = visible == bool(condition.get("visible", false))
		return {"ok": true, "sample": sample}
	if condition_type == "property":
		var property := str(condition.get("property", ""))
		var exists := node is Object and _has_property(node, property)
		var value = (node as Object).get(property) if exists else null
		var operator := str(condition.get("operator", "equals"))
		var actual_payload := {"path": property, "operator": operator, "expected": condition.get("expected", null), "actual": value, "present": exists}
		var matched := false
		if operator == "present":
			matched = exists
		elif operator == "absent":
			matched = not exists
		elif not exists:
			actual_payload["mismatch_type"] = "property_not_found"
		elif operator == "equals":
			matched = value == condition.get("expected", null)
			if not matched:
				actual_payload["mismatch_type"] = "value_mismatch"
				_add_state_numeric_mismatch_diagnostics(actual_payload, value, condition.get("expected", null))
		elif operator == "approx_equals" or operator == "approximately_equals":
			matched = _state_values_approx_equal(value, condition.get("expected", null), condition)
			if not matched:
				actual_payload["mismatch_type"] = "approx_mismatch"
				actual_payload["epsilon"] = _state_match_epsilon(condition)
		elif operator == "greater_than":
			matched = _state_numeric_compare(value, condition.get("expected", null), ">")
			if not matched:
				actual_payload["mismatch_type"] = "greater_than_mismatch"
		elif operator == "less_than":
			matched = _state_numeric_compare(value, condition.get("expected", null), "<")
			if not matched:
				actual_payload["mismatch_type"] = "less_than_mismatch"
		elif operator == "within":
			matched = _state_values_within(value, condition.get("expected", null), float(condition.get("tolerance", 0.0)))
			if not matched:
				actual_payload["mismatch_type"] = "within_mismatch"
				actual_payload["tolerance"] = condition.get("tolerance", null)
		elif operator == "contains":
			matched = _state_value_contains(value, condition.get("expected", null))
			if not matched:
				actual_payload["mismatch_type"] = "contains_mismatch"
		sample["actual"] = actual_payload
		sample["matched"] = matched
		return {"ok": true, "sample": sample}
	if condition_type == "state_matches":
		var state_result := _resolve_state_source(node, params, target, resolved)
		if not bool(state_result.get("ok", false)):
			return state_result
		var state = state_result.get("state", {})
		var mismatches: Array = []
		var matched: Array = []
		for raw_check in condition.get("matches", []):
			if not (raw_check is Dictionary):
				mismatches.append({"mismatch_type": "invalid_check", "expected": "check object", "actual": raw_check})
				continue
			var check_result := _evaluate_state_match(state, raw_check)
			if bool(check_result.get("ok", false)):
				matched.append(check_result.get("data", {}))
			else:
				mismatches.append(check_result.get("data", {}))
		sample["source"] = state_result.get("source", "")
		sample["method"] = state_result.get("method", "")
		sample["actual"] = {"matched": matched, "mismatches": mismatches}
		sample["matched"] = mismatches.is_empty()
		return {"ok": true, "sample": sample}
	return _error("invalid_assertion_args", "observe_until condition type is unsupported", {"command": "assert.observe_until", "actual": condition_type})


func _observe_until_first_late_sample(node: Node, params: Dictionary, condition: Dictionary, window: Dictionary, target: String, resolved: String) -> Variant:
	var cadence := max(int(window.get("sample_every_frames", 1)), 1)
	await _runtime.process_frames(cadence)
	var late_window := window.duplicate(true)
	late_window["sample"] = int(window.get("sample", 0)) + 1
	var result := _observe_until_sample_payload(node, params, condition, late_window, target, resolved)
	if bool(result.get("ok", false)):
		return result.get("sample", {})
	return null


func _observe_until_sample_exceeds_window(window: Dictionary, sample: Dictionary) -> bool:
	if str(window.get("mode", "frames")) != "seconds":
		return false
	var sample_msec := int(sample.get("time_msec", Time.get_ticks_msec()))
	var start_msec := int(window.get("start_msec", sample_msec))
	var elapsed := float(sample_msec - start_msec) / 1000.0
	return elapsed > float(window.get("seconds", 0.0))


func _observe_until_why_no_match(samples: Array, first_late_sample: Variant) -> String:
	if first_late_sample is Dictionary and bool((first_late_sample as Dictionary).get("matched", false)):
		return "matched_after_window"
	if samples.is_empty():
		return "no_samples_collected"
	return "no_observed_sample_matched"


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


func _resolve_state_source(node: Node, params: Dictionary, target: String = "", resolved: String = "") -> Dictionary:
	var source := str(params.get("source", "auto")).strip_edges()
	if source == "":
		source = "auto"
	if source == "provider":
		var provider := str(params.get("provider", params.get("provider_id", ""))).strip_edges()
		if provider == "":
			return _error("invalid_assertion_args", "state_matches source: provider requires a non-empty provider", {"command": "assert.state_matches", "field": "provider", "actual": params.get("provider", null)})
		var provider_result: Dictionary = _runtime.read_state_provider(provider)
		if not bool(provider_result.get("ok", false)):
			return provider_result
		return {
			"ok": true,
			"source": "provider",
			"provider": provider_result.get("provider", provider),
			"method": provider_result.get("method", ""),
			"state": provider_result.get("state", null),
		}
	if node == null or not is_instance_valid(node):
		return _error("target_freed", "state_matches target was freed before state could be read", {"command": "assert.state_matches", "target": target, "resolved": resolved})
	var method := str(params.get("method", "")).strip_edges()
	if source == "method" or source == "qa_method" or method != "":
		return _call_state_method(node, method, source, target, resolved)
	if source == "metadata" or source == "meta" or source == "auto":
		return {"ok": true, "source": "metadata", "method": "", "state": _node_metadata_state(node)}
	if source == "inspect":
		return {"ok": true, "source": "inspect", "method": "", "state": QaSnapshot.node_to_dict(node)}
	return _error("invalid_assertion_args", "state_matches source must be metadata, inspect, method, provider, or auto", {"command": "assert.state_matches", "field": "source", "actual": source})


func _call_state_method(node: Node, method: String, source: String, target: String = "", resolved: String = "") -> Dictionary:
	var context := _state_method_error_context(node, method, source, target, resolved)
	if method == "":
		context["field"] = "method"
		context["actual"] = method
		context["rejection_reason"] = "name"
		context["expected"] = "non-empty method matching allowed_patterns"
		return _error("invalid_assertion_args", "state_matches method source requires a method name", context)
	var name_check := _validate_state_method_name(method)
	if not bool(name_check.get("ok", false)):
		context["field"] = "method"
		context["actual"] = method
		context["rejection_reason"] = "name"
		context["expected"] = "method matching allowed_patterns"
		context["name_rejection_reason"] = str(name_check.get("name_rejection_reason", "invalid_name"))
		return _error("invalid_assertion_args", "state_matches method must match an approved read-only QA state pattern", context)
	if not node.has_method(method):
		context["rejection_reason"] = "missing_method"
		context["expected"] = "resolved target exposes requested QA state method"
		context["actual"] = "method_not_found"
		return _error("missing_state_source", "state_matches target does not expose the requested QA method", context)
	var method_is_zero_arg := false
	for method_info in node.get_method_list():
		if not (method_info is Dictionary):
			continue
		var info: Dictionary = method_info
		if str(info.get("name", "")) != method:
			continue
		var args = info.get("args", [])
		method_is_zero_arg = args is Array and (args as Array).is_empty()
		break
	if not method_is_zero_arg:
		context["rejection_reason"] = "arguments"
		context["expected"] = "zero arguments"
		context["actual"] = "requires_arguments"
		return _error("invalid_assertion_args", "state_matches QA methods must take zero arguments", context)
	var runtime_error_count_before := _runtime_errors.size()
	var value = node.call(method)
	if _runtime_errors.size() > runtime_error_count_before:
		var runtime_errors: Array = _runtime_errors.slice(runtime_error_count_before)
		context["rejection_reason"] = "runtime_error"
		context["expected"] = "QA state method returns without recording runtime errors"
		context["actual"] = runtime_errors.duplicate(true)
		return _error("runtime_errors_present", "state_matches QA method recorded runtime errors", context)
	if not _json_compatible(value):
		context["rejection_reason"] = "non_json_return"
		context["expected"] = "JSON-compatible Dictionary or Array"
		context["actual"] = "non_json_compatible_return"
		context["actual_type"] = type_string(typeof(value))
		return _error("invalid_state_source", "state_matches QA method returned non JSON-compatible data", context)
	if not (value is Dictionary or value is Array):
		context["rejection_reason"] = "non_json_return"
		context["expected"] = "Dictionary or Array"
		context["actual"] = value
		context["actual_type"] = type_string(typeof(value))
		return _error("invalid_state_source", "state_matches QA method must return a Dictionary or Array", context)
	return {"ok": true, "source": "method", "method": method, "state": value}


func _state_method_error_context(node: Node, method: String, source: String, target: String, resolved: String) -> Dictionary:
	var resolved_node := resolved
	if resolved_node == "":
		resolved_node = _runtime.describe_node(node)
	return {
		"command": "assert.state_matches",
		"target": target,
		"resolved": resolved_node,
		"actual": _runtime.describe_node(node),
		"source": source,
		"method": method,
		"allowed_patterns": _state_method_allowed_patterns(),
	}


func _state_method_allowed_patterns() -> Array:
	return ["qa_*", "get_qa_*", "get_*_qa_state"]


func _validate_state_method_name(method: String) -> Dictionary:
	if method.begins_with("_"):
		return {"ok": false, "name_rejection_reason": "private_method"}
	var mutating_prefixes := ["set_", "add_", "remove_", "delete_", "clear_", "queue_", "free", "call_", "emit_", "change_", "qa_mutate_"]
	for prefix in mutating_prefixes:
		if method.begins_with(prefix):
			return {"ok": false, "name_rejection_reason": "mutating_method_prefix"}
	if method.begins_with("qa_") or method.begins_with("get_qa_"):
		return {"ok": true}
	if method.begins_with("get_") and method.ends_with("_qa_state") and method.length() > "get__qa_state".length():
		return {"ok": true}
	return {"ok": false, "name_rejection_reason": "unsupported_method_name"}


func _node_metadata_state(node: Node) -> Dictionary:
	var state := {}
	for meta_name in node.get_meta_list():
		state[str(meta_name)] = node.get_meta(str(meta_name))
	return state


func _state_match_checks(params: Dictionary) -> Array:
	if params.has("matches"):
		var matches = params.get("matches", [])
		if matches is Array:
			return matches
		if matches is Dictionary:
			var checks: Array = []
			for path in (matches as Dictionary).keys():
				checks.append({"path": path, "operator": "equals", "expected": (matches as Dictionary).get(path)})
			return checks
		return []
	if not (params.has("path") or params.has("field") or params.has("key") or params.has("operator") or params.has("op")):
		return []
	return [params]


func _evaluate_state_match(state: Variant, check: Dictionary) -> Dictionary:
	var path := check.get("path", check.get("field", check.get("key", "")))
	var operator := str(check.get("operator", check.get("op", "equals"))).strip_edges()
	if operator == "":
		operator = "equals"
	if not _state_match_operators().has(operator):
		return {"ok": false, "data": {"mismatch_type": "invalid_operator", "path": path, "expected": _state_match_operators(), "actual": operator}}
	var value_result := _selected_state_path_value(state, check, path) if check.has("where") else _state_path_value(state, path)
	var exists := bool(value_result.get("exists", false))
	var actual = value_result.get("value", null)
	var payload := {"path": path, "operator": operator, "expected": check.get("expected", null), "actual": actual, "present": exists}
	for key in ["where", "value_path", "selection_count", "selection_index", "selection_mismatch_type"]:
		if value_result.has(key):
			payload[key] = value_result.get(key)
	if operator == "present":
		return {"ok": exists, "data": payload}
	if operator == "absent":
		return {"ok": not exists, "data": payload}
	if not exists:
		payload["mismatch_type"] = "state_path_missing"
		return {"ok": false, "data": payload}
	if operator == "equals":
		var expected = check.get("expected", null)
		var equals: bool = actual == expected
		if not equals:
			payload["mismatch_type"] = "value_mismatch"
			_add_state_numeric_mismatch_diagnostics(payload, actual, expected)
		return {"ok": equals, "data": payload}
	if operator == "approx_equals" or operator == "approximately_equals":
		var approx_equals := _state_values_approx_equal(actual, check.get("expected", null), check)
		if not approx_equals:
			payload["mismatch_type"] = "approx_mismatch"
			payload["epsilon"] = _state_match_epsilon(check)
		return {"ok": approx_equals, "data": payload}
	if operator == "greater_than":
		var greater_than := _state_numeric_compare(actual, check.get("expected", null), ">")
		if not greater_than:
			payload["mismatch_type"] = "greater_than_mismatch"
		return {"ok": greater_than, "data": payload}
	if operator == "less_than":
		var less_than := _state_numeric_compare(actual, check.get("expected", null), "<")
		if not less_than:
			payload["mismatch_type"] = "less_than_mismatch"
		return {"ok": less_than, "data": payload}
	if operator == "within":
		var tolerance_check := _state_match_within_tolerance(check)
		if not bool(tolerance_check.get("ok", false)):
			payload["mismatch_type"] = "invalid_tolerance"
			payload["tolerance"] = check.get("tolerance", null)
			payload["expected"] = "positive numeric tolerance"
			return {"ok": false, "data": payload}
		var tolerance := float(tolerance_check.get("tolerance", 0.0))
		payload["tolerance"] = tolerance
		var within := _state_values_within(actual, check.get("expected", null), tolerance)
		if not within:
			payload["mismatch_type"] = "within_mismatch"
			_add_state_numeric_mismatch_diagnostics(payload, actual, check.get("expected", null))
		return {"ok": within, "data": payload}
	var contains: bool = _state_value_contains(actual, check.get("expected", null))
	if not contains:
		payload["mismatch_type"] = "contains_mismatch"
	return {"ok": contains, "data": payload}


func _state_match_operators() -> Array[String]:
	return ["equals", "contains", "present", "absent", "approx_equals", "approximately_equals", "greater_than", "less_than", "within"]


func _state_match_epsilon(check: Dictionary) -> float:
	return max(0.0, float(check.get("epsilon", 0.0001)))


func _state_values_approx_equal(actual: Variant, expected: Variant, check: Dictionary) -> bool:
	if not _state_value_is_number(actual) or not _state_value_is_number(expected):
		return false
	return abs(float(actual) - float(expected)) <= _state_match_epsilon(check)


func _state_match_within_tolerance(check: Dictionary) -> Dictionary:
	# `within` uses an absolute tolerance, distinct from approx_equals' epsilon:
	# it is required and must be a positive finite number.
	var raw = check.get("tolerance", null)
	if not _state_value_is_number(raw):
		return {"ok": false}
	var tolerance := float(raw)
	if not is_finite(tolerance) or tolerance <= 0.0:
		return {"ok": false}
	return {"ok": true, "tolerance": tolerance}


func _state_values_within(actual: Variant, expected: Variant, tolerance: float) -> bool:
	if not _state_value_is_number(actual) or not _state_value_is_number(expected):
		return false
	return abs(float(actual) - float(expected)) <= tolerance


func _state_numeric_compare(actual: Variant, expected: Variant, operator: String) -> bool:
	if not _state_value_is_number(actual) or not _state_value_is_number(expected):
		return false
	if operator == ">":
		return float(actual) > float(expected)
	return float(actual) < float(expected)


func _add_state_numeric_mismatch_diagnostics(payload: Dictionary, actual: Variant, expected: Variant) -> void:
	if not _state_value_is_number(actual) or not _state_value_is_number(expected):
		return
	payload["actual_full_precision"] = _state_number_full_precision(actual)
	payload["expected_full_precision"] = _state_number_full_precision(expected)
	payload["numeric_delta"] = float(actual) - float(expected)


func _state_number_full_precision(value: Variant) -> String:
	if typeof(value) == TYPE_INT:
		return str(int(value))
	return String.num(float(value), 17)


func _state_value_is_number(value: Variant) -> bool:
	var value_type := typeof(value)
	return value_type == TYPE_INT or value_type == TYPE_FLOAT


func _selected_state_path_value(state: Variant, check: Dictionary, path: Variant) -> Dictionary:
	var value_path = check.get("value_path", null)
	if value_path == null or str(value_path).strip_edges() == "":
		return {
			"exists": false,
			"value": null,
			"selection_mismatch_type": "missing_value_path",
			"where": check.get("where", null),
			"value_path": value_path,
		}
	var where = check.get("where")
	if not where is Dictionary:
		return {
			"exists": false,
			"value": null,
			"selection_mismatch_type": "invalid_where",
			"where": where,
			"value_path": value_path,
		}
	var array_result := _state_path_value(state, path)
	if not bool(array_result.get("exists", false)):
		return {
			"exists": false,
			"value": null,
			"selection_mismatch_type": "state_path_missing",
			"where": where,
			"value_path": value_path,
		}
	var array_value = array_result.get("value", null)
	if not array_value is Array:
		return {
			"exists": false,
			"value": null,
			"selection_mismatch_type": "selection_target_not_array",
			"where": where,
			"value_path": value_path,
			"actual": array_value,
		}
	var matches: Array = []
	var match_indexes: Array[int] = []
	for index in range((array_value as Array).size()):
		var item = (array_value as Array)[index]
		if _state_item_matches_where(item, where as Dictionary):
			matches.append(item)
			match_indexes.append(index)
	if matches.is_empty():
		return {
			"exists": false,
			"value": null,
			"selection_mismatch_type": "selection_no_matches",
			"where": where,
			"value_path": value_path,
			"selection_count": 0,
		}
	if matches.size() > 1:
		return {
			"exists": false,
			"value": matches,
			"selection_mismatch_type": "selection_multiple_matches",
			"where": where,
			"value_path": value_path,
			"selection_count": matches.size(),
		}
	var selected = matches[0]
	var selected_value := _state_path_value(selected, value_path)
	selected_value["where"] = where
	selected_value["value_path"] = value_path
	selected_value["selection_count"] = 1
	selected_value["selection_index"] = match_indexes[0]
	return selected_value


func _state_item_matches_where(item: Variant, where: Dictionary) -> bool:
	if not item is Dictionary:
		return false
	for key in where.keys():
		var expected = where.get(key)
		var actual_result := _state_path_value(item, key)
		if not bool(actual_result.get("exists", false)):
			return false
		if actual_result.get("value") != expected:
			return false
	return true


func _state_path_value(state: Variant, path: Variant) -> Dictionary:
	if path == null or str(path) == "":
		return {"exists": true, "value": state}
	var parts: Array = []
	if path is Array:
		parts = path
	else:
		for part in str(path).split("."):
			parts.append(part)
	var current = state
	for raw_part in parts:
		var part := str(raw_part)
		if current is Dictionary:
			var dict: Dictionary = current
			if not dict.has(part):
				return {"exists": false, "value": null}
			current = dict.get(part)
		elif current is Array and part.is_valid_int():
			var array: Array = current
			var index := int(part)
			if index < 0 or index >= array.size():
				return {"exists": false, "value": null}
			current = array[index]
		else:
			return {"exists": false, "value": null}
	return {"exists": true, "value": current}


func _state_value_contains(actual: Variant, expected: Variant) -> bool:
	if actual is String:
		return (actual as String).contains(str(expected))
	if actual is Array:
		return (actual as Array).has(expected)
	if actual is Dictionary:
		var dict: Dictionary = actual
		if expected is Dictionary:
			for key in (expected as Dictionary).keys():
				if not dict.has(key) or dict.get(key) != (expected as Dictionary).get(key):
					return false
			return true
		return dict.has(expected)
	return false


func _assert_temporal_max_delta_position(params: Dictionary) -> Dictionary:
	var target := str(params.get("target", ""))
	var max_delta := float(params.get("max", -1.0))
	var window := _temporal_window(params, target)
	if not bool(window.get("ok", false)):
		return window
	var resolution = _require_target(target)
	if not resolution.get("ok", false):
		return resolution
	var node = resolution.get("node")
	var anchor = _node_position_value(node)
	if anchor == null:
		return _error("unsupported_node_type", "Resolved target does not expose a global position", {"target": target, "actual": _runtime.describe_node(node)})
	var peak_delta := 0.0
	var peak_sample := 0
	var peak_sample_payload := {}
	var samples: Array = []
	while _temporal_should_sample(window):
		await _advance_temporal_sample(window)
		var position = _node_position_value(node)
		if position == null:
			return _error("unsupported_node_type", "Resolved target stopped exposing a global position", {"target": target, "actual": _runtime.describe_node(node)})
		var delta := _position_distance(position, anchor)
		var sample := {"sample": int(window.get("sample", 0)), "frame": Engine.get_process_frames(), "time_msec": Time.get_ticks_msec(), "position": _position_payload(position), "delta": delta}
		samples.append(sample)
		if delta > peak_delta:
			peak_delta = delta
			peak_sample = int(sample.get("sample", 0))
			peak_sample_payload = sample.duplicate(true)
	if peak_delta <= max_delta:
		return _ok({"target": target, "resolved": resolution.get("resolved", ""), "window": _window_payload(window), "max": max_delta, "actual_max_delta": peak_delta, "failing_sample": null})
	var failure_payload := {"target": target, "resolved": resolution.get("resolved", ""), "window": _window_payload(window), "expected": {"max": max_delta, "window": _window_payload(window)}, "actual": {"max": peak_delta, "sample": peak_sample}, "sample": peak_sample_payload, "expected_max_delta": max_delta, "actual_max_delta": peak_delta, "failing_sample": peak_sample, "samples": _nearby_samples(samples, peak_sample, _evidence_context(params))}
	_attach_temporal_evidence("max_delta_position", target, params, failure_payload)
	return _error("max_delta_position_mismatch", "target exceeded max displacement bound", failure_payload)


func _assert_temporal_follows(params: Dictionary) -> Dictionary:
	var leader_target := str(params.get("leader", ""))
	var follower_target := str(params.get("follower", ""))
	var tolerance := float(params.get("tolerance", -1.0))
	var window := _temporal_window(params, "%s -> %s" % [leader_target, follower_target])
	if not bool(window.get("ok", false)):
		return window
	var leader_resolution = _require_target(leader_target)
	if not leader_resolution.get("ok", false):
		return leader_resolution
	var follower_resolution = _require_target(follower_target)
	if not follower_resolution.get("ok", false):
		return follower_resolution
	var leader = leader_resolution.get("node")
	var follower = follower_resolution.get("node")
	var leader_anchor = _node_position_value(leader)
	var follower_anchor = _node_position_value(follower)
	if leader_anchor == null or follower_anchor == null:
		return _error("unsupported_node_type", "Leader and follower must both expose global positions", {"leader": leader_target, "follower": follower_target})
	var offset = _position_subtract(follower_anchor, leader_anchor)
	var peak_offset_error := 0.0
	var peak_sample := 0
	var peak_sample_payload := {}
	var samples: Array = []
	while _temporal_should_sample(window):
		await _advance_temporal_sample(window)
		var leader_position = _node_position_value(leader)
		var follower_position = _node_position_value(follower)
		if leader_position == null or follower_position == null:
			return _error("unsupported_node_type", "Leader and follower stopped exposing global positions", {"leader": leader_target, "follower": follower_target})
		var expected = _position_add(leader_position, offset)
		var error := _position_distance(follower_position, expected)
		var sample := {"sample": int(window.get("sample", 0)), "frame": Engine.get_process_frames(), "time_msec": Time.get_ticks_msec(), "leader_position": _position_payload(leader_position), "follower_position": _position_payload(follower_position), "expected_position": _position_payload(expected), "offset_error": error}
		samples.append(sample)
		if error > peak_offset_error:
			peak_offset_error = error
			peak_sample = int(sample.get("sample", 0))
			peak_sample_payload = sample.duplicate(true)
	if peak_offset_error <= tolerance:
		return _ok({"leader": leader_target, "follower": follower_target, "window": _window_payload(window), "tolerance": tolerance, "actual_max_error": peak_offset_error, "failing_sample": null})
	var failure_payload := {"leader": leader_target, "follower": follower_target, "target": follower_target, "window": _window_payload(window), "expected": {"tolerance": tolerance, "window": _window_payload(window)}, "actual": {"max_error": peak_offset_error, "sample": peak_sample}, "sample": peak_sample_payload, "expected_tolerance": tolerance, "actual_max_error": peak_offset_error, "failing_sample": peak_sample, "samples": _nearby_samples(samples, peak_sample, _evidence_context(params))}
	_attach_temporal_evidence("follows", "%s_%s" % [leader_target, follower_target], params, failure_payload)
	return _error("follows_mismatch", "follower exceeded follow tolerance", failure_payload)


func _assert_temporal_never(params: Dictionary) -> Dictionary:
	var forbidden := _forbidden_values(params.get("forbidden", []))
	var state_observation := _temporal_state_observation(params, "assert.temporal.never")
	if not bool(state_observation.get("ok", false)):
		return state_observation
	var observes_state := str(state_observation.get("mode", "node")) != "node"
	var target := str(state_observation.get("label", "")) if observes_state else str(params.get("target", ""))
	if forbidden.is_empty():
		return _error("invalid_forbidden", "never requires at least one forbidden value", {"target": target, "expected": "non-empty forbidden value list", "actual": params.get("forbidden", null)})
	var window := _temporal_window(params, target)
	if not bool(window.get("ok", false)):
		return window
	var node: Node = null
	var observation := {}
	if not observes_state:
		var resolution = _require_target(target)
		if not resolution.get("ok", false):
			return resolution
		node = resolution.get("node")
		observation = _never_observation(params)
		if not bool(observation.get("ok", false)):
			return observation
	var source_name := str(state_observation.get("mode", "")) if observes_state else str(observation.get("source", ""))
	var field_name := str(state_observation.get("path", "")) if observes_state else str(observation.get("field", ""))
	var observation_payload := _temporal_observation_payload(state_observation)
	var samples: Array = []
	while _temporal_should_sample(window):
		await _advance_temporal_sample(window)
		var value_result := _read_temporal_state_value(state_observation, "assert.temporal.never") if observes_state else _read_never_value(node, observation)
		if not bool(value_result.get("ok", false)):
			return value_result
		var value: Variant = value_result.get("value")
		samples.append({"sample": int(window.get("sample", 0)), "frame": Engine.get_process_frames(), "time_msec": Time.get_ticks_msec(), "source": source_name, "field": field_name, "value": value})
		for entry in forbidden:
			if str(value) == str(entry):
				var failing_sample := int(window.get("sample", 0))
				var after_count := min(_evidence_context(params), _remaining_frame_samples(window))
				for _after in range(after_count):
					if not _temporal_should_sample(window):
						break
					await _advance_temporal_sample(window)
					var after_value_result := _read_temporal_state_value(state_observation, "assert.temporal.never") if observes_state else _read_never_value(node, observation)
					if not bool(after_value_result.get("ok", false)):
						return after_value_result
					samples.append({"sample": int(window.get("sample", 0)), "frame": Engine.get_process_frames(), "time_msec": Time.get_ticks_msec(), "source": source_name, "field": field_name, "value": after_value_result.get("value")})
				var failing_sample_payload := (samples[failing_sample - 1] as Dictionary).duplicate(true)
				var failure_payload := {"target": target, "source": source_name, "field": field_name, "forbidden": forbidden, "window": _window_payload(window), "expected": {"never": forbidden, "source": source_name, "field": field_name, "window": _window_payload(window)}, "actual": {"value": value, "sample": failing_sample}, "sample": failing_sample_payload, "failing_sample": failing_sample, "samples": _nearby_samples(samples, failing_sample, _evidence_context(params))}
				failure_payload.merge(observation_payload)
				_attach_temporal_evidence("never", target, params, failure_payload)
				return _error("never_mismatch", "target entered a forbidden value", failure_payload)
	var success_payload := {"target": target, "source": source_name, "field": field_name, "forbidden": forbidden, "window": _window_payload(window), "failing_sample": null}
	success_payload.merge(observation_payload)
	return _ok(success_payload)


func _assert_temporal_within_limits(params: Dictionary) -> Dictionary:
	var min_value := float(params.get("min", 0.0))
	var max_value := float(params.get("max", 0.0))
	var observation := _temporal_state_observation(params, "assert.temporal.within_limits")
	if not bool(observation.get("ok", false)):
		return observation
	var observes_state := str(observation.get("mode", "node")) != "node"
	var target := str(observation.get("label", "")) if observes_state else str(params.get("target", ""))
	var field := str(params.get("path", "")) if observes_state else str(params.get("property", params.get("field", "position_delta")))
	var window := _temporal_window(params, target)
	if not bool(window.get("ok", false)):
		return window
	if min_value > max_value:
		return _error("invalid_bounds", "within_limits min must be less than or equal to max", {"target": target, "expected": "min <= max", "actual": {"min": min_value, "max": max_value}})
	var node: Node = null
	var anchor: Variant = null
	if not observes_state:
		var resolution = _require_target(target)
		if not resolution.get("ok", false):
			return resolution
		node = resolution.get("node")
		anchor = _node_position_value(node)
	var observation_payload := _temporal_observation_payload(observation)
	var samples: Array = []
	while _temporal_should_sample(window):
		await _advance_temporal_sample(window)
		var value_result := _read_numeric_temporal_state_value(observation, "assert.temporal.within_limits") if observes_state else _numeric_temporal_value(node, field, anchor)
		if not bool(value_result.get("ok", false)):
			return value_result
		var value := float(value_result.get("value", 0.0))
		var sample := {"sample": int(window.get("sample", 0)), "frame": Engine.get_process_frames(), "time_msec": Time.get_ticks_msec(), "field": value_result.get("field", field), "value": value}
		samples.append(sample)
		if value < min_value or value > max_value:
			var failing_sample := int(sample.get("sample", 0))
			var failure_payload := {"target": target, "field": value_result.get("field", field), "window": _window_payload(window), "expected": {"min": min_value, "max": max_value, "field": value_result.get("field", field), "window": _window_payload(window)}, "actual": {"value": value, "sample": failing_sample}, "sample": sample.duplicate(true), "failing_sample": failing_sample, "samples": _nearby_samples(samples, failing_sample, _evidence_context(params))}
			failure_payload.merge(observation_payload)
			_attach_temporal_evidence("within_limits", target, params, failure_payload)
			return _error("within_limits_mismatch", "target value left configured limits", failure_payload)
	var success_payload := {"target": target, "field": field, "window": _window_payload(window), "min": min_value, "max": max_value, "failing_sample": null}
	success_payload.merge(observation_payload)
	return _ok(success_payload)


func _assert_temporal_trend(params: Dictionary) -> Dictionary:
	var direction := str(params.get("trend", "")).strip_edges()
	if not TREND_DIRECTIONS.has(direction):
		return _error("invalid_assertion_args", "trend requires a supported direction", {"command": "assert.temporal.trend", "field": "trend", "expected": TREND_DIRECTIONS, "actual": params.get("trend", null)})
	var has_min_delta := params.has("min_delta")
	var min_delta := 0.0
	if has_min_delta:
		var raw_min_delta = params.get("min_delta")
		if not _state_value_is_number(raw_min_delta) or not is_finite(float(raw_min_delta)) or float(raw_min_delta) < 0.0:
			return _error("invalid_assertion_args", "trend min_delta must be a finite non-negative number", {"command": "assert.temporal.trend", "field": "min_delta", "expected": "finite non-negative number", "actual": raw_min_delta})
		min_delta = float(raw_min_delta)
	var tolerance := 0.0
	if params.has("tolerance"):
		var raw_tolerance = params.get("tolerance")
		if not _state_value_is_number(raw_tolerance) or not is_finite(float(raw_tolerance)) or float(raw_tolerance) < 0.0:
			return _error("invalid_assertion_args", "trend tolerance must be a finite non-negative number", {"command": "assert.temporal.trend", "field": "tolerance", "expected": "finite non-negative number", "actual": raw_tolerance})
		tolerance = float(raw_tolerance)
	var observation := _temporal_state_observation(params, "assert.temporal.trend")
	if not bool(observation.get("ok", false)):
		return observation
	var observes_state := str(observation.get("mode", "node")) != "node"
	var target := str(observation.get("label", "")) if observes_state else str(params.get("target", ""))
	var field := str(params.get("path", "")) if observes_state else str(params.get("property", params.get("field", "position_delta")))
	var window := _temporal_window(params, target)
	if not bool(window.get("ok", false)):
		return window
	var node: Node = null
	var anchor: Variant = null
	if not observes_state:
		var resolution = _require_target(target)
		if not resolution.get("ok", false):
			return resolution
		node = resolution.get("node")
		anchor = _node_position_value(node)
	var observation_payload := _temporal_observation_payload(observation)
	var samples: Array = []
	while _temporal_should_sample(window):
		await _advance_temporal_sample(window)
		var value_result := _read_numeric_temporal_state_value(observation, "assert.temporal.trend") if observes_state else _numeric_temporal_value(node, field, anchor)
		if not bool(value_result.get("ok", false)):
			return value_result
		samples.append({"sample": int(window.get("sample", 0)), "frame": Engine.get_process_frames(), "time_msec": Time.get_ticks_msec(), "field": value_result.get("field", field), "value": float(value_result.get("value", 0.0))})
	if samples.size() < 2:
		return _error("invalid_window", "trend requires at least two samples in the bounded window", {"target": target, "expected": "window and sample_every_frames producing >= 2 samples", "actual": samples.size()})
	var first_value := float((samples[0] as Dictionary).get("value", 0.0))
	var last_value := float((samples[samples.size() - 1] as Dictionary).get("value", 0.0))
	var total_delta := last_value - first_value
	var mismatch_type := ""
	var failing_sample := 0
	var failing_step_delta := 0.0
	for index in range(1, samples.size()):
		var step_delta := float((samples[index] as Dictionary).get("value", 0.0)) - float((samples[index - 1] as Dictionary).get("value", 0.0))
		var step_ok := true
		match direction:
			"increasing":
				step_ok = step_delta >= -tolerance
			"decreasing":
				step_ok = step_delta <= tolerance
			"non_decreasing":
				step_ok = step_delta >= 0.0
			"non_increasing":
				step_ok = step_delta <= 0.0
		if not step_ok:
			mismatch_type = "sample_direction_mismatch"
			failing_sample = int((samples[index] as Dictionary).get("sample", 0))
			failing_step_delta = step_delta
			break
	if mismatch_type == "":
		if direction == "increasing" and total_delta <= 0.0:
			mismatch_type = "total_delta_sign_mismatch"
		elif direction == "decreasing" and total_delta >= 0.0:
			mismatch_type = "total_delta_sign_mismatch"
	if mismatch_type == "" and has_min_delta:
		var directed_delta := total_delta if direction == "increasing" or direction == "non_decreasing" else -total_delta
		if directed_delta < min_delta:
			mismatch_type = "min_delta_not_met"
	var series := _bounded_sample_series(samples, TEMPORAL_SAMPLE_ECHO_CAP)
	var payload := {
		"target": target,
		"trend": direction,
		"field": field,
		"window": _window_payload(window),
		"tolerance": tolerance,
		"sample_count": samples.size(),
		"first_value": first_value,
		"last_value": last_value,
		"total_delta": total_delta,
		"samples": series.get("series", []),
		"omitted_samples": int(series.get("omitted", 0)),
	}
	if has_min_delta:
		payload["min_delta"] = min_delta
	payload.merge(observation_payload)
	if mismatch_type == "":
		payload["failing_sample"] = null
		return _ok(payload)
	payload["mismatch_type"] = mismatch_type
	var expected_payload := {"trend": direction, "tolerance": tolerance, "window": _window_payload(window)}
	if has_min_delta:
		expected_payload["min_delta"] = min_delta
	payload["expected"] = expected_payload
	var actual_payload := {"total_delta": total_delta, "first_value": first_value, "last_value": last_value, "mismatch_type": mismatch_type}
	if mismatch_type == "sample_direction_mismatch":
		payload["failing_sample"] = failing_sample
		actual_payload["failing_sample"] = failing_sample
		actual_payload["step_delta"] = failing_step_delta
		payload["sample"] = _nearby_samples(samples, failing_sample, 0)
	payload["actual"] = actual_payload
	_attach_temporal_evidence("trend", target, params, payload)
	return _error("trend_mismatch", "sampled series did not follow the required trend", payload)


func _assert_temporal_frame_time(params: Dictionary) -> Dictionary:
	var bounds := {}
	for bound_name in ["max_avg_ms", "max_p95_ms", "min_fps"]:
		if params.has(bound_name):
			var raw = params.get(bound_name)
			if not _state_value_is_number(raw) or not is_finite(float(raw)) or float(raw) <= 0.0:
				return _error("invalid_assertion_args", "frame_time bounds must be positive numbers", {"command": "assert.temporal.frame_time", "field": bound_name, "expected": "positive number", "actual": raw})
			bounds[bound_name] = float(raw)
	if bounds.is_empty():
		return _error("invalid_assertion_args", "frame_time requires at least one bound", {"command": "assert.temporal.frame_time", "expected": ["max_avg_ms", "max_p95_ms", "min_fps"], "actual": params.duplicate(true)})
	# frame_time measures wall-clock frame durations, so deterministic sampling
	# retunes (fixed_delta / max_frame_duration) must never apply to its window.
	var window_params := params.duplicate(true)
	window_params.erase("temporal_sampling")
	var window := _temporal_window(window_params, "frame_time")
	if not bool(window.get("ok", false)):
		return window
	var cadence := max(int(window.get("sample_every_frames", 1)), 1)
	var samples: Array = []
	var frame_times_ms: Array = []
	var last_usec := Time.get_ticks_usec()
	while _temporal_should_sample(window):
		await _advance_temporal_sample(window)
		var now_usec := Time.get_ticks_usec()
		var frame_ms := float(max(now_usec - last_usec, 0)) / 1000.0 / float(cadence)
		last_usec = now_usec
		frame_times_ms.append(frame_ms)
		samples.append({"sample": int(window.get("sample", 0)), "frame": Engine.get_process_frames(), "frame_time_ms": frame_ms})
	if frame_times_ms.is_empty():
		return _error("invalid_window", "frame_time window produced no samples", {"expected": "window producing >= 1 sample", "actual": 0})
	var total_ms := 0.0
	for frame_ms in frame_times_ms:
		total_ms += float(frame_ms)
	var sorted_times := frame_times_ms.duplicate()
	sorted_times.sort()
	var avg_ms := total_ms / float(frame_times_ms.size())
	var p95_index := clampi(int(ceil(0.95 * float(sorted_times.size()))) - 1, 0, sorted_times.size() - 1)
	var p95_ms := float(sorted_times[p95_index])
	var max_ms := float(sorted_times[sorted_times.size() - 1])
	var min_fps_observed := (1000.0 / max_ms) if max_ms > 0.0001 else 1000000.0
	var aggregates := {
		"sample_count": frame_times_ms.size(),
		"avg_ms": avg_ms,
		"p95_ms": p95_ms,
		"max_ms": max_ms,
		"min_fps": min_fps_observed,
	}
	var violations: Array = []
	if bounds.has("max_avg_ms") and avg_ms > float(bounds.get("max_avg_ms")):
		violations.append({"bound": "max_avg_ms", "expected": bounds.get("max_avg_ms"), "actual": avg_ms})
	if bounds.has("max_p95_ms") and p95_ms > float(bounds.get("max_p95_ms")):
		violations.append({"bound": "max_p95_ms", "expected": bounds.get("max_p95_ms"), "actual": p95_ms})
	if bounds.has("min_fps") and min_fps_observed < float(bounds.get("min_fps")):
		violations.append({"bound": "min_fps", "expected": bounds.get("min_fps"), "actual": min_fps_observed})
	var series := _bounded_sample_series(samples, TEMPORAL_SAMPLE_ECHO_CAP)
	var payload := {
		"window": _window_payload(window),
		"sample_every_frames": cadence,
		"bounds": bounds.duplicate(true),
		"aggregates": aggregates,
		"samples": series.get("series", []),
		"omitted_samples": int(series.get("omitted", 0)),
	}
	if violations.is_empty():
		return _ok(payload)
	payload["expected"] = bounds.duplicate(true)
	payload["actual"] = aggregates.duplicate(true)
	payload["violations"] = violations
	_attach_temporal_evidence("frame_time", "frame_time", params, payload)
	return _error("frame_time_mismatch", "frame timing exceeded configured bounds", payload)


func _assert_temporal_distance_changed(params: Dictionary) -> Dictionary:
	var a := str(params.get("a", params.get("target", "")))
	var b := str(params.get("b", params.get("other", "")))
	var min_raw = params.get("min", null)
	if a.strip_edges() == "" or b.strip_edges() == "":
		return _error(
			"invalid_distance_threshold",
			"distance_changed assertion requires non-empty a and b targets",
			{
				"a": a,
				"b": b,
				"expected": "non-empty target locators",
				"actual": {"a": a, "b": b},
			},
		)
	if not params.has("min") or min_raw == null:
		return _error(
			"invalid_distance_threshold",
			"distance_changed assertion requires a min threshold",
			{
				"a": a,
				"b": b,
				"expected": "finite min >= 0",
				"actual": {"min": null},
			},
		)
	if typeof(min_raw) != TYPE_INT and typeof(min_raw) != TYPE_FLOAT:
		return _error(
			"invalid_distance_threshold",
			"distance_changed min threshold must be finite and non-negative",
			{
				"a": a,
				"b": b,
				"expected": "finite min >= 0",
				"actual": {"min": min_raw},
			},
		)
	var min_distance := float(min_raw)
	if not is_finite(min_distance) or min_distance < 0.0:
		return _error(
			"invalid_distance_threshold",
			"distance_changed min threshold must be finite and non-negative",
			{
				"a": a,
				"b": b,
				"expected": "finite min >= 0",
				"actual": {"min": min_distance},
			},
		)

	var window := _temporal_window(params, "%s vs %s" % [a, b])
	if not bool(window.get("ok", false)):
		return window

	var initial_left = _resolve_target_position(a)
	if not bool(initial_left.get("ok", false)):
		return initial_left
	var initial_right = _resolve_target_position(b)
	if not bool(initial_right.get("ok", false)):
		return initial_right
	var coordinate_space := str(initial_left.get("coordinate_space", ""))
	if coordinate_space != str(initial_right.get("coordinate_space", "")):
		return _error(
			"unsupported_coordinate_space",
			"distance assertion requires matching coordinate spaces",
			{
				"mismatch_type": "unsupported_coordinate_space",
				"a": a,
				"b": b,
				"a_resolved": initial_left.get("resolved", ""),
				"b_resolved": initial_right.get("resolved", ""),
				"a_position": initial_left.get("position", {}),
				"b_position": initial_right.get("position", {}),
				"expected": {"coordinate_space": coordinate_space},
				"actual": {"coordinate_space": initial_right.get("coordinate_space", "")},
			},
		)

	var initial_distance := _position_distance(initial_left.get("value", null), initial_right.get("value", null))
	if not is_finite(initial_distance):
		return _error(
			"unsupported_coordinate_space",
			"distance assertion could not compare resolved positions",
			{
				"mismatch_type": "unsupported_coordinate_space",
				"a": a,
				"b": b,
				"a_resolved": initial_left.get("resolved", ""),
				"b_resolved": initial_right.get("resolved", ""),
				"a_position": initial_left.get("position", {}),
				"b_position": initial_right.get("position", {}),
				"expected": {"coordinate_space": coordinate_space},
				"actual": {"coordinate_space": coordinate_space},
			},
		)

	var max_delta := 0.0
	var peak_sample := 0
	var peak_sample_payload := {}
	var latest_distance := initial_distance
	var latest_left_position = initial_left.get("position", {})
	var latest_right_position = initial_right.get("position", {})
	var samples: Array = []
	while _temporal_should_sample(window):
		await _advance_temporal_sample(window)
		var left = _resolve_target_position(a)
		if not bool(left.get("ok", false)):
			return left
		var right = _resolve_target_position(b)
		if not bool(right.get("ok", false)):
			return right
		if str(left.get("coordinate_space", "")) != coordinate_space or str(right.get("coordinate_space", "")) != coordinate_space:
			return _error(
				"unsupported_coordinate_space",
				"distance assertion requires stable matching coordinate spaces",
				{
					"mismatch_type": "unsupported_coordinate_space",
					"a": a,
					"b": b,
					"a_resolved": left.get("resolved", ""),
					"b_resolved": right.get("resolved", ""),
					"a_position": left.get("position", {}),
					"b_position": right.get("position", {}),
					"expected": {"coordinate_space": coordinate_space},
					"actual": {"a_coordinate_space": left.get("coordinate_space", ""), "b_coordinate_space": right.get("coordinate_space", "")},
				},
			)
		var distance := _position_distance(left.get("value", null), right.get("value", null))
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
					"expected": {"coordinate_space": coordinate_space},
					"actual": {"coordinate_space": coordinate_space},
				},
			)
		var delta := absf(distance - initial_distance)
		var sample := {
			"sample": int(window.get("sample", 0)),
			"frame": Engine.get_process_frames(),
			"time_msec": Time.get_ticks_msec(),
			"distance": distance,
			"distance_delta": delta,
			"a_position": left.get("position", {}),
			"b_position": right.get("position", {}),
		}
		samples.append(sample)
		latest_distance = distance
		latest_left_position = left.get("position", {})
		latest_right_position = right.get("position", {})
		if delta > max_delta:
			max_delta = delta
			peak_sample = int(sample.get("sample", 0))
			peak_sample_payload = sample.duplicate(true)

	if max_delta >= min_distance:
		return _ok(
			{
				"a": a,
				"b": b,
				"a_resolved": initial_left.get("resolved", ""),
				"b_resolved": initial_right.get("resolved", ""),
				"coordinate_space": coordinate_space,
				"window": _window_payload(window),
				"threshold": {"min": min_distance},
				"initial_distance": initial_distance,
				"final_distance": latest_distance,
				"actual_delta": max_delta,
				"failing_sample": null,
			},
		)

	var failing_sample := peak_sample
	var failing_payload := peak_sample_payload.duplicate(true)
	if failing_sample == 0:
		failing_sample = int(window.get("sample", 0))
		failing_payload = {
			"sample": failing_sample,
			"frame": Engine.get_process_frames(),
			"time_msec": Time.get_ticks_msec(),
			"distance": latest_distance,
			"distance_delta": max_delta,
			"a_position": latest_left_position,
			"b_position": latest_right_position,
		}
	var failure_payload := {
		"mismatch_type": "changed_less_than_min",
		"a": a,
		"b": b,
		"a_resolved": initial_left.get("resolved", ""),
		"b_resolved": initial_right.get("resolved", ""),
		"coordinate_space": coordinate_space,
		"window": _window_payload(window),
		"threshold": {"min": min_distance},
		"expected": {"min_delta": min_distance, "window": _window_payload(window)},
		"actual": {"delta": max_delta, "sample": failing_sample},
		"initial_distance": initial_distance,
		"peak_distance_delta": max_delta,
		"final_distance": latest_distance,
		"sample": failing_payload,
		"failing_sample": failing_sample,
		"samples": _nearby_samples(samples, failing_sample, _evidence_context(params)),
	}
	_attach_temporal_evidence("distance_changed", "%s_%s" % [a, b], params, failure_payload)
	return _error("distance_changed_mismatch", "distance changed less than the configured minimum", failure_payload)


func _assert_visible_pixels(params: Dictionary) -> Dictionary:
	var validation := _validate_visual_pixel_params(params, "assert.visible_pixels", true)
	if not bool(validation.get("ok", false)):
		return validation
	var target := str(validation.get("target", ""))
	var probe := await _visual_probe(target, params, "assert.visible_pixels")
	if not bool(probe.get("ok", false)):
		return probe
	var ratio := float(probe.get("visible_pixel_ratio", 0.0))
	var count := int(probe.get("visible_pixel_count", 0))
	var ratio_passes := true
	if bool(validation.get("has_min_ratio", false)):
		ratio_passes = ratio >= float(validation.get("min_ratio", 0.0))
	var count_passes := true
	if bool(validation.get("has_min_count", false)):
		count_passes = count >= int(validation.get("min_count", 0))
	var threshold: Dictionary = validation.get("threshold", {})
	if ratio_passes and count_passes:
		var payload: Dictionary = probe.duplicate(true)
		payload["threshold"] = threshold
		return _ok(payload)
	var fail_payload: Dictionary = probe.duplicate(true)
	fail_payload["threshold"] = threshold
	fail_payload["reason"] = "visible_pixel_count_below_threshold" if not count_passes else "visible_pixel_ratio_below_threshold"
	return _error("visible_pixels_mismatch", "target rendered contribution was below threshold", fail_payload)


func _assert_not_occluded(params: Dictionary) -> Dictionary:
	var validation := _validate_visual_pixel_params(params, "assert.not_occluded", false)
	if not bool(validation.get("ok", false)):
		return validation
	var occluder := str(params.get("occluder", "")).strip_edges()
	if params.has("occluder") and occluder == "":
		return _error("invalid_assertion_args", "not_occluded occluder must be non-empty when provided", {"command": "assert.not_occluded", "target": validation.get("target", ""), "snapshot": validation.get("snapshot", ""), "field": "occluder", "actual": params.get("occluder", null)})
	var probe := await _visual_probe(str(validation.get("target", "")), params, "assert.not_occluded")
	if not bool(probe.get("ok", false)):
		return probe
	var threshold: Dictionary = validation.get("threshold", {})
	if not bool(validation.get("has_min_ratio", false)):
		threshold["min_ratio"] = 0.0001
	var ratio_threshold := float(threshold.get("min_ratio", 0.0001))
	var ratio := float(probe.get("visible_pixel_ratio", 0.0))
	var count := int(probe.get("visible_pixel_count", 0))
	var ratio_passes := ratio >= ratio_threshold
	var count_passes := true
	if bool(validation.get("has_min_count", false)):
		count_passes = count >= int(validation.get("min_count", 0))
	if ratio_passes and count_passes:
		var pass_payload: Dictionary = probe.duplicate(true)
		pass_payload["threshold"] = threshold
		pass_payload["occluder"] = occluder
		return _ok(pass_payload)
	var fail_payload: Dictionary = probe.duplicate(true)
	fail_payload["threshold"] = threshold
	fail_payload["reason"] = "target_visible_pixel_count_below_threshold" if not count_passes else "target_has_no_or_low_visible_contribution"
	fail_payload["occluder"] = occluder
	return _error("not_occluded_mismatch", "target appears fully occluded", fail_payload)


func _assert_surface_probe(params: Dictionary) -> Dictionary:
	if params.has("probe_id"):
		return _assert_named_surface_probe(params)
	var target := str(params.get("target", ""))
	var target_resolution := _require_target(target)
	if not bool(target_resolution.get("ok", false)):
		return target_resolution
	var point_resolution := _resolve_target_position(target)
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
	return _ok({"command": "assert.surface_probe", "target": target, "resolved": target_resolution.get("resolved", ""), "coordinate_space": coordinate_space, "provider": provider.get("provider", ""), "provider_method": provider.get("provider_method", ""), "point_count": sample_points.size(), "points": sample_points})


func _assert_named_surface_probe(params: Dictionary) -> Dictionary:
	var provider_target := str(params.get("provider", params.get("target", "")))
	var probe_id := str(params.get("probe_id", ""))
	if provider_target == "" or probe_id == "":
		return _surface_failure("invalid_assertion_args", "named surface probe requires provider and probe_id", params, {"expected": "provider and probe_id", "actual": params})
	var provider_resolution := _require_target(provider_target)
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
	return _ok({"command": "assert.surface_probe", "provider": provider_target, "resolved": provider_resolution.get("resolved", ""), "probe_id": probe_id, "actual": probe_result})


func _assert_surface_matches(params: Dictionary) -> Dictionary:
	if _surface_match_temporal_requested(params):
		return await _assert_surface_matches_temporal(params)
	return _assert_surface_matches_once(params)


func _assert_surface_matches_once(params: Dictionary) -> Dictionary:
	var target := str(params.get("target", ""))
	var target_resolution := _require_target(target)
	if not bool(target_resolution.get("ok", false)):
		return target_resolution
	var expected_label := str(params.get("surface", params.get("expected", "")))
	if not params.has("provider") and not params.has("provider_target") and not params.has("provider_method"):
		var actor_only_result := _actor_surface_label(target_resolution.get("node"), params)
		if not bool(actor_only_result.get("ok", false)):
			return actor_only_result
		var actor_only_label := str(actor_only_result.get("label", ""))
		if expected_label == "" or actor_only_label == expected_label:
			return _ok({"command": "assert.surface_matches", "target": target, "resolved": target_resolution.get("resolved", ""), "expected": expected_label, "actual": actor_only_label, "provider": "actor"})
		return _surface_failure("surface_mismatch", "actor surface does not match expected surface", params, {"command": "assert.surface_matches", "target": target, "resolved": target_resolution.get("resolved", ""), "expected": expected_label, "actual": actor_only_label, "provider": "actor"})
	var point_resolution := _resolve_target_position(target)
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
		return _ok({"command": "assert.surface_matches", "target": target, "resolved": target_resolution.get("resolved", ""), "coordinate_space": coordinate_space, "point": world_point, "expected": world_label, "actual": actor_label, "provider": provider.get("provider", ""), "provider_method": provider.get("provider_method", "")})
	return _surface_failure("surface_mismatch", "actor surface does not match sampled world surface", params, {"command": "assert.surface_matches", "target": target, "resolved": target_resolution.get("resolved", ""), "point": world_point, "coordinate_space": coordinate_space, "expected": world_label, "actual": actor_label, "provider": provider.get("provider", ""), "provider_method": provider.get("provider_method", "")})


func _surface_match_temporal_requested(params: Dictionary) -> bool:
	return params.has("window_frames") or params.has("window_seconds") or params.has("sample_every_frames") or params.has("dwell_frames") or params.has("dwell_seconds")


func _assert_surface_matches_temporal(params: Dictionary) -> Dictionary:
	var window_frames := _temporal_frame_count(params.get("window_frames", null), params.get("window_seconds", null), 1)
	var sample_every := max(1, int(params.get("sample_every_frames", 1)))
	var dwell_frames := _temporal_frame_count(params.get("dwell_frames", null), params.get("dwell_seconds", null), 1)
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
				return _ok(ok_data)
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


func _assert_renders_above(params: Dictionary) -> Dictionary:
	return await _assert_render_order(params, true)


func _assert_renders_below(params: Dictionary) -> Dictionary:
	return await _assert_render_order(params, false)


func _assert_render_order(params: Dictionary, expect_above: bool) -> Dictionary:
	var command_name := "assert.renders_above" if expect_above else "assert.renders_below"
	var validation := _validate_visual_order_params(params, command_name)
	if not bool(validation.get("ok", false)):
		return validation
	var a := str(validation.get("target", ""))
	var b := str(validation.get("other", ""))
	var snapshot := _visual_snapshot(params)
	var left = _require_target(a)
	if not bool(left.get("ok", false)):
		return _visual_error_with_context(left, a, params, command_name)
	var right = _require_target(b)
	if not bool(right.get("ok", false)):
		return _visual_error_with_context(right, b, params, command_name)
	var node_a: Node = left.get("node")
	var node_b: Node = right.get("node")
	var base_payload := {
		"command": command_name,
		"target": a,
		"other": b,
		"snapshot": snapshot,
		"a": a,
		"b": b,
		"a_resolved": left.get("resolved", ""),
		"b_resolved": right.get("resolved", ""),
		"expected": "target_above_other" if expect_above else "target_below_other",
	}
	var requested_evidence := str(params.get("evidence", "auto")).strip_edges()
	if requested_evidence == "":
		requested_evidence = "auto"
	if not ["auto", "canvas_order", "pixel_overlap"].has(requested_evidence):
		var evidence_payload := base_payload.duplicate(true)
		evidence_payload["actual"] = requested_evidence
		evidence_payload["expected"] = ["auto", "canvas_order", "pixel_overlap"]
		return _error("invalid_assertion_args", "render-order evidence must be auto, canvas_order, or pixel_overlap", evidence_payload)
	var require_overlap := bool(params.get("require_overlap", false)) or requested_evidence == "pixel_overlap"
	if not (node_a is CanvasItem and node_b is CanvasItem):
		var canvas_payload := base_payload.duplicate(true)
		canvas_payload["requested_space"] = "viewport_2d"
		canvas_payload["actual_space"] = ""
		canvas_payload["coordinate_space"] = ""
		canvas_payload["rect_source"] = "none"
		canvas_payload["missing_evidence"] = "viewport_rect"
		canvas_payload["actual"] = {
			"target_class": node_a.get_class(),
			"other_class": node_b.get_class(),
			"target_is_canvas_item": node_a is CanvasItem,
			"other_is_canvas_item": node_b is CanvasItem,
		}
		canvas_payload["expected"] = "CanvasItem targets with measurable viewport-space geometry"
		canvas_payload["remediation"] = _visual_rect_remediation()
		canvas_payload["artifacts"] = {}
		return _error("unsupported_visual_assertion", "render-order assertions require CanvasItem targets", canvas_payload)
	var rect_evidence_a := _visual_rect_for_node(node_a, command_name, a, params, left)
	if not bool(rect_evidence_a.get("ok", false)):
		return rect_evidence_a
	var rect_evidence_b := _visual_rect_for_node(node_b, command_name, b, params, right)
	if not bool(rect_evidence_b.get("ok", false)):
		return rect_evidence_b
	var rect_a: Rect2 = rect_evidence_a.get("rect")
	var rect_b: Rect2 = rect_evidence_b.get("rect")
	# The geometry evidence (target_rect/other_rect below) stays in canvas space,
	# but pixel sampling and overlays index the window-resolution screenshot, so
	# convert to screenshot space before touching image pixels.
	var canvas_to_screenshot := _canvas_to_screenshot_transform()
	var rect_a_screenshot: Rect2 = canvas_to_screenshot * rect_a
	var rect_b_screenshot: Rect2 = canvas_to_screenshot * rect_b
	var overlaps := rect_a.intersects(rect_b)
	if require_overlap and not overlaps:
		var no_overlap_payload := base_payload.duplicate(true)
		no_overlap_payload["target_rect"] = _rect_payload(rect_a)
		no_overlap_payload["other_rect"] = _rect_payload(rect_b)
		no_overlap_payload["target_rect_source"] = rect_evidence_a.get("rect_source", "")
		no_overlap_payload["other_rect_source"] = rect_evidence_b.get("rect_source", "")
		no_overlap_payload["target_missing_evidence"] = rect_evidence_a.get("missing_evidence", "")
		no_overlap_payload["other_missing_evidence"] = rect_evidence_b.get("missing_evidence", "")
		no_overlap_payload["requested_space"] = "viewport_2d"
		no_overlap_payload["target_actual_space"] = rect_evidence_a.get("coordinate_space", "")
		no_overlap_payload["other_actual_space"] = rect_evidence_b.get("coordinate_space", "")
		no_overlap_payload["actual"] = {"target_rect": _rect_payload(rect_a), "other_rect": _rect_payload(rect_b), "overlap": false}
		no_overlap_payload["expected"] = "overlapping viewport-space target and other rects"
		no_overlap_payload["remediation"] = "use targets with overlapping rendered viewport rects or set require_overlap false when canvas ordering is sufficient"
		no_overlap_payload["artifacts"] = {}
		no_overlap_payload["evidence"] = requested_evidence
		return _error("unsupported_visual_assertion", "render-order assertions require overlapping CanvasItem targets", no_overlap_payload)
	var order := _snapshot_pixel_order_evidence(node_a as CanvasItem, node_b as CanvasItem, rect_a_screenshot, rect_b_screenshot, params)
	if not bool(order.get("ok", false)):
		order = _canvas_order_evidence(node_a as CanvasItem, node_b as CanvasItem)
	var visual_dir := str(params.get("visual_dir", ""))
	var payload := base_payload.duplicate(true)
	payload["target_rect"] = _rect_payload(rect_a)
	payload["other_rect"] = _rect_payload(rect_b)
	payload["overlap_rect"] = _rect_payload(rect_a.intersection(rect_b)) if overlaps else []
	payload["target_rect_source"] = rect_evidence_a.get("rect_source", "")
	payload["other_rect_source"] = rect_evidence_b.get("rect_source", "")
	payload["target_coordinate_space"] = rect_evidence_a.get("coordinate_space", "")
	payload["other_coordinate_space"] = rect_evidence_b.get("coordinate_space", "")
	payload["target_missing_evidence"] = rect_evidence_a.get("missing_evidence", "")
	payload["other_missing_evidence"] = rect_evidence_b.get("missing_evidence", "")
	payload["evidence_mode"] = "canvas_order"
	payload["requested_evidence"] = requested_evidence
	payload["require_overlap"] = require_overlap
	payload["order_evidence"] = order
	payload["artifacts"] = {}
	if visual_dir != "" and not visual_dir.contains("qa/baselines"):
		var dir_err := DirAccess.make_dir_recursive_absolute(visual_dir)
		if dir_err == OK:
			var safe_name := _sanitize_visual_name("%s_%s_%s" % [command_name, a, b])
			var screenshot_path := "%s/%s_order.png" % [visual_dir, safe_name]
			var overlay_path := "%s/%s_order_overlay.png" % [visual_dir, safe_name]
			var capture := await _capture_visual_image(screenshot_path)
			if bool(capture.get("ok", false)):
				var image: Image = capture.get("image")
				_write_rect_overlay(image, rect_a_screenshot.intersection(rect_b_screenshot) if overlaps else rect_a_screenshot, overlay_path)
				payload["screenshot"] = screenshot_path
				payload["artifacts"] = {"screenshot": screenshot_path, "overlay": overlay_path}
			else:
				payload["capture_warning"] = _visual_capture_warning(capture, visual_dir, screenshot_path)
		else:
			payload["capture_warning"] = {"type": "screenshot_write_failed", "actual": dir_err, "visual_dir": visual_dir}
	if not bool(order.get("ok", false)):
		payload["reason"] = str(order.get("reason", "ambiguous_canvas_order"))
		return _error("unsupported_visual_assertion", "render-order evidence was not conclusive", payload)
	var target_above := bool(order.get("target_above_other", false))
	var passes := target_above if expect_above else not target_above
	if passes:
		return _ok(payload)
	payload["reason"] = "target_rendered_below_other" if expect_above else "target_rendered_above_other"
	return _error("render_order_mismatch", "target render order did not match expectation", payload)


func _validate_visual_pixel_params(params: Dictionary, command_name: String, require_threshold: bool) -> Dictionary:
	var target := str(params.get("target", "")).strip_edges()
	var snapshot := _visual_snapshot(params).strip_edges()
	var base := {"command": command_name, "target": target, "snapshot": snapshot}
	if target == "":
		base["field"] = "target"
		base["actual"] = params.get("target", null)
		return _error("invalid_assertion_args", "visual assertions require a non-empty target", base)
	if snapshot == "":
		base["field"] = "snapshot"
		base["actual"] = params.get("snapshot", null)
		return _error("invalid_assertion_args", "visual assertions require a non-empty snapshot", base)
	var has_min_ratio := params.has("min_ratio") or params.has("threshold")
	var has_min_count := params.has("min_count")
	if require_threshold and not has_min_ratio and not has_min_count:
		base["field"] = "threshold"
		base["actual"] = {"min_ratio": null, "min_count": null}
		return _error("invalid_assertion_args", "visible_pixels requires min_ratio or min_count", base)
	var threshold := {}
	var result := {
		"ok": true,
		"target": target,
		"snapshot": snapshot,
		"has_min_ratio": has_min_ratio,
		"has_min_count": has_min_count,
		"threshold": threshold,
	}
	if has_min_ratio:
		var ratio_value = params.get("min_ratio", params.get("threshold", null))
		if typeof(ratio_value) != TYPE_FLOAT and typeof(ratio_value) != TYPE_INT:
			base["field"] = "min_ratio"
			base["actual"] = ratio_value
			return _error("invalid_assertion_args", "visual assertion min_ratio must be a number between 0.0 and 1.0", base)
		var ratio := float(ratio_value)
		if is_nan(ratio) or is_inf(ratio) or ratio < 0.0 or ratio > 1.0:
			base["field"] = "min_ratio"
			base["actual"] = ratio_value
			return _error("invalid_assertion_args", "visual assertion min_ratio must be a number between 0.0 and 1.0", base)
		threshold["min_ratio"] = ratio
		result["min_ratio"] = ratio
	if has_min_count:
		var count_value = params.get("min_count", null)
		if (typeof(count_value) != TYPE_INT and typeof(count_value) != TYPE_FLOAT) or int(count_value) < 0 or float(count_value) != float(int(count_value)):
			base["field"] = "min_count"
			base["actual"] = count_value
			return _error("invalid_assertion_args", "visual assertion min_count must be a non-negative integer", base)
		threshold["min_count"] = int(count_value)
		result["min_count"] = int(count_value)
	return result


func _validate_visual_order_params(params: Dictionary, command_name: String) -> Dictionary:
	var target := str(params.get("a", params.get("target", ""))).strip_edges()
	var other := str(params.get("b", params.get("other", ""))).strip_edges()
	if other == "":
		other = str(params.get("peer", "")).strip_edges()
	var snapshot := _visual_snapshot(params).strip_edges()
	var base := {"command": command_name, "target": target, "other": other, "snapshot": snapshot}
	if target == "":
		base["field"] = "target"
		base["actual"] = params.get("target", params.get("a", null))
		return _error("invalid_assertion_args", "render-order assertions require a non-empty target", base)
	if other == "":
		base["field"] = "other"
		base["actual"] = params.get("other", params.get("peer", params.get("b", null)))
		return _error("invalid_assertion_args", "render-order assertions require a non-empty other target", base)
	if snapshot == "":
		base["field"] = "snapshot"
		base["actual"] = params.get("snapshot", null)
		return _error("invalid_assertion_args", "render-order assertions require a non-empty snapshot", base)
	return {"ok": true, "target": target, "other": other, "snapshot": snapshot}


func _visual_probe(target: String, params: Dictionary, command_name: String) -> Dictionary:
	var resolution = _require_target(target)
	if not bool(resolution.get("ok", false)):
		return _visual_error_with_context(resolution, target, params, command_name)
	var node: Node = resolution.get("node")
	var rect_result := _visual_rect_for_node(node, command_name, target, params, resolution)
	if not bool(rect_result.get("ok", false)):
		return rect_result
	var rect: Rect2 = rect_result.get("rect")
	if rect.size.x <= 0.0 or rect.size.y <= 0.0:
		return _error(
			"unsupported_visual_assertion",
			"target did not expose a measurable viewport rect",
			_visual_probe_failure_payload(
				command_name,
				target,
				params,
				resolution,
				rect_result,
				rect_result.get("missing_evidence", "viewport_rect"),
				_rect_payload(rect),
				"viewport-space rect with positive width and height",
				rect_result.get("remediation", _visual_rect_remediation()),
			),
		)
	var visual_dir := str(params.get("visual_dir", ""))
	if visual_dir == "" or visual_dir.contains("qa/baselines"):
		return _error("invalid_visual_dir", "visual assertions require safe visual_dir outside qa/baselines", {"command": command_name, "target": target, "snapshot": _visual_snapshot(params), "visual_dir": visual_dir})
	var dir_err := DirAccess.make_dir_recursive_absolute(visual_dir)
	if dir_err != OK:
		return _error("screenshot_write_failed", "visual artifact directory could not be created", {"command": command_name, "target": target, "snapshot": _visual_snapshot(params), "visual_dir": visual_dir, "actual": dir_err})
	var safe_name := _sanitize_visual_name("%s_%s_%s" % [command_name, _visual_snapshot(params), target])
	var before_path := "%s/%s_original.png" % [visual_dir, safe_name]
	var hidden_path := "%s/%s_hidden.png" % [visual_dir, safe_name]
	var visibility_state := _visual_visibility_state(node)
	if not bool(visibility_state.get("ok", false)):
		return _error(
			"unsupported_visual_assertion",
			"visual assertions require CanvasItem visibility or qa_set_visual_visible",
			_visual_probe_failure_payload(
				command_name,
				target,
				params,
				resolution,
				rect_result,
				visibility_state.get("missing_evidence", "visibility_control"),
				visibility_state.get("actual", {}),
				"CanvasItem.visible or qa_set_visual_visible(visible) support",
				visibility_state.get("remediation", _visual_visibility_remediation()),
			),
		)
	var before_capture := await _capture_visual_image(before_path)
	if not bool(before_capture.get("ok", false)):
		return _visual_capture_error(target, resolution, visual_dir, before_capture, params, command_name)
	if not _safe_set_visual_visible(node, false):
		return _target_freed_visual_probe_error(target, resolution, params, command_name, {"original": before_path})
	var hidden_capture := await _capture_visual_image(hidden_path)
	if not _safe_restore_visual_visible(node, visibility_state):
		return _target_freed_visual_probe_error(target, resolution, params, command_name, {"original": before_path, "target_hidden": hidden_path})
	await _runtime.process_frames(1)
	if not bool(hidden_capture.get("ok", false)):
		return _visual_capture_error(target, resolution, visual_dir, hidden_capture, params, command_name)
	var before_img: Image = before_capture.get("image")
	var hidden_img: Image = hidden_capture.get("image")
	if before_img.get_width() != hidden_img.get_width() or before_img.get_height() != hidden_img.get_height():
		return _error("screenshot_capture_failed", "visual assertions require matching real viewport screenshot image evidence", {"command": command_name, "target": target, "snapshot": _visual_snapshot(params), "resolved": resolution.get("resolved", ""), "resolved_path": resolution.get("resolved", ""), "visual_dir": visual_dir, "display_server": DisplayServer.get_name(), "headless": OS.has_feature("headless"), "attempted_screenshot_path": hidden_path, "capture_details": {"original_size": [before_img.get_width(), before_img.get_height()], "hidden_size": [hidden_img.get_width(), hidden_img.get_height()]}, "actual": {"original_size": [before_img.get_width(), before_img.get_height()], "hidden_size": [hidden_img.get_width(), hidden_img.get_height()]}, "expected": "matching real rendered viewport pixel screenshots", "remediation": _visual_capture_remediation(), "artifacts": {"original": before_path, "target_hidden": hidden_path}})
	# Target rects come from get_global_rect()/qa_visual_rect() in the 2D canvas
	# coordinate space, but screenshots are captured at the OS window resolution.
	# When content scaling maps canvas != window, canvas coords index the wrong
	# screenshot pixels, so convert the rect into screenshot space before sampling.
	var canvas_to_screenshot := _canvas_to_screenshot_transform()
	var image_size := Vector2(before_img.get_width(), before_img.get_height())
	var geometry_error := _screenshot_geometry_mismatch_error(canvas_to_screenshot, image_size, command_name, target, params, resolution, before_path, hidden_path)
	if not geometry_error.is_empty():
		return geometry_error
	var rect_screenshot: Rect2 = canvas_to_screenshot * rect
	var metrics := _count_changed_pixels(before_img, hidden_img, rect_screenshot)
	var mask_path := "%s/%s_mask.png" % [visual_dir, safe_name]
	var overlay_path := "%s/%s_overlay.png" % [visual_dir, safe_name]
	_write_visual_diagnostics(before_img, hidden_img, rect_screenshot, mask_path, overlay_path)
	return {
		"ok": true,
		"command": command_name,
		"target": target,
		"snapshot": _visual_snapshot(params),
		"resolved": resolution.get("resolved", ""),
		"visible_pixel_count": metrics.get("changed", 0),
		"visible_pixel_ratio": metrics.get("ratio", 0.0),
		"evidence_mode": metrics.get("mode", "pixel_delta"),
		"target_rect": _rect_payload(rect),
		"target_rect_screenshot": _rect_payload(rect_screenshot),
		"canvas_to_screenshot": _canvas_to_screenshot_payload(canvas_to_screenshot),
		"screenshot_size": [before_img.get_width(), before_img.get_height()],
		"rect_source": rect_result.get("rect_source", ""),
		"coordinate_space": rect_result.get("coordinate_space", "viewport_2d"),
		"missing_evidence": rect_result.get("missing_evidence", ""),
		"remediation": rect_result.get("remediation", ""),
		"screenshot": before_path,
		"artifacts": {"original": before_path, "target_hidden": hidden_path, "mask": mask_path, "overlay": overlay_path},
	}


func _count_changed_pixels(before: Image, hidden: Image, rect: Rect2) -> Dictionary:
	var x0 := maxi(0, int(floor(rect.position.x)))
	var y0 := maxi(0, int(floor(rect.position.y)))
	var x1 := mini(before.get_width(), int(ceil(rect.end.x)))
	var y1 := mini(before.get_height(), int(ceil(rect.end.y)))
	var total := maxi((x1 - x0) * (y1 - y0), 1)
	var changed := 0
	for y in range(y0, y1):
		for x in range(x0, x1):
			if _pixels_differ(before.get_pixel(x, y), hidden.get_pixel(x, y)):
				changed += 1
	return {"changed": changed, "ratio": float(changed) / float(total), "mode": "pixel_delta"}


func _visual_rect_for_node(node: Node, command_name: String, target: String, params: Dictionary, resolution: Dictionary) -> Dictionary:
	var context := {"command": command_name, "target": target, "snapshot": _visual_snapshot(params), "resolved": resolution.get("resolved", ""), "resolved_path": resolution.get("resolved", ""), "requested_space": "viewport_2d", "actual_space": "", "coordinate_space": "", "artifacts": {}}
	if node == null:
		context["missing_evidence"] = "resolved_target"
		context["actual"] = null
		context["expected"] = "resolved live target node"
		context["remediation"] = "resolve the assertion target to a live node before requesting viewport pixel evidence"
		return _error("unsupported_visual_assertion", "visual assertions require a resolved target node", context)
	var snapshot_evidence := QaSnapshot.visual_evidence_for_node(node)
	var snapshot_rect := _rect_from_visual_evidence(snapshot_evidence)
	if bool(snapshot_rect.get("ok", false)):
		return _normalized_visual_rect_evidence(snapshot_rect.get("rect"), snapshot_evidence.get("rect_source", ""), snapshot_evidence.get("coordinate_space", "viewport_2d"))
	if str(snapshot_evidence.get("unsupported_reason", "")) == "unsupported_coordinate_space":
		var space_payload := context.duplicate(true)
		space_payload["requested_space"] = "viewport_2d"
		space_payload["actual_space"] = snapshot_evidence.get("coordinate_space", "")
		space_payload["coordinate_space"] = snapshot_evidence.get("coordinate_space", "")
		space_payload["actual"] = snapshot_evidence.get("actual", null)
		space_payload["expected"] = "viewport_2d"
		space_payload["missing_evidence"] = "coordinate_conversion"
		space_payload["remediation"] = _visual_rect_remediation()
		return _error("unsupported_coordinate_space", "qa_visual_rect must return viewport_2d evidence", space_payload)
	context["rect_source"] = snapshot_evidence.get("rect_source", "")
	context["coordinate_space"] = snapshot_evidence.get("coordinate_space", "")
	context["actual_space"] = snapshot_evidence.get("coordinate_space", "")
	context["missing_evidence"] = snapshot_evidence.get("unsupported_reason", "viewport_rect")
	context["actual"] = snapshot_evidence.get("actual", {"class": node.get_class(), "has_qa_visual_rect": node.has_method("qa_visual_rect") or node.has_meta("qa_visual_rect")})
	context["expected"] = "measurable viewport-space geometry"
	context["remediation"] = _visual_rect_remediation()
	return _error("unsupported_visual_assertion", "target did not expose measurable viewport geometry", context)


func _explicit_visual_rect(node: Node) -> Dictionary:
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
	parsed["rect_source"] = "qa_visual_rect"
	return parsed


func _parse_visual_rect(value: Variant) -> Dictionary:
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
			return {"ok": false, "missing": false, "error_type": "unsupported_coordinate_space", "coordinate_space": coordinate_space, "actual_space": coordinate_space}
		if rect.has("rect"):
			var parsed_rect := _parse_visual_rect(rect.get("rect"))
			if not bool(parsed_rect.get("ok", false)):
				parsed_rect["coordinate_space"] = coordinate_space
				parsed_rect["actual_space"] = parsed_rect.get("actual_space", coordinate_space)
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
	return {"ok": false, "missing": false}


func _visual_visibility_state(node: Node) -> Dictionary:
	if node == null or not is_instance_valid(node):
		return {"ok": false, "reason": "target_freed", "missing_evidence": "visibility_control", "actual": "target_freed", "remediation": _visual_visibility_remediation()}
	if node is CanvasItem:
		return {"ok": true, "mode": "canvas_item", "previous": bool((node as CanvasItem).visible)}
	if node.has_method("qa_set_visual_visible"):
		var previous := true
		if node.has_method("qa_visual_visible"):
			previous = bool(node.call("qa_visual_visible"))
		elif node.has_meta("qa_visual_visible"):
			previous = bool(node.get_meta("qa_visual_visible"))
		return {"ok": true, "mode": "qa_set_visual_visible", "previous": previous}
	return {"ok": false, "reason": "missing_visibility_control", "missing_evidence": "visibility_control", "actual": {"class": node.get_class(), "is_canvas_item": false, "has_qa_set_visual_visible": false}, "remediation": _visual_visibility_remediation()}


func _visual_visibility_remediation() -> String:
	return "make the visual target a CanvasItem or expose qa_set_visual_visible(visible) with optional qa_visual_visible evidence so pixel-delta assertions can hide and restore it"


func _set_visual_visible(node: Node, visible: bool) -> void:
	if node is CanvasItem:
		(node as CanvasItem).visible = visible
	elif node.has_method("qa_set_visual_visible"):
		node.call("qa_set_visual_visible", visible)


func _restore_visual_visible(node: Node, visibility_state: Dictionary) -> void:
	_set_visual_visible(node, bool(visibility_state.get("previous", true)))


func _safe_set_visual_visible(node: Node, visible: bool) -> bool:
	if node == null or not is_instance_valid(node):
		return false
	_set_visual_visible(node, visible)
	return is_instance_valid(node)


func _safe_restore_visual_visible(node: Node, visibility_state: Dictionary) -> bool:
	if node == null or not is_instance_valid(node):
		return false
	_restore_visual_visible(node, visibility_state)
	return is_instance_valid(node)


func _capture_visual_image(path: String) -> Dictionary:
	var capture_result: Dictionary = await QaSnapshot.capture_screenshot(_root_window, path, false)
	if not bool(capture_result.get("ok", false)):
		return capture_result
	if not FileAccess.file_exists(path):
		return _visual_image_capture_failure(path, "visual screenshot artifact was not written", "missing_screenshot_file", null, capture_result)
	var image := Image.load_from_file(path)
	if image == null or image.get_width() <= 0 or image.get_height() <= 0:
		return _visual_image_capture_failure(path, "visual screenshot artifact could not be loaded", "non_loadable_screenshot_image", null, capture_result)
	if image.get_format() != Image.FORMAT_RGBA8:
		image.convert(Image.FORMAT_RGBA8)
	if OS.has_feature("headless") or DisplayServer.get_name() == "headless":
		return _visual_image_capture_failure(path, "visual assertions cannot use headless diagnostic screenshot output", "headless_screenshot_unsupported", image, capture_result)
	if _visual_image_is_low_information(image):
		return _visual_image_capture_failure(path, "visual screenshot did not contain real rendered viewport pixels", "blank_or_fallback_screenshot_image", image, capture_result)
	return {"ok": true, "image": image, "artifacts": capture_result.get("artifacts", {})}


func _visual_image_capture_failure(path: String, message: String, reason: String, image: Image, capture_result: Dictionary) -> Dictionary:
	var actual := {
		"path": path,
		"reason": reason,
		"display_server": DisplayServer.get_name(),
		"headless": OS.has_feature("headless"),
	}
	if image != null:
		actual["image_size"] = [image.get_width(), image.get_height()]
	return {
		"ok": false,
		"failures": [
			{
				"type": "screenshot_capture_failed",
				"message": message,
				"expected": "real rendered viewport pixels from the active Godot display backend",
				"actual": actual,
				"attempted_screenshot_path": path,
				"capture_reason": reason,
				"display_server": DisplayServer.get_name(),
				"headless": OS.has_feature("headless"),
			},
		],
		"artifacts": capture_result.get("artifacts", {"screenshot": path}),
	}


func _visual_capture_error(target: String, resolution: Dictionary, visual_dir: String, capture: Dictionary, params := {}, command_name := "") -> Dictionary:
	var failures = capture.get("failures", [])
	var failure := {}
	if failures is Array and not failures.is_empty() and failures[0] is Dictionary:
		failure = (failures[0] as Dictionary).duplicate(true)
	var artifacts: Dictionary = capture.get("artifacts", {}) if capture.get("artifacts", {}) is Dictionary else {}
	var attempted_path := str(failure.get("attempted_screenshot_path", artifacts.get("screenshot", "")))
	if attempted_path == "" and failure.get("actual", null) is String:
		attempted_path = str(failure.get("actual"))
	var capture_details := failure.duplicate(true)
	capture_details["capture_result"] = capture.duplicate(true)
	return _error(
		"screenshot_capture_failed",
		str(failure.get("message", "visual assertions require real rendered viewport pixel evidence")),
		{
			"command": str(command_name),
			"target": target,
			"snapshot": _visual_snapshot(params),
			"resolved": resolution.get("resolved", ""),
			"resolved_path": resolution.get("resolved", ""),
			"visual_dir": visual_dir,
			"display_server": DisplayServer.get_name(),
			"headless": OS.has_feature("headless"),
			"attempted_screenshot_path": attempted_path,
			"capture_details": capture_details,
			"actual": failure.get("actual", capture),
			"expected": "real rendered viewport pixels from the active Godot display backend",
			"remediation": _visual_capture_remediation(),
			"artifacts": artifacts,
		},
	)


func _visual_capture_warning(capture: Dictionary, visual_dir: String = "", attempted_screenshot_path: String = "") -> Dictionary:
	var failures = capture.get("failures", [])
	var failure := {}
	if failures is Array and not failures.is_empty() and failures[0] is Dictionary:
		failure = (failures[0] as Dictionary).duplicate(true)
	var artifacts: Dictionary = capture.get("artifacts", {}) if capture.get("artifacts", {}) is Dictionary else {}
	var attempted_path := attempted_screenshot_path
	if attempted_path == "":
		attempted_path = str(failure.get("actual", artifacts.get("screenshot", "")))
	return {
		"type": "screenshot_capture_failed",
		"message": str(failure.get("message", "visual assertions require real rendered viewport pixel evidence")),
		"visual_dir": visual_dir,
		"display_server": DisplayServer.get_name(),
		"headless": OS.has_feature("headless"),
		"attempted_screenshot_path": attempted_path,
		"capture_details": failure if not failure.is_empty() else capture,
		"actual": failure.get("actual", capture),
		"expected": "real rendered viewport pixels from the active Godot display backend",
		"remediation": _visual_capture_remediation(),
		"artifacts": artifacts,
	}


func _visual_capture_remediation() -> String:
	return "visual-pixel assertions require real rendered viewport pixels; run with a display backend that exposes screenshots, or use non-visual state/probe assertions when pixel evidence is unavailable"


func _visual_image_is_low_information(image: Image) -> bool:
	var width := image.get_width()
	var height := image.get_height()
	if width <= 0 or height <= 0:
		return true
	var first := image.get_pixel(0, 0)
	var visible_count := 0
	var different_count := 0
	var step_x := maxi(1, int(floor(float(width) / 32.0)))
	var step_y := maxi(1, int(floor(float(height) / 32.0)))
	for y in range(0, height, step_y):
		for x in range(0, width, step_x):
			var color := image.get_pixel(x, y)
			if color.a > 0.01:
				visible_count += 1
			if _pixels_differ(first, color):
				different_count += 1
	return visible_count == 0 or different_count == 0


func _visual_probe_failure_payload(
	command_name: String,
	target: String,
	params: Dictionary,
	resolution: Dictionary,
	rect_result: Dictionary,
	missing_evidence: String,
	actual: Variant,
	expected: Variant,
	remediation: String,
) -> Dictionary:
	return {
		"command": command_name,
		"target": target,
		"snapshot": _visual_snapshot(params),
		"resolved": resolution.get("resolved", ""),
		"resolved_path": resolution.get("resolved", ""),
		"rect_source": rect_result.get("rect_source", ""),
		"coordinate_space": rect_result.get("coordinate_space", "viewport_2d"),
		"requested_space": "viewport_2d",
		"actual_space": rect_result.get("coordinate_space", "viewport_2d"),
		"missing_evidence": missing_evidence,
		"actual": actual,
		"expected": expected,
		"remediation": remediation,
		"artifacts": {},
	}


func _target_freed_visual_probe_error(target: String, resolution: Dictionary, params: Dictionary, command_name: String, artifacts: Dictionary) -> Dictionary:
	return _error(
		"target_freed_during_visual_probe",
		"visual assertion target was freed while screenshot evidence was being captured",
		{
			"command": command_name,
			"target": target,
			"snapshot": _visual_snapshot(params),
			"resolved": resolution.get("resolved", ""),
			"artifacts": artifacts,
		},
	)


func _pixels_differ(a: Color, b: Color) -> bool:
	return (
		abs(a.r - b.r) > 0.01
		or abs(a.g - b.g) > 0.01
		or abs(a.b - b.b) > 0.01
		or abs(a.a - b.a) > 0.01
	)


func _write_visual_diagnostics(before: Image, hidden: Image, rect: Rect2, mask_path: String, overlay_path: String) -> void:
	var width := before.get_width()
	var height := before.get_height()
	var mask := Image.create(width, height, false, Image.FORMAT_RGBA8)
	var overlay := Image.create(width, height, false, Image.FORMAT_RGBA8)
	overlay.blit_rect(before, Rect2i(0, 0, width, height), Vector2i.ZERO)
	var x0 := maxi(0, int(floor(rect.position.x)))
	var y0 := maxi(0, int(floor(rect.position.y)))
	var x1 := mini(width, int(ceil(rect.end.x)))
	var y1 := mini(height, int(ceil(rect.end.y)))
	for y in range(y0, y1):
		for x in range(x0, x1):
			if _pixels_differ(before.get_pixel(x, y), hidden.get_pixel(x, y)):
				mask.set_pixel(x, y, Color(1.0, 1.0, 1.0, 1.0))
				var base := overlay.get_pixel(x, y)
				overlay.set_pixel(x, y, base.lerp(Color(1.0, 0.0, 0.0, 1.0), 0.55))
	mask.save_png(mask_path)
	overlay.save_png(overlay_path)


func _write_rect_overlay(source: Image, rect: Rect2, overlay_path: String) -> void:
	var width := source.get_width()
	var height := source.get_height()
	var overlay := Image.create(width, height, false, Image.FORMAT_RGBA8)
	overlay.blit_rect(source, Rect2i(0, 0, width, height), Vector2i.ZERO)
	var x0 := maxi(0, int(floor(rect.position.x)))
	var y0 := maxi(0, int(floor(rect.position.y)))
	var x1 := mini(width, int(ceil(rect.end.x)))
	var y1 := mini(height, int(ceil(rect.end.y)))
	for y in range(y0, y1):
		for x in range(x0, x1):
			if x == x0 or x == x1 - 1 or y == y0 or y == y1 - 1:
				overlay.set_pixel(x, y, Color(1.0, 0.0, 0.0, 1.0))
			else:
				var base := overlay.get_pixel(x, y)
				overlay.set_pixel(x, y, base.lerp(Color(1.0, 0.0, 0.0, 1.0), 0.25))
	overlay.save_png(overlay_path)


func _canvas_order_evidence(target: CanvasItem, other: CanvasItem) -> Dictionary:
	if not target.is_visible_in_tree() or not other.is_visible_in_tree():
		return {"ok": false, "reason": "targets_must_be_visible_in_tree"}
	var target_layer := _canvas_layer_index(target)
	var other_layer := _canvas_layer_index(other)
	if target_layer != other_layer:
		return {
			"ok": true,
			"mode": "canvas_layer",
			"target_above_other": target_layer > other_layer,
			"target_layer": target_layer,
			"other_layer": other_layer,
		}
	var target_z := _effective_canvas_z(target)
	var other_z := _effective_canvas_z(other)
	if target_z != other_z:
		return {
			"ok": true,
			"mode": "z_index",
			"target_above_other": target_z > other_z,
			"target_z": target_z,
			"other_z": other_z,
			"target_z_as_relative": bool(target.z_as_relative),
			"other_z_as_relative": bool(other.z_as_relative),
			"target_layer": target_layer,
			"other_layer": other_layer,
		}
	if target.get_parent() != null and target.get_parent() == other.get_parent():
		return {
			"ok": true,
			"mode": "direct_sibling_index",
			"target_above_other": target.get_index() > other.get_index(),
			"target_index": target.get_index(),
			"other_index": other.get_index(),
			"target_z": target_z,
			"other_z": other_z,
			"target_z_as_relative": bool(target.z_as_relative),
			"other_z_as_relative": bool(other.z_as_relative),
			"target_layer": target_layer,
			"other_layer": other_layer,
		}
	return {
		"ok": false,
		"reason": "same_z_order_without_direct_sibling_evidence",
		"target_z": target_z,
		"other_z": other_z,
		"target_z_as_relative": bool(target.z_as_relative),
		"other_z_as_relative": bool(other.z_as_relative),
		"target_layer": target_layer,
		"other_layer": other_layer,
	}


func _canvas_layer_index(item: CanvasItem) -> int:
	var current: Node = item
	while current != null:
		if current is CanvasLayer:
			return int((current as CanvasLayer).layer)
		current = current.get_parent()
	return 0


func _effective_canvas_z(item: CanvasItem) -> int:
	var total := 0
	var current: Node = item
	while current != null:
		if not (current is CanvasItem):
			break
		var canvas_item := current as CanvasItem
		total += int(canvas_item.z_index)
		if not canvas_item.z_as_relative:
			return total
		current = current.get_parent()
	return total


func _visual_snapshot(params) -> String:
	if params is Dictionary:
		return str((params as Dictionary).get("snapshot", ""))
	return ""


func _visual_error_with_context(result: Dictionary, target: String, params: Dictionary, command_name: String) -> Dictionary:
	var error := (result.get("error", {}) as Dictionary).duplicate(true)
	var data := (error.get("data", {}) as Dictionary).duplicate(true)
	if not data.has("command"):
		data["command"] = command_name
	if not data.has("target"):
		data["target"] = target
	if not data.has("snapshot"):
		data["snapshot"] = _visual_snapshot(params)
	error["data"] = data
	return {"ok": false, "error": error}


func _snapshot_pixel_order_evidence(target_item: CanvasItem, other_item: CanvasItem, target_rect: Rect2, other_rect: Rect2, params: Dictionary) -> Dictionary:
	var snapshot := _visual_snapshot(params)
	var visual_dir := str(params.get("visual_dir", ""))
	if snapshot == "" or visual_dir == "":
		return {"ok": false, "reason": "missing_snapshot_pixel_evidence"}
	if not (target_item is ColorRect and other_item is ColorRect):
		return {"ok": false, "reason": "snapshot_pixel_evidence_requires_color_rects"}
	var overlap := target_rect.intersection(other_rect)
	if overlap.size.x <= 0.0 or overlap.size.y <= 0.0:
		return {"ok": false, "reason": "no_overlap"}
	var snapshot_path := "%s/snapshots/%s/screenshot.png" % [visual_dir.get_base_dir(), snapshot]
	if not FileAccess.file_exists(snapshot_path):
		return {"ok": false, "reason": "missing_snapshot_screenshot", "path": snapshot_path}
	var image := Image.new()
	var load_err := image.load(snapshot_path)
	if load_err != OK:
		return {"ok": false, "reason": "snapshot_screenshot_load_failed", "path": snapshot_path, "actual": load_err}
	var sample_x := clampi(int(floor(overlap.get_center().x)), 0, image.get_width() - 1)
	var sample_y := clampi(int(floor(overlap.get_center().y)), 0, image.get_height() - 1)
	var pixel := image.get_pixel(sample_x, sample_y)
	var target_color := (target_item as ColorRect).color
	var other_color := (other_item as ColorRect).color
	var target_distance := _color_distance(pixel, target_color)
	var other_distance := _color_distance(pixel, other_color)
	if target_distance == other_distance:
		return {"ok": false, "reason": "ambiguous_snapshot_pixel_order", "path": snapshot_path, "point": [sample_x, sample_y]}
	return {
		"ok": true,
		"mode": "snapshot_pixel_overlap",
		"path": snapshot_path,
		"point": [sample_x, sample_y],
		"pixel": [pixel.r, pixel.g, pixel.b, pixel.a],
		"target_color_distance": target_distance,
		"other_color_distance": other_distance,
		"target_above_other": target_distance < other_distance,
	}


func _color_distance(a: Color, b: Color) -> float:
	return absf(a.r - b.r) + absf(a.g - b.g) + absf(a.b - b.b) + absf(a.a - b.a)


func _normalized_visual_rect_evidence(rect: Rect2, rect_source: String, coordinate_space: String = "viewport_2d") -> Dictionary:
	return {
		"ok": true,
		"rect": rect,
		"rect_source": rect_source,
		"coordinate_space": coordinate_space,
		"missing_evidence": "",
		"remediation": "",
	}


func _rect_from_visual_evidence(evidence: Dictionary) -> Dictionary:
	var raw_rect = evidence.get("rect", null)
	if not (raw_rect is Array):
		return {"ok": false}
	var values: Array = raw_rect
	if values.size() < 4:
		return {"ok": false}
	return {"ok": true, "rect": Rect2(float(values[0]), float(values[1]), float(values[2]), float(values[3]))}


func _visual_rect_remediation() -> String:
	return "provide qa_visual_rect as a Rect2, [x, y, width, height], or {rect, coordinate_space: \"viewport_2d\"} in rendered viewport pixels"


func _canvas_item_rect(item: CanvasItem) -> Dictionary:
	if item is Control:
		return _normalized_visual_rect_evidence((item as Control).get_global_rect(), "control_global_rect")
	if item is Line2D:
		return _line2d_rect(item as Line2D)
	return _normalized_visual_rect_evidence(Rect2(item.get_global_transform_with_canvas().origin - Vector2(2, 2), Vector2(4, 4)), "canvas_item_origin")


func _line2d_rect(line: Line2D) -> Dictionary:
	var points := line.points
	var base_evidence := {
		"ok": false,
		"rect_source": "line2d_points",
		"coordinate_space": "viewport_2d",
		"actual": {
			"class": line.get_class(),
			"point_count": points.size(),
			"width": float(line.width),
		},
		"remediation": _visual_rect_remediation(),
	}
	if points.size() < 2:
		base_evidence["missing_evidence"] = "nonzero_line2d_points"
		return base_evidence
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
		base_evidence["missing_evidence"] = "nonzero_line2d_length"
		base_evidence["actual"]["bounds"] = _rect_payload(Rect2(Vector2(min_x, min_y), Vector2(max_x - min_x, max_y - min_y)))
		return base_evidence
	var stroke_padding := maxf(float(line.width) * 0.5, 0.5) + 1.0
	var rect := Rect2(Vector2(min_x, min_y), Vector2(max_x - min_x, max_y - min_y)).grow(stroke_padding)
	if rect.size.x <= 0.0 or rect.size.y <= 0.0:
		base_evidence["missing_evidence"] = "measurable_line2d_rect"
		base_evidence["actual"]["bounds"] = _rect_payload(rect)
		return base_evidence
	return _normalized_visual_rect_evidence(rect, "line2d_points")


func _rect_payload(rect: Rect2) -> Array[float]:
	return [float(rect.position.x), float(rect.position.y), float(rect.size.x), float(rect.size.y)]


# Transform that maps 2D canvas coordinates (get_global_rect/qa_visual_rect) into
# the window/screenshot pixel space that visual captures are taken in. Under
# content scaling (project display/window/stretch/mode) this is a scale + offset;
# with no scaling it is the identity, so existing captures are unchanged.
func _canvas_to_screenshot_transform() -> Transform2D:
	if _root_window == null:
		return Transform2D.IDENTITY
	return _root_window.get_final_transform()


func _canvas_to_screenshot_payload(transform: Transform2D) -> Dictionary:
	var scale := transform.get_scale()
	return {
		"scale": [float(scale.x), float(scale.y)],
		"offset": [float(transform.origin.x), float(transform.origin.y)],
	}


func _canvas_visible_rect() -> Rect2:
	if _root_window == null:
		return Rect2()
	return _root_window.get_visible_rect()


# Loud guard against unforeseen capture/scaling mismatches: the full canvas rect,
# once mapped into screenshot space, must match the captured image size. If it
# does not, canvas-space rects cannot be indexed into this screenshot at all, so
# fail with a diagnostic error instead of silently sampling the wrong pixels.
# Returns {} when geometry is consistent, or a populated _error() dict otherwise.
func _screenshot_geometry_mismatch_error(transform: Transform2D, image_size: Vector2, command_name: String, target: String, params: Dictionary, resolution: Dictionary, before_path: String, hidden_path: String) -> Dictionary:
	var canvas_rect := _canvas_visible_rect()
	var expected: Rect2 = transform * canvas_rect
	if absf(expected.size.x - image_size.x) <= 2.0 and absf(expected.size.y - image_size.y) <= 2.0:
		return {}
	return _error(
		"screenshot_geometry_mismatch",
		"visual assertion screenshot size does not match the canvas-to-screenshot transform",
		{
			"command": command_name,
			"target": target,
			"snapshot": _visual_snapshot(params),
			"resolved": resolution.get("resolved", ""),
			"canvas_to_screenshot": _canvas_to_screenshot_payload(transform),
			"canvas_rect": _rect_payload(canvas_rect),
			"expected": {"screenshot_size": [expected.size.x, expected.size.y]},
			"actual": {"screenshot_size": [image_size.x, image_size.y]},
			"remediation": "the captured screenshot size must equal the content-scaled canvas render; ensure the visual capture is a real screenshot of the root window (not a fallback/synthetic image or a mismatched sub-viewport) taken at the active content-scale resolution",
			"artifacts": {"original": before_path, "target_hidden": hidden_path},
		},
	)


func _sanitize_visual_name(raw: String) -> String:
	return raw.replace(":", "_").replace("/", "_").replace("\\", "_")


func _require_target(target: String) -> Dictionary:
	var resolution = _runtime.resolve_target(target)
	if not bool(resolution.get("ok", false)):
		return {
			"ok": false,
			"error": (resolution.get("error", {}) as Dictionary).duplicate(true),
		}
	return resolution


func _temporal_window(params: Dictionary, target: String) -> Dictionary:
	var has_frames := params.has("window_frames")
	var has_seconds := params.has("window_seconds")
	if has_frames == has_seconds:
		return _error("invalid_window", "temporal assertions require exactly one bounded window", {"target": target, "expected": "exactly one of window_frames or window_seconds", "actual": {"window_frames": params.get("window_frames", null), "window_seconds": params.get("window_seconds", null)}})
	var sample_every_frames := int(params.get("sample_every_frames", 1))
	if sample_every_frames <= 0:
		return _error("invalid_sampling", "sample_every_frames must be greater than zero", {"target": target, "expected": "positive integer", "actual": sample_every_frames})
	var sampling := _effective_temporal_sampling(params)
	if not bool(sampling.get("ok", false)):
		return _error("invalid_sampling", "temporal_sampling must use a supported positive cadence", {"target": target, "expected": {"fixed_delta": "fixed_delta_seconds > 0", "max_frame_duration": "max_frame_duration_seconds > 0"}, "actual": params.get("temporal_sampling", null)})
	var sampling_payload: Dictionary = sampling.get("temporal_sampling", {})
	if has_frames:
		var frames := int(params.get("window_frames", 0))
		if frames <= 0:
			return _error("invalid_window", "window_frames must be greater than zero", {"target": target, "expected": "window_frames > 0", "actual": frames})
		return {"ok": true, "mode": "frames", "frames": frames, "remaining_frames": frames, "sample_every_frames": sample_every_frames, "sample": 0, "temporal_sampling": sampling_payload}
	var seconds := float(params.get("window_seconds", 0.0))
	if seconds <= 0.0:
		return _error("invalid_window", "window_seconds must be greater than zero", {"target": target, "expected": "window_seconds > 0", "actual": seconds})
	return {"ok": true, "mode": "seconds", "seconds": seconds, "start_msec": Time.get_ticks_msec(), "sample_every_frames": sample_every_frames, "sample": 0, "temporal_sampling": sampling_payload}


func _temporal_should_sample(window: Dictionary) -> bool:
	if str(window.get("mode", "frames")) == "seconds":
		var elapsed := (Time.get_ticks_msec() - int(window.get("start_msec", Time.get_ticks_msec()))) / 1000.0
		return elapsed < float(window.get("seconds", 0.0)) or int(window.get("sample", 0)) == 0
	return int(window.get("remaining_frames", 0)) > 0


func _advance_temporal_sample(window: Dictionary) -> void:
	var cadence := max(int(window.get("sample_every_frames", 1)), 1)
	var sampling: Dictionary = window.get("temporal_sampling", {})
	var original_max_fps := Engine.max_fps
	var original_physics_ticks_per_second := Engine.physics_ticks_per_second
	_apply_runtime_temporal_sampling(sampling)
	if str(window.get("mode", "frames")) == "seconds":
		for _i in range(cadence):
			await Engine.get_main_loop().process_frame
	else:
		var advance = min(cadence, int(window.get("remaining_frames", 0)))
		for _i in range(advance):
			await Engine.get_main_loop().process_frame
		window["remaining_frames"] = max(int(window.get("remaining_frames", 0)) - advance, 0)
	if not sampling.is_empty():
		Engine.max_fps = original_max_fps
		Engine.physics_ticks_per_second = original_physics_ticks_per_second
	window["sample"] = int(window.get("sample", 0)) + 1


func _window_payload(window: Dictionary) -> Dictionary:
	var payload := {
		"sample_every_frames": int(window.get("sample_every_frames", 1)),
		"cadence": {"sample_every_frames": int(window.get("sample_every_frames", 1))},
	}
	var sampling: Dictionary = window.get("temporal_sampling", {})
	if not sampling.is_empty():
		payload["temporal_sampling"] = sampling.duplicate(true)
		payload["sampling_mode"] = str(sampling.get("mode", ""))
	if str(window.get("mode", "frames")) == "seconds":
		payload["seconds"] = float(window.get("seconds", 0.0))
	else:
		payload["frames"] = int(window.get("frames", 0))
	return payload


func _effective_temporal_sampling(params: Dictionary) -> Dictionary:
	var raw = params.get("temporal_sampling", {})
	if raw == null:
		return {"ok": true, "temporal_sampling": {}}
	if not raw is Dictionary:
		return {"ok": false}
	var sampling: Dictionary = (raw as Dictionary).duplicate(true)
	if sampling.is_empty():
		return {"ok": true, "temporal_sampling": {}}
	var mode := str(sampling.get("mode", ""))
	if mode == "fixed_delta":
		var seconds := float(sampling.get("fixed_delta_seconds", 0.0))
		if seconds <= 0.0 or not is_finite(seconds):
			return {"ok": false}
		sampling["cadence_seconds"] = seconds
		sampling["effective_fps"] = max(int(round(1.0 / seconds)), 1)
		return {"ok": true, "temporal_sampling": sampling}
	if mode == "max_frame_duration":
		var max_seconds := float(sampling.get("max_frame_duration_seconds", 0.0))
		if max_seconds <= 0.0 or not is_finite(max_seconds):
			return {"ok": false}
		sampling["cadence_seconds"] = max_seconds
		sampling["effective_max_fps"] = max(int(ceil(1.0 / max_seconds)), 1)
		return {"ok": true, "temporal_sampling": sampling}
	return {"ok": false}


func _apply_runtime_temporal_sampling(sampling: Dictionary) -> void:
	if sampling.is_empty():
		return
	var mode := str(sampling.get("mode", ""))
	if mode == "fixed_delta":
		var fps := max(int(sampling.get("effective_fps", int(round(1.0 / float(sampling.get("fixed_delta_seconds", 1.0)))))), 1)
		Engine.physics_ticks_per_second = fps
		Engine.max_fps = fps
	elif mode == "max_frame_duration":
		Engine.max_fps = max(int(sampling.get("effective_max_fps", int(ceil(1.0 / float(sampling.get("max_frame_duration_seconds", 1.0)))))), 1)


func _remaining_frame_samples(window: Dictionary) -> int:
	if str(window.get("mode", "frames")) != "frames":
		return 2
	var cadence := max(int(window.get("sample_every_frames", 1)), 1)
	return int(ceil(float(int(window.get("remaining_frames", 0))) / float(cadence)))


func _evidence_context(params: Dictionary) -> int:
	return max(int(params.get("evidence_samples", 2)), 1)


func _node_position_value(node: Variant) -> Variant:
	if node is Node2D:
		return (node as Node2D).global_position
	if node is Control:
		return (node as Control).global_position
	if node is Node3D:
		return (node as Node3D).global_position
	return null


func _position_payload(position: Variant) -> Dictionary:
	if position is Vector2:
		return {"x": position.x, "y": position.y}
	if position is Vector3:
		return {"x": position.x, "y": position.y, "z": position.z}
	return {}


func _position_distance(a: Variant, b: Variant) -> float:
	if a is Vector2 and b is Vector2:
		return (a as Vector2).distance_to(b as Vector2)
	if a is Vector3 and b is Vector3:
		return (a as Vector3).distance_to(b as Vector3)
	return INF


func _position_subtract(a: Variant, b: Variant) -> Variant:
	if a is Vector2 and b is Vector2:
		return (a as Vector2) - (b as Vector2)
	if a is Vector3 and b is Vector3:
		return (a as Vector3) - (b as Vector3)
	return null


func _position_add(a: Variant, b: Variant) -> Variant:
	if a is Vector2 and b is Vector2:
		return (a as Vector2) + (b as Vector2)
	if a is Vector3 and b is Vector3:
		return (a as Vector3) + (b as Vector3)
	return null


func _forbidden_values(value: Variant) -> Array:
	if value is Array:
		return (value as Array).duplicate(true)
	if value is String:
		var raw := str(value)
		var entries: Array = []
		for item in raw.split(","):
			var trimmed := str(item).strip_edges()
			if trimmed != "":
				entries.append(trimmed)
		return entries
	return []


func _never_observation(params: Dictionary) -> Dictionary:
	var present: Array = []
	for source in ["property", "text", "token"]:
		if params.has(source):
			present.append(source)
	if present.size() != 1:
		return _error("invalid_observation", "never requires exactly one observation source", {"expected": ["property", "text", "token"], "actual": present})
	var source := str(present[0])
	var field := "text" if source == "text" else str(params.get(source, ""))
	if field.strip_edges() == "":
		return _error("invalid_observation", "never observation field must be non-empty", {"source": source, "expected": "non-empty field", "actual": field})
	return {"ok": true, "source": source, "field": field.strip_edges()}


func _read_never_value(node: Variant, observation: Dictionary) -> Dictionary:
	var field := str(observation.get("field", ""))
	var source := str(observation.get("source", ""))
	if source == "text":
		if node is Control and _has_property(node, "text"):
			return {"ok": true, "value": str(node.get("text"))}
		return _error("property_not_found", "target does not expose text", {"field": field, "expected": "text property", "actual": _runtime.describe_node(node)})
	if node is Object and _has_property(node, field):
		return {"ok": true, "value": (node as Object).get(field)}
	return _error("property_not_found", "target does not expose requested observation field", {"field": field, "source": source, "expected": "property on resolved node", "actual": _runtime.describe_node(node)})


func _numeric_temporal_value(node: Variant, field: String, anchor: Variant) -> Dictionary:
	var normalized := field.strip_edges()
	if normalized == "":
		normalized = "position_delta"
	if normalized == "position_delta":
		var position = _node_position_value(node)
		if position == null or anchor == null:
			return _error("unsupported_node_type", "target does not expose a position for position_delta", {"field": normalized, "actual": _runtime.describe_node(node)})
		return {"ok": true, "field": normalized, "value": _position_distance(position, anchor)}
	if normalized == "position":
		var current_position = _node_position_value(node)
		if current_position == null:
			return _error("unsupported_node_type", "target does not expose a position", {"field": normalized, "actual": _runtime.describe_node(node)})
		return {"ok": true, "field": normalized, "value": _position_magnitude(current_position)}
	if normalized == "velocity":
		var velocity = _runtime.velocity_for_node(node)
		if velocity == null:
			return _error("unsupported_node_type", "target does not expose velocity", {"field": normalized, "actual": _runtime.describe_node(node)})
		return {"ok": true, "field": normalized, "value": float(velocity)}
	if normalized == "rotation":
		var rotation = _runtime.rotation_for_node(node)
		var rotation_value = _numeric_or_vector_magnitude(rotation)
		if rotation_value == null:
			return _error("unsupported_node_type", "target does not expose rotation", {"field": normalized, "actual": _runtime.describe_node(node)})
		return {"ok": true, "field": normalized, "value": float(rotation_value)}
	if node is Object and _has_property(node, normalized):
		var value = (node as Object).get(normalized)
		var numeric = _numeric_or_vector_magnitude(value)
		if numeric != null:
			return {"ok": true, "field": normalized, "value": float(numeric)}
		return _error("property_not_numeric", "requested within_limits property is not numeric", {"field": normalized, "expected": "number or vector", "actual": str(value)})
	return _error("property_not_found", "target does not expose requested within_limits field", {"field": normalized, "expected": "property or supported runtime field", "actual": _runtime.describe_node(node)})


func _position_magnitude(position: Variant) -> Variant:
	if position is Vector2:
		return (position as Vector2).length()
	if position is Vector3:
		return (position as Vector3).length()
	return null


func _numeric_or_vector_magnitude(value: Variant) -> Variant:
	if typeof(value) == TYPE_INT or typeof(value) == TYPE_FLOAT:
		return float(value)
	if value is Vector2:
		return (value as Vector2).length()
	if value is Vector3:
		return (value as Vector3).length()
	return null


func _temporal_state_observation(params: Dictionary, command: String) -> Dictionary:
	# Temporal assertions may observe a state-provider expression instead of a
	# node property, using the same source vocabulary as state_matches /
	# wait_until. Absent `source` keeps the legacy node observation.
	var source := str(params.get("source", "")).strip_edges()
	if source == "":
		return {"ok": true, "mode": "node"}
	var state_path = params.get("path", "")
	if source == "provider":
		var provider := str(params.get("provider", params.get("provider_id", ""))).strip_edges()
		if provider == "":
			return _error("invalid_assertion_args", "temporal state observations with source: provider require a non-empty provider", {"command": command, "field": "provider", "actual": params.get("provider", null)})
		return {"ok": true, "mode": "provider", "provider": provider, "path": state_path, "label": provider}
	if source == "method" or source == "qa_method":
		var target := str(params.get("target", "")).strip_edges()
		if target == "":
			return _error("invalid_assertion_args", "temporal state observations with source: method require a non-empty target", {"command": command, "field": "target", "actual": params.get("target", null)})
		var method := str(params.get("method", "")).strip_edges()
		if method == "":
			return _error("invalid_assertion_args", "temporal state observations with source: method require a non-empty method", {"command": command, "field": "method", "actual": params.get("method", null)})
		var resolution = _require_target(target)
		if not bool(resolution.get("ok", false)):
			return resolution
		return {"ok": true, "mode": "method", "target": target, "resolved": str(resolution.get("resolved", "")), "node": resolution.get("node"), "method": method, "path": state_path, "label": target}
	return _error("invalid_assertion_args", "temporal state observations require source: provider or method", {"command": command, "field": "source", "expected": ["provider", "method"], "actual": source})


func _temporal_observation_payload(observation: Dictionary) -> Dictionary:
	var mode := str(observation.get("mode", "node"))
	if mode == "node":
		return {}
	var payload := {"source": ("provider" if mode == "provider" else "method"), "path": str(observation.get("path", ""))}
	if mode == "provider":
		payload["provider"] = str(observation.get("provider", ""))
	else:
		payload["method"] = str(observation.get("method", ""))
		payload["resolved"] = str(observation.get("resolved", ""))
	return payload


func _read_temporal_state_value(observation: Dictionary, command: String) -> Dictionary:
	var state: Variant = null
	if str(observation.get("mode", "")) == "provider":
		var provider_result: Dictionary = _runtime.read_state_provider(str(observation.get("provider", "")))
		if not bool(provider_result.get("ok", false)):
			return provider_result
		state = provider_result.get("state", null)
	else:
		var node = observation.get("node")
		if node == null or not is_instance_valid(node):
			return _error("target_freed", "temporal state observation target was freed before state could be read", {"command": command, "target": observation.get("target", ""), "resolved": observation.get("resolved", "")})
		var state_result := _call_state_method(node, str(observation.get("method", "")), "method", str(observation.get("target", "")), str(observation.get("resolved", "")))
		if not bool(state_result.get("ok", false)):
			return state_result
		state = state_result.get("state", null)
	var path_result := _state_path_value(state, observation.get("path", ""))
	if not bool(path_result.get("exists", false)):
		return _error("state_path_missing", "temporal state observation path was missing from the observed state", {"command": command, "path": observation.get("path", ""), "provider": observation.get("provider", ""), "method": observation.get("method", ""), "expected": "state path present in observed state", "actual": null})
	return {"ok": true, "value": path_result.get("value")}


func _read_numeric_temporal_state_value(observation: Dictionary, command: String) -> Dictionary:
	var value_result := _read_temporal_state_value(observation, command)
	if not bool(value_result.get("ok", false)):
		return value_result
	var value = value_result.get("value")
	if not _state_value_is_number(value):
		return _error("state_value_not_numeric", "temporal state observation value is not numeric", {"command": command, "path": observation.get("path", ""), "expected": "number", "actual": value})
	return {"ok": true, "value": float(value)}


func _bounded_sample_series(samples: Array, cap: int) -> Dictionary:
	if samples.size() <= cap:
		return {"series": samples.duplicate(true), "omitted": 0}
	var head := int(cap / 2.0)
	var tail := cap - head
	var series: Array = samples.slice(0, head).duplicate(true)
	series.append_array(samples.slice(samples.size() - tail, samples.size()).duplicate(true))
	return {"series": series, "omitted": samples.size() - cap}


func _nearby_samples(samples: Array, failing_sample: int, context: int = 2) -> Array:
	var nearby: Array = []
	for sample in samples:
		if not sample is Dictionary:
			continue
		var sample_index := int(sample.get("sample", 0))
		if abs(sample_index - failing_sample) <= context:
			nearby.append((sample as Dictionary).duplicate(true))
	return nearby


func _attach_temporal_evidence(kind: String, target: String, params: Dictionary, payload: Dictionary) -> void:
	var evidence_payload := payload.duplicate(true)
	evidence_payload["kind"] = kind
	if payload.has("window") and payload.get("window", {}) is Dictionary:
		var window_payload: Dictionary = payload.get("window", {})
		if window_payload.has("temporal_sampling"):
			evidence_payload["temporal_sampling"] = (window_payload.get("temporal_sampling", {}) as Dictionary).duplicate(true)
		if window_payload.has("cadence"):
			evidence_payload["cadence"] = (window_payload.get("cadence", {}) as Dictionary).duplicate(true)
	var artifacts = _runtime.write_temporal_evidence(kind, target, _merge_evidence_params(params, evidence_payload))
	if artifacts is Dictionary:
		payload["artifacts"] = (artifacts as Dictionary).duplicate(true)


func _merge_evidence_params(params: Dictionary, payload: Dictionary) -> Dictionary:
	var merged := payload.duplicate(true)
	if params.has("temporal_dir"):
		merged["temporal_dir"] = str(params.get("temporal_dir", ""))
	return merged


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
	if frame_value != null:
		return max(1, int(frame_value))
	if second_value != null:
		var fps := Engine.get_frames_per_second()
		if fps <= 0.0:
			fps = 60.0
		return max(1, int(ceil(float(second_value) * fps)))
	return max(1, default_frames)


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
		var provider_resolution := _require_target(provider_target)
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
	if not _json_compatible(result):
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
	if not _json_compatible(value):
		return _surface_failure("invalid_probe_result", "actor surface state was not JSON-compatible", params, {"provider_method": method_name, "actual_type": typeof(value)})
	return {"ok": true, "label": str(value)}


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
		if _json_compatible(value):
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
		var resolution := _require_target(locator)
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
	return _error(error_type, message, payload)


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
