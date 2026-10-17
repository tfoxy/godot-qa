extends CharacterBody2D

const ROLE_SCENE := "scene"
const ROLE_SLIDE := "slide_actor"
const ROLE_BOUNCY := "bouncy_actor"
const ROLE_SOFT := "soft_actor"
const ROLE_DAMPING := "damping_actor"
const ROLE_STUCK := "stuck_actor"

const ACTOR_ROLES := [
	ROLE_SLIDE,
	ROLE_BOUNCY,
	ROLE_SOFT,
	ROLE_DAMPING,
	ROLE_STUCK,
]

var frame_index := 0
var rescue_push_count := 0
var _initial_positions: Dictionary = {}


func _ready() -> void:
	var role := _role()
	if role == ROLE_SCENE:
		set_meta("qa_id", "collision_report_scene")
		add_to_group("qa.id.collision_report_scene")
		for actor_role in ACTOR_ROLES:
			var actor := _actor(actor_role)
			if actor == null:
				continue
			actor.set_meta("collision_report_role", actor_role)
			actor.set_meta("qa_id", actor_role)
			actor.add_to_group("qa.id.%s" % actor_role)
			_initial_positions[actor_role] = actor.global_position
	else:
		set_meta("qa_id", role)
		add_to_group("qa.id.%s" % role)


func _physics_process(_delta: float) -> void:
	if _role() != ROLE_SCENE:
		return
	frame_index += 1
	for actor_role in ACTOR_ROLES:
		_apply_actor_frame(actor_role)


func get_qa_collision_state() -> Dictionary:
	return _collision_state()


func get_collision_qa_state() -> Dictionary:
	return _collision_state()


func get_qa_physics_state() -> Dictionary:
	return _collision_state()


func get_physics_qa_state() -> Dictionary:
	return _collision_state()


func qa_collision_probe() -> Dictionary:
	return _collision_state()


func _collision_state() -> Dictionary:
	var role := _role()
	if role == ROLE_SCENE:
		role = ROLE_SLIDE
	return _state_for_role(role)


func _state_for_role(role: String) -> Dictionary:
	var frame := _frame_for_provider() % 180
	var contact_start := 18
	var in_contact := frame >= contact_start and frame <= 150
	var state := {
		"fixture": "collision_report_demo",
		"qa_id": role,
		"frame_index": frame,
		"contacts": [],
		"surface_type": "unknown",
		"normal": _vec(0.0, 0.0),
		"depth": 0.0,
		"velocity": _vec(0.0, 0.0),
		"angular_velocity": 0.0,
		"progress_scalar": float(frame),
		"rescue_push_count": 0,
	}
	match role:
		ROLE_SLIDE:
			state["surface_type"] = "hard"
			state["normal"] = _vec(1.0, 0.0)
			state["velocity"] = _vec(-18.0 if frame < contact_start else -3.0, 96.0)
			state["angular_velocity"] = 0.15
			state["progress_scalar"] = float(frame) * 1.6
			if in_contact:
				state["depth"] = 2.0
				state["contacts"] = [_contact("slide_surface", "hard", state["normal"], state["depth"], state["velocity"], false)]
		ROLE_BOUNCY:
			var bounce_contact := frame >= contact_start and frame <= 96
			var sign := -1.0 if int(frame / 12) % 2 == 0 else 1.0
			state["surface_type"] = "hard"
			state["normal"] = _vec(1.0, 0.0)
			state["velocity"] = _vec(sign * 140.0, 12.0)
			state["angular_velocity"] = 34.0 if bounce_contact else 2.0
			state["progress_scalar"] = 80.0 + sign * 8.0
			if bounce_contact:
				state["depth"] = 4.0
				state["contacts"] = [_contact("rebound_wall", "hard", state["normal"], state["depth"], state["velocity"], false)]
		ROLE_SOFT:
			var soft_contact := frame >= 24 and frame <= 132
			state["surface_type"] = "soft"
			state["normal"] = _vec(-1.0, 0.0)
			state["velocity"] = _vec(62.0, 0.0)
			state["angular_velocity"] = 0.0
			state["progress_scalar"] = float(frame) * 1.1
			if soft_contact:
				state["depth"] = 1.0
				state["contacts"] = [_contact("soft_trigger", "soft", state["normal"], state["depth"], state["velocity"], true)]
		ROLE_DAMPING:
			var damping_contact := frame >= contact_start and frame <= 170
			var damping_speed := maxf(8.0, 100.0 - float(maxi(frame - contact_start, 0)) * 0.7)
			state["surface_type"] = "hard"
			state["normal"] = _vec(0.0, -1.0)
			state["velocity"] = _vec(0.0, damping_speed)
			state["angular_velocity"] = maxf(0.5, 12.0 - float(maxi(frame - contact_start, 0)) * 0.08)
			state["progress_scalar"] = minf(120.0, float(frame) * 0.8)
			if damping_contact:
				state["depth"] = 2.5
				state["contacts"] = [_contact("damping_pad", "hard", state["normal"], state["depth"], state["velocity"], false)]
		ROLE_STUCK:
			var stuck_contact := frame >= 12 and frame <= 180
			state["surface_type"] = "hard"
			state["normal"] = _vec(1.0, 0.0)
			state["velocity"] = _vec(0.0, 0.0)
			state["angular_velocity"] = 0.0
			state["progress_scalar"] = 4.0
			state["rescue_push_count"] = int(frame / 48)
			if stuck_contact:
				state["depth"] = 6.0 + float(mini(maxi(frame - 12, 0), 60)) * 0.05
				state["contacts"] = [_contact("pinch_gap", "hard", state["normal"], state["depth"], state["velocity"], false)]
	return state


