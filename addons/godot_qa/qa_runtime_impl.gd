extends RefCounted

const QaRuntimeAudio = preload("res://addons/godot_qa/qa_runtime_audio.gd")
const QaRuntimeFixtures = preload("res://addons/godot_qa/qa_runtime_fixtures.gd")
const QaRuntimeInput = preload("res://addons/godot_qa/qa_runtime_input.gd")
const QaRuntimePhysics = preload("res://addons/godot_qa/qa_runtime_physics.gd")
const QaRuntimePointer = preload("res://addons/godot_qa/qa_runtime_pointer.gd")
const QaRuntimeProviders = preload("res://addons/godot_qa/qa_runtime_providers.gd")
const QaRuntimeShapes = preload("res://addons/godot_qa/qa_runtime_shapes.gd")
const QaRuntimeTrace = preload("res://addons/godot_qa/qa_runtime_trace.gd")
const QaAssertions = preload("res://addons/godot_qa/qa_assertions.gd")
const QaSnapshot = preload("res://addons/godot_qa/qa_snapshot.gd")

const AUDIO_EXPORT_MAX_BYTES := 10485760
const AUDIO_CAPTURE_MAX_SECONDS := 60.0
const PROVIDER_ID_ALLOWED_EXTRA := "_.:-"
const PROVIDER_MUTATING_PREFIXES := ["set_", "add_", "remove_", "delete_", "clear_", "queue_", "free", "call_", "emit_", "change_", "qa_mutate_"]

const SUPPORTED_KEYS := {
	"enter": {"name": "Enter", "keycode": KEY_ENTER},
	"return": {"name": "Enter", "keycode": KEY_ENTER},
	"escape": {"name": "Escape", "keycode": KEY_ESCAPE},
	"esc": {"name": "Escape", "keycode": KEY_ESCAPE},
	"space": {"name": "Space", "keycode": KEY_SPACE, "unicode": 32},
	"tab": {"name": "Tab", "keycode": KEY_TAB},
	"backspace": {"name": "Backspace", "keycode": KEY_BACKSPACE},
	"up": {"name": "Up", "keycode": KEY_UP},
	"arrowup": {"name": "Up", "keycode": KEY_UP},
	"down": {"name": "Down", "keycode": KEY_DOWN},
	"arrowdown": {"name": "Down", "keycode": KEY_DOWN},
	"left": {"name": "Left", "keycode": KEY_LEFT},
	"arrowleft": {"name": "Left", "keycode": KEY_LEFT},
	"right": {"name": "Right", "keycode": KEY_RIGHT},
	"arrowright": {"name": "Right", "keycode": KEY_RIGHT},
	"0": {"name": "0", "keycode": KEY_0, "unicode": 48},
	"1": {"name": "1", "keycode": KEY_1, "unicode": 49},
	"2": {"name": "2", "keycode": KEY_2, "unicode": 50},
	"3": {"name": "3", "keycode": KEY_3, "unicode": 51},
	"4": {"name": "4", "keycode": KEY_4, "unicode": 52},
	"5": {"name": "5", "keycode": KEY_5, "unicode": 53},
	"6": {"name": "6", "keycode": KEY_6, "unicode": 54},
	"7": {"name": "7", "keycode": KEY_7, "unicode": 55},
	"8": {"name": "8", "keycode": KEY_8, "unicode": 56},
	"9": {"name": "9", "keycode": KEY_9, "unicode": 57},
	"a": {"name": "A", "keycode": KEY_A, "unicode": 97},
	"b": {"name": "B", "keycode": KEY_B, "unicode": 98},
	"c": {"name": "C", "keycode": KEY_C, "unicode": 99},
	"d": {"name": "D", "keycode": KEY_D, "unicode": 100},
	"e": {"name": "E", "keycode": KEY_E, "unicode": 101},
	"f": {"name": "F", "keycode": KEY_F, "unicode": 102},
	"g": {"name": "G", "keycode": KEY_G, "unicode": 103},
	"h": {"name": "H", "keycode": KEY_H, "unicode": 104},
	"i": {"name": "I", "keycode": KEY_I, "unicode": 105},
	"j": {"name": "J", "keycode": KEY_J, "unicode": 106},
	"k": {"name": "K", "keycode": KEY_K, "unicode": 107},
	"l": {"name": "L", "keycode": KEY_L, "unicode": 108},
	"m": {"name": "M", "keycode": KEY_M, "unicode": 109},
	"n": {"name": "N", "keycode": KEY_N, "unicode": 110},
	"o": {"name": "O", "keycode": KEY_O, "unicode": 111},
	"p": {"name": "P", "keycode": KEY_P, "unicode": 112},
	"q": {"name": "Q", "keycode": KEY_Q, "unicode": 113},
	"r": {"name": "R", "keycode": KEY_R, "unicode": 114},
	"s": {"name": "S", "keycode": KEY_S, "unicode": 115},
	"t": {"name": "T", "keycode": KEY_T, "unicode": 116},
	"u": {"name": "U", "keycode": KEY_U, "unicode": 117},
	"v": {"name": "V", "keycode": KEY_V, "unicode": 118},
	"w": {"name": "W", "keycode": KEY_W, "unicode": 119},
	"x": {"name": "X", "keycode": KEY_X, "unicode": 120},
	"y": {"name": "Y", "keycode": KEY_Y, "unicode": 121},
	"z": {"name": "Z", "keycode": KEY_Z, "unicode": 122},
}

