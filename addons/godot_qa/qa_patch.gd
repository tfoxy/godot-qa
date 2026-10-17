extends RefCounted

const PATCH_FORMAT := "godot-qa.patch.v1"

const _CANVAS_ITEM_PROPERTIES := {
	"visible": true,
	"modulate": true,
	"self_modulate": true,
}

const _CONTROL_PROPERTIES := {
	"custom_minimum_size": true,
	"offset_left": true,
	"offset_top": true,
	"offset_right": true,
	"offset_bottom": true,
	"focus_mode": true,
}


static func is_property_supported(node: Node, property_name: String) -> bool:
	if node is BaseButton and property_name in {"text": true, "disabled": true}:
		return true
	if node is Label and property_name == "text":
		return true
	if node is Control and _CONTROL_PROPERTIES.has(property_name):
		return true
	if node is CanvasItem and _CANVAS_ITEM_PROPERTIES.has(property_name):
		return true
	return false


static func is_method_supported(node: Node, method_name: String) -> bool:
	return method_name == "grab_focus" and node is Control


static func encode_value(value: Variant) -> Variant:
	match typeof(value):
		TYPE_NIL, TYPE_BOOL, TYPE_INT, TYPE_FLOAT, TYPE_STRING:
			return value
		TYPE_VECTOR2:
			var vector := value as Vector2
			return {"type": "Vector2", "value": [vector.x, vector.y]}
		TYPE_VECTOR2I:
			var vectori := value as Vector2i
			return {"type": "Vector2i", "value": [vectori.x, vectori.y]}
		TYPE_COLOR:
			var color := value as Color
			return {"type": "Color", "value": [color.r, color.g, color.b, color.a]}
	return null


static func decode_patch_value(value: Variant) -> Dictionary:
	if typeof(value) in [TYPE_NIL, TYPE_BOOL, TYPE_INT, TYPE_FLOAT, TYPE_STRING]:
		return {"ok": true, "value": value}
	if not value is Dictionary:
		return {
			"ok": false,
			"type": "patch_invalid",
			"message": "patch value must be a supported JSON scalar or typed object",
			"expected": "JSON scalar or typed object",
			"actual": type_string(typeof(value)),
		}
	var typed_value := value as Dictionary
	var type_name := str(typed_value.get("type", ""))
	var raw_value = typed_value.get("value")
	match type_name:
		"Vector2":
			if raw_value is Array and raw_value.size() == 2:
				return {"ok": true, "value": Vector2(float(raw_value[0]), float(raw_value[1]))}
		"Vector2i":
			if raw_value is Array and raw_value.size() == 2:
				return {"ok": true, "value": Vector2i(int(raw_value[0]), int(raw_value[1]))}
		"Color":
			if raw_value is Array and raw_value.size() == 4:
				return {"ok": true, "value": Color(float(raw_value[0]), float(raw_value[1]), float(raw_value[2]), float(raw_value[3]))}
	return {
		"ok": false,
		"type": "patch_invalid",
		"message": "patch value type is not supported",
		"expected": ["String", "bool", "int", "float", "Vector2", "Vector2i", "Color"],
		"actual": value,
	}


static func coerce_live_value(current_value: Variant, value_input: String) -> Dictionary:
	var trimmed := value_input.strip_edges()
	match typeof(current_value):
		TYPE_STRING:
			return {"ok": true, "value": value_input}
		TYPE_BOOL:
			if trimmed.to_lower() in ["true", "1", "yes", "on"]:
				return {"ok": true, "value": true}
			if trimmed.to_lower() in ["false", "0", "no", "off"]:
				return {"ok": true, "value": false}
		TYPE_INT:
			if trimmed.is_valid_int():
				return {"ok": true, "value": int(trimmed)}
		TYPE_FLOAT:
			if trimmed.is_valid_float():
				return {"ok": true, "value": float(trimmed)}
		TYPE_VECTOR2:
			var vector2 := _parse_vector(value_input, false)
			if vector2.get("ok", false):
				return {"ok": true, "value": vector2.get("value")}
		TYPE_VECTOR2I:
			var vector2i := _parse_vector(value_input, true)
			if vector2i.get("ok", false):
				return {"ok": true, "value": vector2i.get("value")}
		TYPE_COLOR:
			var color_result := _parse_color(value_input)
			if color_result.get("ok", false):
				return {"ok": true, "value": color_result.get("value")}
	var encoded := encode_value(current_value)
	return {
		"ok": false,
		"type": "edit_property_value_invalid",
		"message": "live edit value could not be coerced to the target property type",
		"expected": encoded if encoded != null else type_string(typeof(current_value)),
		"actual": value_input,
	}


static func _parse_vector(value_input: String, as_int := false) -> Dictionary:
	var parts := value_input.split(",", false)
	if parts.size() != 2:
		return {"ok": false}
	var first := parts[0].strip_edges()
	var second := parts[1].strip_edges()
	if as_int:
		if first.is_valid_int() and second.is_valid_int():
			return {"ok": true, "value": Vector2i(int(first), int(second))}
		return {"ok": false}
	if first.is_valid_float() and second.is_valid_float():
		return {"ok": true, "value": Vector2(float(first), float(second))}
	return {"ok": false}


static func _parse_color(value_input: String) -> Dictionary:
	var trimmed := value_input.strip_edges()
	if Color.html_is_valid(trimmed):
		return {"ok": true, "value": Color.from_string(trimmed, Color.BLACK)}
	var parts := trimmed.split(",", false)
	if parts.size() not in [3, 4]:
		return {"ok": false}
	for part in parts:
		if not String(part).strip_edges().is_valid_float():
			return {"ok": false}
	var red := float(String(parts[0]).strip_edges())
	var green := float(String(parts[1]).strip_edges())
	var blue := float(String(parts[2]).strip_edges())
	var alpha := 1.0
	if parts.size() == 4:
		alpha = float(String(parts[3]).strip_edges())
	return {"ok": true, "value": Color(red, green, blue, alpha)}