func _apply_actor_frame(role: String) -> void:
	var actor := _actor(role)
	if actor == null:
		return
	var state := _state_for_role(role)
	actor.velocity = _payload_to_vec(state.get("velocity", {}))
	actor.global_position = (_initial_positions.get(role, actor.global_position) as Vector2) + _position_offset(role)


func _position_offset(role: String) -> Vector2:
	match role:
		ROLE_SLIDE:
			return Vector2(minf(float(frame_index) * 1.0, 190.0), minf(float(frame_index) * 0.7, 120.0))
		ROLE_BOUNCY:
			return Vector2(sin(float(frame_index) / 6.0) * 32.0, 0.0)
		ROLE_SOFT:
			return Vector2(minf(float(frame_index) * 0.8, 140.0), 0.0)
		ROLE_DAMPING:
			return Vector2(0.0, minf(float(frame_index) * 0.35, 70.0))
		ROLE_STUCK:
			return Vector2(0.0, 0.0)
	return Vector2.ZERO


func _actor(role: String) -> CharacterBody2D:
	var node := get_node_or_null(_actor_path(role))
	return node as CharacterBody2D


func _actor_path(role: String) -> NodePath:
	match role:
		ROLE_SLIDE:
			return NodePath("SlideActor")
		ROLE_BOUNCY:
			return NodePath("BouncyActor")
		ROLE_SOFT:
			return NodePath("SoftActor")
		ROLE_DAMPING:
			return NodePath("DampingActor")
		ROLE_STUCK:
			return NodePath("StuckActor")
	return NodePath("")


func _role() -> String:
	return str(get_meta("collision_report_role", ROLE_SCENE))


func _frame_for_provider() -> int:
	if _role() == ROLE_SCENE:
		return frame_index
	var root := _collision_report_root()
	if root != null and root.has_method("_collision_report_frame"):
		return int(root.call("_collision_report_frame"))
	return frame_index


func _collision_report_root() -> Node:
	var node: Node = self
	while node != null:
		if str(node.get_meta("collision_report_role", "")) == ROLE_SCENE:
			return node
		node = node.get_parent()
	return get_tree().current_scene


func _collision_report_frame() -> int:
	return frame_index


func _vec(x: float, y: float) -> Dictionary:
	return {"x": x, "y": y}


func _payload_to_vec(payload: Variant) -> Vector2:
	if payload is Dictionary:
		return Vector2(float(payload.get("x", 0.0)), float(payload.get("y", 0.0)))
	return Vector2.ZERO


func _contact(contact_id: String, surface_type: String, normal: Dictionary, depth: float, relative_velocity: Dictionary, is_soft: bool) -> Dictionary:
	return {
		"contact_id": contact_id,
		"other_id": contact_id,
		"surface_type": surface_type,
		"normal": normal,
		"depth": depth,
		"relative_velocity": relative_velocity,
		"is_soft": is_soft,
	}
