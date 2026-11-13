extends RefCounted

const QaSnapshot = preload("res://addons/godot_qa/qa_snapshot.gd")
const QaJsonEquality = preload("res://addons/godot_qa/qa_json_equality.gd")
const STATE_RECORD_MAX_VALUE_BYTES := 65536

var _owner: WeakRef
var _runtime: RefCounted:
	get:
		return _owner.get_ref()._runtime
var _runtime_errors: Array:
	get:
		return _owner.get_ref()._runtime_errors


func _init(owner: RefCounted) -> void:
	_owner = weakref(owner)


func _assert_state_matches(params: Dictionary) -> Dictionary:
	var source := str(params.get("source", "auto")).strip_edges()
	if source == "provider":
		var state_result := _resolve_state_source(null, params, "", "")
		if not bool(state_result.get("ok", false)):
			return state_result
		return _assert_state_result_matches(params, state_result, "", "")
	var target := str(params.get("target", "")).strip_edges()
	if target == "":
		return _owner.get_ref()._error("invalid_assertion_args", "state_matches requires a non-empty target", {"command": "assert.state_matches", "field": "target", "actual": params.get("target", null)})
	var resolution = _owner.get_ref()._require_target(target)
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
		return _owner.get_ref()._error("invalid_assertion_args", "state_matches requires path/operator or a non-empty matches array", {"command": "assert.state_matches", "target": target, "source": state_result.get("source", ""), "actual": params})
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
		return _owner.get_ref()._ok(payload)
	payload["mismatches"] = mismatches
	payload["expected"] = "all state checks match"
	payload["actual"] = mismatches
	return _owner.get_ref()._error("state_matches_mismatch", "runtime state did not match expected checks", payload)


