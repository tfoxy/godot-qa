extends Control

const QaProfiler = preload("res://addons/godot_qa/qa_profiler.gd")
const QaRuntime = preload("res://addons/godot_qa/qa_runtime.gd")

@onready var title_label: Label = $Canvas/TitleLabel
@onready var subtitle_label: Label = $Canvas/SubtitleLabel
@onready var menu_buttons: VBoxContainer = $Canvas/MenuButtons
@onready var start_button: Button = $Canvas/MenuButtons/StartButton
@onready var settings_button: Button = $Canvas/MenuButtons/SettingsButton
@onready var pause_menu: Control = $PauseMenu
@onready var resume_button: Button = $PauseMenu/Panel/Margin/Buttons/ResumeButton

const TEMPORAL_PENDING_BUTTON_QA_ID := "temporal_pending_button"
const TEMPORAL_PENDING_START_FRAME := 2
const TEMPORAL_PENDING_COMPLETE_FRAME := 4
const QA_PROXIMITY_NEAR_ACTOR_POS := Vector2(120.0, 520.0)
const QA_PROXIMITY_NEAR_TARGET_POS := Vector2(192.0, 520.0)
const QA_PROXIMITY_FAR_ACTOR_POS := Vector2(120.0, 520.0)
const QA_PROXIMITY_FAR_TARGET_POS := Vector2(280.0, 520.0)
const QA_PROXIMITY_INTERACT_RADIUS := 96.0
const QA_ACCELERATED_TIMER_SECONDS := 2.0

var _temporal_mode: String = "idle"
var _temporal_tick: int = 0
var _temporal_pending_status: String = "idle"
var _temporal_pending_tick: int = 0
var _accelerated_timer_status: String = "idle"
var _accelerated_timer_started_msec: int = 0
var _accelerated_timer_completed_msec: int = 0
var _accelerated_timer: SceneTreeTimer
var _demo_actor: TemporalProbeNode
var _demo_follower: Node2D
var _proximity_actor: Node2D
var _proximity_target: Node2D
var _proximity_motion_enabled: bool = false
var _occlusion_target: ColorRect
var _occlusion_cover: ColorRect
var _locator_resolution_probes: Array[Node] = []
var _qa_debug_overlay: QaDebugOverlayNode


class TemporalProbeNode:
	extends Node2D

	const QA_SURFACE_ALLOWED_LABELS := ["walkable", "blocked", "water"]
	const QA_SURFACE_POINT_LABELS := {
		"-64,-32": "walkable",
		"0,0": "walkable",
		"64,0": "blocked",
		"0,64": "water",
		"96,32": "blocked",
	}

	var qa_temporal_state: String = "steady"
	var qa_surface_state: String = "walkable"

	func qa_current_surface() -> String:
		return qa_surface_state

	func qa_probe_actor_surface_state() -> Dictionary:
		return {
			"surface": qa_surface_state,
			"provider": "qa:demo_actor",
			"provider_method": "qa_probe_actor_surface_state",
		}

	func get_raft_motion_qa_state() -> Dictionary:
		return {
			"actor": {
				"id": "demo_actor",
				"locator": "qa:demo_actor",
			},
			"motion": {
				"position": {
					"x": float(position.x),
					"y": float(position.y),
				},
				"state": qa_temporal_state,
			},
			"surface": {
				"current": qa_surface_state,
				"allowed": QA_SURFACE_ALLOWED_LABELS,
			},
		}

	func get_qa_probe_values_state() -> Array:
		return [
			{
				"id": "front",
				"force": 1.4000001,
				"offset": 3.0,
			},
			{
				"id": "back",
				"force": -1.3999999,
				"offset": -2.0,
			},
		]

	func get_runtime_state() -> Dictionary:
		return {"fixture": "rejected_non_qa_getter"}

	func get_state() -> Dictionary:
		return {"fixture": "rejected_generic_getter"}

	func _get_secret_qa_state() -> Dictionary:
		return {"fixture": "rejected_private_getter"}

	func set_raft_motion_qa_state() -> Dictionary:
		return {"fixture": "rejected_mutating_setter"}

	func qa_mutate_motion_state() -> Dictionary:
		return {"fixture": "rejected_mutating_qa_method"}

	func get_bad_qa_state_with_arg(_arg: Variant) -> Dictionary:
		return {"fixture": "rejected_argument_method"}

	func get_argument_qa_state(_arg: Variant) -> Dictionary:
		return {"fixture": "rejected_argument_method"}

	func get_object_qa_state() -> Object:
		return RefCounted.new()

	func get_error_qa_state() -> Dictionary:
		var bridge := get_node_or_null("/root/GodotQaBridge")
		if bridge != null and bridge.has_method("record_runtime_error"):
			bridge.call(
				"record_runtime_error",
				"state_matches negative fixture recorded a runtime error",
				"state_matches_fixture",
				{
					"qa_id": "demo_actor",
					"method": "get_error_qa_state",
				},
			)
		return {"fixture": "runtime_error"}

	func qa_probe_surface(request: Dictionary) -> Dictionary:
		var points: Array = request.get("points", [])
		var coordinate_space := str(request.get("coordinate_space", "world_2d"))
		var results: Array = []
		for raw_point in points:
			if not raw_point is Dictionary:
				continue
			var point: Dictionary = raw_point
			var x := float(point.get("x", 0.0))
			var y := float(point.get("y", 0.0))
			var local_point := Vector2(x, y)
			if coordinate_space == "world_2d":
				local_point = local_point - global_position
			var key := "%d,%d" % [int(round(local_point.x)), int(round(local_point.y))]
			var label := str(QA_SURFACE_POINT_LABELS.get(key, "blocked"))
			var entry := {
				"point": {
					"x": x,
					"y": y,
				},
				"coordinate_space": coordinate_space,
				"label": label,
				"surface": label,
				"provider": "qa:demo_actor",
				"provider_method": "qa_probe_surface",
			}
			if point.has("z"):
				entry["point"]["z"] = float(point.get("z", 0.0))
			if point.has("expected"):
				entry["expected"] = str(point.get("expected", ""))
			results.append(entry)
		return {
			"provider": "qa:demo_actor",
			"provider_method": "qa_probe_surface",
			"coordinate_space": coordinate_space,
			"allowed_surfaces": QA_SURFACE_ALLOWED_LABELS,
			"points": results,
		}


