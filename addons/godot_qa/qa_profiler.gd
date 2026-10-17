extends RefCounted

static var _active_stack := []
static var _stats := {}
static var _enabled := false


static func reset() -> void:
	_active_stack.clear()
	_stats.clear()


static func set_enabled(enabled: bool) -> void:
	_enabled = enabled
	if not _enabled:
		_active_stack.clear()


static func is_enabled() -> bool:
	return _enabled


static func begin(label: String) -> void:
	if not _enabled:
		return
	var normalized := label.strip_edges()
	if normalized == "":
		return
	_active_stack.append({
		"label": normalized,
		"started_usec": Time.get_ticks_usec(),
		"child_usec": 0,
	})


static func end(expected_label := "") -> Dictionary:
	if not _enabled:
		return {}
	if _active_stack.is_empty():
		return {}
	var entry: Dictionary = _active_stack.pop_back()
	var label := str(entry.get("label", ""))
	if expected_label.strip_edges() != "" and label != expected_label.strip_edges():
		label = "%s (ended as %s)" % [label, expected_label.strip_edges()]
	var duration_usec := max(Time.get_ticks_usec() - int(entry.get("started_usec", 0)), 0)
	var child_usec := max(int(entry.get("child_usec", 0)), 0)
	var self_usec := max(duration_usec - child_usec, 0)
	if not _active_stack.is_empty():
		var parent: Dictionary = _active_stack[_active_stack.size() - 1]
		parent["child_usec"] = int(parent.get("child_usec", 0)) + duration_usec
	return _record(label, duration_usec, self_usec)


static func time(label: String, callable: Callable, args: Array = []) -> Variant:
	if not callable.is_valid():
		return null
	if not _enabled:
		return callable.callv(args)
	begin(label)
	var result = callable.callv(args)
	end(label)
	return result


static func snapshot(top := 10, reset_after := false) -> Dictionary:
	var methods: Array[Dictionary] = []
	var total_call_count := 0
	for label in _stats.keys():
		var stat: Dictionary = _stats.get(label, {})
		var call_count := int(stat.get("call_count", 0))
		total_call_count += call_count
		var total_ms := float(stat.get("total_usec", 0)) / 1000.0
		var self_ms := float(stat.get("self_usec", 0)) / 1000.0
		methods.append({
			"label": str(label),
			"call_count": call_count,
			"total_ms": total_ms,
			"self_ms": self_ms,
			"avg_ms": total_ms / float(max(call_count, 1)),
			"max_ms": float(stat.get("max_usec", 0)) / 1000.0,
			"last_ms": float(stat.get("last_usec", 0)) / 1000.0,
		})
	methods.sort_custom(func(a: Dictionary, b: Dictionary) -> bool:
		var total_a := float(a.get("total_ms", 0.0))
		var total_b := float(b.get("total_ms", 0.0))
		if total_a == total_b:
			return int(a.get("call_count", 0)) > int(b.get("call_count", 0))
		return total_a > total_b
	)
	var limit := max(top, 0)
	if limit > 0 and methods.size() > limit:
		methods = methods.slice(0, limit)
	var payload := {
		"status": "available",
		"instrumentation": "opt_in",
		"method_count": _stats.size(),
		"sample_count": total_call_count,
		"methods": methods,
	}
	if reset_after:
		reset()
	return payload


static func _record(label: String, duration_usec: int, self_usec: int) -> Dictionary:
	if label == "":
		return {}
	var stat: Dictionary = _stats.get(label, {
		"call_count": 0,
		"total_usec": 0,
		"self_usec": 0,
		"max_usec": 0,
		"last_usec": 0,
	})
	stat["call_count"] = int(stat.get("call_count", 0)) + 1
	stat["total_usec"] = int(stat.get("total_usec", 0)) + duration_usec
	stat["self_usec"] = int(stat.get("self_usec", 0)) + self_usec
	stat["max_usec"] = max(int(stat.get("max_usec", 0)), duration_usec)
	stat["last_usec"] = duration_usec
	_stats[label] = stat
	return {
		"label": label,
		"duration_ms": float(duration_usec) / 1000.0,
		"self_ms": float(self_usec) / 1000.0,
	}
