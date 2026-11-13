extends RefCounted

const QaSnapshot = preload("res://addons/godot_qa/qa_snapshot.gd")
const QaJsonEquality = preload("res://addons/godot_qa/qa_json_equality.gd")
const QaAssertionsState = preload("res://addons/godot_qa/qa_assertions_state.gd")
const TREND_DIRECTIONS: Array[String] = ["increasing", "decreasing", "non_increasing", "non_decreasing"]
const TEMPORAL_SAMPLE_ECHO_CAP := 32

var _owner: WeakRef
var _runtime: RefCounted:
	get:
		return _owner.get_ref()._runtime
var _root_window: Window:
	get:
		return _owner.get_ref()._root_window
var _session_fixed_fps: int:
	get:
		return _owner.get_ref()._session_fixed_fps
var _state: QaAssertionsState:
	get:
		return _owner.get_ref()._state


func _init(owner: RefCounted) -> void:
	_owner = weakref(owner)


func _assert_observe_until(params: Dictionary) -> Dictionary:
	var target := str(params.get("target", "")).strip_edges()
	if target == "":
		return _owner.get_ref()._error("invalid_assertion_args", "observe_until requires a non-empty target", {"command": "assert.observe_until", "field": "target", "actual": params.get("target", null)})
	var window := _temporal_window(params, target)
	if not bool(window.get("ok", false)):
		return window
	var condition := _observe_until_condition_payload(params)
	if not bool(condition.get("ok", false)):
		return condition
	var resolution = _owner.get_ref()._require_target(target)
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
			var success_evidence_error := _attach_temporal_evidence("observe_until", target, params, success_payload)
			if not success_evidence_error.is_empty():
				return success_evidence_error
			return _owner.get_ref()._ok(success_payload)
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
	var evidence_error := _attach_temporal_evidence("observe_until", target, params, failure_payload)
	if not evidence_error.is_empty():
		return evidence_error
	return _owner.get_ref()._error("observe_until_mismatch", "condition did not become true within the bounded window", failure_payload)


func _observe_until_condition_payload(params: Dictionary) -> Dictionary:
	var families: Array = []
	if params.has("visible"):
		families.append("visible")
	if params.has("property"):
		families.append("property")
	if params.has("matches"):
		families.append("state_matches")
	if families.size() != 1:
		return _owner.get_ref()._error("invalid_assertion_args", "observe_until requires exactly one condition family", {"command": "assert.observe_until", "expected": ["visible", "property", "matches"], "actual": families})
	var family := str(families[0])
	if family == "visible":
		if typeof(params.get("visible", null)) != TYPE_BOOL:
			return _owner.get_ref()._error("invalid_assertion_args", "observe_until visible condition must be a boolean", {"command": "assert.observe_until", "field": "visible", "actual": params.get("visible", null)})
		return {"ok": true, "condition": {"type": "visible", "visible": bool(params.get("visible", false))}, "expected": {"visible": bool(params.get("visible", false))}}
	if family == "property":
		var property := str(params.get("property", "")).strip_edges()
		var operator := str(params.get("operator", params.get("op", "equals"))).strip_edges()
		if operator == "":
			operator = "equals"
		if property == "" or not _state._state_match_operators().has(operator):
			return _owner.get_ref()._error("invalid_assertion_args", "observe_until property conditions require a property and supported operator", {"command": "assert.observe_until", "expected": {"property": "non-empty", "operator": _state._state_match_operators()}, "actual": {"property": property, "operator": operator}})
		if ["equals", "contains", "within"].has(operator) and not params.has("expected"):
			return _owner.get_ref()._error("invalid_assertion_args", "observe_until property condition requires expected", {"command": "assert.observe_until", "field": "expected", "actual": null})
		if operator == "within" and not bool(_state._state_match_within_tolerance(params).get("ok", false)):
			return _owner.get_ref()._error("invalid_assertion_args", "observe_until within conditions require a positive numeric tolerance", {"command": "assert.observe_until", "field": "tolerance", "expected": "positive number", "actual": params.get("tolerance", null)})
		return {"ok": true, "condition": {"type": "property", "property": property, "operator": operator, "expected": params.get("expected", null), "tolerance": params.get("tolerance", null)}, "expected": {"property": property, "operator": operator, "value": params.get("expected", null)}}
	var state_params := params.duplicate(true)
	var checks := _state._state_match_checks(state_params)
	if checks.is_empty():
		return _owner.get_ref()._error("invalid_assertion_args", "observe_until state condition requires matches", {"command": "assert.observe_until", "field": "matches", "actual": params.get("matches", null)})
	return {"ok": true, "condition": {"type": "state_matches", "source": str(params.get("source", "auto")), "method": str(params.get("method", "")), "matches": checks}, "expected": {"matches": checks}}


