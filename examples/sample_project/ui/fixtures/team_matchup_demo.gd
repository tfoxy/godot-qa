extends Node2D

const QaRuntime = preload("res://addons/godot_qa/qa_runtime.gd")

const ACTOR_SPEED := 180.0
const TOUCH_TOLERANCE := 28.0
const HOLD_TOLERANCE := 8.0

# The GAME owns team composition: devices [0, 1] are team "a", devices [2, 3]
# are team "b". A scenario's drive_policy params.teams chooses each team's STYLE
# for a matchup (rush vs hold); it never redefines the roster. Per-team dispatch
# lives here in game code, not in the tool — see docs/scenario-format.md
# "Team-vs-team matchups".
const TEAM_OF_DEVICE := {0: "a", 1: "a", 2: "b", 3: "b"}

@onready var objective: Node2D = $Objective
@onready var _actors := {
	0: $Actor0,
	1: $Actor1,
	2: $Actor2,
	3: $Actor3,
}
@onready var _home_posts := {
	0: $Home0,
	1: $Home1,
	2: $Home2,
	3: $Home3,
}

var winner := ""
var winning_device := -1
var decided_frame := -1
var _frame := 0


func _ready() -> void:
	set_meta("qa_id", "team_matchup_demo")
	add_to_group("qa.id.team_matchup_demo")
	objective.set_meta("qa_id", "match_objective")
	objective.add_to_group("qa.id.match_objective")
	for device in _actors:
		var actor: Node2D = _actors[device]
		var actor_id := "match_actor_%d" % device
		actor.set_meta("qa_id", actor_id)
		actor.add_to_group("qa.id.%s" % actor_id)
	QaRuntime.register_policy_provider("team_matchup", self, "team_matchup_inputs", {"fixture": "team_matchup_demo"})


func _process(delta: float) -> void:
	# Every actor moves ONLY from real per-device joypad input, exactly like four
	# couch players on four gamepads. drive_policy must supply that input through
	# the normal injected-event path; nothing teleports an actor.
	_frame += 1
	for device in _actors:
		var actor: Node2D = _actors[device]
		var direction := Vector2(
			Input.get_joy_axis(device, JOY_AXIS_LEFT_X),
			Input.get_joy_axis(device, JOY_AXIS_LEFT_Y),
		)
		if direction.length() > 1.0:
			direction = direction.normalized()
		actor.position += direction * ACTOR_SPEED * delta
	_resolve_objective()


# First team to touch the shared objective wins; ties on a frame resolve to the
# lower device. Fully deterministic (no RNG), so the outcome is stable per seed.
func _resolve_objective() -> void:
	if winner != "":
		return
	for device in _actors:
		var actor: Node2D = _actors[device]
		if actor.global_position.distance_to(objective.global_position) <= TOUCH_TOLERANCE:
			winner = str(TEAM_OF_DEVICE.get(device, ""))
			winning_device = device
			decided_frame = _frame
			return


# Read-only registered META-policy: ONE policy drives BOTH teams. Each tick it
# reads the per-team STYLE from request.params.teams and dispatches a per-device
# brain, returning left-stick motion for EVERY device in a single tick. It never
# mutates game state; the QA runtime validates the shape and emits the joypad
# events. Arity-1, non-mutating name — it satisfies the drive_policy integrity
# contract.
func team_matchup_inputs(request: Dictionary) -> Dictionary:
	var params: Dictionary = request.get("params", {})
	var teams: Dictionary = params.get("teams", {})
	var inputs: Array = []
	for team_key in teams:
		var team: Dictionary = teams[team_key]
		var style := str(team.get("style", "rush"))
		for raw_device in team.get("devices", []):
			inputs.append_array(_brain_inputs(int(raw_device), style))
	return {"inputs": inputs}


# Per-device brain. "rush" steers straight at the shared objective; "hold"
# steers back toward the device's home post and idles once there. Both are the
# same player-visible left-stick shape a human would produce.
func _brain_inputs(device: int, style: String) -> Array:
	var actor: Node2D = _actors.get(device, null)
	if actor == null:
		return []
	var target := objective.global_position
	var tolerance := TOUCH_TOLERANCE
	if style == "hold":
		var post: Node2D = _home_posts.get(device, null)
		target = post.global_position if post != null else actor.global_position
		tolerance = HOLD_TOLERANCE
	var offset := target - actor.global_position
	var direction := Vector2.ZERO
	if offset.length() > tolerance:
		direction = offset.normalized()
	return [
		{"device": device, "motion": {"axis": "left_x", "value": clampf(direction.x, -1.0, 1.0)}},
		{"device": device, "motion": {"axis": "left_y", "value": clampf(direction.y, -1.0, 1.0)}},
	]


func get_qa_team_matchup_state() -> Dictionary:
	var positions := {}
	for device in _actors:
		var actor: Node2D = _actors[device]
		positions[str(device)] = {
			"team": str(TEAM_OF_DEVICE.get(device, "")),
			"x": actor.global_position.x,
			"y": actor.global_position.y,
			"objective_distance": actor.global_position.distance_to(objective.global_position),
		}
	return {
		"fixture": "team_matchup_demo",
		"teams": {"a": [0, 1], "b": [2, 3]},
		"objective": {"x": objective.global_position.x, "y": objective.global_position.y},
		"positions": positions,
		"decided": winner != "",
		"outcome": {
			"winner": winner,
			"winning_device": winning_device,
			"decided": winner != "",
			"decided_frame": decided_frame,
		},
	}