const SUPPORTED_JOY_BUTTONS := {
	"a": {"name": "a", "button_index": JOY_BUTTON_A},
	"b": {"name": "b", "button_index": JOY_BUTTON_B},
	"x": {"name": "x", "button_index": JOY_BUTTON_X},
	"y": {"name": "y", "button_index": JOY_BUTTON_Y},
	"back": {"name": "back", "button_index": JOY_BUTTON_BACK},
	"select": {"name": "back", "button_index": JOY_BUTTON_BACK},
	"guide": {"name": "guide", "button_index": JOY_BUTTON_GUIDE},
	"start": {"name": "start", "button_index": JOY_BUTTON_START},
	"menu": {"name": "start", "button_index": JOY_BUTTON_START},
	"left_shoulder": {"name": "left_shoulder", "button_index": JOY_BUTTON_LEFT_SHOULDER},
	"right_shoulder": {"name": "right_shoulder", "button_index": JOY_BUTTON_RIGHT_SHOULDER},
	"left_stick": {"name": "left_stick", "button_index": JOY_BUTTON_LEFT_STICK},
	"right_stick": {"name": "right_stick", "button_index": JOY_BUTTON_RIGHT_STICK},
	"dpad_up": {"name": "dpad_up", "button_index": JOY_BUTTON_DPAD_UP},
	"dpad_down": {"name": "dpad_down", "button_index": JOY_BUTTON_DPAD_DOWN},
	"dpad_left": {"name": "dpad_left", "button_index": JOY_BUTTON_DPAD_LEFT},
	"dpad_right": {"name": "dpad_right", "button_index": JOY_BUTTON_DPAD_RIGHT},
}

const SUPPORTED_JOY_AXES := {
	"left_x": {"name": "left_x", "axis_index": JOY_AXIS_LEFT_X},
	"left_y": {"name": "left_y", "axis_index": JOY_AXIS_LEFT_Y},
	"right_x": {"name": "right_x", "axis_index": JOY_AXIS_RIGHT_X},
	"right_y": {"name": "right_y", "axis_index": JOY_AXIS_RIGHT_Y},
	"trigger_left": {"name": "trigger_left", "axis_index": JOY_AXIS_TRIGGER_LEFT},
	"left_trigger": {"name": "trigger_left", "axis_index": JOY_AXIS_TRIGGER_LEFT},
	"lt": {"name": "trigger_left", "axis_index": JOY_AXIS_TRIGGER_LEFT},
	"trigger_right": {"name": "trigger_right", "axis_index": JOY_AXIS_TRIGGER_RIGHT},
	"right_trigger": {"name": "trigger_right", "axis_index": JOY_AXIS_TRIGGER_RIGHT},
	"rt": {"name": "trigger_right", "axis_index": JOY_AXIS_TRIGGER_RIGHT},
	"joy_axis_left_x": {"name": "left_x", "axis_index": JOY_AXIS_LEFT_X},
	"joy_axis_left_y": {"name": "left_y", "axis_index": JOY_AXIS_LEFT_Y},
	"joy_axis_right_x": {"name": "right_x", "axis_index": JOY_AXIS_RIGHT_X},
	"joy_axis_right_y": {"name": "right_y", "axis_index": JOY_AXIS_RIGHT_Y},
	"joy_axis_trigger_left": {"name": "trigger_left", "axis_index": JOY_AXIS_TRIGGER_LEFT},
	"joy_axis_trigger_right": {"name": "trigger_right", "axis_index": JOY_AXIS_TRIGGER_RIGHT},
}

# Hard cap on telemetry.record row collection so a long sampling window stays
# memory-bounded inside the game process (3600 rows = 60s of per-frame samples
# at 60 FPS; the recording is marked truncated when the cap is hit).
const MAX_TELEMETRY_ROWS := 3600

# Hard cap on frames a single input gesture (touch move slide) may span, ~30s at
# 60 FPS, aligned with the bridge queue wait so a long gesture cannot starve
# queued commands into a timeout. Larger frame counts fail with invalid_params.
const MAX_INPUT_GESTURE_FRAMES := 1800

static var _state_providers: Dictionary = {}
static var _probe_providers: Dictionary = {}
static var _fixture_drivers: Dictionary = {}
static var _policy_providers: Dictionary = {}
# Non-zero joypad axes injected via input.joy_motion. Joypad axis state lives in
# the process-global Input singleton and is NOT cleared by release_held_actions
# or a scene reload, so a held trigger/stick would leak into the next warm
# scenario. Tracked statically (keyed "device:axis_index") so any runtime
# instance can zero exactly what was touched across the process.
static var _touched_joy_axes: Dictionary = {}
# Joypad buttons currently pressed via injected events. input.joy_button always
# releases within its own step, but policy.drive_tick may hold a button across
# ticks, so pressed buttons are tracked (keyed "device:button_index") for the
# same cross-scenario cleanup guarantee as _touched_joy_axes.
static var _touched_joy_buttons: Dictionary = {}
# Screen touch points currently held down via injected InputEventScreenTouch,
# keyed by int finger index -> {"position": Vector2}. Touch state is process-
# global like the joypad sets, so release_held_touches lifts any finger still
# down when a scenario ends or input.reset runs.
static var _touched_screen_points: Dictionary = {}
# Set once per process the first time injected touch runs, so the mouse-emulation
# informational note prints at most once.
static var _touch_emulation_note_emitted := false
# Joypad devices announced as connected via setup.joy_connect, which fires the
# engine's Input.joy_connection_changed signal so lobby connect/disconnect
# handlers run under headless QA. (On Godot 4.6.2 the roster-mutating method is
# not bound to GDScript, so Input.get_connected_joypads() is NOT faked; only the
# signal is fired — see _joy_connect.) This tracking set is process-global like
# _touched_joy_axes/_touched_joy_buttons: it survives a scene reload, so a fake
# left tracked would let joy_disconnect's real-pad guard mis-classify a device in
# the next warm scenario and would skip re-announcing it. Tracked statically
# (keyed by int device -> {device, name}) so any runtime instance can announce a
# matching disconnect for exactly what was faked across the process.
static var _faked_joypads: Dictionary = {}

var _scene_root: Node = null
var _root_window: Window = null
var _runtime_errors: Array = []
var _held_actions: Dictionary = {}
var _held_keys: Dictionary = {}
var _assertions: RefCounted = null

var _audio: RefCounted = QaRuntimeAudio.new(self)

var _fixtures: RefCounted = QaRuntimeFixtures.new(self)

var _input: RefCounted = QaRuntimeInput.new(self)

var _physics: RefCounted = QaRuntimePhysics.new(self)