func _observe_until_sample_payload(node: Node, params: Dictionary, condition: Dictionary, window: Dictionary, target: String, resolved: String) -> Dictionary:
	if node == null or not is_instance_valid(node):
		return _owner.get_ref()._error("target_freed", "observe_until target was freed before state could be read", {"command": "assert.observe_until", "target": target, "resolved": resolved})
	var condition_type := str(condition.get("type", ""))
	var sample := {"sample": int(window.get("sample", 0)), "frame": Engine.get_process_frames(), "time_msec": Time.get_ticks_msec(), "target": target, "resolved": resolved, "condition": condition.duplicate(true), "matched": false, "actual": null}
	if condition_type == "visible":
		var visible: bool = _runtime.is_target_visible(node)
		sample["actual"] = {"visible": visible}
		sample["matched"] = visible == bool(condition.get("visible", false))
		return {"ok": true, "sample": sample}
	if condition_type == "property":
		var property := str(condition.get("property", ""))
		var exists: bool = node is Object and _owner.get_ref()._has_property(node, property)
		var value = (node as Object).get(property) if exists else null
		var operator := str(condition.get("operator", "equals"))
		var actual_payload := {"path": property, "operator": operator, "expected": condition.get("expected", null), "actual": _observe_until_finite_evidence(QaSnapshot._json_safe_value(value)), "present": exists}
		var matched := false
		if operator == "present":
			matched = exists
		elif operator == "absent":
			matched = not exists
		elif not exists:
			actual_payload["mismatch_type"] = "property_not_found"
		elif operator == "equals":
			matched = QaJsonEquality.equals(value, condition.get("expected", null))
			if not matched:
				actual_payload["mismatch_type"] = "value_mismatch"
				_state._add_state_numeric_mismatch_diagnostics(actual_payload, value, condition.get("expected", null))
		elif operator == "approx_equals" or operator == "approximately_equals":
			matched = _state._state_values_approx_equal(value, condition.get("expected", null), condition)
			if not matched:
				actual_payload["mismatch_type"] = "approx_mismatch"
				actual_payload["epsilon"] = _state._state_match_epsilon(condition)
		elif operator == "greater_than":
			matched = _state._state_numeric_compare(value, condition.get("expected", null), ">")
			if not matched:
				actual_payload["mismatch_type"] = "greater_than_mismatch"
		elif operator == "less_than":
			matched = _state._state_numeric_compare(value, condition.get("expected", null), "<")
			if not matched:
				actual_payload["mismatch_type"] = "less_than_mismatch"
		elif operator == "within":
			matched = _state._state_values_within(value, condition.get("expected", null), float(condition.get("tolerance", 0.0)))
			if not matched:
				actual_payload["mismatch_type"] = "within_mismatch"
				actual_payload["tolerance"] = condition.get("tolerance", null)
		elif operator == "contains":
			matched = _state._state_value_contains(value, condition.get("expected", null))
			if not matched:
				actual_payload["mismatch_type"] = "contains_mismatch"
		sample["actual"] = actual_payload
		sample["matched"] = matched
		return {"ok": true, "sample": sample}
	if condition_type == "state_matches":
		var state_result := _state._resolve_state_source(node, params, target, resolved)
		if not bool(state_result.get("ok", false)):
			return state_result
		var state = state_result.get("state", {})
		var mismatches: Array = []
		var matched: Array = []
		for raw_check in condition.get("matches", []):
			if not (raw_check is Dictionary):
				mismatches.append({"mismatch_type": "invalid_check", "expected": "check object", "actual": raw_check})
				continue
			var check_result := _state._evaluate_state_match(state, raw_check)
			if bool(check_result.get("ok", false)):
				matched.append(check_result.get("data", {}))
			else:
				mismatches.append(check_result.get("data", {}))
		sample["source"] = state_result.get("source", "")
		sample["method"] = state_result.get("method", "")
		sample["actual"] = {"matched": matched, "mismatches": mismatches}
		sample["matched"] = mismatches.is_empty()
		return {"ok": true, "sample": sample}
	return _owner.get_ref()._error("invalid_assertion_args", "observe_until condition type is unsupported", {"command": "assert.observe_until", "actual": condition_type})


func _observe_until_finite_evidence(value: Variant) -> Variant:
	match typeof(value):
		TYPE_FLOAT:
			return value if is_finite(value) else str(value)
		TYPE_ARRAY:
			var items: Array = []
			for item in value:
				items.append(_observe_until_finite_evidence(item))
			return items
		TYPE_DICTIONARY:
			var result := {}
			for key in value:
				result[key] = _observe_until_finite_evidence(value[key])
			return result
	return value


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