class QaDebugOverlayNode:
	extends ColorRect

	const LOCATOR := "qa:qa_debug_overlay"

	func get_qa_overlay_state() -> Dictionary:
		return get_hidden_overlay_qa_state()

	func get_hidden_overlay_qa_state() -> Dictionary:
		return {
			"overlay_locator": LOCATOR,
			"hidden": not visible,
			"visible": visible,
			"visible_in_tree": is_visible_in_tree(),
			"verification": {
				"source": "project_owned_qa_method",
				"method": "get_hidden_overlay_qa_state",
			},
		}


func _ready() -> void:
	if not InputMap.has_action("ui_start"):
		InputMap.add_action("ui_start")
	if not InputMap.has_action("qa_interact"):
		InputMap.add_action("qa_interact")
	set_meta("qa_id", "main_menu")
	title_label.set_meta("qa_id", "main_menu_title")
	subtitle_label.set_meta("qa_id", "main_menu_subtitle")
	menu_buttons.set_meta("qa_id", "main_menu_buttons")
	start_button.set_meta("qa_id", "start_button")
	start_button.set_meta("qa_aliases", PackedStringArray(["primary_start", "qa:primary_start"]))
	settings_button.set_meta("qa_id", "settings_button")
	start_button.set_focus_neighbor(SIDE_BOTTOM, NodePath("../SettingsButton"))
	settings_button.set_focus_neighbor(SIDE_TOP, NodePath("../StartButton"))
	start_button.pressed.connect(_open_pause_menu)
	settings_button.pressed.connect(_open_settings_menu)
	_create_temporal_pending_button()
	pause_menu.visible = false
	_create_hidden_overlay_probe_node()
	_create_temporal_probe_nodes()
	_create_proximity_probe_nodes()
	_create_occlusion_probe_nodes()
	_create_locator_resolution_probe_nodes()
	start_button.grab_focus()


func _process(_delta: float) -> void:
	QaProfiler.begin("MainMenu._process")
	if _demo_actor != null and _demo_follower != null:
		if _temporal_mode == "idle":
			_demo_actor.set("qa_temporal_state", "steady")
		else:
			_temporal_tick += 1
			var base_position := Vector2(120.0 + float(_temporal_tick * 8), 420.0)
			_demo_actor.position = base_position
			_demo_follower.position = base_position + Vector2(-10.0, 0.0)

			if _temporal_mode == "failure" and _temporal_tick >= 6:
				_demo_actor.set("qa_temporal_state", "forbidden")
			else:
				_demo_actor.set("qa_temporal_state", "steady")

	if _temporal_pending_status == "armed" or _temporal_pending_status == "pending":
		_temporal_pending_tick += 1
		if _temporal_pending_tick >= TEMPORAL_PENDING_COMPLETE_FRAME:
			_temporal_pending_status = "complete"
		elif _temporal_pending_tick >= TEMPORAL_PENDING_START_FRAME:
			_temporal_pending_status = "pending"

	if _proximity_motion_enabled and _proximity_actor != null:
		_proximity_actor.position.x += 4.0
	QaProfiler.end("MainMenu._process")