var _pointer: RefCounted = QaRuntimePointer.new(self)

var _providers: RefCounted = QaRuntimeProviders.new(self)

var _shapes: RefCounted = QaRuntimeShapes.new(self)

var _trace: RefCounted = QaRuntimeTrace.new(self)


func _init(scene_root: Node, root_window: Window, runtime_errors := [], held_actions := {}, held_keys := {}) -> void:
	_scene_root = scene_root
	_root_window = root_window
	_runtime_errors = runtime_errors if runtime_errors is Array else []
	_held_actions = held_actions if held_actions is Dictionary else {}
	_held_keys = held_keys if held_keys is Dictionary else {}
	_assertions = QaAssertions.new(self, _root_window, _runtime_errors)


func execute(command_name: String, params := {}) -> Dictionary:
	var command_params: Dictionary = params if params is Dictionary else {}
	if _scene_root == null or _root_window == null:
		return _error(
			"scene_unavailable",
			"QA runtime scene is not ready",
			{
				"command": command_name,
			},
		)

	match command_name:
		"tree.dump":
			var snapshot_scope := _resolve_snapshot_filter(command_params)
			if not bool(snapshot_scope.get("ok", false)):
				return {"ok": false, "error": snapshot_scope.get("error", {})}
			return _ok(QaSnapshot.node_tree_to_dict(snapshot_scope.get("root"), snapshot_scope.get("options", {})))
		"ui.dump":
			return _ok(QaSnapshot.all_controls_to_dict(_scene_root))
		"focus.get":
			var visible_controls: Array[Control] = QaSnapshot.visible_controls(_scene_root)
			return _ok(QaSnapshot.focus_to_dict(_root_window, visible_controls))
		"focus.graph":
			var visible_controls: Array[Control] = QaSnapshot.visible_controls(_scene_root)
			return _ok(QaSnapshot.focus_to_dict(_root_window, visible_controls))
		"node.inspect":
			return _inspect_node(str(command_params.get("target", "")))
		"physics.inspect":
			return await _physics_inspect(command_params)
		"physics.overlay.collect":
			return await _physics_overlay_collect(command_params)
		"surface.audit_probe":
			return _surface_audit_probe(command_params)
		"visual.audit_targets":
			return _visual_audit_targets(command_params)
		"trace.record":
			return await _trace_record(command_params)
		"event.trace":
			return await _event_trace(command_params)
		"telemetry.record":
			return await _telemetry_record(command_params)
		"audio.capture":
			return await _audio_capture(command_params)
		"audio.export":
			return await _audio_export(command_params)
		"audio.render":
			return await _audio_render(command_params)
		"errors.read":
			return _ok(QaSnapshot.errors_to_dict(_runtime_errors))
		"input.tap_action":
			return await _tap_action(str(command_params.get("action", "")))
		"input.hold_action":
			return await _hold_action(str(command_params.get("action", "")), command_params.get("frames", 0))
		"input.release_action":
			return await _release_action(str(command_params.get("action", "")))
		"input.click_target":
			return await _click_target(str(command_params.get("target", "")))
		"input.mouse_click":
			return await _mouse_click(command_params.get("x", null), command_params.get("y", null))
		"input.mouse_wheel":
			return await _mouse_wheel(command_params)
		"input.touch_down":
			return await _touch_down(command_params)
		"input.touch_move":
			return await _touch_move(command_params)
		"input.touch_up":
			return await _touch_up(command_params)
		"input.touch_tap":
			return await _touch_tap(command_params)
		"input.drag":
			return await _drag(command_params)
		"input.key":
			return await _press_key(command_params)
		"input.key_hold":
			return await _hold_key(command_params)
		"input.key_release":
			return await _release_key(str(command_params.get("key", "")))
		"input.text":
			return await _type_text(command_params)
		"input.joy_button":
			return await _press_joy_button(command_params)
		"input.joy_motion":
			return await _joy_motion(command_params)
		"input.joy_state":
			return await _input_joy_state(command_params)
		"input.reset":
			var reset_failures: Array[Dictionary] = await reset_input()
			return _ok({
				"released_actions": reset_failures.is_empty(),
				"release_failures": reset_failures.duplicate(true),
			})
		"setup.teleport_node":
			return await _teleport_node(command_params)
		"fixture.teleport_to_target":
			return await _fixture_teleport_to_target(command_params)
		"setup.spawn_scene":
			return await _spawn_scene(command_params)
		"setup.joy_connect":
			return await _joy_connect(command_params)
		"setup.joy_disconnect":
			return await _joy_disconnect(command_params)
		"setup.set_property":
			return await _set_property(command_params)
		"setup.call_method":
			return await _call_method(command_params)
		"fixture.call":
			return await _fixture_call(command_params)
		"policy.drive_tick":
			return _policy_drive_tick(command_params)
		"policy.release_inputs":
			return await _policy_release_inputs(command_params)
		"qa_state.read":
			return await _assertions.execute(command_name, command_params)
		"state.record":
			return await _assertions.execute(command_name, command_params)
		"assert.exists", "assert.not_exists", "assert.visible", "assert.hidden", "assert.enabled", "assert.disabled", "assert.focused", "assert.text_equals", "assert.text_contains", "assert.inside_viewport", "assert.no_overlap", "assert.no_errors", "assert.distance", "assert.state_matches", "assert.observe_until", "assert.surface_probe", "assert.surface_matches", "assert.temporal.max_delta_position", "assert.temporal.follows", "assert.temporal.never", "assert.temporal.within_limits", "assert.temporal.trend", "assert.temporal.frame_time", "assert.temporal.distance_changed", "assert.visible_pixels", "assert.not_occluded", "assert.renders_above", "assert.renders_below":
			return await _assertions.execute(command_name, command_params)

	return _error(
		"unsupported_command",
		"QA bridge command is not supported in this milestone",
		{
			"command": command_name,
			"params": command_params.duplicate(true),
		},
	)


func _surface_audit_probe(params: Dictionary) -> Dictionary:
	return _assertions.surface_audit_probe(params)