func _assert_temporal_max_delta_position(params: Dictionary) -> Dictionary:
	var target := str(params.get("target", ""))
	var max_delta := float(params.get("max", -1.0))
	var window := _temporal_window(params, target)
	if not bool(window.get("ok", false)):
		return window
	var resolution = _owner.get_ref()._require_target(target)
	if not resolution.get("ok", false):
		return resolution
	var node = resolution.get("node")
	var anchor = _node_position_value(node)
	if anchor == null:
		return _owner.get_ref()._error("unsupported_node_type", "Resolved target does not expose a global position", {"target": target, "actual": _runtime.describe_node(node)})
	var peak_delta := 0.0
	var peak_sample := 0
	var peak_sample_payload := {}
	var samples: Array = []
	while _temporal_should_sample(window):
		await _advance_temporal_sample(window)
		var position = _node_position_value(node)
		if position == null:
			return _owner.get_ref()._error("unsupported_node_type", "Resolved target stopped exposing a global position", {"target": target, "actual": _runtime.describe_node(node)})
		var delta := _position_distance(position, anchor)
		var sample := {"sample": int(window.get("sample", 0)), "frame": Engine.get_process_frames(), "time_msec": Time.get_ticks_msec(), "position": _position_payload(position), "delta": delta}
		samples.append(sample)
		if delta > peak_delta:
			peak_delta = delta
			peak_sample = int(sample.get("sample", 0))
			peak_sample_payload = sample.duplicate(true)
	if peak_delta <= max_delta:
		return _owner.get_ref()._ok({"target": target, "resolved": resolution.get("resolved", ""), "window": _window_payload(window), "max": max_delta, "actual_max_delta": peak_delta, "failing_sample": null})
	var failure_payload := {"target": target, "resolved": resolution.get("resolved", ""), "window": _window_payload(window), "expected": {"max": max_delta, "window": _window_payload(window)}, "actual": {"max": peak_delta, "sample": peak_sample}, "sample": peak_sample_payload, "expected_max_delta": max_delta, "actual_max_delta": peak_delta, "failing_sample": peak_sample, "samples": _nearby_samples(samples, peak_sample, _evidence_context(params))}
	var evidence_error := _attach_temporal_evidence("max_delta_position", target, params, failure_payload)
	if not evidence_error.is_empty():
		return evidence_error
	return _owner.get_ref()._error("max_delta_position_mismatch", "target exceeded max displacement bound", failure_payload)


func _assert_temporal_follows(params: Dictionary) -> Dictionary:
	var leader_target := str(params.get("leader", ""))
	var follower_target := str(params.get("follower", ""))
	var tolerance := float(params.get("tolerance", -1.0))
	var window := _temporal_window(params, "%s -> %s" % [leader_target, follower_target])
	if not bool(window.get("ok", false)):
		return window
	var leader_resolution = _owner.get_ref()._require_target(leader_target)
	if not leader_resolution.get("ok", false):
		return leader_resolution
	var follower_resolution = _owner.get_ref()._require_target(follower_target)
	if not follower_resolution.get("ok", false):
		return follower_resolution
	var leader = leader_resolution.get("node")
	var follower = follower_resolution.get("node")
	var leader_anchor = _node_position_value(leader)
	var follower_anchor = _node_position_value(follower)
	if leader_anchor == null or follower_anchor == null:
		return _owner.get_ref()._error("unsupported_node_type", "Leader and follower must both expose global positions", {"leader": leader_target, "follower": follower_target})
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
			return _owner.get_ref()._error("unsupported_node_type", "Leader and follower stopped exposing global positions", {"leader": leader_target, "follower": follower_target})
		var expected = _position_add(leader_position, offset)
		var error := _position_distance(follower_position, expected)
		var sample := {"sample": int(window.get("sample", 0)), "frame": Engine.get_process_frames(), "time_msec": Time.get_ticks_msec(), "leader_position": _position_payload(leader_position), "follower_position": _position_payload(follower_position), "expected_position": _position_payload(expected), "offset_error": error}
		samples.append(sample)
		if error > peak_offset_error:
			peak_offset_error = error
			peak_sample = int(sample.get("sample", 0))
			peak_sample_payload = sample.duplicate(true)
	if peak_offset_error <= tolerance:
		return _owner.get_ref()._ok({"leader": leader_target, "follower": follower_target, "window": _window_payload(window), "tolerance": tolerance, "actual_max_error": peak_offset_error, "failing_sample": null})
	var failure_payload := {"leader": leader_target, "follower": follower_target, "target": follower_target, "window": _window_payload(window), "expected": {"tolerance": tolerance, "window": _window_payload(window)}, "actual": {"max_error": peak_offset_error, "sample": peak_sample}, "sample": peak_sample_payload, "expected_tolerance": tolerance, "actual_max_error": peak_offset_error, "failing_sample": peak_sample, "samples": _nearby_samples(samples, peak_sample, _evidence_context(params))}
	var evidence_error := _attach_temporal_evidence("follows", "%s_%s" % [leader_target, follower_target], params, failure_payload)
	if not evidence_error.is_empty():
		return evidence_error
	return _owner.get_ref()._error("follows_mismatch", "follower exceeded follow tolerance", failure_payload)


