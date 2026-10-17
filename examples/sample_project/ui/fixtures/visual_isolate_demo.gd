extends Control

const QaRuntime = preload("res://addons/godot_qa/qa_runtime.gd")

const LAYER_CONFIGS := {
	"red_panel": {
		"rect": Rect2(160, 120, 520, 360),
		"color": Color(0.9, 0.16, 0.14, 1.0),
		"z_index": 0,
	},
	"blue_panel": {
		"rect": Rect2(360, 240, 520, 360),
		"color": Color(0.12, 0.36, 0.94, 0.82),
		"z_index": 1,
	},
}

var layer_nodes := {}
var visual_layer_mode := "normal"
var visual_layer_focus := ""


func _ready() -> void:
	set_meta("qa_id", "visual_isolate_demo")
	add_to_group("qa.id.visual_isolate_demo")
	size = get_viewport_rect().size
	for layer_id in LAYER_CONFIGS.keys():
		_create_layer(layer_id, LAYER_CONFIGS[layer_id])
	QaRuntime.register_state_provider("visual_isolate_demo_layers", self, "get_visual_isolate_demo_layers_state", {"fixture": "visual_isolate_demo"})
	QaRuntime.register_fixture_driver("visual_isolate_demo", self, "visual_isolate_demo_fixture_driver", {"fixture": "visual_isolate_demo"})


func _create_layer(layer_id: String, config: Dictionary) -> void:
	var layer := ColorRect.new()
	layer.name = layer_id
	layer.color = config.get("color", Color.WHITE)
	layer.position = (config.get("rect", Rect2()) as Rect2).position
	layer.size = (config.get("rect", Rect2()) as Rect2).size
	layer.z_index = int(config.get("z_index", 0))
	layer.set_meta("qa_id", layer_id)
	layer.add_to_group("qa.id.%s" % layer_id)
	add_child(layer)
	layer_nodes[layer_id] = layer


func get_visual_isolate_demo_layers_state() -> Dictionary:
	var layers: Array[Dictionary] = []
	for layer_id in LAYER_CONFIGS.keys():
		var layer: ColorRect = layer_nodes.get(layer_id)
		layers.append({
			"id": layer_id,
			"visible": layer != null and layer.visible,
			"visible_in_normal_gameplay": true,
			"draw_intent": "sample overlapping colored rectangle",
			"renderer": "ColorRect",
			"draws_filled_rectangles": true,
			"can_create_chunk_cutoff": false,
		})
	return {
		"provider": "visual_isolate_demo_layers",
		"mode": visual_layer_mode,
		"focus_layer": visual_layer_focus,
		"layers": layers,
		"no_visible_debug_overlay": true,
	}


func visual_isolate_demo_fixture_driver(request: Dictionary) -> Dictionary:
	var action := str(request.get("action", ""))
	var params: Dictionary = request.get("params", {}) if request.get("params", {}) is Dictionary else {}
	if action == "set_visual_layer_mode":
		return {
			"ok": true,
			"data": set_visual_layer_mode(str(params.get("mode", "normal")), str(params.get("layer", ""))),
		}
	return {
		"ok": false,
		"error": {
			"type": "unknown_fixture_action",
			"message": "visual isolate demo fixture action is not supported",
			"data": {"action": action},
		},
	}


func set_visual_layer_mode(mode: String, layer_id: String = "") -> Dictionary:
	visual_layer_mode = mode
	visual_layer_focus = layer_id
	for id in layer_nodes.keys():
		var layer: ColorRect = layer_nodes[id]
		layer.visible = true
	if mode == "only":
		for id in layer_nodes.keys():
			var layer: ColorRect = layer_nodes[id]
			layer.visible = id == layer_id
	elif mode == "without":
		var layer: ColorRect = layer_nodes.get(layer_id)
		if layer != null:
			layer.visible = false
	elif mode != "normal" and mode != "all":
		visual_layer_mode = "normal"
		visual_layer_focus = ""
	return get_visual_isolate_demo_layers_state()