func _qa_state_read(params: Dictionary) -> Dictionary:
	var provider := str(params.get("provider", params.get("provider_id", ""))).strip_edges()
	if provider != "":
		var provider_result: Dictionary = _runtime.read_state_provider(provider)
		if not bool(provider_result.get("ok", false)):
			return provider_result
		return _owner.get_ref()._ok({
			"provider": provider_result.get("provider", provider),
			"source": "provider",
			"method": provider_result.get("method", ""),
			"state": provider_result.get("state", null),
		})
	var target := str(params.get("target", "")).strip_edges()
	if target == "":
		return _owner.get_ref()._error("invalid_assertion_args", "qa_state.read requires a non-empty target", {"command": "qa_state.read", "field": "target", "actual": params.get("target", null)})
	var method := str(params.get("method", "")).strip_edges()
	if method == "":
		return _owner.get_ref()._error("invalid_assertion_args", "qa_state.read requires a non-empty method", {"command": "qa_state.read", "target": target, "field": "method", "actual": params.get("method", null)})
	var resolution = _owner.get_ref()._require_target(target)
	if not bool(resolution.get("ok", false)):
		return resolution
	var state_result := _call_state_method(resolution.get("node"), method, "method", target, str(resolution.get("resolved", "")))
	if not bool(state_result.get("ok", false)):
		return state_result
	return _owner.get_ref()._ok({
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
		return _owner.get_ref()._error("state_record_invalid_args", "state.record requires a non-empty name", {"command": "state.record", "field": "name", "actual": params.get("name", null)})
	var source := str(params.get("source", "auto")).strip_edges()
	var target := ""
	var resolved := ""
	var state_result: Dictionary
	if source == "provider":
		state_result = _resolve_state_source(null, params, "", "")
	else:
		target = str(params.get("target", "")).strip_edges()
		if target == "":
			return _owner.get_ref()._error("state_record_invalid_args", "state.record method source requires a non-empty target", {"command": "state.record", "name": name, "field": "target", "actual": params.get("target", null)})
		var resolution = _owner.get_ref()._require_target(target)
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
		return _owner.get_ref()._error("state_record_value_too_large", "state.record value exceeded the bounded evidence size", {
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
	return _owner.get_ref()._ok(record)


func _resolve_state_source(node: Node, params: Dictionary, target: String = "", resolved: String = "") -> Dictionary:
	var source := str(params.get("source", "auto")).strip_edges()
	if source == "":
		source = "auto"
	if source == "provider":
		var provider := str(params.get("provider", params.get("provider_id", ""))).strip_edges()
		if provider == "":
			return _owner.get_ref()._error("invalid_assertion_args", "state_matches source: provider requires a non-empty provider", {"command": "assert.state_matches", "field": "provider", "actual": params.get("provider", null)})
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
		return _owner.get_ref()._error("target_freed", "state_matches target was freed before state could be read", {"command": "assert.state_matches", "target": target, "resolved": resolved})
	var method := str(params.get("method", "")).strip_edges()
	if source == "method" or source == "qa_method" or method != "":
		return _call_state_method(node, method, source, target, resolved)
	if source == "metadata" or source == "meta" or source == "auto":
		return {"ok": true, "source": "metadata", "method": "", "state": _node_metadata_state(node)}
	if source == "inspect":
		return {"ok": true, "source": "inspect", "method": "", "state": QaSnapshot.node_to_dict(node)}
	return _owner.get_ref()._error("invalid_assertion_args", "state_matches source must be metadata, inspect, method, provider, or auto", {"command": "assert.state_matches", "field": "source", "actual": source})


func _call_state_method(node: Node, method: String, source: String, target: String = "", resolved: String = "") -> Dictionary:
	var context := _state_method_error_context(node, method, source, target, resolved)
	if method == "":
		context["field"] = "method"
		context["actual"] = method
		context["rejection_reason"] = "name"
		context["expected"] = "non-empty method matching allowed_patterns"
		return _owner.get_ref()._error("invalid_assertion_args", "state_matches method source requires a method name", context)
	var name_check := _validate_state_method_name(method)
	if not bool(name_check.get("ok", false)):
		context["field"] = "method"
		context["actual"] = method
		context["rejection_reason"] = "name"
		context["expected"] = "method matching allowed_patterns"
		context["name_rejection_reason"] = str(name_check.get("name_rejection_reason", "invalid_name"))
		return _owner.get_ref()._error("invalid_assertion_args", "state_matches method must match an approved read-only QA state pattern", context)
	if not node.has_method(method):
		context["rejection_reason"] = "missing_method"
		context["expected"] = "resolved target exposes requested QA state method"
		context["actual"] = "method_not_found"
		return _owner.get_ref()._error("missing_state_source", "state_matches target does not expose the requested QA method", context)
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
		return _owner.get_ref()._error("invalid_assertion_args", "state_matches QA methods must take zero arguments", context)
	var runtime_error_count_before := _runtime_errors.size()
	var value = node.call(method)
	if _runtime_errors.size() > runtime_error_count_before:
		var runtime_errors: Array = _runtime_errors.slice(runtime_error_count_before)
		context["rejection_reason"] = "runtime_error"
		context["expected"] = "QA state method returns without recording runtime errors"
		context["actual"] = runtime_errors.duplicate(true)
		return _owner.get_ref()._error("runtime_errors_present", "state_matches QA method recorded runtime errors", context)
	if not _owner.get_ref()._json_compatible(value):
		context["rejection_reason"] = "non_json_return"
		context["expected"] = "JSON-compatible Dictionary or Array"
		context["actual"] = "non_json_compatible_return"
		context["actual_type"] = type_string(typeof(value))
		return _owner.get_ref()._error("invalid_state_source", "state_matches QA method returned non JSON-compatible data", context)
	if not (value is Dictionary or value is Array):
		context["rejection_reason"] = "non_json_return"
		context["expected"] = "Dictionary or Array"
		context["actual"] = value
		context["actual_type"] = type_string(typeof(value))
		return _owner.get_ref()._error("invalid_state_source", "state_matches QA method must return a Dictionary or Array", context)
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
		var equals := QaJsonEquality.equals(actual, expected)
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
		if not QaJsonEquality.equals(actual_result.get("value"), expected):
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
		for item in actual:
			if QaJsonEquality.equals(item, expected):
				return true
		return false
	if actual is Dictionary:
		var dict: Dictionary = actual
		if expected is Dictionary:
			for key in (expected as Dictionary).keys():
				if not dict.has(key) or not QaJsonEquality.equals(dict.get(key), (expected as Dictionary).get(key)):
					return false
			return true
		return dict.has(expected)
	return false


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
			return _owner.get_ref()._error("invalid_assertion_args", "temporal state observations with source: provider require a non-empty provider", {"command": command, "field": "provider", "actual": params.get("provider", null)})
		return {"ok": true, "mode": "provider", "provider": provider, "path": state_path, "label": provider}
	if source == "method" or source == "qa_method":
		var target := str(params.get("target", "")).strip_edges()
		if target == "":
			return _owner.get_ref()._error("invalid_assertion_args", "temporal state observations with source: method require a non-empty target", {"command": command, "field": "target", "actual": params.get("target", null)})
		var method := str(params.get("method", "")).strip_edges()
		if method == "":
			return _owner.get_ref()._error("invalid_assertion_args", "temporal state observations with source: method require a non-empty method", {"command": command, "field": "method", "actual": params.get("method", null)})
		var resolution = _owner.get_ref()._require_target(target)
		if not bool(resolution.get("ok", false)):
			return resolution
		return {"ok": true, "mode": "method", "target": target, "resolved": str(resolution.get("resolved", "")), "node": resolution.get("node"), "method": method, "path": state_path, "label": target}
	return _owner.get_ref()._error("invalid_assertion_args", "temporal state observations require source: provider or method", {"command": command, "field": "source", "expected": ["provider", "method"], "actual": source})


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
			return _owner.get_ref()._error("target_freed", "temporal state observation target was freed before state could be read", {"command": command, "target": observation.get("target", ""), "resolved": observation.get("resolved", "")})
		var state_result := _call_state_method(node, str(observation.get("method", "")), "method", str(observation.get("target", "")), str(observation.get("resolved", "")))
		if not bool(state_result.get("ok", false)):
			return state_result
		state = state_result.get("state", null)
	var path_result := _state_path_value(state, observation.get("path", ""))
	if not bool(path_result.get("exists", false)):
		return _owner.get_ref()._error("state_path_missing", "temporal state observation path was missing from the observed state", {"command": command, "path": observation.get("path", ""), "provider": observation.get("provider", ""), "method": observation.get("method", ""), "expected": "state path present in observed state", "actual": null})
	return {"ok": true, "value": path_result.get("value")}


func _read_numeric_temporal_state_value(observation: Dictionary, command: String) -> Dictionary:
	var value_result := _read_temporal_state_value(observation, command)
	if not bool(value_result.get("ok", false)):
		return value_result
	var value = value_result.get("value")
	if not _state_value_is_number(value):
		return _owner.get_ref()._error("state_value_not_numeric", "temporal state observation value is not numeric", {"command": command, "path": observation.get("path", ""), "expected": "number", "actual": value})
	return {"ok": true, "value": float(value)}