func _assert_temporal_never(params: Dictionary) -> Dictionary:
	var forbidden := _forbidden_values(params.get("forbidden", []))
	var state_observation := _state._temporal_state_observation(params, "assert.temporal.never")
	if not bool(state_observation.get("ok", false)):
		return state_observation
	var observes_state := str(state_observation.get("mode", "node")) != "node"
	var target := str(state_observation.get("label", "")) if observes_state else str(params.get("target", ""))
	if forbidden.is_empty():
		return _owner.get_ref()._error("invalid_forbidden", "never requires at least one forbidden value", {"target": target, "expected": "non-empty forbidden value list", "actual": params.get("forbidden", null)})
	var window := _temporal_window(params, target)
	if not bool(window.get("ok", false)):
		return window
	var node: Node = null
	var observation := {}
	if not observes_state:
		var resolution = _owner.get_ref()._require_target(target)
		if not resolution.get("ok", false):
			return resolution
		node = resolution.get("node")
		observation = _never_observation(params)
		if not bool(observation.get("ok", false)):
			return observation
	var source_name := str(state_observation.get("mode", "")) if observes_state else str(observation.get("source", ""))
	var field_name := str(state_observation.get("path", "")) if observes_state else str(observation.get("field", ""))
	var observation_payload := _state._temporal_observation_payload(state_observation)
	var samples: Array = []
	while _temporal_should_sample(window):
		await _advance_temporal_sample(window)
		var value_result := _state._read_temporal_state_value(state_observation, "assert.temporal.never") if observes_state else _read_never_value(node, observation)
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
					var after_value_result := _state._read_temporal_state_value(state_observation, "assert.temporal.never") if observes_state else _read_never_value(node, observation)
					if not bool(after_value_result.get("ok", false)):
						return after_value_result
					samples.append({"sample": int(window.get("sample", 0)), "frame": Engine.get_process_frames(), "time_msec": Time.get_ticks_msec(), "source": source_name, "field": field_name, "value": after_value_result.get("value")})
				var failing_sample_payload := (samples[failing_sample - 1] as Dictionary).duplicate(true)
				var failure_payload := {"target": target, "source": source_name, "field": field_name, "forbidden": forbidden, "window": _window_payload(window), "expected": {"never": forbidden, "source": source_name, "field": field_name, "window": _window_payload(window)}, "actual": {"value": value, "sample": failing_sample}, "sample": failing_sample_payload, "failing_sample": failing_sample, "samples": _nearby_samples(samples, failing_sample, _evidence_context(params))}
				failure_payload.merge(observation_payload)
				var evidence_error := _attach_temporal_evidence("never", target, params, failure_payload)
				if not evidence_error.is_empty():
					return evidence_error
				return _owner.get_ref()._error("never_mismatch", "target entered a forbidden value", failure_payload)
	var success_payload := {"target": target, "source": source_name, "field": field_name, "forbidden": forbidden, "window": _window_payload(window), "failing_sample": null}
	success_payload.merge(observation_payload)
	return _owner.get_ref()._ok(success_payload)


