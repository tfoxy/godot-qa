extends Node2D

const QaRuntime = preload("res://addons/godot_qa/qa_runtime.gd")

const ACTOR_SPEED := 180.0
const ARRIVE_TOLERANCE := 24.0

@onready var actor: Node2D = $SteerActor
@onready var target: Node2D = $SteerTarget
@onready var home: Node2D = $SteerHome


func _ready() -> void:
	set_meta("qa_id", "drive_policy_demo")
	add_to_group("qa.id.drive_policy_demo")
	actor.set_meta("qa_id", "steer_actor")
	actor.add_to_group("qa.id.steer_actor")
	target.set_meta("qa_id", "steer_target")
	target.add_to_group("qa.id.steer_target")
	home.set_meta("qa_id", "steer_home")
	home.add_to_group("qa.id.steer_home")
	QaRuntime.register_policy_provider("steer_home_policy", self, "steer_home_policy_inputs", {"fixture": "drive_policy_demo"})


func _process(delta: float) -> void:
	# The actor only moves from real joypad input on device 0, exactly like a
	# player-controlled character. drive_policy must supply that input through
	# the normal injected-event path; nothing teleports the actor.
	var direction := Vector2(
		Input.get_joy_axis(0, JOY_AXIS_LEFT_X),
		Input.get_joy_axis(0, JOY_AXIS_LEFT_Y),
	)
	if direction.length() > 1.0:
		direction = direction.normalized()
	actor.position += direction * ACTOR_SPEED * delta


func get_qa_steer_state() -> Dictionary:
	var target_distance := actor.global_position.distance_to(target.global_position)
	var home_distance := actor.global_position.distance_to(home.global_position)
	return {
		"fixture": "drive_policy_demo",
		"target_distance": target_distance,
		"arrived_target": target_distance <= ARRIVE_TOLERANCE,
		"home_distance": home_distance,
		"arrived_home": home_distance <= ARRIVE_TOLERANCE,
	}


# Read-only registered policy provider: computes desired left-stick input from
# player-visible positions and returns it. The QA runtime validates the shape
# and emits the actual joypad events; this method must not mutate game state.
func steer_home_policy_inputs(request: Dictionary) -> Dictionary:
	var devices: Array = request.get("devices", [])
	var device := int(devices[0]) if not devices.is_empty() else 0
	var offset := home.global_position - actor.global_position
	var direction := Vector2.ZERO
	if offset.length() > ARRIVE_TOLERANCE:
		direction = offset.normalized()
	return {"inputs": [
		{"device": device, "motion": {"axis": "left_x", "value": clampf(direction.x, -1.0, 1.0)}},
		{"device": device, "motion": {"axis": "left_y", "value": clampf(direction.y, -1.0, 1.0)}},
	]}
