extends SceneTree

const JsonEquality = preload("res://addons/godot_qa/qa_json_equality.gd")
const Runtime = preload("res://addons/godot_qa/qa_runtime.gd")

class Subject extends Node2D:
	var measurement: float = 0.0
	var vector_measurement := Vector2.ZERO
	var resource := Resource.new()
	var companion := RefCounted.new()
	var large_value := "x".repeat(65536)
	var native_values: Array = [Vector2(5, 6), {"resource": Resource.new()}]

var checks: Array = []
var failures: Array = []
var serialized_results: Array = []
var serialized_evidence: Array = []
var runtime: RefCounted
var subject: Subject
var delayed_frames := 0
var delay_mode := ""


func check(name: String, condition: bool, actual: Variant) -> void:
	checks.append(name)
	if not condition:
		failures.append({"name": name, "actual": str(actual).left(2048)})


func _initialize() -> void:
	call_deferred("run")


func delay_frame() -> void:
	delayed_frames += 1
	var delay_ms := 60 if delay_mode == "short" or delayed_frames == 21 else 1
	OS.delay_msec(delay_ms)


func within_params() -> Dictionary:
	return {"target": "qa:subject", "property": "measurement", "min": -1, "max": 1, "window_frames": 1, "temporal_dir": "res://evidence"}


func json_compatible(value: Variant) -> bool:
	match typeof(value):
		TYPE_NIL, TYPE_BOOL, TYPE_INT, TYPE_STRING:
			return true
		TYPE_FLOAT:
			return is_finite(value)
		TYPE_ARRAY:
			for item in value:
				if not json_compatible(item):
					return false
			return true
		TYPE_DICTIONARY:
			for key in value:
				if typeof(key) != TYPE_STRING or not json_compatible(value[key]):
					return false
			return true
	return false


func observe_params(directory: String) -> Dictionary:
	return {"target": "qa:subject", "property": "measurement", "operator": "present", "window_frames": 1, "temporal_dir": directory}


func check_write_failure(result: Dictionary, operation: String, command: String) -> void:
	check("write failure cannot pass", not result.get("ok", true), result)
	check("structured artifact_write_failed", result.get("error", {}).get("type") == "artifact_write_failed", result)
	var data: Dictionary = result.get("error", {}).get("data", {})
	check("write failure identifies command and target", data.get("command") == command and data.get("target") == "qa:subject", result)
	check("write failure identifies path and operation", str(data.get("path", "")).ends_with(".json") and data.get("actual", {}).get("operation") == operation, result)
	check("failed artifact is not claimed", not data.has("artifacts") and not result.get("data", {}).has("artifacts"), result)


func evidence_failure(case: String) -> void:
	var directory := "res://write_failure"
	var operation := "write"
	if case == "evidence_mkdir_failure":
		var blocker := FileAccess.open("res://blocked", FileAccess.WRITE)
		blocker.store_string("directory creation must fail beneath a regular file")
		blocker.close()
		directory = "res://blocked/evidence"
		operation = "mkdir"
	elif case == "evidence_open_failure":
		directory = "res://open_failure"
		DirAccess.make_dir_recursive_absolute(directory + "/observe_until_qa_subject.json")
		operation = "open"
	var params := observe_params(directory)
	if case == "evidence_write_failure":
		params["property"] = "large_value"
	var result: Dictionary = await runtime.execute("assert.observe_until", params)
	check_write_failure(result, operation, "assert.observe_until")
	if operation == "mkdir":
		params["operator"] = "equals"
		params["expected"] = 1
		result = await runtime.execute("assert.observe_until", params)
		check_write_failure(result, operation, "assert.observe_until")
		subject.measurement = 2
		params = within_params()
		params["temporal_dir"] = directory
		result = await runtime.execute("assert.temporal.within_limits", params)
		check_write_failure(result, operation, "assert.temporal.within_limits")


func native_properties() -> void:
	subject.position = Vector2(3, 4)
	for property in ["position", "resource", "companion", "native_values"]:
		var params := observe_params("res://native_evidence")
		params["property"] = property
		var result: Dictionary = await runtime.execute("assert.observe_until", params)
		check("native property present check succeeds", result.get("ok", false), result)
		check("all result values are JSON compatible", json_compatible(result), result)
		var decoded: Variant = JSON.parse_string(JSON.stringify(result))
		check("native property result round-trips", decoded is Dictionary and JsonEquality.equals(decoded, result), result)
		var data: Dictionary = result.get("data", {})
		var actual: Variant = data.get("first_matching_sample", {}).get("actual", {}).get("actual")
		if property == "position":
			check("Vector2 encoded as coordinate array", actual is Array and actual == [3.0, 4.0], actual)
		elif property == "resource" or property == "companion":
			check("Resource/Object encoded as string", actual is String and actual == str(subject.get(property)), actual)
		var path := str(data.get("artifacts", {}).get("temporal_evidence", ""))
		check("success artifact persisted", FileAccess.file_exists(path), data)
		if FileAccess.file_exists(path):
			var evidence: Variant = JSON.parse_string(FileAccess.get_file_as_string(path))
			check("persisted sample matches returned sample", evidence is Dictionary and JsonEquality.equals(evidence.get("first_matching_sample"), data.get("first_matching_sample")), evidence)