func _assert_temporal_within_limits(params: Dictionary) -> Dictionary:
	var min_value := float(params.get("min", 0.0))
	var max_value := float(params.get("max", 0.0))
	var observation := _state._temporal_state_observation(params, "assert.temporal.within_limits")
	if not bool(observation.get("ok", false)):
		return observation
	var observes_state := str(observation.get("mode", "node")) != "node"
	var target := str(observation.get("label", "")) if observes_state else str(params.get("target", ""))
	var field := str(params.get("path", "")) if observes_state else str(params.get("property", params.get("field", "position_delta")))
	var window := _temporal_window(params, target)
	if not bool(window.get("ok", false)):
		return window
	if not is_finite(min_value) or not is_finite(max_value) or min_value > max_value:
		return _owner.get_ref()._error("invalid_bounds", "within_limits bounds must be finite with min less than or equal to max", {"command": "assert.temporal.within_limits", "target": target, "expected": "finite min <= max", "actual": {"min": min_value if is_finite(min_value) else str(min_value), "max": max_value if is_finite(max_value) else str(max_value)}})
	var node: Node = null
	var anchor: Variant = null
	if not observes_state:
		var resolution = _owner.get_ref()._require_target(target)
		if not resolution.get("ok", false):
			return resolution
		node = resolution.get("node")
		anchor = _node_position_value(node)
	var observation_payload := _state._temporal_observation_payload(observation)
	var samples: Array = []
	while _temporal_should_sample(window):
		await _advance_temporal_sample(window)
		var value_result := _state._read_numeric_temporal_state_value(observation, "assert.temporal.within_limits") if observes_state else _numeric_temporal_value(node, field, anchor)
		if not bool(value_result.get("ok", false)):
			return value_result
		var value := float(value_result.get("value", 0.0))
		var sample := {"sample": int(window.get("sample", 0)), "frame": Engine.get_process_frames(), "time_msec": Time.get_ticks_msec(), "field": value_result.get("field", field), "value": value if is_finite(value) else str(value)}
		samples.append(sample)
		if not is_finite(value) or value < min_value or value > max_value:
			var failing_sample := int(sample.get("sample", 0))
			var failure_payload := {"command": "assert.temporal.within_limits", "target": target, "field": value_result.get("field", field), "window": _window_payload(window), "expected": {"min": min_value, "max": max_value, "field": value_result.get("field", field), "window": _window_payload(window)}, "actual": {"value": sample.get("value"), "sample": failing_sample}, "sample": sample.duplicate(true), "failing_sample": failing_sample, "samples": _nearby_samples(samples, failing_sample, _evidence_context(params))}
			failure_payload.merge(observation_payload)
			var evidence_error := _attach_temporal_evidence("within_limits", target, params, failure_payload)
			if not evidence_error.is_empty():
				return evidence_error
			return _owner.get_ref()._error("within_limits_mismatch", "target value left configured limits", failure_payload)
	var success_payload := {"target": target, "field": field, "window": _window_payload(window), "min": min_value, "max": max_value, "failing_sample": null}
	success_payload.merge(observation_payload)
	return _owner.get_ref()._ok(success_payload)


func _assert_temporal_trend(params: Dictionary) -> Dictionary:
	var direction := str(params.get("trend", "")).strip_edges()
	if not TREND_DIRECTIONS.has(direction):
		return _owner.get_ref()._error("invalid_assertion_args", "trend requires a supported direction", {"command": "assert.temporal.trend", "field": "trend", "expected": TREND_DIRECTIONS, "actual": params.get("trend", null)})
	var has_min_delta := params.has("min_delta")
	var min_delta := 0.0
	if has_min_delta:
		var raw_min_delta = params.get("min_delta")
		if not _state._state_value_is_number(raw_min_delta) or not is_finite(float(raw_min_delta)) or float(raw_min_delta) < 0.0:
			return _owner.get_ref()._error("invalid_assertion_args", "trend min_delta must be a finite non-negative number", {"command": "assert.temporal.trend", "field": "min_delta", "expected": "finite non-negative number", "actual": raw_min_delta})
		min_delta = float(raw_min_delta)
	var tolerance := 0.0
	if params.has("tolerance"):
		var raw_tolerance = params.get("tolerance")
		if not _state._state_value_is_number(raw_tolerance) or not is_finite(float(raw_tolerance)) or float(raw_tolerance) < 0.0:
			return _owner.get_ref()._error("invalid_assertion_args", "trend tolerance must be a finite non-negative number", {"command": "assert.temporal.trend", "field": "tolerance", "expected": "finite non-negative number", "actual": raw_tolerance})
		tolerance = float(raw_tolerance)
	var observation := _state._temporal_state_observation(params, "assert.temporal.trend")
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
		var resolution = _owner.get_ref()._require_target(target)
		if not resolution.get("ok", false):
			return resolution
		node = resolution.get("node")
		anchor = _node_position_value(node)
	var observation_payload := _state._temporal_observation_payload(observation)
	var samples: Array = []
	while _temporal_should_sample(window):
		await _advance_temporal_sample(window)
		var value_result := _state._read_numeric_temporal_state_value(observation, "assert.temporal.trend") if observes_state else _numeric_temporal_value(node, field, anchor)
		if not bool(value_result.get("ok", false)):
			return value_result
		samples.append({"sample": int(window.get("sample", 0)), "frame": Engine.get_process_frames(), "time_msec": Time.get_ticks_msec(), "field": value_result.get("field", field), "value": float(value_result.get("value", 0.0))})
	if samples.size() < 2:
		return _owner.get_ref()._error("invalid_window", "trend requires at least two samples in the bounded window", {"target": target, "expected": "window and sample_every_frames producing >= 2 samples", "actual": samples.size()})
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
		return _owner.get_ref()._ok(payload)
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
	var evidence_error := _attach_temporal_evidence("trend", target, params, payload)
	if not evidence_error.is_empty():
		return evidence_error
	return _owner.get_ref()._error("trend_mismatch", "sampled series did not follow the required trend", payload)