func _exit_tree() -> void:
	if is_instance_valid(_demo_actor):
		_demo_actor.queue_free()
	if is_instance_valid(_demo_follower):
		_demo_follower.queue_free()
	if is_instance_valid(_proximity_actor):
		_proximity_actor.queue_free()
	if is_instance_valid(_proximity_target):
		_proximity_target.queue_free()
	if is_instance_valid(_occlusion_target):
		_occlusion_target.queue_free()
	if is_instance_valid(_occlusion_cover):
		_occlusion_cover.queue_free()
	if is_instance_valid(_qa_debug_overlay):
		_qa_debug_overlay.queue_free()
	for probe in _locator_resolution_probes:
		if is_instance_valid(probe):
			probe.queue_free()
	_locator_resolution_probes.clear()
	var marker := _find_interaction_result()
	if is_instance_valid(marker):
		marker.queue_free()
	_demo_actor = null
	_demo_follower = null
	_proximity_actor = null
	_proximity_target = null
	_occlusion_target = null
	_occlusion_cover = null
	_qa_debug_overlay = null


func _input(event: InputEvent) -> void:
	if event is InputEventMouseButton and event.button_index == MOUSE_BUTTON_LEFT and event.pressed:
		if start_button.get_global_rect().has_point(event.position):
			_open_pause_menu()


func _unhandled_input(event: InputEvent) -> void:
	if event.is_action_pressed("ui_start"):
		_open_pause_menu()
	if event.is_action_pressed("qa_interact"):
		_try_mark_interaction()


func _open_pause_menu() -> void:
	menu_buttons.visible = false
	pause_menu.visible = true
	resume_button.grab_focus()


func _open_settings_menu() -> void:
	get_tree().change_scene_to_file("res://ui/settings_menu.tscn")


func qa_set_subtitle(value: String) -> String:
	subtitle_label.text = value
	return subtitle_label.text


func qa_temporal_set_mode(mode: String) -> String:
	# Fixture setup only: picks deterministic sample behavior while motion still comes from _process.
	var normalized := mode.strip_edges().to_lower()
	if normalized != "failure":
		normalized = "steady"
	_temporal_mode = normalized
	_temporal_tick = 0
	if _demo_actor != null and _demo_follower != null:
		_demo_actor.position = Vector2(120.0, 420.0)
		_demo_follower.position = Vector2(110.0, 420.0)
		_demo_actor.set("qa_temporal_state", "steady")
	return _temporal_mode


func get_temporal_pending_qa_state() -> Dictionary:
	return {
		"status": _temporal_pending_status,
		"tick": _temporal_pending_tick,
		"button": "qa:%s" % TEMPORAL_PENDING_BUTTON_QA_ID,
	}


func qa_start_accelerated_timer() -> Dictionary:
	_accelerated_timer_status = "running"
	_accelerated_timer_started_msec = Time.get_ticks_msec()
	_accelerated_timer_completed_msec = 0
	_accelerated_timer = get_tree().create_timer(QA_ACCELERATED_TIMER_SECONDS)
	_accelerated_timer.timeout.connect(_complete_accelerated_timer)
	return get_accelerated_timer_qa_state()


func get_accelerated_timer_qa_state() -> Dictionary:
	var elapsed_msec := 0
	if _accelerated_timer_started_msec > 0:
		var end_msec := _accelerated_timer_completed_msec
		if end_msec <= 0:
			end_msec = Time.get_ticks_msec()
		elapsed_msec = max(0, end_msec - _accelerated_timer_started_msec)
	return {
		"status": _accelerated_timer_status,
		"duration_seconds": QA_ACCELERATED_TIMER_SECONDS,
		"elapsed_wall_seconds": float(elapsed_msec) / 1000.0,
	}


func qa_proximity_reset() -> String:
	_proximity_motion_enabled = false
	if _proximity_actor != null:
		_proximity_actor.position = QA_PROXIMITY_NEAR_ACTOR_POS
	if _proximity_target != null:
		_proximity_target.position = QA_PROXIMITY_NEAR_TARGET_POS
	_remove_interaction_result()
	return "near"