func non_finite_properties() -> void:
	var observations := [
		{"property": "measurement", "value": NAN, "expected": str(NAN)},
		{"property": "measurement", "value": INF, "expected": str(INF)},
		{"property": "measurement", "value": -INF, "expected": str(-INF)},
		{"property": "vector_measurement", "value": Vector2(NAN, INF), "expected": [str(NAN), str(INF)]},
		{"property": "native_values", "value": [Vector2(-INF, 4), {"nested": [NAN, Rect2(INF, 2, 3, -INF)]}], "expected": [[str(-INF), 4.0], {"nested": [str(NAN), [str(INF), 2.0, 3.0, str(-INF)]]}]},
	]
	for observation in observations:
		subject.set(observation.property, observation.value)
		var params := observe_params("res://non_finite_evidence")
		params["property"] = observation.property
		var result: Dictionary = await runtime.execute("assert.observe_until", params)
		check("non-finite property is present", result.get("ok", false), result)
		check("result contains no non-finite floats", json_compatible(result), result)
		var encoded := JSON.stringify(result)
		serialized_results.append(encoded)
		var decoded: Variant = JSON.parse_string(encoded)
		check("non-finite result round-trips through JSON", decoded is Dictionary and json_compatible(decoded) and JsonEquality.equals(decoded, result), decoded)
		var data: Dictionary = result.get("data", {})
		var actual: Variant = data.get("first_matching_sample", {}).get("actual", {}).get("actual")
		check("non-finite values encoded recursively as strings", JsonEquality.equals(actual, observation.expected), actual)
		var path := str(data.get("artifacts", {}).get("temporal_evidence", ""))
		check("non-finite evidence persisted", FileAccess.file_exists(path), data)
		if FileAccess.file_exists(path):
			var evidence_json := FileAccess.get_file_as_string(path)
			serialized_evidence.append(evidence_json)
			var evidence: Variant = JSON.parse_string(evidence_json)
			check("persisted non-finite sample matches result", evidence is Dictionary and json_compatible(evidence) and JsonEquality.equals(evidence.get("first_matching_sample"), data.get("first_matching_sample")), evidence)


func run() -> void:
	subject = Subject.new()
	subject.set_meta("qa_id", "subject")
	get_root().add_child(subject)
	runtime = Runtime.new(subject, get_root(), [], {})
	var case := str(OS.get_cmdline_user_args()[0])
	match case:
		"evidence_mkdir_failure", "evidence_open_failure", "evidence_write_failure":
			await evidence_failure(case)
		"native_properties":
			await native_properties()
		"non_finite_properties":
			await non_finite_properties()
		"non_finite_observations":
			for value in [NAN, INF, -INF]:
				subject.measurement = value
				var result: Dictionary = await runtime.execute("assert.temporal.within_limits", within_params())
				check("non-finite observation fails", not result.get("ok", true), result)
				check("observation has structured mismatch", result.get("error", {}).get("type") == "within_limits_mismatch", result)
				var decoded: Variant = JSON.parse_string(JSON.stringify(result))
				check("observation evidence survives JSON", decoded is Dictionary and decoded.get("ok") == false, decoded)
		"non_finite_bounds":
			for bound in ["min", "max"]:
				for value in [NAN, INF, -INF]:
					var params := within_params()
					params[bound] = value
					var result: Dictionary = await runtime.execute("assert.temporal.within_limits", params)
					check("non-finite bound fails", not result.get("ok", true), result)
					check("bound has structured invalid_bounds", result.get("error", {}).get("type") == "invalid_bounds", result)
		"short_window":
			delay_mode = "short"
			process_frame.connect(delay_frame)
			var result: Dictionary = await runtime.execute("assert.temporal.frame_time", {"window_frames": 1, "sample_every_frames": 60, "max_avg_ms": 30, "temporal_dir": "res://evidence"})
			process_frame.disconnect(delay_frame)
			check("slow short window fails", not result.get("ok", true), result)
			check("one frame measured without cadence dilution", float(result.get("data", result.get("error", {}).get("data", {})).get("aggregates", {}).get("avg_ms", 0)) >= 50, result)
		"remainder_window":
			delay_mode = "remainder"
			process_frame.connect(delay_frame)
			var start_usec := Time.get_ticks_usec()
			var result: Dictionary = await runtime.execute("assert.temporal.frame_time", {"window_frames": 21, "sample_every_frames": 20, "max_avg_ms": 1000})
			var elapsed_ms := float(Time.get_ticks_usec() - start_usec) / 1000.0
			process_frame.disconnect(delay_frame)
			var samples: Array = result.get("data", {}).get("samples", [])
			check("remainder yields two intervals", samples.size() == 2, result)
			if samples.size() == 2:
				var first := float(samples[0].frame_time_ms)
				var last := float(samples[1].frame_time_ms)
				var aggregates: Dictionary = result.get("data", result.get("error", {}).get("data", {})).get("aggregates", {})
				check("remainder divisor is one frame", last >= 50, result)
				check("average weighted by advanced frames", abs(float(aggregates.get("avg_ms", 0)) - (first * 20 + last) / 21.0) < 0.01, result)
				check("average matches whole-window elapsed time", abs(float(aggregates.get("avg_ms", 0)) - elapsed_ms / 21.0) < 2, result)
				check("percentile weighted by advanced frames", is_equal_approx(float(aggregates.get("p95_ms", 0)), first), result)
				check("max includes full remainder duration", is_equal_approx(float(aggregates.get("max_ms", 0)), last), result)
	var report := FileAccess.open("res://report.json", FileAccess.WRITE)
	report.store_string(JSON.stringify({"checks": checks, "failures": failures, "serialized_results": serialized_results, "serialized_evidence": serialized_evidence}))
	report.close()
	runtime.teardown()
	runtime = null
	quit(0 if failures.is_empty() else 1)