func _visual_audit_targets(params: Dictionary) -> Dictionary:
	return _shapes._visual_audit_targets(params)


func find_target_matches(target: String) -> Array:
	var matches: Array = []
	if target.begins_with("qa:"):
		var qa_id := target.trim_prefix("qa:")
		var group_result := _collect_by_qa_groups(qa_id)
		matches = group_result.get("matches", [])
		if matches.is_empty() or not (group_result.get("rejected_candidates", []) as Array).is_empty():
			_collect_by_qa_id(_scene_root, qa_id, matches)
	elif target.begins_with("path:"):
		var node := _root_window.get_node_or_null(NodePath(target.trim_prefix("path:")))
		if node != null:
			matches.append(node)
	elif target.begins_with("name:"):
		_collect_by_name(_scene_root, target.trim_prefix("name:"), matches)
	return matches


func resolve_target(target: String) -> Dictionary:
	var matches: Array = find_target_matches(target)
	var qa_candidate_diagnostics := {}
	if target.begins_with("qa:"):
		var group_result := _collect_by_qa_groups(target.trim_prefix("qa:"))
		var rejected_candidates: Array = group_result.get("rejected_candidates", [])
		if not rejected_candidates.is_empty():
			qa_candidate_diagnostics = {
				"candidate_status": "rejected_group_candidates",
				"rejected_candidates": rejected_candidates,
			}

	if matches.size() > 1:
		return {
			"ok": false,
			"error": _error_payload(
				"ambiguous_target",
				"Multiple nodes matched the requested target",
				{
					"target": target,
					"expected": "unique runtime node",
					"actual": describe_matches(matches),
				},
			),
		}
	if matches.is_empty():
		var data := {
			"target": target,
			"expected": "existing runtime node",
			"actual": "missing",
		}
		for key in qa_candidate_diagnostics.keys():
			data[key] = qa_candidate_diagnostics[key]
		return {
			"ok": false,
			"error": _error_payload(
				"target_not_found",
				"No node matched the requested target",
				data,
			),
		}
	return {
		"ok": true,
		"node": matches[0],
		"resolved": "path:%s" % str((matches[0] as Node).get_path()),
	}


func resolve_snapshot_filter(params := {}) -> Dictionary:
	return _resolve_snapshot_filter(params if params is Dictionary else {})


func resolve_point_provider(point_request: Variant, offset: Variant = null) -> Dictionary:
	return _fixtures._resolve_setup_point_provider(point_request, offset)


func is_target_visible(node: Node) -> bool:
	if node is CanvasItem:
		return (node as CanvasItem).is_visible_in_tree()
	return false


func viewport_rect() -> Rect2:
	if _scene_root != null and _scene_root.get_viewport() != null:
		return _scene_root.get_viewport().get_visible_rect()
	if _root_window != null:
		return _root_window.get_visible_rect()
	return Rect2()


func ok(data := {}) -> Dictionary:
	return _ok(data)


func error(error_type: String, message: String, data := {}) -> Dictionary:
	return _error(error_type, message, data)


func process_frames(count: int) -> void:
	await _process_frames(count)


func _resolve_snapshot_filter(params: Dictionary) -> Dictionary:
	var validation := QaSnapshot.validate_filter_options(params)
	if not bool(validation.get("ok", false)):
		return validation
	var options: Dictionary = validation.get("options", {})
	var root := _scene_root
	var target := str(options.get("target", "")).strip_edges()
	if target != "":
		var resolution := resolve_target(target)
		if not bool(resolution.get("ok", false)):
			return {
				"ok": false,
				"error": (resolution.get("error", {}) as Dictionary).duplicate(true),
			}
		root = resolution.get("node") as Node
		options["resolved_target"] = str(resolution.get("resolved", ""))
	return {
		"ok": true,
		"root": root,
		"options": options,
	}


static func register_qa_id(node: Node, qa_id: String, aliases: Variant = null) -> bool:
	if node == null:
		return false
	var normalized_id := str(qa_id).strip_edges()
	if normalized_id == "":
		return false
	node.set_meta("qa_id", normalized_id)
	node.add_to_group("qa.id.%s" % normalized_id)
	node.add_to_group("qa.%s" % normalized_id)
	var alias_values: Array[String] = []
	if aliases is String:
		var alias_string := str(aliases).strip_edges()
		if alias_string != "":
			alias_values.append(alias_string)
	elif aliases is Array or aliases is PackedStringArray:
		for raw_alias in aliases:
			var alias := str(raw_alias).strip_edges()
			if alias != "" and not alias_values.has(alias):
				alias_values.append(alias)
	if not alias_values.is_empty():
		var existing_aliases: Array[String] = []
		if node.has_meta("qa_aliases"):
			var existing = node.get_meta("qa_aliases")
			if existing is String:
				var existing_alias := str(existing).strip_edges()
				if existing_alias != "":
					existing_aliases.append(existing_alias)
			elif existing is Array or existing is PackedStringArray:
				for raw_existing in existing:
					var existing_alias := str(raw_existing).strip_edges()
					if existing_alias != "" and not existing_aliases.has(existing_alias):
						existing_aliases.append(existing_alias)
		for alias in alias_values:
			if not existing_aliases.has(alias):
				existing_aliases.append(alias)
		node.set_meta("qa_aliases", PackedStringArray(existing_aliases))
	return true


static func register_state_provider(provider_id: String, owner: Object, method_name: String, metadata: Dictionary = {}) -> Dictionary:
	return QaRuntimeProviders._register_provider(_state_providers, "state_provider", provider_id, owner, method_name, metadata, 0, true)


static func unregister_state_provider(provider_id: String) -> Dictionary:
	return QaRuntimeProviders._unregister_provider(_state_providers, "state_provider", provider_id)


static func register_probe_provider(provider_id: String, owner: Object, method_name: String, metadata: Dictionary = {}) -> Dictionary:
	return QaRuntimeProviders._register_provider(_probe_providers, "probe_provider", provider_id, owner, method_name, metadata, 1, true)


