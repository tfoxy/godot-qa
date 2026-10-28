extends RefCounted
## Request-bound assertion ledger. Legacy requests without a marker remain supported.

var enabled := false
var _entries: Array[Dictionary] = []
var _by_path := {}
var _run := {}
var _excluded := 0


func initialize(request: Dictionary) -> Dictionary:
	enabled = request.has("assertion_proof")
	if not enabled:
		return {}
	_run = request.get("run", {})
	var manifest = request.get("assertion_proof")
	if not manifest is Dictionary or manifest.get("schema_version") != 1 or not manifest.get("entries") is Array:
		return {"message": "assertion_proof must contain schema_version 1 and entries"}
	if str(_run.get("id", "")) == "" or str(_run.get("nonce", "")) == "":
		return {"message": "assertion proof requires run id and nonce"}
	var excluded = manifest.get("cli_assertions_excluded")
	if not (excluded is int or excluded is float) or excluded < 0 or float(excluded) != floor(float(excluded)):
		return {"message": "assertion proof requires a nonnegative CLI exclusion count"}
	_excluded = int(excluded)
	var scenario = request.get("scenario", {})
	if not scenario is Dictionary or not scenario.get("steps", []) is Array or not scenario.get("assert", []) is Array:
		return {"message": "assertion proof requires scenario step and assertion arrays"}
	for step in scenario.get("steps", []):
		if step is Dictionary and not step.get("assert", []) is Array:
			return {"message": "assertion proof requires step assertion arrays"}
	var actual := _runtime_assertions(request.get("scenario", {}))
	if actual.size() != manifest.entries.size():
		return {"message": "assertion proof does not cover every runtime assertion"}
	var source_ids := {}
	for index in range(actual.size()):
		var entry = manifest.entries[index]
		var assertion: Dictionary = actual[index]
		if not entry is Dictionary:
			return {"message": "assertion proof entry must be an object"}
		if not assertion.assertion is Dictionary:
			return {"message": "assertion proof requires assertion objects"}
		var source_id := str(entry.get("source_id", ""))
		var runtime_path := str(entry.get("runtime_path", ""))
		if _has_unrepresentable_integer(entry.get("assertion_identity", [])):
			return {"message": "assertion contains an integer that cannot be represented exactly by Godot JSON binary64", "source_id": source_id}
		var identity: Array = definition_identity(assertion.assertion)
		if source_id == "" or source_ids.has(source_id) or runtime_path != assertion.path or entry.get("assertion_identity") != identity or entry.get("operation") != _operation(assertion.assertion):
			return {"message": "assertion proof differs from runtime assertion definitions; check matching CLI/addon revisions and Godot JSON numeric precision", "source_id": source_id, "runtime_path": runtime_path}
		source_ids[source_id] = true
		_by_path[runtime_path] = _entries.size()
		_entries.append({"source_id": source_id, "runtime_path": runtime_path, "operation": str(entry.operation), "assertion": assertion.assertion.duplicate(true), "assertion_identity": identity, "status": "not_started"})
	return {}


func begin(runtime_path: String) -> String:
	if not enabled or not _by_path.has(runtime_path):
		return ""
	var index := int(_by_path[runtime_path])
	_entries[index].status = "running"
	return str(_entries[index].source_id)


func finish(runtime_path: String, passed: bool) -> void:
	if enabled and _by_path.has(runtime_path):
		_entries[int(_by_path[runtime_path])].status = "passed" if passed else "failed"


func payload() -> Dictionary:
	var started := 0
	var passed := 0
	var failed := 0
	for entry in _entries:
		if entry.status != "not_started":
			started += 1
		if entry.status == "passed":
			passed += 1
		elif entry.status == "failed":
			failed += 1
	return {"schema_version": 1, "run_id": str(_run.get("id", "")), "nonce": str(_run.get("nonce", "")), "expected": _entries.size(), "started": started, "finished": passed + failed, "passed": passed, "failed": failed, "complete": passed + failed == _entries.size(), "cli_assertions_excluded": _excluded, "entries": _entries.duplicate(true)}


func _runtime_assertions(scenario: Dictionary) -> Array[Dictionary]:
	var result: Array[Dictionary] = []
	var steps = scenario.get("steps", [])
	for step_index in range(steps.size()):
		var assertions = steps[step_index].get("assert", []) if steps[step_index] is Dictionary else []
		for assertion_index in range(assertions.size()):
			result.append({"path": "/steps/%d/assert/%d" % [step_index, assertion_index], "assertion": assertions[assertion_index]})
	var assertions = scenario.get("assert", [])
	for assertion_index in range(assertions.size()):
		result.append({"path": "/assert/%d" % assertion_index, "assertion": assertions[assertion_index]})
	return result


func _operation(assertion: Variant) -> String:
	if assertion is Dictionary:
		for key in assertion.keys():
			if str(key) != "temporal":
				return str(key)
	return ""


func definition_identity(value: Variant) -> Array:
	if value == null:
		return ["null"]
	if value is bool:
		return ["bool", value]
	if value is int or value is float:
		var bytes := PackedByteArray()
		bytes.resize(8)
		bytes.encode_double(0, 0.0 if float(value) == 0.0 else float(value))
		return ["number", bytes.hex_encode()]
	if value is String:
		return ["string", value]
	if value is Array:
		var items: Array = []
		for item in value:
			items.append(definition_identity(item))
		return ["array", items]
	var fields := {}
	for key in value.keys():
		fields[str(key)] = definition_identity(value[key])
	return ["object", fields]


func _has_unrepresentable_integer(identity: Variant) -> bool:
	if identity is Array:
		if not identity.is_empty() and identity[0] is String and identity[0] == "unrepresentable_integer":
			return true
		for item in identity:
			if _has_unrepresentable_integer(item):
				return true
	elif identity is Dictionary:
		for item in identity.values():
			if _has_unrepresentable_integer(item):
				return true
	return false
