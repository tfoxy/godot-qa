extends Control

const MOVE_STEP := 24.0
const MAX_MOVE_RIGHT := 144.0
const INTERACT_RANGE := 240.0

@onready var playfield: ColorRect = $Playfield
@onready var actor: ColorRect = $Playfield/Actor
@onready var interactable: ColorRect = $Playfield/Interactable
@onready var debug_overlay: PanelContainer = $DebugOverlay
@onready var invalid_result: Label = $InvalidActionResult
@onready var interaction_result: Label = $InteractionResult

var _actor_start_x := 0.0


func _ready() -> void:
	set_meta("qa_id", "recorder_negative_suggestions_fixture")
	set_meta("qa_recorder_fixture", true)
	playfield.set_meta("qa_id", "qa_playfield")
	playfield.set_meta("qa_recorder_role", "playfield")
	actor.set_meta("qa_id", "qa_actor")
	actor.set_meta("qa_recorder_role", "actor")
	actor.set_meta("qa_recorder_bounds", {"max_right": MAX_MOVE_RIGHT})
	actor.set_meta("qa_recorder_playfield", "qa:qa_playfield")
	interactable.set_meta("qa_id", "qa_interactable")
	interactable.set_meta("qa_recorder_role", "interactable")
	interactable.set_meta("qa_recorder_proximity_target", "qa:qa_actor")
	interactable.set_meta("qa_recorder_proximity_range", INTERACT_RANGE)
	debug_overlay.set_meta("qa_id", "qa_debug_overlay")
	debug_overlay.set_meta("qa_recorder_role", "debug_overlay")
	debug_overlay.set_meta("qa_recorder_playfield", "qa:qa_playfield")
	invalid_result.set_meta("qa_id", "qa_invalid_action_result")
	invalid_result.set_meta("qa_recorder_role", "invalid_action_result")
	interaction_result.set_meta("qa_id", "qa_interaction_result")
	interaction_result.set_meta("qa_recorder_role", "interaction_result")
	_actor_start_x = actor.position.x
	invalid_result.text = "invalid:no-op"
	interaction_result.text = "interaction:idle"
	debug_overlay.visible = false


func _unhandled_input(event: InputEvent) -> void:
	if event is InputEventAction and event.is_pressed() and not event.is_echo():
		if event.is_action_pressed("qa_invalid_action"):
			_handle_invalid_action()
		elif event.is_action_pressed("qa_cancel"):
			_handle_cancel()
		elif event.is_action_pressed("qa_move_right"):
			_handle_move_right()
		elif event.is_action_pressed("qa_toggle_debug"):
			_handle_toggle_debug()
		elif event.is_action_pressed("qa_interact"):
			_handle_interact()


func _handle_invalid_action() -> void:
	# Explicitly no-op; keeps deterministic state for negative suggestion evidence.
	invalid_result.text = "invalid:no-op"


func _handle_cancel() -> void:
	interaction_result.text = "interaction:cancelled"


func _handle_move_right() -> void:
	var max_x := _actor_start_x + MAX_MOVE_RIGHT
	actor.position.x = min(actor.position.x + MOVE_STEP, max_x)


func _handle_toggle_debug() -> void:
	debug_overlay.visible = not debug_overlay.visible


func _handle_interact() -> void:
	var distance := actor.global_position.distance_to(interactable.global_position)
	if distance <= INTERACT_RANGE:
		interaction_result.text = "interaction:success"
	else:
		interaction_result.text = "interaction:too_far"