static func unregister_probe_provider(provider_id: String) -> Dictionary:
	return QaRuntimeProviders._unregister_provider(_probe_providers, "probe_provider", provider_id)


static func register_fixture_driver(driver_id: String, owner: Object, method_name: String, metadata: Dictionary = {}) -> Dictionary:
	return QaRuntimeProviders._register_provider(_fixture_drivers, "fixture_driver", driver_id, owner, method_name, metadata, 1, false)


static func unregister_fixture_driver(driver_id: String) -> Dictionary:
	return QaRuntimeProviders._unregister_provider(_fixture_drivers, "fixture_driver", driver_id)


# Policy providers compute desired joypad inputs for drive_policy steps. They
# are read-only consumers: the method receives one request dictionary and must
# return {"inputs": [...]} without mutating game state, so mutating method
# names are rejected exactly like state providers.
static func register_policy_provider(provider_id: String, owner: Object, method_name: String, metadata: Dictionary = {}) -> Dictionary:
	return QaRuntimeProviders._register_provider(_policy_providers, "policy_provider", provider_id, owner, method_name, metadata, 1, true)


static func unregister_policy_provider(provider_id: String) -> Dictionary:
	return QaRuntimeProviders._unregister_provider(_policy_providers, "policy_provider", provider_id)


static func clear_provider_registry() -> void:
	_state_providers.clear()
	_probe_providers.clear()
	_fixture_drivers.clear()
	_policy_providers.clear()


# Release only this scene's registrations; autoload providers outlive swaps.
static func release_scene_providers(scene: Node) -> void:
	for registry in [_state_providers, _probe_providers, _fixture_drivers, _policy_providers]:
		for provider_id in registry.keys():
			var owner = registry[provider_id].get("owner")
			if not is_instance_valid(owner):
				registry.erase(provider_id)
			elif owner is Node and (owner == scene or scene.is_ancestor_of(owner)):
				registry.erase(provider_id)


static func list_state_providers() -> Array[Dictionary]:
	return QaRuntimeProviders._list_providers(_state_providers)


static func list_probe_providers() -> Array[Dictionary]:
	return QaRuntimeProviders._list_providers(_probe_providers)


static func list_fixture_drivers() -> Array[Dictionary]:
	return QaRuntimeProviders._list_providers(_fixture_drivers)


static func list_policy_providers() -> Array[Dictionary]:
	return QaRuntimeProviders._list_providers(_policy_providers)


# QA-aware replacement for Input.get_connected_joypads().
#
# A local-multiplayer lobby that POLLS the connected-pad roster (pad enumeration
# on menu open, "press A to join" slot counts) should read it through this helper
# so it works under BOTH real hardware and QA-faked connections. On Godot 4.6.2
# the engine roster cannot be faked from script (Input.joy_connection_changed is
# a signal, not a bound method — see _joy_connect), so this returns the sorted,
# deduped union of the real Input.get_connected_joypads() and the devices
# announced via the joy_connect step. With no fakes active it degrades to exactly
# the engine roster, so real-hardware behaviour is unchanged and a game can adopt
# it unconditionally.
#
# Opt-in is a one-line change at the single read site:
#   Input.get_connected_joypads()  ->  QaRuntime.effective_connected_joypads()
# mirroring how QaArgs.effective_user_args() shims OS.get_cmdline_user_args()
# for warm-session launch args.
static func effective_connected_joypads() -> Array[int]:
	var devices: Array[int] = []
	for device in Input.get_connected_joypads():
		if not devices.has(int(device)):
			devices.append(int(device))
	for device in _faked_joypads.keys():
		if not devices.has(int(device)):
			devices.append(int(device))
	devices.sort()
	return devices


func describe_node(node: Variant) -> String:
	if node == null:
		return "none"
	if node is Node:
		var locator := stable_locator_for_node(node as Node)
		if locator != "":
			return locator
	return str(node)


func read_state_provider(provider_id: String) -> Dictionary:
	return _providers.read_state_provider(provider_id)


func call_probe_provider(provider_id: String, request: Dictionary) -> Dictionary:
	return _providers.call_probe_provider(provider_id, request)


func describe_probe_provider(provider_id: String) -> Dictionary:
	return _providers.describe_probe_provider(provider_id)


func stable_locator_for_node(node: Node) -> String:
	if node == null:
		return ""
	if node.has_meta("qa_id"):
		var qa_id := str(node.get_meta("qa_id"))
		if qa_id != "":
			return "qa:%s" % qa_id
	var node_name := str(node.name)
	if node_name != "":
		var name_matches: Array = []
		_collect_by_name(_scene_root, node_name, name_matches)
		if name_matches.size() == 1 and name_matches[0] == node:
			return "name:%s" % node_name
	return "path:%s" % str(node.get_path())


func release_held_actions() -> Array[Dictionary]:
	return await _input.release_held_actions()


func release_held_keys() -> Array[Dictionary]:
	return await _input.release_held_keys()


func release_held_axes() -> Array[Dictionary]:
	return await _input.release_held_axes()


func release_held_buttons() -> Array[Dictionary]:
	return await _input.release_held_buttons()


func release_held_touches() -> Array[Dictionary]:
	return await _input.release_held_touches()


func release_faked_joypads() -> Array[Dictionary]:
	return await _input.release_faked_joypads()


func reset_input() -> Array[Dictionary]:
	return await _input.reset_input()


func teardown() -> void:
	if _assertions != null and _assertions.has_method("teardown"):
		_assertions.call("teardown")
	_assertions = null
	_scene_root = null
	_root_window = null
	_runtime_errors = []
	_held_actions = {}
	_held_keys = {}


func _tap_action(action: String) -> Dictionary:
	return await _input._tap_action(action)


func _hold_action(action: String, frames: Variant) -> Dictionary:
	return await _input._hold_action(action, frames)


func _release_action(action: String) -> Dictionary:
	return await _input._release_action(action)


func _click_target(target: String) -> Dictionary:
	return await _pointer._click_target(target)