func qa_proximity_place_far() -> String:
	_proximity_motion_enabled = false
	if _proximity_actor != null:
		_proximity_actor.position = QA_PROXIMITY_FAR_ACTOR_POS
	if _proximity_target != null:
		_proximity_target.position = QA_PROXIMITY_FAR_TARGET_POS
	_remove_interaction_result()
	return "far"


func qa_proximity_place_near() -> String:
	return qa_proximity_reset()


func qa_proximity_enable_distance_changed_motion() -> String:
	qa_proximity_place_near()
	_proximity_motion_enabled = true
	return "motion"


func qa_surface_place_matching() -> String:
	if _demo_actor == null:
		return "missing"
	_demo_actor.position = Vector2(120.0, 420.0)
	_demo_actor.set("qa_surface_state", "walkable")
	return "matching"


func qa_surface_place_mismatch() -> String:
	if _demo_actor == null:
		return "missing"
	_demo_actor.position = Vector2(120.0, 420.0)
	_demo_actor.set("qa_surface_state", "water")
	return "mismatch"


func get_hidden_overlay_qa_state() -> Dictionary:
	if _qa_debug_overlay == null:
		return {
			"overlay_locator": "qa:qa_debug_overlay",
			"hidden": false,
			"visible": false,
			"visible_in_tree": false,
			"verification": {
				"source": "project_owned_qa_method",
				"method": "get_hidden_overlay_qa_state",
				"status": "missing_overlay_node",
			},
		}
	return _qa_debug_overlay.get_hidden_overlay_qa_state()


func get_raft_motion_qa_state() -> Dictionary:
	if _demo_actor == null:
		return {
			"actor": {
				"id": "demo_actor",
				"locator": "qa:demo_actor",
				"status": "missing",
			},
			"motion": {},
			"surface": {},
		}
	return _demo_actor.get_raft_motion_qa_state()


func _create_temporal_pending_button() -> void:
	var button := Button.new()
	button.name = "TemporalPendingButton"
	button.text = "Temporal Pending"
	button.set_meta("qa_id", TEMPORAL_PENDING_BUTTON_QA_ID)
	button.pressed.connect(_start_temporal_pending_transition)
	menu_buttons.add_child(button)


func _start_temporal_pending_transition() -> void:
	_temporal_pending_status = "armed"
	_temporal_pending_tick = 0


func _complete_accelerated_timer() -> void:
	_accelerated_timer_status = "complete"
	_accelerated_timer_completed_msec = Time.get_ticks_msec()


func _create_hidden_overlay_probe_node() -> void:
	_qa_debug_overlay = QaDebugOverlayNode.new()
	_qa_debug_overlay.name = "QAHiddenDebugOverlay"
	_qa_debug_overlay.set_meta("qa_id", "qa_debug_overlay")
	_qa_debug_overlay.set_meta("qa_aliases", PackedStringArray(["godot_qa_overlay", "qa:godot_qa_overlay"]))
	_qa_debug_overlay.color = Color(0.1, 0.2, 0.3, 0.65)
	_qa_debug_overlay.size = Vector2(320.0, 120.0)
	_qa_debug_overlay.position = Vector2(24.0, 24.0)
	_qa_debug_overlay.visible = false
	_qa_debug_overlay.z_index = 100
	$Canvas.add_child(_qa_debug_overlay)


func _create_temporal_probe_nodes() -> void:
	_demo_actor = TemporalProbeNode.new()
	_demo_actor.name = "QADemoActor"
	_demo_actor.set_meta("qa_id", "demo_actor")
	_demo_actor.set_meta("qa_surface_probe_method", "qa_probe_surface")
	_demo_actor.set_meta("qa_surface_state_method", "qa_current_surface")
	_demo_actor.position = Vector2(120.0, 420.0)
	_demo_actor.set("qa_temporal_state", "steady")
	_demo_actor.set("qa_surface_state", "walkable")
	add_child(_demo_actor)
	QaRuntime.register_probe_provider("demo_surface", _demo_actor, "qa_probe_surface", {"kind": "surface"})

	_demo_follower = Node2D.new()
	_demo_follower.name = "QADemoFollower"
	_demo_follower.set_meta("qa_id", "demo_follower")
	_demo_follower.position = Vector2(110.0, 420.0)
	add_child(_demo_follower)


func _create_proximity_probe_nodes() -> void:
	_proximity_actor = Node2D.new()
	_proximity_actor.name = "QAProximityActor"
	_proximity_actor.set_meta("qa_id", "actor")
	_proximity_actor.position = QA_PROXIMITY_NEAR_ACTOR_POS
	add_child(_proximity_actor)

	_proximity_target = Node2D.new()
	_proximity_target.name = "QAProximityTarget"
	_proximity_target.set_meta("qa_id", "target")
	_proximity_target.position = QA_PROXIMITY_NEAR_TARGET_POS
	add_child(_proximity_target)