func _assert_temporal_frame_time(params: Dictionary) -> Dictionary:
	var bounds := {}
	for bound_name in ["max_avg_ms", "max_p95_ms", "min_fps"]:
		if params.has(bound_name):
			var raw = params.get(bound_name)
			if not _state._state_value_is_number(raw) or not is_finite(float(raw)) or float(raw) <= 0.0:
				return _owner.get_ref()._error("invalid_assertion_args", "frame_time bounds must be positive numbers", {"command": "assert.temporal.frame_time", "field": bound_name, "expected": "positive number", "actual": raw})
			bounds[bound_name] = float(raw)
	if bounds.is_empty():
		return _owner.get_ref()._error("invalid_assertion_args", "frame_time requires at least one bound", {"command": "assert.temporal.frame_time", "expected": ["max_avg_ms", "max_p95_ms", "min_fps"], "actual": params.duplicate(true)})
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
	var total_ms := 0.0
	var total_frames := 0
	var last_usec := Time.get_ticks_usec()
	while _temporal_should_sample(window):
		var advanced_frames := await _advance_temporal_sample(window)
		var now_usec := Time.get_ticks_usec()
		var elapsed_ms := float(max(now_usec - last_usec, 0)) / 1000.0
		var frame_ms := elapsed_ms / float(advanced_frames)
		last_usec = now_usec
		total_ms += elapsed_ms
		total_frames += advanced_frames
		frame_times_ms.append({"ms": frame_ms, "frames": advanced_frames})
		samples.append({"sample": int(window.get("sample", 0)), "frame": Engine.get_process_frames(), "frame_time_ms": frame_ms})
	if frame_times_ms.is_empty():
		return _owner.get_ref()._error("invalid_window", "frame_time window produced no samples", {"expected": "window producing >= 1 sample", "actual": 0})
	var sorted_times := frame_times_ms.duplicate()
	sorted_times.sort_custom(func(a: Dictionary, b: Dictionary) -> bool: return float(a.ms) < float(b.ms))
	var avg_ms := total_ms / float(total_frames)
	var p95_rank := int(ceil(0.95 * float(total_frames)))
	var cumulative_frames := 0
	var p95_ms := 0.0
	for interval in sorted_times:
		cumulative_frames += int(interval.frames)
		if cumulative_frames >= p95_rank:
			p95_ms = float(interval.ms)
			break
	var max_ms := float(sorted_times.back().ms)
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
		return _owner.get_ref()._ok(payload)
	payload["expected"] = bounds.duplicate(true)
	payload["actual"] = aggregates.duplicate(true)
	payload["violations"] = violations
	var evidence_error := _attach_temporal_evidence("frame_time", "frame_time", params, payload)
	if not evidence_error.is_empty():
		return evidence_error
	return _owner.get_ref()._error("frame_time_mismatch", "frame timing exceeded configured bounds", payload)


