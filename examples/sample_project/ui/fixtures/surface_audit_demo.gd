extends Node2D

const BLOCKED_SPAWN := Vector2(260.0, 120.0)
const SOFT_PATCH := Vector2(210.0, 235.0)
const DEAD_END_CENTER := Vector2(455.0, 190.0)


func _ready() -> void:
	set_meta("qa_surface_probe_method", "qa_surface_probe")


func qa_surface_probe(request: Dictionary) -> Dictionary:
	return _probe_points(request)


func get_qa_surface_state(request: Dictionary) -> Dictionary:
	return _probe_points(request)


func get_surface_qa_state(request: Dictionary) -> Dictionary:
	return _probe_points(request)


func qa_probe_surface(request: Dictionary) -> Dictionary:
	return _probe_points(request)


func _probe_points(request: Dictionary) -> Dictionary:
	var points_value = request.get("points", [])
	var points: Array = points_value if points_value is Array else []
	var samples: Array = []
	for point in points:
		samples.append(_sample_point(point))
	return {"points": samples}


func _sample_point(point: Variant) -> Dictionary:
	var sample_point := _point_from_payload(point)
	var label := "walkable"
	var solid := false
	var soft := false
	var cost := 1.0
	var drag := 0.0
	var push_strength := 0.0
	var metadata := {"case": "walkable"}

	if sample_point.distance_to(BLOCKED_SPAWN) <= 46.0:
		label = "blocked"
		solid = true
		cost = 999.0
		metadata = {"case": "solid_blocked", "cluster": "blocked_spawn"}
	elif sample_point.distance_to(SOFT_PATCH) <= 54.0:
		label = "soft"
		soft = true
		cost = 3.0
		drag = 0.35
		push_strength = 0.15
		metadata = {"case": "soft_blocker", "cluster": "soft_patch"}
	elif abs(sample_point.y - 320.0) <= 18.0 and sample_point.x >= 115.0 and sample_point.x <= 485.0:
		label = "walkable"
		metadata = {"case": "narrow_corridor", "width": 36.0}
	elif sample_point.x >= 115.0 and sample_point.x <= 485.0 and sample_point.y >= 280.0 and sample_point.y <= 360.0:
		label = "blocked"
		solid = true
		cost = 999.0
		metadata = {"case": "corridor_wall"}
	elif sample_point.x >= 405.0 and sample_point.x <= 505.0 and sample_point.y >= 145.0 and sample_point.y <= 255.0:
		label = "walkable"
		cost = 1.5
		metadata = {"case": "dead_end_pocket", "center_distance": sample_point.distance_to(DEAD_END_CENTER)}
	elif sample_point.x >= 430.0 and sample_point.x <= 525.0 and sample_point.y >= 440.0 and sample_point.y <= 525.0:
		label = "blocked"
		solid = true
		cost = 999.0
		metadata = {"case": "mirrored_delta", "side": "right"}
	elif sample_point.x >= 75.0 and sample_point.x <= 170.0 and sample_point.y >= 440.0 and sample_point.y <= 525.0:
		label = "walkable"
		metadata = {"case": "mirrored_delta", "side": "left"}

	return {
		"point": {"x": sample_point.x, "y": sample_point.y},
		"surface": label,
		"label": label,
		"solid": solid,
		"soft": soft,
		"cost": cost,
		"drag": drag,
		"push_strength": push_strength,
		"metadata": metadata,
	}


func _point_from_payload(point: Variant) -> Vector2:
	if not (point is Dictionary):
		return Vector2.ZERO
	var payload: Dictionary = point
	var coordinate_source: Dictionary = payload
	if payload.get("point", null) is Dictionary:
		coordinate_source = payload.get("point")
	return Vector2(float(coordinate_source.get("x", 0.0)), float(coordinate_source.get("y", 0.0)))