func _inspect_node(target: String) -> Dictionary:
	var resolution := resolve_target(target)
	if not bool(resolution.get("ok", false)):
		return {
			"ok": false,
			"error": (resolution.get("error", {}) as Dictionary).duplicate(true),
		}

	var node := resolution.get("node") as Node
	var node_payload := QaSnapshot.node_to_dict(node)
	node_payload["locator"] = stable_locator_for_node(node)
	node_payload["script_path"] = _script_path_for_node(node)

	var parent := node.get_parent()
	if parent != null:
		node_payload["parent"] = QaSnapshot.node_summary_to_dict(parent, stable_locator_for_node(parent))
	else:
		node_payload["parent"] = {}

	var children: Array[Dictionary] = []
	for child in node.get_children():
		var child_node := child as Node
		if child_node == null:
			continue
		children.append(QaSnapshot.node_summary_to_dict(child_node, stable_locator_for_node(child_node)))
	node_payload["children"] = children

	if node is Control:
		node_payload["control"] = QaSnapshot.control_state_to_dict(node as Control)

	return _ok(
		{
			"target": target,
			"resolved": resolution.get("resolved", ""),
			"node": node_payload,
		}
	)


func _audio_capture(params: Dictionary) -> Dictionary:
	return await _audio._audio_capture(params)


func _stop_after_state_params(params: Dictionary) -> Dictionary:
	var raw = params.get("stop_after_state", {})
	if not (raw is Dictionary):
		return {}
	var stop_after_state: Dictionary = (raw as Dictionary).duplicate(true)
	if stop_after_state.is_empty():
		return {}
	if not stop_after_state.has("source"):
		stop_after_state["source"] = "method"
	if not stop_after_state.has("sample_every_frames"):
		stop_after_state["sample_every_frames"] = 1
	return stop_after_state


func _audio_export(params: Dictionary) -> Dictionary:
	return await _audio._audio_export(params)


func _audio_render(params: Dictionary) -> Dictionary:
	return await _audio._audio_render(params)


func _trace_record(params: Dictionary) -> Dictionary:
	return await _trace._trace_record(params)


func _event_trace(params: Dictionary) -> Dictionary:
	return await _trace._event_trace(params)


func _telemetry_record(params: Dictionary) -> Dictionary:
	return await _trace._telemetry_record(params)


func _physics_inspect(params: Dictionary) -> Dictionary:
	return await _physics._physics_inspect(params)


func _physics_overlay_collect(params: Dictionary) -> Dictionary:
	return await _physics._physics_overlay_collect(params)


func _method_has_zero_required_arguments(provider: Node, method: String) -> bool:
	for method_info in provider.get_method_list():
		if not method_info is Dictionary:
			continue
		var info: Dictionary = method_info
		if str(info.get("name", "")) != method:
			continue
		var args: Array = info.get("args", []) if info.get("args", []) is Array else []
		var default_args: Array = info.get("default_args", []) if info.get("default_args", []) is Array else []
		return args.size() - default_args.size() <= 0
	return true


func _json_compatible_value(value: Variant) -> Variant:
	match typeof(value):
		TYPE_NIL, TYPE_BOOL, TYPE_INT, TYPE_FLOAT, TYPE_STRING:
			return value
		TYPE_VECTOR2:
			return _shapes._vector2_payload(value)
		TYPE_VECTOR2I:
			var vector2i: Vector2i = value
			return {"x": vector2i.x, "y": vector2i.y}
		TYPE_VECTOR3:
			var vector3: Vector3 = value
			return {"x": vector3.x, "y": vector3.y, "z": vector3.z, "length": vector3.length()}
		TYPE_RECT2:
			return _shapes._rect2_payload(value)
		TYPE_ARRAY:
			var array_payload: Array = []
			for item in value:
				array_payload.append(_json_compatible_value(item))
			return array_payload
		TYPE_DICTIONARY:
			var dictionary_payload := {}
			var dictionary: Dictionary = value
			for key in dictionary.keys():
				dictionary_payload[str(key)] = _json_compatible_value(dictionary[key])
			return dictionary_payload
		_:
			return str(value)


func _is_json_compatible_project_state_value(value: Variant) -> bool:
	match typeof(value):
		TYPE_NIL, TYPE_BOOL, TYPE_INT, TYPE_FLOAT, TYPE_STRING, TYPE_VECTOR2, TYPE_VECTOR2I, TYPE_VECTOR3, TYPE_RECT2:
			return true
		TYPE_ARRAY:
			for item in value:
				if not _is_json_compatible_project_state_value(item):
					return false
			return true
		TYPE_DICTIONARY:
			var dictionary: Dictionary = value
			for key in dictionary.keys():
				if not _is_json_compatible_project_state_value(dictionary[key]):
					return false
			return true
		_:
			return false


func _script_path_for_node(node: Node) -> String:
	var script = node.get_script()
	if script == null or not script is Script:
		return ""
	return str((script as Script).resource_path)


func _teleport_node(params: Dictionary) -> Dictionary:
	return await _fixtures._teleport_node(params)


func _fixture_teleport_to_target(params: Dictionary) -> Dictionary:
	return await _fixtures._fixture_teleport_to_target(params)


func _spawn_scene(params: Dictionary) -> Dictionary:
	return await _fixtures._spawn_scene(params)


func _set_property(params: Dictionary) -> Dictionary:
	return await _fixtures._set_property(params)


func _call_method(params: Dictionary) -> Dictionary:
	return await _fixtures._call_method(params)


func _fixture_call(params: Dictionary) -> Dictionary:
	return await _fixtures._fixture_call(params)


func _policy_drive_tick(params: Dictionary) -> Dictionary:
	return _input._policy_drive_tick(params)


func _policy_release_inputs(params: Dictionary) -> Dictionary:
	return await _input._policy_release_inputs(params)


func _is_number(value: Variant) -> bool:
	return typeof(value) == TYPE_INT or typeof(value) == TYPE_FLOAT


func _mouse_click(raw_x: Variant, raw_y: Variant) -> Dictionary:
	return await _pointer._mouse_click(raw_x, raw_y)