func _create_occlusion_probe_nodes() -> void:
	_occlusion_target = ColorRect.new()
	_occlusion_target.name = "QAImportantActor"
	_occlusion_target.set_meta("qa_id", "important_actor")
	_occlusion_target.color = Color(0.95, 0.35, 0.2, 1.0)
	_occlusion_target.size = Vector2(96.0, 96.0)
	_occlusion_target.position = Vector2(860.0, 220.0)
	_occlusion_target.z_index = 1
	$Canvas.add_child(_occlusion_target)

	_occlusion_cover = ColorRect.new()
	_occlusion_cover.name = "QAOverlay"
	_occlusion_cover.set_meta("qa_id", "overlay")
	_occlusion_cover.color = Color(0.0, 0.0, 0.0, 1.0)
	_occlusion_cover.size = Vector2(96.0, 96.0)
	_occlusion_cover.position = _occlusion_target.position
	_occlusion_cover.visible = false
	_occlusion_cover.z_index = 2
	$Canvas.add_child(_occlusion_cover)


func _create_locator_resolution_probe_nodes() -> void:
	var metadata_only_target := Node.new()
	metadata_only_target.name = "QAMetadataOnlyTarget"
	metadata_only_target.set_meta("qa_id", "metadata_only_target")
	add_child(metadata_only_target)
	_locator_resolution_probes.append(metadata_only_target)

	var canonical_group_only_target := Node.new()
	canonical_group_only_target.name = "QACanonicalGroupOnlyTarget"
	canonical_group_only_target.add_to_group("qa.id.canonical_group_only_target")
	add_child(canonical_group_only_target)
	_locator_resolution_probes.append(canonical_group_only_target)

	var compat_group_only_target := Node.new()
	compat_group_only_target.name = "QACompatGroupOnlyTarget"
	compat_group_only_target.add_to_group("qa.compat_group_only_target")
	add_child(compat_group_only_target)
	_locator_resolution_probes.append(compat_group_only_target)

	var alias_only_target := Node.new()
	alias_only_target.name = "QAAliasOnlyTarget"
	alias_only_target.set_meta(
		"qa_aliases",
		PackedStringArray(["alias_only_target", "qa:alias_only_prefixed_target"]),
	)
	add_child(alias_only_target)
	_locator_resolution_probes.append(alias_only_target)

	var duplicate_a := Node.new()
	duplicate_a.name = "QADuplicateLocatorTargetA"
	duplicate_a.set_meta("qa_id", "duplicate_locator_target")
	add_child(duplicate_a)
	_locator_resolution_probes.append(duplicate_a)

	var duplicate_b := Node.new()
	duplicate_b.name = "QADuplicateLocatorTargetB"
	duplicate_b.set_meta("qa_id", "duplicate_locator_target")
	add_child(duplicate_b)
	_locator_resolution_probes.append(duplicate_b)

	var mismatched_group_target := Node.new()
	mismatched_group_target.name = "QAMismatchedGroupTarget"
	mismatched_group_target.set_meta("qa_id", "different_locator_target")
	mismatched_group_target.add_to_group("qa.id.mismatched_group_target")
	add_child(mismatched_group_target)
	_locator_resolution_probes.append(mismatched_group_target)


func qa_occlusion_set(mode: String) -> String:
	var normalized := mode.strip_edges().to_lower()
	if _occlusion_cover == null:
		return "missing"
	if normalized == "occluded":
		_occlusion_cover.visible = true
	else:
		normalized = "clear"
		_occlusion_cover.visible = false
	return normalized


func _try_mark_interaction() -> void:
	if _proximity_actor == null or _proximity_target == null:
		return
	var distance := _proximity_actor.global_position.distance_to(_proximity_target.global_position)
	if distance <= QA_PROXIMITY_INTERACT_RADIUS:
		if _find_interaction_result() == null:
			var marker := Node2D.new()
			marker.name = "QAInteractionResult"
			marker.set_meta("qa_id", "interaction_result")
			marker.position = _proximity_target.position
			add_child(marker)


func _remove_interaction_result() -> void:
	var marker := _find_interaction_result()
	if marker != null:
		marker.queue_free()


func _find_interaction_result() -> Node:
	for child in get_children():
		if child.has_meta("qa_id") and String(child.get_meta("qa_id")) == "interaction_result":
			return child
	return null