func _assert_temporal_distance_changed(params: Dictionary) -> Dictionary:
	var a := str(params.get("a", params.get("target", "")))
	var b := str(params.get("b", params.get("other", "")))
	var min_raw = params.get("min", null)
	if a.strip_edges() == "" or b.strip_edges() == "":
		return _owner.get_ref()._error(
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
		return _owner.get_ref()._error(
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
		return _owner.get_ref()._error(
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
		return _owner.get_ref()._error(
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

	var initial_left = _owner.get_ref()._resolve_target_position(a)
	if not bool(initial_left.get("ok", false)):
		return initial_left
	var initial_right = _owner.get_ref()._resolve_target_position(b)
	if not bool(initial_right.get("ok", false)):
		return initial_right
	var coordinate_space := str(initial_left.get("coordinate_space", ""))
	if coordinate_space != str(initial_right.get("coordinate_space", "")):
		return _owner.get_ref()._error(
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
		return _owner.get_ref()._error(
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
		var left = _owner.get_ref()._resolve_target_position(a)
		if not bool(left.get("ok", false)):
			return left
		var right = _owner.get_ref()._resolve_target_position(b)
		if not bool(right.get("ok", false)):
			return right
		if str(left.get("coordinate_space", "")) != coordinate_space or str(right.get("coordinate_space", "")) != coordinate_space:
			return _owner.get_ref()._error(
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
			return _owner.get_ref()._error(
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
		return _owner.get_ref()._ok(
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
	var evidence_error := _attach_temporal_evidence("distance_changed", "%s_%s" % [a, b], params, failure_payload)
	if not evidence_error.is_empty():
		return evidence_error
	return _owner.get_ref()._error("distance_changed_mismatch", "distance changed less than the configured minimum", failure_payload)


func _temporal_window(params: Dictionary, target: String) -> Dictionary:
	var has_frames := params.has("window_frames")
	var has_seconds := params.has("window_seconds")
	if has_frames == has_seconds:
		return _owner.get_ref()._error("invalid_window", "temporal assertions require exactly one bounded window", {"target": target, "expected": "exactly one of window_frames or window_seconds", "actual": {"window_frames": params.get("window_frames", null), "window_seconds": params.get("window_seconds", null)}})
	var sample_every_frames := int(params.get("sample_every_frames", 1))
	if sample_every_frames <= 0:
		return _owner.get_ref()._error("invalid_sampling", "sample_every_frames must be greater than zero", {"target": target, "expected": "positive integer", "actual": sample_every_frames})
	var sampling := _effective_temporal_sampling(params)
	if not bool(sampling.get("ok", false)):
		return _owner.get_ref()._error("invalid_sampling", "temporal_sampling must use a supported positive cadence", {"target": target, "expected": {"fixed_delta": "fixed_delta_seconds > 0", "max_frame_duration": "max_frame_duration_seconds > 0"}, "actual": params.get("temporal_sampling", null)})
	var sampling_payload: Dictionary = sampling.get("temporal_sampling", {})
	if has_frames:
		var frames := int(params.get("window_frames", 0))
		if frames <= 0:
			return _owner.get_ref()._error("invalid_window", "window_frames must be greater than zero", {"target": target, "expected": "window_frames > 0", "actual": frames})
		return {"ok": true, "mode": "frames", "frames": frames, "remaining_frames": frames, "sample_every_frames": sample_every_frames, "sample": 0, "temporal_sampling": sampling_payload}
	var seconds := float(params.get("window_seconds", 0.0))
	if seconds <= 0.0:
		return _owner.get_ref()._error("invalid_window", "window_seconds must be greater than zero", {"target": target, "expected": "window_seconds > 0", "actual": seconds})
	var window := {"ok": true, "mode": "seconds", "seconds": seconds, "start_msec": Time.get_ticks_msec(), "sample_every_frames": sample_every_frames, "sample": 0, "temporal_sampling": sampling_payload}
	if _temporal_fixed_fps() > 0:
		window["mode"] = "frames"
		window["frames"] = _temporal_frame_count(null, seconds, 1)
		window["remaining_frames"] = window["frames"]
	return window


func _temporal_should_sample(window: Dictionary) -> bool:
	if str(window.get("mode", "frames")) == "seconds":
		var elapsed := (Time.get_ticks_msec() - int(window.get("start_msec", Time.get_ticks_msec()))) / 1000.0
		return elapsed < float(window.get("seconds", 0.0)) or int(window.get("sample", 0)) == 0
	return int(window.get("remaining_frames", 0)) > 0


func _advance_temporal_sample(window: Dictionary) -> int:
	var cadence := max(int(window.get("sample_every_frames", 1)), 1)
	var advanced_frames: int = cadence
	var sampling: Dictionary = window.get("temporal_sampling", {})
	var original_max_fps := Engine.max_fps
	var original_physics_ticks_per_second := Engine.physics_ticks_per_second
	_apply_runtime_temporal_sampling(sampling)
	if str(window.get("mode", "frames")) == "seconds":
		for _i in range(cadence):
			await Engine.get_main_loop().process_frame
	else:
		advanced_frames = min(cadence, int(window.get("remaining_frames", 0)))
		for _i in range(advanced_frames):
			await Engine.get_main_loop().process_frame
		window["remaining_frames"] = max(int(window.get("remaining_frames", 0)) - advanced_frames, 0)
	if not sampling.is_empty():
		Engine.max_fps = original_max_fps
		Engine.physics_ticks_per_second = original_physics_ticks_per_second
	window["sample"] = int(window.get("sample", 0)) + 1
	return advanced_frames


func _window_payload(window: Dictionary) -> Dictionary:
	var payload := {
		"sample_every_frames": int(window.get("sample_every_frames", 1)),
		"cadence": {"sample_every_frames": int(window.get("sample_every_frames", 1))},
	}
	var sampling: Dictionary = window.get("temporal_sampling", {})
	if not sampling.is_empty():
		payload["temporal_sampling"] = sampling.duplicate(true)
		payload["sampling_mode"] = str(sampling.get("mode", ""))
	if window.has("seconds"):
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
		return _owner.get_ref()._error("invalid_observation", "never requires exactly one observation source", {"expected": ["property", "text", "token"], "actual": present})
	var source := str(present[0])
	var field := "text" if source == "text" else str(params.get(source, ""))
	if field.strip_edges() == "":
		return _owner.get_ref()._error("invalid_observation", "never observation field must be non-empty", {"source": source, "expected": "non-empty field", "actual": field})
	return {"ok": true, "source": source, "field": field.strip_edges()}


func _read_never_value(node: Variant, observation: Dictionary) -> Dictionary:
	var field := str(observation.get("field", ""))
	var source := str(observation.get("source", ""))
	if source == "text":
		if node is Control and _owner.get_ref()._has_property(node, "text"):
			return {"ok": true, "value": str(node.get("text"))}
		return _owner.get_ref()._error("property_not_found", "target does not expose text", {"field": field, "expected": "text property", "actual": _runtime.describe_node(node)})
	if node is Object and _owner.get_ref()._has_property(node, field):
		return {"ok": true, "value": (node as Object).get(field)}
	return _owner.get_ref()._error("property_not_found", "target does not expose requested observation field", {"field": field, "source": source, "expected": "property on resolved node", "actual": _runtime.describe_node(node)})


func _numeric_temporal_value(node: Variant, field: String, anchor: Variant) -> Dictionary:
	var normalized := field.strip_edges()
	if normalized == "":
		normalized = "position_delta"
	if normalized == "position_delta":
		var position = _node_position_value(node)
		if position == null or anchor == null:
			return _owner.get_ref()._error("unsupported_node_type", "target does not expose a position for position_delta", {"field": normalized, "actual": _runtime.describe_node(node)})
		return {"ok": true, "field": normalized, "value": _position_distance(position, anchor)}
	if normalized == "position":
		var current_position = _node_position_value(node)
		if current_position == null:
			return _owner.get_ref()._error("unsupported_node_type", "target does not expose a position", {"field": normalized, "actual": _runtime.describe_node(node)})
		return {"ok": true, "field": normalized, "value": _position_magnitude(current_position)}
	if normalized == "velocity":
		var velocity = _runtime.velocity_for_node(node)
		if velocity == null:
			return _owner.get_ref()._error("unsupported_node_type", "target does not expose velocity", {"field": normalized, "actual": _runtime.describe_node(node)})
		return {"ok": true, "field": normalized, "value": float(velocity)}
	if normalized == "rotation":
		var rotation = _runtime.rotation_for_node(node)
		var rotation_value = _numeric_or_vector_magnitude(rotation)
		if rotation_value == null:
			return _owner.get_ref()._error("unsupported_node_type", "target does not expose rotation", {"field": normalized, "actual": _runtime.describe_node(node)})
		return {"ok": true, "field": normalized, "value": float(rotation_value)}
	if node is Object and _owner.get_ref()._has_property(node, normalized):
		var value = (node as Object).get(normalized)
		var numeric = _numeric_or_vector_magnitude(value)
		if numeric != null:
			return {"ok": true, "field": normalized, "value": float(numeric)}
		return _owner.get_ref()._error("property_not_numeric", "requested within_limits property is not numeric", {"field": normalized, "expected": "number or vector", "actual": str(value)})
	return _owner.get_ref()._error("property_not_found", "target does not expose requested within_limits field", {"field": normalized, "expected": "property or supported runtime field", "actual": _runtime.describe_node(node)})


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


func _attach_temporal_evidence(kind: String, target: String, params: Dictionary, payload: Dictionary) -> Dictionary:
	var evidence_payload := payload.duplicate(true)
	evidence_payload["kind"] = kind
	if payload.has("window") and payload.get("window", {}) is Dictionary:
		var window_payload: Dictionary = payload.get("window", {})
		if window_payload.has("temporal_sampling"):
			evidence_payload["temporal_sampling"] = (window_payload.get("temporal_sampling", {}) as Dictionary).duplicate(true)
		if window_payload.has("cadence"):
			evidence_payload["cadence"] = (window_payload.get("cadence", {}) as Dictionary).duplicate(true)
	var artifacts = _runtime.write_temporal_evidence(kind, target, _merge_evidence_params(params, evidence_payload))
	if not bool(artifacts.get("ok", true)):
		return artifacts
	payload["artifacts"] = artifacts.duplicate(true)
	return {}


func _merge_evidence_params(params: Dictionary, payload: Dictionary) -> Dictionary:
	var merged := payload.duplicate(true)
	if params.has("temporal_dir"):
		merged["temporal_dir"] = str(params.get("temporal_dir", ""))
	return merged


func _temporal_fixed_fps() -> int:
	# Godot consumes --fixed-fps before exposing cmdline args to scripts. Use
	# CLI-resolved runner/session metadata, never the measured render FPS.
	return int(_root_window.get_meta("_godot_qa_temporal_fixed_fps", _session_fixed_fps)) if _root_window != null else _session_fixed_fps


func _temporal_frame_count(frame_value: Variant, second_value: Variant, default_frames: int) -> int:
	if frame_value != null:
		return max(1, int(frame_value))
	if second_value != null:
		var fixed_fps := _temporal_fixed_fps()
		var fps := float(fixed_fps) if fixed_fps > 0 else Engine.get_frames_per_second()
		if fps <= 0.0:
			fps = 60.0
		return max(1, int(ceil(float(second_value) * fps)))
	return max(1, default_frames)