func _mouse_wheel(params: Dictionary) -> Dictionary:
	return await _pointer._mouse_wheel(params)


func _touch_down(params: Dictionary) -> Dictionary:
	return await _pointer._touch_down(params)


func _touch_move(params: Dictionary) -> Dictionary:
	return await _pointer._touch_move(params)


func _touch_up(params: Dictionary) -> Dictionary:
	return await _pointer._touch_up(params)


func _touch_tap(params: Dictionary) -> Dictionary:
	return await _pointer._touch_tap(params)


func _drag(params: Dictionary) -> Dictionary:
	return await _pointer._drag(params)


func _press_key(params: Dictionary) -> Dictionary:
	return await _input._press_key(params)


func _hold_key(params: Dictionary) -> Dictionary:
	return await _input._hold_key(params)


func _release_key(raw_key: String) -> Dictionary:
	return await _input._release_key(raw_key)


func _type_text(params: Dictionary) -> Dictionary:
	return await _input._type_text(params)


func _press_joy_button(params: Dictionary) -> Dictionary:
	return await _input._press_joy_button(params)


func _joy_motion(params: Dictionary) -> Dictionary:
	return await _input._joy_motion(params)


func _input_joy_state(params: Dictionary) -> Dictionary:
	return await _input._input_joy_state(params)


func _joy_connect(params: Dictionary) -> Dictionary:
	return await _input._joy_connect(params)


func _joy_disconnect(params: Dictionary) -> Dictionary:
	return await _input._joy_disconnect(params)


func _process_frames(count: int) -> void:
	for _i in range(max(count, 0)):
		await _root_window.get_tree().process_frame


func _collect_by_qa_id(node: Node, qa_id: String, matches: Array) -> void:
	if node == null:
		return
	if node.has_meta("qa_id") and str(node.get_meta("qa_id")) == qa_id and not _matches_has_node(matches, node):
		matches.append(node)
	if _node_has_qa_alias(node, qa_id) and not _matches_has_node(matches, node):
		matches.append(node)
	for group_name in ["qa.id.%s" % qa_id, "qa.%s" % qa_id]:
		if node.is_in_group(group_name) and _node_matches_qa_id(node, qa_id) and not _matches_has_node(
			matches,
			node,
		):
			matches.append(node)
	for child in node.get_children():
		var child_node := child as Node
		if child_node == null:
			continue
		_collect_by_qa_id(child_node, qa_id, matches)


func _collect_by_qa_groups(qa_id: String) -> Dictionary:
	var matches: Array = []
	var rejected_candidates: Array[Dictionary] = []
	if _root_window == null or _root_window.get_tree() == null:
		return {
			"matches": matches,
			"rejected_candidates": rejected_candidates,
		}
	var seen := {}
	for group_name in ["qa.id.%s" % qa_id, "qa.%s" % qa_id]:
		for candidate in _root_window.get_tree().get_nodes_in_group(group_name):
			var instance_id := _candidate_instance_id(candidate)
			var seen_key := "%s:%s" % [group_name, str(instance_id)]
			if seen.has(seen_key):
				continue
			seen[seen_key] = true
			var node := candidate as Node
			var validation := _validate_qa_group_candidate(node, candidate, qa_id, group_name)
			if bool(validation.get("ok", false)):
				if not _matches_has_node(matches, node):
					matches.append(node)
			else:
				rejected_candidates.append(validation.get("candidate", {}) as Dictionary)
	return {
		"matches": matches,
		"rejected_candidates": rejected_candidates,
	}


func _validate_qa_group_candidate(node: Node, candidate: Variant, qa_id: String, group_name: String) -> Dictionary:
	var candidate_data := {
		"group": group_name,
		"expected": "qa:%s" % qa_id,
	}
	if candidate == null:
		candidate_data["status"] = "freed"
		candidate_data["actual"] = "null"
		return {"ok": false, "candidate": candidate_data}
	if node == null or not is_instance_valid(candidate):
		candidate_data["status"] = "stale"
		candidate_data["actual"] = type_string(typeof(candidate))
		return {"ok": false, "candidate": candidate_data}
	candidate_data["path"] = str(node.get_path()) if node.is_inside_tree() else str(node.name)
	candidate_data["type"] = node.get_class()
	if not node.is_inside_tree():
		candidate_data["status"] = "outside_tree"
		candidate_data["actual"] = "outside_tree"
		return {"ok": false, "candidate": candidate_data}
	if _scene_root != null and node != _scene_root and not _scene_root.is_ancestor_of(node):
		candidate_data["status"] = "outside_scene_root"
		candidate_data["actual"] = "outside_scene_root"
		return {"ok": false, "candidate": candidate_data}
	if not _node_matches_qa_id(node, qa_id):
		candidate_data["status"] = "mismatched_qa_id"
		candidate_data["actual"] = {
			"qa_id": str(node.get_meta("qa_id", "")),
			"qa_aliases": _qa_aliases_to_array(node),
		}
		return {"ok": false, "candidate": candidate_data}
	return {"ok": true, "candidate": candidate_data}


func _node_matches_qa_id(node: Node, qa_id: String) -> bool:
	if node == null:
		return false
	if node.has_meta("qa_id") and str(node.get_meta("qa_id")) == qa_id:
		return true
	if _node_has_qa_alias(node, qa_id):
		return true
	var has_qa_id := node.has_meta("qa_id") and str(node.get_meta("qa_id")) != ""
	var has_qa_aliases := node.has_meta("qa_aliases") and not _qa_aliases_to_array(node).is_empty()
	return not has_qa_id and not has_qa_aliases


func _qa_aliases_to_array(node: Node) -> Array[String]:
	var result: Array[String] = []
	if node == null or not node.has_meta("qa_aliases"):
		return result
	var aliases = node.get_meta("qa_aliases")
	if aliases is String:
		var alias := str(aliases)
		if alias != "":
			result.append(alias)
	elif aliases is Array or aliases is PackedStringArray:
		for raw_alias in aliases:
			var alias := str(raw_alias)
			if alias != "" and not result.has(alias):
				result.append(alias)
	return result


