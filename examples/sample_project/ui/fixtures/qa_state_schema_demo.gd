extends Control

const QaRuntime = preload("res://addons/godot_qa/qa_runtime.gd")

var sample_frame := 0
var provider_status := "ready"


func _ready() -> void:
	set_meta("qa_id", "schema_state_provider")
	add_to_group("schema_state_provider")
	add_to_group("qa.id.schema_state_provider")
	sample_frame = 12
	QaRuntime.register_state_provider("schema_demo", self, "schema_provider_state", {"fixture": "qa_state_schema_demo"})
	QaRuntime.register_probe_provider("schema_surface", self, "schema_surface_probe", {"kind": "surface"})
	QaRuntime.register_fixture_driver("schema_driver", self, "schema_fixture_driver", {"fixture": "qa_state_schema_demo"})


func schema_provider_state() -> Dictionary:
	var state := get_qa_schema_demo_state()
	state["status"] = provider_status
	state["provider_contract"] = "registered_state_provider"
	return state


func schema_surface_probe(request: Dictionary) -> Dictionary:
	var points: Array = []
	for raw_point in request.get("points", []):
		if not (raw_point is Dictionary):
			continue
		var point: Dictionary = raw_point
		var entry := point.duplicate(true)
		entry["label"] = "water"
		entry["surface"] = "water"
		entry["walkable"] = true
		if point.has("expected"):
			entry["expected"] = str(point.get("expected", ""))
		points.append(entry)
	return {
		"provider_id": request.get("provider_id", "schema_surface"),
		"points": points,
	}


func schema_fixture_driver(request: Dictionary) -> Dictionary:
	var action := str(request.get("action", ""))
	var params: Dictionary = request.get("params", {}) if request.get("params", {}) is Dictionary else {}
	if action == "set_status":
		provider_status = str(params.get("status", "ready"))
		return {"ok": true, "data": {"status": provider_status}}
	# Emit ONE real engine-level error from game code, so a scenario can prove the
	# CLI reports `godot_log_error` identically for a cold run (which owns the
	# whole process log) and a warm run (which is sliced out of a shared pooled
	# session log). push_error is what a game uses for a genuine fault, so this is
	# the faithful hook — it must never fire unless a scenario asks for it.
	if action == "push_engine_error":
		var message := str(params.get("message", "qa_state_schema_demo engine error probe"))
		push_error(message)
		return {"ok": true, "data": {"pushed": message}}
	return {
		"ok": false,
		"error": {
			"type": "unknown_fixture_action",
			"message": "schema fixture driver action is not supported",
			"data": {"action": action},
		},
	}


func get_qa_schema_demo_state() -> Dictionary:
	return {
		"fixture": "qa_state_schema_demo",
		"schema_version": 1,
		"status": "ready",
		"score": 42.5,
		"frame": sample_frame,
		"tags": ["schema", "demo"],
		"metrics": {
			"health": 95,
			"stamina": 64,
		},
		"controls": {
			"focused": true,
			"enabled": true,
		},
		"samples": [
			{
				"name": "baseline",
				"value": 1.25,
			},
			{
				"name": "settled",
				"value": 2.5,
			},
		],
	}


func get_bad_qa_schema_demo_state() -> Dictionary:
	return {
		"fixture": "qa_state_schema_demo",
		"schema_version": 1,
		"status": "broken",
		"score": "too-high",
		"frame": sample_frame,
		"tags": ["schema", "demo"],
		"metrics": {
			"health": 95,
			"stamina": 64,
		},
		"controls": {
			"focused": true,
			"enabled": true,
		},
		"samples": [
			{
				"name": "baseline",
				"value": 1.25,
			},
		],
	}
