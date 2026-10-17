extends RefCounted


func run_qa_test() -> Dictionary:
	var scene: PackedScene = load("res://ui/main_menu.tscn")
	if scene == null:
		return _failure("failed to load main menu scene fixture")
	var node: Node = scene.instantiate()
	if node == null:
		return _failure("failed to instantiate main menu scene fixture")
	if node.has_method("_create_temporal_probe_nodes"):
		node.call("_create_temporal_probe_nodes")

	var actor: Node = node.find_child("QADemoActor", true, false)
	if actor == null:
		node.queue_free()
		return _failure("missing QADemoActor surface fixture node")

	var probe_method := str(actor.get_meta("qa_surface_probe_method", ""))
	if probe_method != "qa_probe_surface":
		node.queue_free()
		return _failure("unexpected qa_surface_probe_method: %s" % probe_method)
	if not actor.has_method(probe_method):
		node.queue_free()
		return _failure("surface probe method not found on fixture: %s" % probe_method)

	var probe_result: Variant = actor.call(probe_method, {
		"coordinate_space": "world_2d",
		"points": [
			{"x": 120, "y": 420},
		],
	})
	node.queue_free()

	if not probe_result is Dictionary:
		return _failure("surface probe result was not a Dictionary")
	var result: Dictionary = probe_result
	var allowed: Array = result.get("allowed_surfaces", [])
	if allowed.is_empty():
		return _failure("surface probe allowed_surfaces is empty")
	for value in allowed:
		if not value is String:
			return _failure("surface probe allowed_surfaces must be String values")

	return {
		"ok": true,
		"failures": [],
		"data": {
			"fixture": "script_smoke",
			"surface_provider_method": probe_method,
			"allowed_surfaces": allowed,
		},
	}


func _failure(message: String) -> Dictionary:
	return {
		"ok": false,
		"failures": [
			{
				"message": message,
			}
		],
		"data": {
			"fixture": "script_smoke",
		},
	}