func _candidate_instance_id(candidate: Variant) -> int:
	if candidate is Object and is_instance_valid(candidate):
		return (candidate as Object).get_instance_id()
	return 0


func _matches_has_node(matches: Array, node: Node) -> bool:
	if node == null:
		return false
	var instance_id := node.get_instance_id()
	for match in matches:
		if match is Node and (match as Node).get_instance_id() == instance_id:
			return true
	return false


func _node_has_qa_alias(node: Node, qa_id: String) -> bool:
	if node == null or not node.has_meta("qa_aliases"):
		return false
	var aliases = node.get_meta("qa_aliases")
	var wanted_prefixed := "qa:%s" % qa_id
	if aliases is String:
		var alias := str(aliases)
		return alias == qa_id or alias == wanted_prefixed
	if aliases is Array or aliases is PackedStringArray:
		for raw_alias in aliases:
			var alias := str(raw_alias)
			if alias == qa_id or alias == wanted_prefixed:
				return true
	return false


func _collect_by_name(node: Node, target_name: String, matches: Array) -> void:
	if node == null:
		return
	if str(node.name) == target_name:
		matches.append(node)
	for child in node.get_children():
		var child_node := child as Node
		if child_node == null:
			continue
		_collect_by_name(child_node, target_name, matches)


func describe_matches(matches: Array) -> Array[String]:
	var described: Array[String] = []
	for match in matches:
		if match is Node:
			described.append("path:%s" % str((match as Node).get_path()))
		else:
			described.append(str(match))
	return described


func _coerce_frame_count(value: Variant) -> Variant:
	match typeof(value):
		TYPE_INT:
			return null if int(value) < 0 else int(value)
		TYPE_FLOAT:
			var number := float(value)
			if number < 0.0 or floor(number) != number:
				return null
			return int(number)
	return null


func _coerce_coordinate(value: Variant) -> Variant:
	match typeof(value):
		TYPE_INT:
			return int(value)
		TYPE_FLOAT:
			var numeric_value := float(value)
			if floor(numeric_value) != numeric_value:
				return null
			return int(numeric_value)
	return null


func _coerce_number_coordinate(value: Variant) -> Variant:
	match typeof(value):
		TYPE_INT:
			return int(value)
		TYPE_FLOAT:
			return float(value)
	return null


func _rect_to_array(value: Rect2) -> Array[float]:
	return [value.position.x, value.position.y, value.size.x, value.size.y]


func _non_negative_int_param(params: Dictionary, field: String, default_value: int, command: String) -> Dictionary:
	var raw_value = params.get(field, default_value)
	if not (raw_value is int or raw_value is float):
		return _error(
			"invalid_params",
			"%s %s must be a non-negative integer" % [command, field],
			{
				"command": command,
				"field": field,
				"expected": "non-negative integer",
				"actual": raw_value,
			},
		)
	var value := int(raw_value)
	if float(value) != float(raw_value) or value < 0:
		return _error(
			"invalid_params",
			"%s %s must be a non-negative integer" % [command, field],
			{
				"command": command,
				"field": field,
				"expected": "non-negative integer",
				"actual": raw_value,
			},
		)
	return {"ok": true, "value": value}


func _append_focus_owner(data: Dictionary) -> void:
	if _root_window == null:
		return
	var focus_owner = _root_window.gui_get_focus_owner()
	if focus_owner == null:
		return
	data["focus_owner"] = stable_locator_for_node(focus_owner)
	data["focus_owner_path"] = str(focus_owner.get_path())


func _action_missing_error(action: String) -> Dictionary:
	return _error(
		"input_action_missing",
		"Input action is not defined in InputMap",
		{
			"action": action,
			"expected": "defined InputMap action",
			"actual": "missing",
		},
	)


func _ok(data := {}) -> Dictionary:
	return {
		"ok": true,
		"data": data if data is Dictionary else {},
	}


func _error(error_type: String, message: String, data := {}) -> Dictionary:
	return {
		"ok": false,
		"error": _error_payload(error_type, message, data),
	}


func _error_payload(error_type: String, message: String, data := {}) -> Dictionary:
	return {
		"type": error_type,
		"message": message,
		"data": data if data is Dictionary else {},
	}


func process_bounded_frames(count: int) -> void:
	await _process_frames(max(count, 0))


func global_position_for_node(node: Node) -> Dictionary:
	return _fixtures._global_position_for_node(node)


func rotation_for_node(node: Node) -> Variant:
	if node is Node2D:
		return (node as Node2D).global_rotation
	if node is Node3D:
		return (node as Node3D).global_rotation
	if node is Control:
		return (node as Control).rotation
	return null


func velocity_for_node(node: Node) -> Variant:
	if node is CharacterBody2D:
		return (node as CharacterBody2D).velocity.length()
	if node is CharacterBody3D:
		return (node as CharacterBody3D).velocity.length()
	if node is RigidBody2D:
		return (node as RigidBody2D).linear_velocity.length()
	if node is RigidBody3D:
		return (node as RigidBody3D).linear_velocity.length()
	return null


func read_node_field(node: Node, field: String) -> Variant:
	if node == null:
		return null
	if field == "text" and node is Control and _fixtures._object_has_property(node, "text"):
		return str(node.get("text"))
	if field == "position":
		return _fixtures._global_position_for_node(node)
	if field == "velocity":
		return velocity_for_node(node)
	if field == "rotation":
		return rotation_for_node(node)
	if _fixtures._object_has_property(node, field):
		return node.get(field)
	return null


func capture_temporal_sample(target: String, field: String) -> Dictionary:
	return _trace.capture_temporal_sample(target, field)


func sample_temporal_window(target: String, field: String, window: Dictionary, sample_every_frames: int) -> Array:
	return await _trace.sample_temporal_window(target, field, window, sample_every_frames)


func write_temporal_evidence(kind: String, target: String, payload: Dictionary) -> Dictionary:
	return _trace.write_temporal_evidence(kind, target, payload)
